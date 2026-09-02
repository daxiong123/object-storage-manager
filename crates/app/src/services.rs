//! 应用服务组装：AccountService（SQLite + Keychain）与 tokio 运行时的组装层。
//!
//! 为什么需要本层（agents.md §5「AppServices 组装」）：gpui 的执行器不提供
//! tokio reactor，而 reqwest/hyper 的 async IO 需要 tokio 上下文。因此 provider
//! 的异步调用统一由 [`AppServices`] 在**同步阻塞方法内部**用 `runtime.block_on`
//! 驱动，UI 只需把整个调用丢进 gpui 后台线程（`background_executor().spawn`），
//! 永不接触 provider、凭证与运行时细节。
//!
//! 红线：本层方法全部阻塞（含网络 IO），**只能在后台线程调用**，不得在 gpui
//! 主线程/UI 渲染路径上直接调用。

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use object_storage_core::{StorageError, StorageProvider};
use object_storage_domain::{Account, Bucket, ListObjectsRequest, ObjectPage, ProviderKind};
use object_storage_persistence::default_db_path;
use tokio::runtime::Runtime;

use crate::account_service::{AccountError, AccountService};

/// AppServices 的错误：账号编排错误（SQLite/Keychain/输入校验）、provider API
/// 错误、运行时创建失败，统一中文化展示给 UI。
#[derive(Debug, thiserror::Error)]
pub enum AppServicesError {
    #[error(transparent)]
    Account(#[from] AccountError),
    #[error(transparent)]
    Provider(#[from] StorageError),
    #[error("创建异步运行时失败：{0}")]
    Runtime(String),
}

/// 组装好的应用服务。UI 以 `Arc<AppServices>` 共享，每个后台任务克隆一份 Arc。
///
/// 线程模型：`AccountService` 内含 rusqlite `Connection`（内部 RefCell，非 Sync），
/// 而 gpui 后台任务要求 `Send + Sync` 的捕获值，故连接统一收进 `Mutex`——
/// 同一时刻至多一个后台线程使用数据库/钥匙串，锁毒化（持锁线程 panic）时
/// 直接 panic 响报，绝不静默吞掉（Fail Fast）。
pub struct AppServices {
    accounts: Mutex<AccountService>,
    runtime: Runtime,
}

impl AppServices {
    /// 打开默认位置（`~/Library/Application Support/CloudStorage/database.sqlite`）
    /// 的数据库并创建 tokio 运行时。失败即报错（Fail Fast），由调用方决定退出。
    pub fn open() -> Result<Self, AppServicesError> {
        Self::open_at(default_db_path().map_err(AccountError::from)?)
    }

    /// 指定数据库路径打开（测试与自定义数据目录用）。
    pub fn open_at(db_path: impl AsRef<Path>) -> Result<Self, AppServicesError> {
        Ok(Self {
            accounts: Mutex::new(AccountService::open(db_path)?),
            runtime: Runtime::new().map_err(|e| AppServicesError::Runtime(e.to_string()))?,
        })
    }

    /// 锁定账号服务。毒化即 panic：说明有线程在持锁期间 panic，状态不可信。
    fn lock_accounts(&self) -> MutexGuard<'_, AccountService> {
        self.accounts
            .lock()
            .expect("AccountService 锁已毒化：持锁线程曾 panic，数据库状态不可信")
    }

    /// 全部账号元数据（不含 Secret）。
    pub fn list_accounts(&self) -> Result<Vec<Account>, AppServicesError> {
        Ok(self.lock_accounts().list()?)
    }

    /// 添加七牛账号（V1 固定 ProviderKind::Qiniu；阿里云由 AccountService 拒绝并
    /// 给出中文错误）。Secret 只入 Keychain，元数据只入 SQLite，编排见 AccountService。
    pub fn add_qiniu_account(
        &self,
        name: &str,
        access_key: &str,
        secret_key: &str,
    ) -> Result<Account, AppServicesError> {
        Ok(self
            .lock_accounts()
            .add(name, ProviderKind::Qiniu, access_key, secret_key)?)
    }

    /// 列举某账号的全部 Bucket（Keychain 取 SK → 构建 provider → 请求）。
    pub fn list_buckets(&self, account_id: &str) -> Result<Vec<Bucket>, AppServicesError> {
        let (_, provider) = self.lock_accounts().build_provider(account_id)?;
        Ok(self.runtime.block_on(provider.list_buckets())?)
    }

    /// 列举 Bucket 内对象（单页；翻页由调用方以 `ObjectPage::next_marker` 驱动）。
    pub fn list_objects(
        &self,
        account_id: &str,
        request: ListObjectsRequest,
    ) -> Result<ObjectPage, AppServicesError> {
        let (_, provider) = self.lock_accounts().build_provider(account_id)?;
        Ok(self.runtime.block_on(provider.list_objects(request))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_db(tag: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "cloudstorage-services-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        (dir.join("database.sqlite"), dir)
    }

    #[test]
    fn open_at_lists_empty_and_rejects_unknown_account() {
        let (db, dir) = temp_db("openat");
        let services = AppServices::open_at(&db).unwrap();

        assert!(services.list_accounts().unwrap().is_empty());

        // 未知账号：NotFound 必须显式报错，而不是返回空数据
        let err = services.list_buckets("no-such-account").unwrap_err();
        assert!(
            matches!(err, AppServicesError::Account(AccountError::NotFound(_))),
            "未知账号应报 NotFound，实际 {err:?}"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn add_qiniu_account_visible_in_list() {
        let (db, dir) = temp_db("add");
        let services = AppServices::open_at(&db).unwrap();

        let account = services
            .add_qiniu_account("测试账号", "ak-services", "sk-services")
            .unwrap();
        let listed = services.list_accounts().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, account.id);
        assert_eq!(listed[0].provider, ProviderKind::Qiniu);

        // 测试写入了真实 Keychain，自清理（失败时残留由 account_service 测试注释说明）
        services.lock_accounts().delete(&account.id).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }
}

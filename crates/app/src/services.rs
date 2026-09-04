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

use crate::provider::BuiltProvider;
use object_storage_core::StorageError;
use object_storage_domain::{Account, Bucket, ListObjectsRequest, ObjectPage, ProviderKind};
use object_storage_persistence::{PersistedTransfer, default_db_path};
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
    /// rename 的复合失败（新对象已建成但旧对象未删等中间态）
    #[error("{0}")]
    Rename(String),
}

/// 会话内 Secret 缓存条目：仅保留最近使用账号的一条 SK。
/// 不落盘、不进日志；切换账号（另一个 id 首次使用）时置换淘汰。
struct CachedSecret {
    account_id: String,
    secret_key: String,
}

/// 组装好的应用服务。UI 以 `Arc<AppServices>` 共享，每个后台任务克隆一份 Arc。
///
/// 线程模型：`AccountService` 内含 rusqlite `Connection`（内部 RefCell，非 Sync），
/// 而 gpui 后台任务要求 `Send + Sync` 的捕获值，故连接统一收进 `Mutex`——
/// 同一时刻至多一个后台线程使用数据库/钥匙串，锁毒化（持锁线程 panic）时
/// 直接 panic 响报，绝不静默吞掉（Fail Fast）。
///
/// 钥匙串授权体验：provider 构建优先用会话缓存里的 SK（`cached_secret`），
/// 只有「选中账号后的第一次操作」才会触碰钥匙串（可能弹授权）；后续切换空间/
/// 翻页/下载全部走缓存，不再弹窗。缓存锁与账号锁永不嵌套获取。
pub struct AppServices {
    accounts: Mutex<AccountService>,
    runtime: Runtime,
    /// 单条 SK 会话缓存（设计见类型注释与本方法族文档）
    cached_secret: Mutex<Option<CachedSecret>>,
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
            cached_secret: Mutex::new(None),
        })
    }

    /// 锁定账号服务。毒化即 panic：说明有线程在持锁期间 panic，状态不可信。
    fn lock_accounts(&self) -> MutexGuard<'_, AccountService> {
        self.accounts
            .lock()
            .expect("AccountService 锁已毒化：持锁线程曾 panic，数据库状态不可信")
    }

    /// 锁定 SK 缓存。毒化即 panic（同上）。
    fn lock_cached_secret(&self) -> MutexGuard<'_, Option<CachedSecret>> {
        self.cached_secret
            .lock()
            .expect("Secret 缓存锁已毒化：持锁线程曾 panic")
    }

    /// 构建可用的 Provider：优先用会话缓存的 SK（不碰钥匙串）；未命中才现取
    /// 钥匙串（唯一可能弹授权的位置：选中账号后的第一次操作）并写入缓存。
    ///
    /// 缓存淘汰：单条置换——另一个账号首次使用即覆盖旧 SK。账号被删除后缓存
    /// 条目可能仍在内存，但任何使用都会因元数据缺失而 NotFound（不会静默复活）。
    ///
    /// pub 语义：Transfer Engine 的任务执行体（UI 层注入的 runner）在异步任务
    /// 内调用它取 provider，拿到后立即 drop 返回值里的锁、不保留引用。
    pub fn build_provider(
        &self,
        account_id: &str,
    ) -> Result<(Account, BuiltProvider), AppServicesError> {
        // 1) 缓存命中：锁即取即放（永不与账号锁嵌套）
        let cached = {
            let cache = self.lock_cached_secret();
            cache
                .as_ref()
                .filter(|c| c.account_id == account_id)
                .map(|c| c.secret_key.clone())
        };
        if let Some(secret) = cached {
            return Ok(self
                .lock_accounts()
                .build_provider_with_secret(account_id, secret)?);
        }

        // 2) 未命中：SQLite 校验 + 钥匙串现取（可能弹授权，每账号每会话至多一次）
        let secret = self.lock_accounts().load_secret(account_id)?;
        let result = self
            .lock_accounts()
            .build_provider_with_secret(account_id, secret.clone())?;

        // 3) 写缓存（单条置换）
        *self.lock_cached_secret() = Some(CachedSecret {
            account_id: account_id.to_string(),
            secret_key: secret,
        });
        Ok(result)
    }

    /// 全部账号元数据（不含 Secret）。
    pub fn list_accounts(&self) -> Result<Vec<Account>, AppServicesError> {
        Ok(self.lock_accounts().list()?)
    }

    /// 添加七牛账号。Secret 只入 Keychain，元数据只入 SQLite。
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

    /// 添加阿里云 OSS 账号。Secret 只入 Keychain，元数据只入 SQLite。
    pub fn add_aliyun_account(
        &self,
        name: &str,
        access_key: &str,
        secret_key: &str,
    ) -> Result<Account, AppServicesError> {
        Ok(self
            .lock_accounts()
            .add(name, ProviderKind::Aliyun, access_key, secret_key)?)
    }

    /// 列举某账号的全部 Bucket（首次触碰钥匙串可能弹授权；之后走会话缓存）。
    pub fn list_buckets(&self, account_id: &str) -> Result<Vec<Bucket>, AppServicesError> {
        let (_, provider) = self.build_provider(account_id)?;
        Ok(self.runtime.block_on(provider.list_buckets())?)
    }

    /// 列举 Bucket 内对象（单页；翻页由调用方以 `ObjectPage::next_marker` 驱动）。
    pub fn list_objects(
        &self,
        account_id: &str,
        request: ListObjectsRequest,
    ) -> Result<ObjectPage, AppServicesError> {
        let (_, provider) = self.build_provider(account_id)?;
        Ok(self.runtime.block_on(provider.list_objects(request))?)
    }

    /// 下载对象到本地文件，返回写入的字节数。
    ///
    /// 注意：先 `build_provider`（拿完 provider 即释放数据库锁），再阻塞在
    /// 下载上——长下载绝不持有 `Mutex<AccountService>`，否则会卡住所有
    /// 其它后台任务（翻页/账号管理）。
    pub fn download_object(
        &self,
        account_id: &str,
        bucket: &str,
        key: &str,
        dest: &Path,
    ) -> Result<u64, AppServicesError> {
        let (_, provider) = self.build_provider(account_id)?;
        Ok(self
            .runtime
            .block_on(provider.download_object_to_file(bucket, key, dest, None))?)
    }

    /// 上传本地文件到远端对象，返回上传字节数。
    pub fn upload_object(
        &self,
        account_id: &str,
        bucket: &str,
        key: &str,
        source: &Path,
    ) -> Result<u64, AppServicesError> {
        let (_, provider) = self.build_provider(account_id)?;
        Ok(self
            .runtime
            .block_on(provider.upload_object_from_file(bucket, key, source, None))?)
    }

    /// 删除远端对象。对象不存在由 provider 报错，不静默当成功。
    pub fn delete_object(
        &self,
        account_id: &str,
        bucket: &str,
        key: &str,
    ) -> Result<(), AppServicesError> {
        let (_, provider) = self.build_provider(account_id)?;
        Ok(self.runtime.block_on(provider.delete_object(bucket, key))?)
    }

    /// 生成对象签名 GET URL。返回值含 token，不得写入日志。
    pub fn signed_get_url(
        &self,
        account_id: &str,
        bucket: &str,
        key: &str,
        ttl_secs: u64,
    ) -> Result<String, AppServicesError> {
        let (_, provider) = self.build_provider(account_id)?;
        Ok(self
            .runtime
            .block_on(provider.signed_get_url(bucket, key, ttl_secs))?)
    }

    /// 复制对象（当前 Bucket 内）：下载到临时文件 → 上传到目标 key。
    /// 上传失败时源对象保持不变；临时文件清理失败会进入错误信息，避免静默。
    pub fn copy_object(
        &self,
        account_id: &str,
        bucket: &str,
        source_key: &str,
        target_key: &str,
    ) -> Result<(), AppServicesError> {
        if source_key == target_key {
            return Err(StorageError::InvalidInput("目标路径与源路径相同".into()).into());
        }
        let (_, provider) = self.build_provider(account_id)?;
        let tmp = std::env::temp_dir().join(format!(
            "cloudstorage-copy-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        self.runtime
            .block_on(provider.download_object_to_file(bucket, source_key, &tmp, None))?;
        let upload_result = self
            .runtime
            .block_on(provider.upload_object_from_file(bucket, target_key, &tmp, None));
        let cleanup = std::fs::remove_file(&tmp);
        if let Err(error) = upload_result {
            return Err(AppServicesError::Rename(format!(
                "上传目标对象失败（源对象保留）：{error}；临时文件清理：{}",
                match cleanup {
                    Ok(()) => "已完成".to_string(),
                    Err(e) => format!("失败 {e}（残留：{}）", tmp.display()),
                }
            )));
        }
        if let Err(e) = cleanup {
            return Err(AppServicesError::Rename(format!(
                "复制已完成，但临时文件清理失败：{e}（残留：{}）",
                tmp.display()
            )));
        }
        Ok(())
    }

    /// 移动对象（当前 Bucket 内）：复制成功后删除源对象。
    /// 删除源失败时目标对象已生成，必须返回复合错误，不能静默。
    pub fn move_object(
        &self,
        account_id: &str,
        bucket: &str,
        source_key: &str,
        target_key: &str,
    ) -> Result<(), AppServicesError> {
        self.copy_object(account_id, bucket, source_key, target_key)?;
        let (_, provider) = self.build_provider(account_id)?;
        if let Err(error) = self
            .runtime
            .block_on(provider.delete_object(bucket, source_key))
        {
            return Err(AppServicesError::Rename(format!(
                "目标对象已生成，但删除源对象失败（远端同时存在 {source_key} 与 {target_key}）：{error}"
            )));
        }
        Ok(())
    }

    /// 远端重命名（云端无原子 rename）：下载到临时文件 → 上传新 key →
    /// 删旧 key。失败语义：
    /// - 下载/上传失败：旧对象完好，报错即止（不删旧）；
    /// - 删旧失败：新对象已建成，报复合错误（不静默，用户可手动清理）。
    ///
    /// 临时文件名以进程 + 纳秒命名，落系统临时目录（非 Secret，可留痕）。
    pub fn rename_object(
        &self,
        account_id: &str,
        bucket: &str,
        old_key: &str,
        new_key: &str,
    ) -> Result<(), AppServicesError> {
        if old_key == new_key {
            return Err(StorageError::InvalidInput("新名称与原名称相同".into()).into());
        }
        let (_, provider) = self.build_provider(account_id)?;
        let tmp = std::env::temp_dir().join(format!(
            "cloudstorage-rename-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        self.runtime
            .block_on(provider.download_object_to_file(bucket, old_key, &tmp, None))?;
        let upload_result = self
            .runtime
            .block_on(provider.upload_object_from_file(bucket, new_key, &tmp, None));
        if let Err(error) = upload_result {
            let cleanup = std::fs::remove_file(&tmp);
            return Err(AppServicesError::Rename(format!(
                "上传新名称失败（旧对象保留）：{error}；临时文件清理：{}",
                match cleanup {
                    Ok(()) => "已完成".to_string(),
                    Err(e) => format!("失败 {e}（残留：{}）", tmp.display()),
                }
            )));
        }
        let cleanup = std::fs::remove_file(&tmp);
        if let Err(e) = cleanup {
            eprintln!(
                "[rename] 临时文件清理失败（不影响结果）：{}：{e}",
                tmp.display()
            );
        }
        if let Err(error) = self
            .runtime
            .block_on(provider.delete_object(bucket, old_key))
        {
            return Err(AppServicesError::Rename(format!(
                "新对象已上传，但删除旧对象失败（远端同时存在 {old_key} 与 {new_key}）：{error}"
            )));
        }
        Ok(())
    }

    /// tokio 运行时句柄（Transfer Engine 用它 spawn 任务 future）。
    pub fn runtime_handle(&self) -> tokio::runtime::Handle {
        self.runtime.handle().clone()
    }

    /// 整表替换传输队列（⌘Q「暂停并退出」落盘）。空切片清空。
    pub fn replace_transfers(&self, items: &[PersistedTransfer]) -> Result<(), AppServicesError> {
        Ok(self.lock_accounts().replace_transfers(items)?)
    }

    /// 原子取出并清空传输队列（启动时恢复；取出后表空，避免下次重复入队）。
    pub fn take_transfers(&self) -> Result<Vec<PersistedTransfer>, AppServicesError> {
        Ok(self.lock_accounts().take_transfers()?)
    }

    /// 丢弃已保存队列（⌘Q「立即退出」，下次启动不要复活）。
    pub fn clear_transfers(&self) -> Result<(), AppServicesError> {
        Ok(self.lock_accounts().clear_transfers()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use object_storage_macos::KeychainCredentialStore;
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

        // 下载同样：未知账号在碰网络前就报 NotFound
        let err = services
            .download_object("no-such-account", "b", "k", &dir.join("out.bin"))
            .unwrap_err();
        assert!(
            matches!(err, AppServicesError::Account(AccountError::NotFound(_))),
            "下载未知账号应报 NotFound，实际 {err:?}"
        );
        assert!(!dir.join("out.bin").exists());
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

    /// 会话缓存：同一账号第二次构建 provider 不再触碰钥匙串——
    /// 直接删掉钥匙串条目后仍能构建成功；切换账号后缓存被置换（旧账号回落
    /// 到钥匙串，条目已删则显式报 MissingSecret，不静默）。
    #[test]
    fn provider_build_reuses_cached_secret_and_evicts_on_switch() {
        let (db, dir) = temp_db("cachefn");
        let services = AppServices::open_at(&db).unwrap();
        let account = services
            .add_qiniu_account("缓存测试", "ak-cache", "sk-cache-123")
            .unwrap();

        // 第一次构建：现取钥匙串并写缓存
        let (_, p1) = services.build_provider(&account.id).unwrap();
        assert_eq!(p1.kind(), ProviderKind::Qiniu);

        // 模拟“钥匙串不再可用”：绕过 service 直接删条目
        KeychainCredentialStore::new().delete(&account.id).unwrap();

        // 第二次构建：命中缓存，仍成功（没碰钥匙串）
        let (_, p2) = services.build_provider(&account.id).unwrap();
        assert_eq!(p2.kind(), ProviderKind::Qiniu);

        // 切到另一个账号：缓存置换。旧账号此后必须重新读钥匙串——
        // 而它的条目已被删，构建应显式报 MissingSecret
        let account2 = services
            .add_qiniu_account("另一个", "ak-cache-2", "sk-cache-456")
            .unwrap();
        let (_, p3) = services.build_provider(&account2.id).unwrap();
        assert_eq!(p3.kind(), ProviderKind::Qiniu);
        let err = services.build_provider(&account.id).unwrap_err();
        assert!(
            matches!(
                err,
                AppServicesError::Account(AccountError::MissingSecret(_))
            ),
            "缓存置换后旧账号应回落到钥匙串并报 MissingSecret，实际 {err:?}"
        );

        services.lock_accounts().delete(&account2.id).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn transfer_queue_replace_take_clear() {
        let (db, dir) = temp_db("xfer");
        let services = AppServices::open_at(&db).unwrap();
        let item = PersistedTransfer {
            kind: "download".into(),
            account_id: "acc".into(),
            bucket: "b".into(),
            object_key: "k.bin".into(),
            dest: "/tmp/k.bin".into(),
            display_name: "k.bin".into(),
            state: "paused".into(),
            enqueued_at_millis: 1,
        };
        services
            .replace_transfers(std::slice::from_ref(&item))
            .unwrap();
        let taken = services.take_transfers().unwrap();
        assert_eq!(taken, vec![item]);
        assert!(services.take_transfers().unwrap().is_empty());
        services
            .replace_transfers(&[PersistedTransfer {
                kind: "download".into(),
                account_id: "acc".into(),
                bucket: "b".into(),
                object_key: "k.bin".into(),
                dest: "/tmp/k.bin".into(),
                display_name: "k.bin".into(),
                state: "queued".into(),
                enqueued_at_millis: 1,
            }])
            .unwrap();
        services.clear_transfers().unwrap();
        assert!(services.take_transfers().unwrap().is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    /// rename 快速失败路径：同名直接拒绝，不建 provider（不碰钥匙串/网络）。
    #[test]
    fn rename_rejects_same_key_without_side_effects() {
        let (db, dir) = temp_db("rename-same");
        let services = AppServices::open_at(&db).unwrap();
        let err = services
            .rename_object("no-such-account", "b", "a.txt", "a.txt")
            .unwrap_err();
        assert!(err.to_string().contains("新名称与原名称相同"), "实际 {err}");
        fs::remove_dir_all(dir).unwrap();
    }
}

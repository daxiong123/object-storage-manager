//! 账号领域服务：SQLite 元数据 + Keychain Secret 的编排层
//!
//! 一致性策略（两存储无事务可用，选择"危害最小"的顺序 + 补偿）：
//! - `add`：先写 Keychain，再写 SQLite；SQLite 失败则补偿删除刚写的
//!   Keychain 条目。补偿本身失败 → 报复合错误，绝不静默吞掉
//!   （残留 Secret 比悬空元数据危害小，但必须让用户知道）
//! - `delete`：先删 SQLite（元数据是引用方），再删 Keychain；
//!   即使 Keychain 删除失败，也不会留下悬空引用，重试删除即清残留
//!
//! 所有 API 同步；调用方（UI）负责放入后台执行器，不阻塞主线程。

use object_storage_aliyun::{AliyunCredential, AliyunProvider};
use object_storage_domain::{Account, ProviderKind};
use object_storage_macos::KeychainCredentialStore;
use object_storage_persistence::{AccountRepository, PersistedTransfer, PersistenceError};
use object_storage_qiniu::{QiniuCredential, QiniuProvider};

use crate::provider::BuiltProvider;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AccountError {
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    #[error(transparent)]
    Keychain(#[from] object_storage_macos::KeychainError),
    #[error("账号不存在：{0}")]
    NotFound(String),
    #[error("无效输入：{0}")]
    InvalidInput(String),
    #[error("数据不一致：账号 {0} 在 SQLite 有元数据，但 Keychain 中没有 Secret")]
    MissingSecret(String),
    #[error("系统时钟异常（{0}），无法生成账号创建时间")]
    Clock(String),
    #[error(
        "添加账号失败：SQLite 写入失败（{sqlite}）；且 Keychain 补偿清理也失败（{cleanup}）。\
         请手动删除 Keychain 中 service=com.example.cloudstorage.credentials account={account} 的条目"
    )]
    AddRollbackCompromised {
        account: String,
        sqlite: String,
        cleanup: String,
    },
}

/// 账号领域服务
#[derive(Debug)]
pub struct AccountService {
    repo: AccountRepository,
    keychain: KeychainCredentialStore,
}

impl AccountService {
    /// 打开数据库（默认路径见 `persistence::default_db_path`）+ 默认 Keychain store
    pub fn open(db_path: impl AsRef<Path>) -> Result<Self, AccountError> {
        Ok(Self {
            repo: AccountRepository::open(db_path)?,
            keychain: KeychainCredentialStore::new(),
        })
    }

    pub fn list(&self) -> Result<Vec<Account>, AccountError> {
        Ok(self.repo.list()?)
    }

    /// 整表替换传输队列（⌘Q 暂停并退出）。空切片清空。
    pub fn replace_transfers(&self, items: &[PersistedTransfer]) -> Result<(), AccountError> {
        Ok(self.repo.replace_transfers(items)?)
    }

    /// 原子取出并清空传输队列（启动恢复用）。
    pub fn take_transfers(&self) -> Result<Vec<PersistedTransfer>, AccountError> {
        Ok(self.repo.take_transfers()?)
    }

    /// 丢弃已保存队列（⌘Q 立即退出）。
    pub fn clear_transfers(&self) -> Result<(), AccountError> {
        Ok(self.repo.clear_transfers()?)
    }

    /// 添加账号：Secret 入 Keychain，元数据入 SQLite
    pub fn add(
        &self,
        name: &str,
        provider: ProviderKind,
        access_key: &str,
        secret_key: &str,
    ) -> Result<Account, AccountError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(AccountError::InvalidInput("账号名称不能为空".into()));
        }
        let access_key = access_key.trim();
        if access_key.is_empty() {
            return Err(AccountError::InvalidInput("Access Key 不能为空".into()));
        }
        if secret_key.trim().is_empty() {
            return Err(AccountError::InvalidInput("Secret Key 不能为空".into()));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let created_at_millis = now_millis()?;

        // 1. 先写 Keychain（失败则整体未开始，无需回滚）
        self.keychain.save(&id, secret_key)?;

        // 2. 再写 SQLite；失败则补偿删除 Keychain 条目
        let account = Account {
            id,
            name: name.to_string(),
            provider,
            access_key: access_key.to_string(),
            created_at_millis,
        };
        if let Err(sqlite) = self.repo.insert(&account) {
            if let Err(cleanup) = self.keychain.delete(&account.id) {
                return Err(AccountError::AddRollbackCompromised {
                    account: account.id,
                    sqlite: sqlite.to_string(),
                    cleanup: cleanup.to_string(),
                });
            }
            return Err(sqlite.into());
        }
        Ok(account)
    }

    /// 重命名（仅显示名，不影响 Keychain / 主键）
    pub fn rename(&self, id: &str, new_name: &str) -> Result<(), AccountError> {
        let new_name = new_name.trim();
        if new_name.is_empty() {
            return Err(AccountError::InvalidInput("账号名称不能为空".into()));
        }
        if !self.repo.rename(id, new_name)? {
            return Err(AccountError::NotFound(id.to_string()));
        }
        Ok(())
    }

    /// 删除账号：先删 SQLite 元数据，再删 Keychain Secret
    pub fn delete(&self, id: &str) -> Result<(), AccountError> {
        if !self.repo.delete(id)? {
            return Err(AccountError::NotFound(id.to_string()));
        }
        // Ok(false)（本就无残留）同样接受：幂等
        self.keychain.delete(id)?;
        Ok(())
    }

    /// 取账号 Secret（每次从 Keychain 现取；本层不做缓存——会话级缓存在
    /// AppServices 编排层，见其 `cached_secret` 字段注释）。
    /// 先校验元数据：未知账号在这里就 NotFound，不触碰钥匙串。
    pub fn load_secret(&self, id: &str) -> Result<String, AccountError> {
        if self.repo.get(id)?.is_none() {
            return Err(AccountError::NotFound(id.to_string()));
        }
        self.keychain
            .load(id)?
            .ok_or_else(|| AccountError::MissingSecret(id.to_string()))
    }

    /// 取账号及其七牛凭证。Secret 每次从 Keychain 现取：不落盘、不进日志
    /// （会话内缓存见 AppServices；本层保持无状态）
    pub fn qiniu_credential(&self, id: &str) -> Result<(Account, QiniuCredential), AccountError> {
        let secret = self.load_secret(id)?;
        self.qiniu_credential_with_secret(id, secret)
    }

    /// 用调用方提供的 Secret 构建凭证（供 AppServices 会话缓存复用，避免
    /// 重复触碰钥匙串）。Secret 来源不校验，格式校验交给 QiniuCredential::new
    pub fn qiniu_credential_with_secret(
        &self,
        id: &str,
        secret_key: String,
    ) -> Result<(Account, QiniuCredential), AccountError> {
        let account = self
            .repo
            .get(id)?
            .ok_or_else(|| AccountError::NotFound(id.to_string()))?;
        let credential = QiniuCredential::new(&account.access_key, &secret_key)
            .map_err(|e| AccountError::InvalidInput(e.to_string()))?;
        Ok((account, credential))
    }

    /// 构造可用的 Provider。Secret 每次从 Keychain 现取。
    pub fn build_provider(&self, id: &str) -> Result<(Account, BuiltProvider), AccountError> {
        let secret = self.load_secret(id)?;
        self.build_provider_with_secret(id, secret)
    }

    /// 用调用方提供的 Secret 构造 Provider（供 AppServices 会话缓存复用）
    pub fn build_provider_with_secret(
        &self,
        id: &str,
        secret_key: String,
    ) -> Result<(Account, BuiltProvider), AccountError> {
        let account = self
            .repo
            .get(id)?
            .ok_or_else(|| AccountError::NotFound(id.to_string()))?;
        match account.provider {
            ProviderKind::Qiniu => {
                let credential = QiniuCredential::new(&account.access_key, &secret_key)
                    .map_err(|e| AccountError::InvalidInput(e.to_string()))?;
                Ok((
                    account,
                    BuiltProvider::Qiniu(QiniuProvider::new(credential)),
                ))
            }
            ProviderKind::Aliyun => {
                let credential = AliyunCredential::new(&account.access_key, &secret_key)
                    .map_err(|e| AccountError::InvalidInput(e.to_string()))?;
                Ok((
                    account,
                    BuiltProvider::Aliyun(AliyunProvider::new(credential)),
                ))
            }
        }
    }
}

fn now_millis() -> Result<i64, AccountError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .map_err(|e| AccountError::Clock(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// 每个测试独立的临时目录（随机名，结束清理）
    fn temp_service(tag: &str) -> (AccountService, PathBuf) {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("ost-app-{}-{}-{nanos}", tag, std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let service = AccountService::open(dir.join("test.sqlite")).unwrap();
        (service, dir)
    }

    fn cleanup_dir(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    /// 注意：本组测试写真实登录 Keychain（account 为随机 UUID，测试内清理）。
    /// 若测试中途 panic 可能残留条目，service=KEYCHAIN_SERVICE，可手动清。
    fn add_test_account(service: &AccountService, tag: &str) -> Account {
        let secret = format!("sk-{tag}-{}", uuid::Uuid::new_v4());
        service
            .add(
                &format!("测试账号-{tag}"),
                ProviderKind::Qiniu,
                &format!("ak-{tag}"),
                &secret,
            )
            .unwrap_or_else(|e| panic!("add 失败：{e}"))
    }

    #[test]
    fn add_rename_delete_round_trip() {
        let (service, dir) = temp_service("roundtrip");
        let account = add_test_account(&service, "rt");

        // list / 元数据
        assert_eq!(service.list().unwrap(), vec![account.clone()]);

        // credential：AK 来自 SQLite，SK 来自 Keychain（回环验证在 macos crate 测试）
        let (loaded, cred) = service.qiniu_credential(&account.id).unwrap();
        assert_eq!(loaded.id, account.id);
        assert_eq!(cred.access_key(), account.access_key);
        // Debug 不泄露 SK（storage-core 的红线）
        assert!(!format!("{cred:?}").contains("sk-rt-"));

        // build_provider
        let (_, provider) = service.build_provider(&account.id).unwrap();
        assert_eq!(provider.kind(), ProviderKind::Qiniu);

        // rename
        service.rename(&account.id, "改名后").unwrap();
        assert_eq!(service.list().unwrap()[0].name, "改名后");

        // delete：元数据 + Keychain 都清掉
        service.delete(&account.id).unwrap();
        assert!(service.list().unwrap().is_empty());
        assert!(matches!(
            service.qiniu_credential(&account.id),
            Err(AccountError::NotFound(_))
        ));
        assert_eq!(
            KeychainCredentialStore::new().load(&account.id).unwrap(),
            None,
            "删除账号后 Keychain 不应残留 Secret"
        );

        cleanup_dir(&dir);
    }

    #[test]
    fn add_validates_input_and_unimplemented_provider() {
        let (service, dir) = temp_service("validate");
        let cases: [(&str, &str, &str, ProviderKind); 3] = [
            ("", "ak", "sk", ProviderKind::Qiniu),
            ("名称", "", "sk", ProviderKind::Qiniu),
            ("名称", "ak", "", ProviderKind::Qiniu),
        ];
        for (name, ak, sk, provider) in cases {
            let result = service.add(name, provider, ak, sk);
            assert!(
                matches!(result, Err(AccountError::InvalidInput(_))),
                "({name}, {ak}, {sk}, {provider:?}) 应被拒绝"
            );
        }
        assert!(service.list().unwrap().is_empty());
        cleanup_dir(&dir);
    }

    #[test]
    fn delete_and_rename_missing_account_is_not_found() {
        let (service, dir) = temp_service("missing");
        assert!(matches!(
            service.delete("no-such-id"),
            Err(AccountError::NotFound(_))
        ));
        assert!(matches!(
            service.rename("no-such-id", "x"),
            Err(AccountError::NotFound(_))
        ));
        cleanup_dir(&dir);
    }

    /// SQLite 有元数据但 Keychain 无 Secret → MissingSecret（数据不一致必须暴露）
    #[test]
    fn missing_secret_is_reported() {
        let (service, dir) = temp_service("missingscret");
        let account = add_test_account(&service, "ms");

        // 模拟不一致：绕过 service 直接删 Keychain 条目
        assert!(KeychainCredentialStore::new().delete(&account.id).unwrap());

        let result = service.qiniu_credential(&account.id);
        assert!(
            matches!(result, Err(AccountError::MissingSecret(_))),
            "Keychain 缺 Secret 必须报 MissingSecret，不能静默"
        );
        cleanup_dir(&dir);
    }

    /// 空白字符输入被 trim，而不是原样入库
    #[test]
    fn add_trims_whitespace() {
        let (service, dir) = temp_service("trim");
        let account = service
            .add("  名字  ", ProviderKind::Qiniu, "  ak-x  ", "  sk-x  ")
            .unwrap();
        assert_eq!(account.name, "名字");
        assert_eq!(account.access_key, "ak-x");
        // Secret 是凭证明文：原样存取，不做 trim（不可直接断言，
        // 但 qiniu_credential 能构建成功说明整链路通）
        assert!(service.qiniu_credential(&account.id).is_ok());
        service.delete(&account.id).unwrap();
        cleanup_dir(&dir);
    }

    #[test]
    fn add_aliyun_account_builds_aliyun_provider() {
        let (service, dir) = temp_service("aliyun");
        let account = service
            .add("oss", ProviderKind::Aliyun, "ak-oss", "sk-oss")
            .unwrap();
        assert_eq!(account.provider, ProviderKind::Aliyun);
        let (_, provider) = service.build_provider(&account.id).unwrap();
        assert_eq!(provider.kind(), ProviderKind::Aliyun);
        service.delete(&account.id).unwrap();
        cleanup_dir(&dir);
    }
}

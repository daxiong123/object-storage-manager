//! macOS Keychain 凭证存取（Security.framework，generic password 条目）
//!
//! 规范依据（spec §18/§19）：
//! - Secret（Qiniu SecretKey / OSS AccessKey Secret / STS Secret / STS Token）
//!   只进 Keychain，SQLite 永不保存
//! - Keychain 条目：`service = KEYCHAIN_SERVICE`，`account = <账号 UUID>`
//!   （用 UUID 而非账号名做 key，因为账号名允许修改）
//!
//! API 全部同步：调用方（app 层/UI）负责放到后台执行器，本模块不做 IO 线程管理。
//!
//! 安全说明：`save` 存在同名条目时会**更新**（SecItemUpdate 语义，见
//! security-framework 3.7 `passwords::set_generic_password` 文档）。

use security_framework::base::Error as SecurityError;
use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};
use security_framework_sys::base::errSecItemNotFound;
use thiserror::Error;

/// Keychain service 名（spec §19 建议 `com.<company>.<app>.credentials`）。
///
/// 集中在一处常量：Bundle ID 定稿后只改这里。
pub const KEYCHAIN_SERVICE: &str = "com.example.cloudstorage.credentials";

#[derive(Debug, Error)]
pub enum KeychainError {
    #[error("Keychain 写入失败（service={service} account={account}）：{source}")]
    Write {
        service: String,
        account: String,
        #[source]
        source: SecurityError,
    },
    #[error("Keychain 读取失败（service={service} account={account}）：{source}")]
    Read {
        service: String,
        account: String,
        #[source]
        source: SecurityError,
    },
    #[error("Keychain 删除失败（service={service} account={account}）：{source}")]
    Delete {
        service: String,
        account: String,
        #[source]
        source: SecurityError,
    },
    #[error("Keychain 条目损坏：Secret 不是有效 UTF-8（service={service} account={account}）")]
    Corrupt { service: String, account: String },
}

/// Keychain 凭证存取器：以 (service, account) 定位一条 generic password
#[derive(Debug, Clone)]
pub struct KeychainCredentialStore {
    service: String,
}

impl KeychainCredentialStore {
    /// 生产实例：使用 `KEYCHAIN_SERVICE`
    pub fn new() -> Self {
        Self {
            service: KEYCHAIN_SERVICE.to_string(),
        }
    }

    /// 自定义 service（测试 / 多租户场景）
    pub fn with_service(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    /// 保存 Secret（条目已存在则更新）
    pub fn save(&self, account: &str, secret: &str) -> Result<(), KeychainError> {
        set_generic_password(&self.service, account, secret.as_bytes()).map_err(|source| {
            KeychainError::Write {
                service: self.service.clone(),
                account: account.to_string(),
                source,
            }
        })
    }

    /// 读取 Secret；`Ok(None)` = 条目不存在（errSecItemNotFound 归一化为正常分支）
    pub fn load(&self, account: &str) -> Result<Option<String>, KeychainError> {
        match get_generic_password(&self.service, account) {
            Ok(bytes) => String::from_utf8(bytes)
                .map(Some)
                .map_err(|_| KeychainError::Corrupt {
                    service: self.service.clone(),
                    account: account.to_string(),
                }),
            Err(e) if e.code() == errSecItemNotFound => Ok(None),
            Err(source) => Err(KeychainError::Read {
                service: self.service.clone(),
                account: account.to_string(),
                source,
            }),
        }
    }

    /// 删除 Secret；`Ok(false)` = 条目本来就不存在（幂等）
    pub fn delete(&self, account: &str) -> Result<bool, KeychainError> {
        match delete_generic_password(&self.service, account) {
            Ok(()) => Ok(true),
            Err(e) if e.code() == errSecItemNotFound => Ok(false),
            Err(source) => Err(KeychainError::Delete {
                service: self.service.clone(),
                account: account.to_string(),
                source,
            }),
        }
    }
}

impl Default for KeychainCredentialStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// 唯一 account：每次运行全新创建，避免触发旧条目 ACL 的钥匙串弹窗；
    /// 测试结束自行清理（正常路径下无残留）
    fn unique_account(tag: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("系统时钟早于 epoch")
            .as_nanos();
        format!("test-{tag}-{}-{nanos}", process::id())
    }

    /// 写真实登录 Keychain 的集成测试：
    /// create → read → update → delete，全链路 + 幂等删除语义
    #[test]
    fn keychain_save_load_update_delete_round_trip() {
        let store = KeychainCredentialStore::new();
        let account = unique_account("roundtrip");

        // 初始不存在
        assert_eq!(
            store.load(&account).unwrap(),
            None,
            "随机 account 不应已存在；若失败请检查 Keychain 残留"
        );

        // 创建
        store.save(&account, "secret-one").unwrap();
        assert_eq!(store.load(&account).unwrap().as_deref(), Some("secret-one"));

        // 更新（set_generic_password 的 update 语义）
        store.save(&account, "secret-two").unwrap();
        assert_eq!(store.load(&account).unwrap().as_deref(), Some("secret-two"));

        // 删除 → 不存在 → 再删除 = 幂等 false
        assert!(store.delete(&account).unwrap());
        assert_eq!(store.load(&account).unwrap(), None);
        assert!(!store.delete(&account).unwrap());
    }

    /// 空字符串是合法 Secret（Keychain 不做业务校验；业务校验在 app 层）
    #[test]
    fn keychain_allows_empty_secret() {
        let store = KeychainCredentialStore::new();
        let account = unique_account("empty");

        store.save(&account, "").unwrap();
        assert_eq!(store.load(&account).unwrap().as_deref(), Some(""));

        store.delete(&account).unwrap();
    }
}

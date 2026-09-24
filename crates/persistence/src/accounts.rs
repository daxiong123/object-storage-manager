//! 账号元数据仓储（SQLite）
//!
//! 只存元数据（id / name / provider / access_key / created_at）。
//! Secret 不在这里——由 app 层的 `AccountService` 写入 macOS Keychain，
//! 以账号 UUID 作为两边（SQLite ↔ Keychain）的关联键。

use object_storage_domain::{Account, ProviderKind};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("数据库打开失败（{path}）：{source}")]
    Open {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },
    #[error("数据库操作失败（{op}）：{source}")]
    Query {
        op: &'static str,
        #[source]
        source: rusqlite::Error,
    },
    #[error("数据损坏：{0}")]
    Corrupt(String),
    #[error("无法定位应用数据目录（Application Support）")]
    DataDir,
    #[error("创建应用数据目录失败（{dir}）：{source}")]
    CreateDataDir {
        dir: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("设置读写失败（{path}）：{source}")]
    SettingsIo {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("设置文件解析失败（{path}）：{message}")]
    SettingsParse { path: PathBuf, message: String },
}

/// 建表语句。红线：**没有 Secret 列**（Secret 在 macOS Keychain，spec §18）
const SQL_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS accounts (
    id                TEXT PRIMARY KEY,
    name              TEXT NOT NULL,
    provider          TEXT NOT NULL CHECK (provider IN ('qiniu', 'aliyun', 'tencent')),
    access_key        TEXT NOT NULL,
    created_at_millis INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS transfers (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    kind                TEXT NOT NULL CHECK (kind IN ('download', 'upload')),
    account_id          TEXT NOT NULL,
    bucket              TEXT NOT NULL,
    object_key          TEXT NOT NULL,
    dest                TEXT NOT NULL,
    display_name        TEXT NOT NULL,
    state               TEXT NOT NULL CHECK (state IN ('queued', 'paused')),
    region              TEXT,
    enqueued_at_millis  INTEGER NOT NULL
);
";

/// accounts.provider 的 CHECK 允许值（建表与迁移共用，避免两处写法漂移）
const PROVIDER_CHECK: &str = "'qiniu', 'aliyun', 'tencent'";

/// 已有库的 accounts 表 CHECK 只有 `qiniu` / `aliyun`。SQLite 不能 ALTER CHECK，
/// 检测到旧 schema 就整表重建（数据原样搬迁）——与 transfers 的迁移同一范式。
///
/// 迁移必须保号：旧库里已有的账号（含七牛/阿里云）要一条不少地活下来，
/// `accounts_allow_tencent_migration_preserves_existing_rows` 把这条钉死。
pub(crate) fn migrate_accounts_allow_tencent(conn: &Connection) -> Result<(), PersistenceError> {
    let sql: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='accounts'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|source| PersistenceError::Query {
            op: "检查 accounts schema",
            source,
        })?;
    let Some(sql) = sql else {
        return Ok(());
    };
    if sql.contains("'tencent'") {
        return Ok(());
    }
    // 整个重建必须在一个事务里：若进程在 rename 之后、copy 完成之前中断，
    // 无事务的各语句已各自提交——下次启动时 SQL_SCHEMA 会建出一张带
    // 'tencent' 的新空表，本函数提前返回，旧账号永久困在 accounts_mig_old。
    // （Codex PR review P2）
    let tx = conn
        .unchecked_transaction()
        .map_err(|source| PersistenceError::Query {
            op: "开启 accounts 迁移事务",
            source,
        })?;
    tx.execute_batch(&format!(
        "
        ALTER TABLE accounts RENAME TO accounts_mig_old;
        CREATE TABLE accounts (
            id                TEXT PRIMARY KEY,
            name              TEXT NOT NULL,
            provider          TEXT NOT NULL CHECK (provider IN ({PROVIDER_CHECK})),
            access_key        TEXT NOT NULL,
            created_at_millis INTEGER NOT NULL
        );
        INSERT INTO accounts (id, name, provider, access_key, created_at_millis)
            SELECT id, name, provider, access_key, created_at_millis FROM accounts_mig_old;
        DROP TABLE accounts_mig_old;
        "
    ))
    .map_err(|source| PersistenceError::Query {
        op: "升级 accounts 表允许 tencent",
        source,
    })?;
    tx.commit().map_err(|source| PersistenceError::Query {
        op: "提交 accounts 迁移事务",
        source,
    })?;
    Ok(())
}

/// accounts 表的全部列名（schema 回归测试的把守依据）
#[cfg(test)]
const ACCOUNT_COLUMNS: [&str; 5] = ["id", "name", "provider", "access_key", "created_at_millis"];

const SQL_COLUMNS: &str = "SELECT id, name, provider, access_key, created_at_millis FROM accounts";

/// 从行内取出的原始字段（provider 尚未解码）
struct RawAccount {
    id: String,
    name: String,
    provider: String,
    access_key: String,
    created_at_millis: i64,
}

fn raw_account_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawAccount> {
    Ok(RawAccount {
        id: row.get(0)?,
        name: row.get(1)?,
        provider: row.get(2)?,
        access_key: row.get(3)?,
        created_at_millis: row.get(4)?,
    })
}

fn materialize(raw: RawAccount) -> Result<Account, PersistenceError> {
    let provider = ProviderKind::from_str_opt(&raw.provider)
        .ok_or_else(|| PersistenceError::Corrupt(raw.provider.clone()))?;
    Ok(Account {
        id: raw.id,
        name: raw.name,
        provider,
        access_key: raw.access_key,
        created_at_millis: raw.created_at_millis,
    })
}

/// 账号元数据仓储。同步 API；异步包装由上层负责（gpui 后台执行器 / tokio）
#[derive(Debug)]
pub struct AccountRepository {
    /// 同 crate 内 transfers 模块共用连接（不对外暴露）
    pub(crate) conn: Connection,
}

impl AccountRepository {
    /// 打开（不存在则创建 schema）
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PersistenceError> {
        let path = path.as_ref();
        let conn = Connection::open(path).map_err(|source| PersistenceError::Open {
            path: path.to_path_buf(),
            source,
        })?;
        conn.execute_batch(SQL_SCHEMA)
            .map_err(|source| PersistenceError::Query {
                op: "建表", source
            })?;
        migrate_accounts_allow_tencent(&conn)?;
        crate::transfers::migrate_transfers_allow_upload(&conn)?;
        crate::transfers::migrate_transfers_add_region(&conn)?;
        Ok(Self { conn })
    }

    /// 内存库（测试用）
    pub fn open_in_memory() -> Result<Self, PersistenceError> {
        let conn = Connection::open_in_memory().map_err(|source| PersistenceError::Open {
            path: PathBuf::from(":memory:"),
            source,
        })?;
        conn.execute_batch(SQL_SCHEMA)
            .map_err(|source| PersistenceError::Query {
                op: "建表", source
            })?;
        migrate_accounts_allow_tencent(&conn)?;
        crate::transfers::migrate_transfers_allow_upload(&conn)?;
        crate::transfers::migrate_transfers_add_region(&conn)?;
        Ok(Self { conn })
    }

    pub fn insert(&self, account: &Account) -> Result<(), PersistenceError> {
        self.conn
            .execute(
                "INSERT INTO accounts (id, name, provider, access_key, created_at_millis)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    account.id,
                    account.name,
                    account.provider.as_str(),
                    account.access_key,
                    account.created_at_millis
                ],
            )
            .map_err(|source| PersistenceError::Query {
                op: "插入账号",
                source,
            })?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<Option<Account>, PersistenceError> {
        let raw = self
            .conn
            .query_row(
                &format!("{SQL_COLUMNS} WHERE id = ?1"),
                params![id],
                raw_account_from_row,
            )
            .optional()
            .map_err(|source| PersistenceError::Query {
                op: "查询账号",
                source,
            })?;
        raw.map(materialize).transpose()
    }

    pub fn list(&self) -> Result<Vec<Account>, PersistenceError> {
        let mut stmt =
            self.conn
                .prepare(SQL_COLUMNS)
                .map_err(|source| PersistenceError::Query {
                    op: "列举账号",
                    source,
                })?;
        let rows =
            stmt.query_map([], raw_account_from_row)
                .map_err(|source| PersistenceError::Query {
                    op: "列举账号",
                    source,
                })?;
        let mut out = Vec::new();
        for row in rows {
            let raw = row.map_err(|source| PersistenceError::Query {
                op: "列举账号",
                source,
            })?;
            out.push(materialize(raw)?);
        }
        Ok(out)
    }

    /// 重命名显示名；`Ok(false)` = 账号不存在
    pub fn rename(&self, id: &str, name: &str) -> Result<bool, PersistenceError> {
        let n = self
            .conn
            .execute(
                "UPDATE accounts SET name = ?2 WHERE id = ?1",
                params![id, name],
            )
            .map_err(|source| PersistenceError::Query {
                op: "重命名账号",
                source,
            })?;
        Ok(n > 0)
    }

    /// 删除账号元数据；`Ok(false)` = 账号不存在。
    /// 注意：只删元数据，Keychain Secret 的删除由 app 层编排。
    pub fn delete(&self, id: &str) -> Result<bool, PersistenceError> {
        let n = self
            .conn
            .execute("DELETE FROM accounts WHERE id = ?1", params![id])
            .map_err(|source| PersistenceError::Query {
                op: "删除账号",
                source,
            })?;
        Ok(n > 0)
    }
}

/// 默认数据库路径：`~/Library/Application Support/CloudStorage/database.sqlite`
///（spec §58；目录不存在则创建）
/// 应用数据目录（`~/Library/Application Support/CloudStorage/`），
/// 不存在则创建。settings.json 与 database.sqlite 共用此目录。
pub fn default_data_dir() -> Result<PathBuf, PersistenceError> {
    let base = dirs::data_dir().ok_or(PersistenceError::DataDir)?;
    let dir = base.join("CloudStorage");
    std::fs::create_dir_all(&dir).map_err(|source| PersistenceError::CreateDataDir {
        dir: dir.clone(),
        source,
    })?;
    Ok(dir)
}

pub fn default_db_path() -> Result<PathBuf, PersistenceError> {
    Ok(default_data_dir()?.join("database.sqlite"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_account(id: &str, name: &str, provider: ProviderKind) -> Account {
        Account {
            id: id.to_string(),
            name: name.to_string(),
            provider,
            access_key: format!("ak-{id}"),
            created_at_millis: 1_700_000_000_000,
        }
    }

    #[test]
    fn account_round_trip() {
        let repo = AccountRepository::open_in_memory().unwrap();
        let a = sample_account("id-a", "工作号", ProviderKind::Qiniu);
        let b = sample_account("id-b", "个人号", ProviderKind::Aliyun);

        repo.insert(&a).unwrap();
        repo.insert(&b).unwrap();

        assert_eq!(repo.list().unwrap(), vec![a.clone(), b.clone()]);
        assert_eq!(repo.get("id-a").unwrap(), Some(a));
        assert_eq!(repo.get("missing").unwrap(), None);

        assert!(repo.rename("id-a", "新名字").unwrap());
        assert_eq!(repo.get("id-a").unwrap().unwrap().name, "新名字");
        assert!(!repo.rename("missing", "x").unwrap());

        assert!(repo.delete("id-a").unwrap());
        assert_eq!(repo.get("id-a").unwrap(), None);
        assert!(!repo.delete("id-a").unwrap());
        assert_eq!(repo.list().unwrap().len(), 1);
    }

    #[test]
    fn duplicate_id_rejected() {
        let repo = AccountRepository::open_in_memory().unwrap();
        let a = sample_account("dup", "x", ProviderKind::Qiniu);
        repo.insert(&a).unwrap();
        assert!(
            repo.insert(&a).is_err(),
            "PRIMARY KEY 冲突必须报错（Fail Fast）"
        );
    }

    #[test]
    fn unknown_provider_rejected_by_check_constraint() {
        let repo = AccountRepository::open_in_memory().unwrap();
        let result = repo.conn.execute(
            "INSERT INTO accounts (id, name, provider, access_key, created_at_millis)
             VALUES ('bad', 'x', 'gcp', 'ak', 0)",
            [],
        );
        assert!(
            result.is_err(),
            "CHECK 约束必须在 DB 层拒绝未知 provider（Fail Fast）"
        );
    }

    #[test]
    fn schema_has_no_secret_column() {
        // 红线回归测试：accounts 表的列必须恰好是元数据列，
        // 任何人给 SQLite 加 Secret 列都会让本测试失败
        let repo = AccountRepository::open_in_memory().unwrap();
        let mut stmt = repo.conn.prepare("PRAGMA table_info(accounts)").unwrap();
        let names: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(names, ACCOUNT_COLUMNS);
    }

    /// 0.3.0 的库（provider CHECK 只有 qiniu/aliyun）必须能原地升级：
    /// 老账号一条不少、字段不变，且升级后 tencent 可以插入。
    ///
    /// 这条测试的价值在「用真的旧 schema 建库再走 open()」——直接测
    /// `open_in_memory()` 得到的是新 schema，迁移分支根本不会被执行。
    #[test]
    fn accounts_allow_tencent_migration_preserves_existing_rows() {
        const OLD_SCHEMA: &str = "
            CREATE TABLE accounts (
                id                TEXT PRIMARY KEY,
                name              TEXT NOT NULL,
                provider          TEXT NOT NULL CHECK (provider IN ('qiniu', 'aliyun')),
                access_key        TEXT NOT NULL,
                created_at_millis INTEGER NOT NULL
            );
        ";
        let dir = std::env::temp_dir().join(format!(
            "cloudstorage-migrate-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("old.sqlite");

        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(OLD_SCHEMA).unwrap();
            conn.execute(
                "INSERT INTO accounts (id, name, provider, access_key, created_at_millis)
                 VALUES ('old-qiniu', '老的七牛号', 'qiniu', 'ak-old', 1700000000000)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO accounts (id, name, provider, access_key, created_at_millis)
                 VALUES ('old-aliyun', '老的阿里云号', 'aliyun', 'ak-oss', 1700000000001)",
                [],
            )
            .unwrap();
            // 前提校验：旧库此刻确实拒绝 tencent，否则这条测试什么都没验证
            assert!(
                conn.execute(
                    "INSERT INTO accounts (id, name, provider, access_key, created_at_millis)
                     VALUES ('pre', 'x', 'tencent', 'ak', 0)",
                    [],
                )
                .is_err(),
                "旧 schema 本应拒绝 tencent，前提不成立"
            );
        }

        let repo = AccountRepository::open(&path).unwrap();

        let surviving = repo.list().unwrap();
        assert_eq!(surviving.len(), 2, "迁移不能丢账号");
        assert_eq!(surviving[0].id, "old-qiniu");
        assert_eq!(surviving[0].name, "老的七牛号");
        assert_eq!(surviving[0].provider, ProviderKind::Qiniu);
        assert_eq!(surviving[0].access_key, "ak-old");
        assert_eq!(surviving[0].created_at_millis, 1_700_000_000_000);
        assert_eq!(surviving[1].provider, ProviderKind::Aliyun);

        // 主键约束必须还在（重建表时最容易漏掉的东西）
        assert!(
            repo.insert(&sample_account("old-qiniu", "dup", ProviderKind::Qiniu))
                .is_err()
        );

        // 升级后 tencent 可插入
        let tencent = sample_account("new-tencent", "腾讯云号", ProviderKind::Tencent);
        repo.insert(&tencent).unwrap();
        assert_eq!(repo.get("new-tencent").unwrap(), Some(tencent));

        // 迁移仍拒未知 provider
        assert!(
            repo.conn
                .execute(
                    "INSERT INTO accounts (id, name, provider, access_key, created_at_millis)
                     VALUES ('bad', 'x', 'gcp', 'ak', 0)",
                    [],
                )
                .is_err()
        );

        // 再打开一次不应重复迁移（幂等）
        drop(repo);
        let repo = AccountRepository::open(&path).unwrap();
        assert_eq!(repo.list().unwrap().len(), 3);

        drop(repo);
        std::fs::remove_dir_all(&dir).ok();
    }
}

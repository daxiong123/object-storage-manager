//! 传输队列持久化（⌘Q「暂停并退出」→ 下次启动恢复）
//!
//! 红线：本表**不存 Secret**（无 SK / token 列）；只存云端三元组 + 本地 dest。
//! SK 仍走 Keychain，恢复后由 AppServices::build_provider 现取。

use rusqlite::{Connection, params};

use crate::accounts::{AccountRepository, PersistenceError};

/// 可持久化的一条传输任务（与 transfer crate 解耦：本 crate 不依赖引擎）。
///
/// `state` 只允许 `queued` / `paused`：
/// - 退出时 Running/Waiting/Queued 一律落成 `queued`（下次启动自动继续）
/// - 用户手动暂停落成 `paused`（下次启动保持暂停）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedTransfer {
    pub kind: String,
    pub account_id: String,
    pub bucket: String,
    pub object_key: String,
    pub dest: String,
    pub display_name: String,
    pub state: String,
    pub enqueued_at_millis: i64,
}

const SQL_COLUMNS: &str = "SELECT kind, account_id, bucket, object_key, dest, display_name, state, enqueued_at_millis FROM transfers ORDER BY id";

#[cfg(test)]
const TRANSFER_COLUMNS: [&str; 9] = [
    "id",
    "kind",
    "account_id",
    "bucket",
    "object_key",
    "dest",
    "display_name",
    "state",
    "enqueued_at_millis",
];

fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<PersistedTransfer> {
    Ok(PersistedTransfer {
        kind: row.get(0)?,
        account_id: row.get(1)?,
        bucket: row.get(2)?,
        object_key: row.get(3)?,
        dest: row.get(4)?,
        display_name: row.get(5)?,
        state: row.get(6)?,
        enqueued_at_millis: row.get(7)?,
    })
}

fn list_on(conn: &Connection) -> Result<Vec<PersistedTransfer>, PersistenceError> {
    let mut stmt = conn
        .prepare(SQL_COLUMNS)
        .map_err(|source| PersistenceError::Query {
            op: "列举传输队列",
            source,
        })?;
    let rows = stmt
        .query_map([], row_to_item)
        .map_err(|source| PersistenceError::Query {
            op: "列举传输队列",
            source,
        })?;
    let mut out = Vec::new();
    for row in rows {
        let item = row.map_err(|source| PersistenceError::Query {
            op: "列举传输队列",
            source,
        })?;
        if item.kind != "download" {
            return Err(PersistenceError::Corrupt(format!(
                "transfers.kind 非法：{}",
                item.kind
            )));
        }
        if item.state != "queued" && item.state != "paused" {
            return Err(PersistenceError::Corrupt(format!(
                "transfers.state 非法：{}",
                item.state
            )));
        }
        out.push(item);
    }
    Ok(out)
}

impl AccountRepository {
    /// 用新快照整表替换（事务：先清空再插入）。空切片 = 清空。
    pub fn replace_transfers(&self, items: &[PersistedTransfer]) -> Result<(), PersistenceError> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|source| PersistenceError::Query {
                op: "开启传输队列事务",
                source,
            })?;
        tx.execute("DELETE FROM transfers", [])
            .map_err(|source| PersistenceError::Query {
                op: "清空传输队列",
                source,
            })?;
        for item in items {
            tx.execute(
                "INSERT INTO transfers (kind, account_id, bucket, object_key, dest, display_name, state, enqueued_at_millis)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    item.kind,
                    item.account_id,
                    item.bucket,
                    item.object_key,
                    item.dest,
                    item.display_name,
                    item.state,
                    item.enqueued_at_millis
                ],
            )
            .map_err(|source| PersistenceError::Query {
                op: "插入传输任务",
                source,
            })?;
        }
        tx.commit().map_err(|source| PersistenceError::Query {
            op: "提交传输队列事务",
            source,
        })?;
        Ok(())
    }

    /// 读取当前持久化队列（不删除）。
    pub fn list_transfers(&self) -> Result<Vec<PersistedTransfer>, PersistenceError> {
        list_on(&self.conn)
    }

    /// 原子取出并清空：启动恢复用，避免恢复成功后下次再入队一份。
    pub fn take_transfers(&self) -> Result<Vec<PersistedTransfer>, PersistenceError> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|source| PersistenceError::Query {
                op: "开启传输队列事务",
                source,
            })?;
        let items = list_on(&tx)?;
        tx.execute("DELETE FROM transfers", [])
            .map_err(|source| PersistenceError::Query {
                op: "清空传输队列",
                source,
            })?;
        tx.commit().map_err(|source| PersistenceError::Query {
            op: "提交传输队列事务",
            source,
        })?;
        Ok(items)
    }

    /// 丢弃已保存队列（「立即退出」：不要下次复活）。
    pub fn clear_transfers(&self) -> Result<(), PersistenceError> {
        self.conn
            .execute("DELETE FROM transfers", [])
            .map_err(|source| PersistenceError::Query {
                op: "清空传输队列",
                source,
            })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str, state: &str) -> PersistedTransfer {
        PersistedTransfer {
            kind: "download".into(),
            account_id: "acc-1".into(),
            bucket: "bkt".into(),
            object_key: format!("dir/{name}"),
            dest: format!("/tmp/{name}"),
            display_name: name.into(),
            state: state.into(),
            enqueued_at_millis: 1_700_000_000_000,
        }
    }

    #[test]
    fn replace_list_take_round_trip() {
        let repo = AccountRepository::open_in_memory().unwrap();
        let a = sample("a.bin", "queued");
        let b = sample("b.bin", "paused");
        repo.replace_transfers(&[a.clone(), b.clone()]).unwrap();
        assert_eq!(repo.list_transfers().unwrap(), vec![a.clone(), b.clone()]);

        let taken = repo.take_transfers().unwrap();
        assert_eq!(taken, vec![a, b]);
        assert!(repo.list_transfers().unwrap().is_empty());
    }

    #[test]
    fn replace_empty_clears() {
        let repo = AccountRepository::open_in_memory().unwrap();
        repo.replace_transfers(&[sample("x.bin", "queued")])
            .unwrap();
        repo.replace_transfers(&[]).unwrap();
        assert!(repo.list_transfers().unwrap().is_empty());
    }

    #[test]
    fn clear_is_idempotent() {
        let repo = AccountRepository::open_in_memory().unwrap();
        repo.clear_transfers().unwrap();
        repo.replace_transfers(&[sample("x.bin", "queued")])
            .unwrap();
        repo.clear_transfers().unwrap();
        repo.clear_transfers().unwrap();
        assert!(repo.list_transfers().unwrap().is_empty());
    }

    #[test]
    fn unknown_kind_rejected_by_check() {
        let repo = AccountRepository::open_in_memory().unwrap();
        let mut bad = sample("x.bin", "queued");
        bad.kind = "upload".into();
        assert!(
            repo.replace_transfers(&[bad]).is_err(),
            "CHECK 约束必须拒绝未知 kind"
        );
    }

    #[test]
    fn unknown_state_rejected_by_check() {
        let repo = AccountRepository::open_in_memory().unwrap();
        let bad = sample("x.bin", "running");
        assert!(
            repo.replace_transfers(&[bad]).is_err(),
            "CHECK 约束必须拒绝未知 state"
        );
    }

    #[test]
    fn transfers_schema_has_no_secret_column() {
        let repo = AccountRepository::open_in_memory().unwrap();
        let mut stmt = repo.conn.prepare("PRAGMA table_info(transfers)").unwrap();
        let names: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(names, TRANSFER_COLUMNS);
        for name in &names {
            let lower = name.to_ascii_lowercase();
            assert!(
                !lower.contains("secret") && !lower.contains("password") && lower != "sk",
                "transfers 表出现疑似 Secret 列：{name}"
            );
        }
    }
}

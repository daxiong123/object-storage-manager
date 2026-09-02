//! SQLite 持久化（`~/Library/Application Support/CloudStorage/`），永不存 Secret
//!
//! 红线（agents.md §6 / spec §18/§58）：
//! - Secret 只进 macOS Keychain，本 crate 的任何表**不建 Secret 列**，
//!   并有 schema 回归测试把守（`schema_has_no_secret_column`）
//! - 默认路径符合 macOS 规范（`dirs::data_dir()`），不乱写 `~/.app/`

mod accounts;

pub use accounts::{AccountRepository, PersistenceError, default_db_path};

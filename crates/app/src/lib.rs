//! Application Services：账号/任务编排等领域服务
//!
//! 职责（agents.md §4）：组合 persistence（SQLite 元数据）与 macos（Keychain
//! Secret），对上层 UI 暴露领域操作；保证两边数据的一致性由本层编排。

mod account_service;
mod services;

pub use account_service::{AccountError, AccountService};
pub use object_storage_persistence::PersistedTransfer;
pub use services::{AppServices, AppServicesError};

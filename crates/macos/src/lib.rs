//! macOS native integration：Keychain / NSPanel / NSWorkspace / QuickLook / Pasteboard / 通知
//!
//! 依赖方向：Security.framework（Rust binding：security-framework 3）等系统框架。
//! 原则：GPUI 负责主 UI，macOS Framework 负责系统级能力（agents.md §3/§5）。

pub mod keychain;

pub use keychain::{KEYCHAIN_SERVICE, KeychainCredentialStore, KeychainError};

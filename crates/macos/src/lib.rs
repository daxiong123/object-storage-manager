//! macOS native integration：Keychain / NSWorkspace / QuickLook / Pasteboard / 通知
//!
//! 依赖方向：Security.framework（Rust binding：security-framework 3）、AppKit
//! （objc 0.2）、Network.framework（NWPathMonitor，C API）等系统框架。
//! 原则：GPUI 负责主 UI（含文件对话框 `cx.prompt_for_new_path`，自建 NSPanel
//! 会在事件处理器内重入 gpui 借用而闪退，见 docs/notes/gpui-api-notes.md），
//! macOS Framework 负责系统级能力（agents.md §3/§5）。

pub mod keychain;
pub mod system_events;

pub use keychain::{KEYCHAIN_SERVICE, KeychainCredentialStore, KeychainError};
pub use system_events::{EventCallback, start_network_monitor, start_sleep_wake_monitor};

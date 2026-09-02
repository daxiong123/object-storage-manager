#![allow(unexpected_cfgs)]

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

/// 交给 macOS 默认应用打开本地文件（预览/编辑入口）。
/// 使用 NSWorkspace，不通过 shell 拼接路径，避免路径注入与空格转义问题。
pub fn open_with_default_app(path: &std::path::Path) -> Result<(), String> {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    use std::ffi::CString;

    let path = path
        .to_str()
        .ok_or_else(|| format!("文件路径不是有效 UTF-8：{}", path.display()))?;
    let path = CString::new(path).map_err(|_| "文件路径包含 NUL 字节".to_string())?;
    unsafe {
        let ns_path: *mut Object = msg_send![class!(NSString), alloc];
        let ns_path: *mut Object = msg_send![ns_path, initWithUTF8String: path.as_ptr()];
        if ns_path.is_null() {
            return Err("创建 macOS 文件路径对象失败".into());
        }
        let url: *mut Object = msg_send![class!(NSURL), fileURLWithPath: ns_path];
        if url.is_null() {
            return Err("创建 macOS 文件 URL 失败".into());
        }
        let workspace: *mut Object = msg_send![class!(NSWorkspace), sharedWorkspace];
        if workspace.is_null() {
            return Err("获取 NSWorkspace 失败".into());
        }
        let opened: bool = msg_send![workspace, openURL: url];
        if opened {
            Ok(())
        } else {
            Err(format!(
                "macOS 没有可打开文件的应用：{}",
                path.to_string_lossy()
            ))
        }
    }
}

//! 原生 NSSavePanel：下载对象的「存储为」对话框。
//!
//! 约束（agents.md §5）：AppKit UI 只能在主线程调用；本函数由 gpui 主线程
//! 的 on_click / on_action 直接调用。用户取消（Cancel / Esc）返回 `None`，
//! 这是正常流程不是错误，调用方静默返回即可。

// objc 0.2 的 msg_send! 宏内部会检查 `cfg(feature = "cargo-clippy")`（宏的
// 兼容性开关），在 rustc 1.80+ 的 check-cfg 机制下产生已知的误报警告。
// 本模块集中承载全部 msg_send!，就地静音，避免污染整个 workspace。
#![allow(unexpected_cfgs)]

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::PathBuf;

use objc::class;
use objc::msg_send;
use objc::runtime::Object;
use objc::sel;
use objc::sel_impl;

/// NSModalResponseOK 的值（AppKit 头文件：NSModalResponseOK = 1）
const NS_MODAL_RESPONSE_OK: isize = 1;

/// 弹出系统「存储为」面板，返回用户选定的目标文件路径。
///
/// * `suggested_name`：建议文件名（对象 Key 的最后一段）
/// * 返回 `None`：用户取消（调用方直接返回，不要报错）
///
/// # Panics
/// NSSavePanel 创建失败或确认后拿不到路径时 panic——这属于环境异常，
/// 按约定 Fail Fast，不静默降级。
pub fn run_save_panel(suggested_name: &str) -> Option<PathBuf> {
    objc::rc::autoreleasepool(|| unsafe {
        let panel: *mut Object = msg_send![class!(NSSavePanel), savePanel];
        assert!(
            !panel.is_null(),
            "run_save_panel: NSSavePanel 创建失败（非主线程调用？）"
        );

        // NSString 不引入 objc-foundation 依赖，直接走 stringWithUTF8String:
        let c_name = CString::new(suggested_name).expect("建议文件名不应包含 NUL");
        let name: *mut Object = msg_send![class!(NSString), stringWithUTF8String: c_name.as_ptr()];
        assert!(!name.is_null(), "run_save_panel: NSString 创建失败");
        let _: () = msg_send![panel, setCanCreateDirectories: true];
        let _: () = msg_send![panel, setNameFieldStringValue: name];

        let modal_response: isize = msg_send![panel, runModal];
        if modal_response != NS_MODAL_RESPONSE_OK {
            return None;
        }

        let url: *mut Object = msg_send![panel, URL];
        assert!(
            !url.is_null(),
            "run_save_panel: 用户确认后 URL 为空，属异常状态"
        );
        let path: *mut Object = msg_send![url, path];
        assert!(
            !path.is_null(),
            "run_save_panel: 用户确认后 path 为空，属异常状态"
        );
        let utf8: *const c_char = msg_send![path, UTF8String];
        assert!(!utf8.is_null(), "run_save_panel: path 转UTF8 失败");
        Some(PathBuf::from(
            CStr::from_ptr(utf8).to_string_lossy().into_owned(),
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 不真正弹窗（会阻塞 CI），只验证常量与模块可编译。
    #[test]
    fn modal_response_ok_is_one() {
        assert_eq!(NS_MODAL_RESPONSE_OK, 1);
    }
}

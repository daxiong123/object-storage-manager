//! macOS 原生「关于」面板（agents.md §3：系统已提供的能力不要重新实现）。
//!
//! 参照实现（oss-browser2，Electron）的「关于」用的就是**系统面板**——它主菜单里
//! 只有 `role: 'about'` 一项，两份 bundle 里都没有自建对话框。所以我们也换成原生：
//! `NSApplication.orderFrontStandardAboutPanelWithOptions:`。
//!
//! 为什么用 **WithOptions** 而不是无参版本：无参版本读 bundle 的 `Info.plist`，
//! 而开发期（`cargo run`）是裸二进制、没有 bundle，会显示可执行文件名和空版本号。
//! 显式传值后，打好的 `.app` 与开发期表现一致。

use std::ffi::CString;

use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};

/// 建一个 NSString（所有权交给 autorelease 池；面板会自行 retain）。
///
/// # Safety
/// 必须在主线程调用（AppKit 要求）。
unsafe fn ns_string(value: &str) -> Result<*mut Object, String> {
    let c = CString::new(value).map_err(|_| "关于面板文本含 NUL 字节".to_string())?;
    unsafe {
        let s: *mut Object = msg_send![class!(NSString), alloc];
        let s: *mut Object = msg_send![s, initWithUTF8String: c.as_ptr()];
        if s.is_null() {
            return Err("创建 NSString 失败".into());
        }
        Ok(s)
    }
}

/// 显示系统「关于」面板。
///
/// `version` 是用户看到的版本号（如 `0.2.0`）。AppKit 的
/// `NSAboutPanelOptionApplicationVersion` 与 `NSAboutPanelOptionVersion` 在两个位置上
/// 各显示一份，这里都给同一个值，避免出现「版本号 X / 构建号空」的缺口。
pub fn show_about_panel(app_name: &str, version: &str) -> Result<(), String> {
    unsafe {
        let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
        if app.is_null() {
            return Err("取 NSApplication 失败".into());
        }
        // 面板是个窗口：应用不在前台时它会开在别的窗口后面，用户看不到。
        let _: () = msg_send![app, activateIgnoringOtherApps: true];

        let dict: *mut Object = msg_send![class!(NSMutableDictionary), dictionary];
        if dict.is_null() {
            return Err("创建选项字典失败".into());
        }
        let options = [
            ("ApplicationName", app_name),
            ("ApplicationVersion", version),
            ("Version", version),
        ];
        for (key_text, value) in options {
            let key = ns_string(key_text)?;
            let value = ns_string(value)?;
            let _: () = msg_send![dict, setObject: value forKey: key];
        }
        let _: () = msg_send![app, orderFrontStandardAboutPanelWithOptions: dict];
        Ok(())
    }
}

//! NSPasteboard（规范 §21：Copy URL / Signed URL；签名链接可 N 秒后自动清除）
//!
//! 直接走 AppKit `NSPasteboard`，不经 shell / pbcopy。类型用
//! `public.utf8-plain-text`（即 `NSPasteboardTypeString`）。

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};
use std::ffi::{CStr, CString};

const PASTEBOARD_TYPE_UTF8: &[u8] = b"public.utf8-plain-text\0";

fn pasteboard_type() -> *mut Object {
    unsafe { msg_send![class!(NSString), stringWithUTF8String: PASTEBOARD_TYPE_UTF8.as_ptr()] }
}

fn ns_string(text: &str) -> Result<*mut Object, String> {
    let cstr = CString::new(text).map_err(|_| "剪贴板文本包含 NUL 字节".to_string())?;
    unsafe {
        let ns: *mut Object = msg_send![class!(NSString), alloc];
        let ns: *mut Object = msg_send![ns, initWithUTF8String: cstr.as_ptr()];
        if ns.is_null() {
            return Err("创建 NSString 失败".into());
        }
        Ok(ns)
    }
}

fn general_pasteboard() -> Result<*mut Object, String> {
    unsafe {
        let pb: *mut Object = msg_send![class!(NSPasteboard), generalPasteboard];
        if pb.is_null() {
            return Err("获取 NSPasteboard 失败".into());
        }
        Ok(pb)
    }
}

/// 把纯文本写入系统剪贴板（先 `clearContents`）。
pub fn copy_text(text: &str) -> Result<(), String> {
    let ns = ns_string(text)?;
    let pb = general_pasteboard()?;
    let ty = pasteboard_type();
    if ty.is_null() {
        return Err("创建剪贴板类型失败".into());
    }
    unsafe {
        let _: i64 = msg_send![pb, clearContents];
        let ok: bool = msg_send![pb, setString: ns forType: ty];
        if ok {
            Ok(())
        } else {
            Err("写入 NSPasteboard 失败".into())
        }
    }
}

/// 读取当前纯文本剪贴板。空或非文本返回 `None`。
pub fn read_text() -> Result<Option<String>, String> {
    let pb = general_pasteboard()?;
    let ty = pasteboard_type();
    if ty.is_null() {
        return Err("创建剪贴板类型失败".into());
    }
    unsafe {
        let ns: *mut Object = msg_send![pb, stringForType: ty];
        if ns.is_null() {
            return Ok(None);
        }
        let ptr: *const std::os::raw::c_char = msg_send![ns, UTF8String];
        if ptr.is_null() {
            return Ok(None);
        }
        let text = CStr::from_ptr(ptr)
            .to_str()
            .map_err(|_| "剪贴板内容不是有效 UTF-8".to_string())?
            .to_string();
        Ok(Some(text))
    }
}

/// 若剪贴板仍等于 `text`（签名 URL 尚未被用户覆盖），则清空。
pub fn clear_if_equals(text: &str) -> Result<(), String> {
    match read_text()? {
        Some(current) if current == text => {
            let pb = general_pasteboard()?;
            unsafe {
                let _: i64 = msg_send![pb, clearContents];
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_and_clear_if_equals_round_trip() {
        let marker = "cloudstorage-clipboard-test-do-not-use";
        copy_text(marker).expect("写入剪贴板");
        assert_eq!(read_text().unwrap().as_deref(), Some(marker));
        clear_if_equals(marker).expect("按值清空");
        assert_ne!(read_text().unwrap().as_deref(), Some(marker));
    }
}

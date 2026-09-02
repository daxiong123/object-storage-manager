//! 系统 Quick Look 面板（规范 §12/§13：复杂格式不自建 Preview Engine）
//!
//! 使用 `QLPreviewPanel`（QuickLookUI.framework）。NSURL 已实现
//! `QLPreviewItem`，数据源只负责返回当前文件 URL。
//! 面板是系统单例；数据源与当前 URL 进程内常驻，切换文件时替换 URL。

use std::ffi::CString;
use std::path::Path;
use std::ptr;
use std::sync::{Mutex, OnceLock};

use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use objc::{class, msg_send, sel, sel_impl};

#[link(name = "Quartz", kind = "framework")]
unsafe extern "C" {}

static DATASOURCE_CLASS: OnceLock<usize> = OnceLock::new();
static DATASOURCE: OnceLock<usize> = OnceLock::new();
/// 当前 QLPreviewItem（NSURL*），0 = 无
static CURRENT_ITEM: Mutex<usize> = Mutex::new(0);

extern "C" fn number_of_items(_this: &Object, _cmd: Sel, _panel: *mut Object) -> i64 {
    let item = *CURRENT_ITEM.lock().expect("Quick Look 当前条目锁被毒化");
    if item == 0 { 0 } else { 1 }
}

extern "C" fn item_at(_this: &Object, _cmd: Sel, _panel: *mut Object, _index: i64) -> *mut Object {
    *CURRENT_ITEM.lock().expect("Quick Look 当前条目锁被毒化") as *mut Object
}

fn datasource_class() -> &'static Class {
    let ptr = DATASOURCE_CLASS.get_or_init(|| {
        let mut decl = ClassDecl::new("CloudStorageQLDataSource", class!(NSObject))
            .expect("注册 Quick Look 数据源类失败（类名冲突？）");
        unsafe {
            decl.add_method(
                sel!(numberOfPreviewItemsInPreviewPanel:),
                number_of_items as extern "C" fn(&Object, Sel, *mut Object) -> i64,
            );
            decl.add_method(
                sel!(previewPanel:previewItemAtIndex:),
                item_at as extern "C" fn(&Object, Sel, *mut Object, i64) -> *mut Object,
            );
        }
        decl.register() as *const Class as usize
    });
    unsafe { &*(*ptr as *const Class) }
}

fn datasource() -> *mut Object {
    let ptr = DATASOURCE.get_or_init(|| {
        let cls = datasource_class();
        unsafe {
            let obj: *mut Object = msg_send![cls, alloc];
            assert!(!obj.is_null(), "分配 Quick Look 数据源失败");
            let obj: *mut Object = msg_send![obj, init];
            assert!(!obj.is_null(), "初始化 Quick Look 数据源失败");
            obj as usize
        }
    });
    *ptr as *mut Object
}

/// 用系统 Quick Look 面板预览本地文件。
pub fn quick_look(path: &Path) -> Result<(), String> {
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
        let _: *mut Object = msg_send![url, retain];
        {
            let mut slot = CURRENT_ITEM.lock().expect("Quick Look 当前条目锁被毒化");
            if *slot != 0 {
                let old = *slot as *mut Object;
                let _: () = msg_send![old, release];
            }
            *slot = url as usize;
        }

        let panel: *mut Object = msg_send![class!(QLPreviewPanel), sharedPreviewPanel];
        if panel.is_null() {
            return Err("获取系统 Quick Look 面板失败".into());
        }
        let _: () = msg_send![panel, setDataSource: datasource()];
        let _: () = msg_send![panel, reloadData];
        let _: () = msg_send![panel, makeKeyAndOrderFront: ptr::null_mut::<Object>()];
        Ok(())
    }
}

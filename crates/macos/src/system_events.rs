//! 系统事件接线：睡眠/唤醒（NSWorkspace）+ 网络通断（NWPathMonitor）
//!
//! 规范依据（spec §25/§26，传输 P0）：
//! - 系统 Sleep / 断网 → 传输挂起（任务转 `Waiting`），**不得**误标 `Failed`
//! - 恢复以「网络重新 satisfied」为准：唤醒瞬间网络常未就绪，若在
//!   didWake 立即 resume 会把重排队任务打成 Failed —— 因此 didWake 故意不动
//! - 请求层每次重试都重建 Provider 与连接（重新连接/重新 DNS，§26），
//!   由 transfer 引擎的 runner 注入机制天然满足
//!
//! 回调约定：闭包可能在任意线程被调用（NSWorkspace 通知主线程投递、
//! NWPathMonitor 走自建 dispatch 队列），必须 `Send + Sync`、非阻塞、
//! 不得触碰 gpui 实体。引擎 `TransferEngine: Send + Sync`，UI 状态经
//! watch 订阅自动刷新，回调里只调引擎即可。
//!
//! 系统事实均验证自本机 SDK 头文件 / 运行时实测（勿凭记忆改）：
//! - 通知名常量 `NSWorkspaceWillSleepNotification` / `NSWorkspaceDidWakeNotification`
//!   （NSWorkspace.h:322-323）；字符串值 = 常量名（Swift 运行时实测）
//! - `nw_path_status_t`：invalid=0 satisfied=1 unsatisfied=2 satisfiable=3
//!   （nw_path.h:52-58）
//! - `nw_path_monitor_create / set_queue / set_update_handler / start`
//!   （path_monitor.h：set_queue/set_update_handler 必须在 start 前调用；
//!   update handler 在 start 后回调一次，之后路径每次变化再回调）
//! - `nw_path_get_status` 对 NULL path 返回 invalid（nw_path.h:73）

use std::mem;
use std::os::raw::c_void;
use std::ptr;
use std::sync::OnceLock;

use block::ConcreteBlock;
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use objc::{class, msg_send, sel, sel_impl};

/// 系统事件回调：非阻塞、`Send + Sync`、不得触碰 gpui 实体（见模块文档）
pub type EventCallback = Box<dyn Fn() + Send + Sync + 'static>;

// ======================================================================
// 睡眠/唤醒（NSWorkspace 通知，objc 0.2 手搓观察者）
// ======================================================================

/// 观察者类的静态注册（类指针存 usize：`objc::runtime::Class` 不是 Sync，
/// 不能直接进 static；usize 是 Send + Sync 的）
static OBSERVER_CLASS: OnceLock<usize> = OnceLock::new();

const SLEEP_IVAR: &str = "cloud_sleep_cb";
const WAKE_IVAR: &str = "cloud_wake_cb";

/// 回调 ivar 里存的堆 Box 指针类型
type CbPtr = *const EventCallback;

extern "C" fn handle_sleep(this: &Object, _cmd: Sel, _notif: *mut Object) {
    let slot: &usize = unsafe { this.get_ivar(SLEEP_IVAR) };
    let cb = *slot as CbPtr;
    assert!(!cb.is_null(), "睡眠回调 ivar 未初始化");
    unsafe { (*cb)() };
}

extern "C" fn handle_wake(this: &Object, _cmd: Sel, _notif: *mut Object) {
    let slot: &usize = unsafe { this.get_ivar(WAKE_IVAR) };
    let cb = *slot as CbPtr;
    assert!(!cb.is_null(), "唤醒回调 ivar 未初始化");
    unsafe { (*cb)() };
}

fn observer_class() -> &'static Class {
    let ptr = OBSERVER_CLASS.get_or_init(|| {
        let mut decl = ClassDecl::new("CloudStorageSystemEventObserver", class!(NSObject))
            .expect("注册系统事件观察者类失败（类名冲突？）");
        decl.add_ivar::<usize>(SLEEP_IVAR);
        decl.add_ivar::<usize>(WAKE_IVAR);
        unsafe {
            decl.add_method(
                sel!(handleSleepNotification:),
                handle_sleep as extern "C" fn(&Object, Sel, *mut Object),
            );
            decl.add_method(
                sel!(handleWakeNotification:),
                handle_wake as extern "C" fn(&Object, Sel, *mut Object),
            );
        }
        decl.register() as *const Class as usize
    });
    unsafe { &*(*ptr as *const Class) }
}

// AppKit 通知名常量（NSWorkspace.h:322-323，`APPKIT_EXTERN NSNotificationName`）。
// 直接引用导出符号：既是常量本体（无需手搭 NSString），也让链接器把
// AppKit 装入本进程——测试二进制没有 gpui 帮我们链接 AppKit，
// `class!(NSWorkspace)` 才能找到类。
#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    static NSWorkspaceWillSleepNotification: *mut Object;
    static NSWorkspaceDidWakeNotification: *mut Object;
}

/// 订阅系统睡眠/唤醒。
///
/// - `on_sleep`：系统即将睡眠（`NSWorkspaceWillSleepNotification`）
/// - `on_wake`：系统已唤醒（`NSWorkspaceDidWakeNotification`）
///
/// 调用方注意（传输 P0）：唤醒回调**不应**直接恢复传输，理由见模块文档——
/// 恢复必须以「网络重新 satisfied」为准（见 [`start_network_monitor`]）。
/// 通知投递在主线程；回调内禁止重入 gpui。
pub fn start_sleep_wake_monitor(on_sleep: EventCallback, on_wake: EventCallback) {
    let cls = observer_class();
    unsafe {
        let observer: *mut Object = msg_send![cls, alloc];
        assert!(!observer.is_null(), "分配系统事件观察者失败");
        let observer: *mut Object = msg_send![observer, init];
        assert!(!observer.is_null(), "初始化系统事件观察者失败");
        // Box::into_raw 有意泄漏：观察者与进程同寿命（NSNotificationCenter
        // retain 观察者，ivar 里的回调随之常驻）
        (*observer).set_ivar(SLEEP_IVAR, Box::into_raw(Box::new(on_sleep)) as usize);
        (*observer).set_ivar(WAKE_IVAR, Box::into_raw(Box::new(on_wake)) as usize);

        let workspace: *mut Object = msg_send![class!(NSWorkspace), sharedWorkspace];
        assert!(!workspace.is_null(), "获取 NSWorkspace 失败");
        let center: *mut Object = msg_send![workspace, notificationCenter];
        assert!(
            !center.is_null(),
            "获取 NSWorkspace notificationCenter 失败"
        );

        let _: () = msg_send![center,
            addObserver: observer
            selector: sel!(handleSleepNotification:)
            name: NSWorkspaceWillSleepNotification
            object: ptr::null_mut::<Object>()
        ];
        let _: () = msg_send![center,
            addObserver: observer
            selector: sel!(handleWakeNotification:)
            name: NSWorkspaceDidWakeNotification
            object: ptr::null_mut::<Object>()
        ];
    }
}

// ======================================================================
// 网络通断（NWPathMonitor C API + block crate）
// ======================================================================

mod network_sys {
    use std::os::raw::{c_char, c_void};

    /// nw_path_status_t（nw_path.h:52-58，SDK 头文件逐值核对）
    pub const NW_PATH_STATUS_SATISFIED: u32 = 1;

    #[link(name = "Network", kind = "framework")]
    unsafe extern "C" {
        /// NW_RETURNS_RETAINED：返回 +1，本模块有意泄漏（进程同寿命）
        pub fn nw_path_monitor_create() -> *mut c_void;
        pub fn nw_path_monitor_set_queue(monitor: *mut c_void, queue: *mut c_void);
        pub fn nw_path_monitor_set_update_handler(monitor: *mut c_void, handler: *mut c_void);
        pub fn nw_path_monitor_start(monitor: *mut c_void);
        pub fn nw_path_get_status(path: *mut c_void) -> u32;
    }

    #[link(name = "System", kind = "dylib")]
    unsafe extern "C" {
        pub fn dispatch_queue_create(label: *const c_char, attr: *mut c_void) -> *mut c_void;
    }
}

/// 订阅网络路径变化（NWPathMonitor，macOS 10.14+）。
///
/// - `on_up`：路径变为 `satisfied`（可用路由）
/// - `on_down`：路径变为其他状态（`unsatisfied`/`satisfiable`/`invalid`；
///   satisfiable 当前无可用路由，保守按断网处理，satisfied 回来即恢复）
///
/// start 后 handler 立即回调一次当前状态（path_monitor.h 官方语义），
/// 之后路径每次变化再回调。回调与引擎幂等操作（suspend_all/resume_all）
/// 直接配合即可，无需去重。
pub fn start_network_monitor(on_down: EventCallback, on_up: EventCallback) {
    unsafe {
        let monitor = network_sys::nw_path_monitor_create();
        assert!(!monitor.is_null(), "创建 NWPathMonitor 失败");
        let queue =
            network_sys::dispatch_queue_create(c"cloudstorage.nwpath".as_ptr(), ptr::null_mut());
        assert!(!queue.is_null(), "创建 dispatch 队列失败");
        network_sys::nw_path_monitor_set_queue(monitor, queue);

        // handler block 与 monitor 同寿命（进程级）；copy 到堆后 forget 保活。
        // satisfied → up，其余状态 → down（理由见函数文档）
        let handler = ConcreteBlock::new(move |path: *mut c_void| {
            let status = network_sys::nw_path_get_status(path);
            if status == network_sys::NW_PATH_STATUS_SATISFIED {
                (on_up)();
            } else {
                (on_down)();
            }
        })
        .copy();
        network_sys::nw_path_monitor_set_update_handler(
            monitor,
            &*handler as *const _ as *mut c_void,
        );
        network_sys::nw_path_monitor_start(monitor);
        mem::forget(handler);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    /// 观察者类经 OnceLock 只注册一次；重复调用 start 不 panic。
    /// 同时验证 AppKit 链接批注在纯测试二进制里也能拉起 NSWorkspace 类。
    #[test]
    fn sleep_wake_monitor_registration_idempotent() {
        start_sleep_wake_monitor(Box::new(|| {}), Box::new(|| {}));
        start_sleep_wake_monitor(Box::new(|| {}), Box::new(|| {}));
    }

    /// NWPathMonitor 全链路冒烟：start 后官方保证至少回调一次初始状态
    /// （不要求具体 up/down——取决于测试机当时网络，断言 ≥1 次即可）
    #[test]
    fn network_monitor_fires_initial_state() {
        let counter = Arc::new(AtomicUsize::new(0));
        let c_down = Arc::clone(&counter);
        let c_up = Arc::clone(&counter);
        start_network_monitor(
            Box::new(move || {
                c_down.fetch_add(1, Ordering::SeqCst);
            }),
            Box::new(move || {
                c_up.fetch_add(1, Ordering::SeqCst);
            }),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if counter.load(Ordering::SeqCst) >= 1 {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("NWPathMonitor 启动后 5s 内未回调初始状态");
    }
}

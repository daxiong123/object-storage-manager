//! 退出与关窗：⌘Q 的传输确认、⌘W 的关窗动画规避。

// 本文件 handle_close_window 里的 objc msg_send! 宏内部会检查
// `cfg(feature = "cargo-clippy")`（宏兼容性开关），在 rustc 1.80+ 的
// check-cfg 机制下产生已知误报警告；就地静音（先例：gpui 平台层自身
// 也如此封装 objc 调用，文件对话框等直接用 gpui API，不自建 NSPanel）。
#![allow(unexpected_cfgs)]
use super::*;

impl WorkspaceView {
    pub(super) fn handle_quit(&mut self, _: &Quit, window: &mut Window, cx: &mut Context<Self>) {
        if self.quit_prompt_open {
            return;
        }
        let active: Vec<TransferTask> = self
            .engine
            .snapshot()
            .into_iter()
            .filter(|t| t.state.is_active())
            .collect();
        if active.is_empty() {
            cx.quit();
            return;
        }
        self.quit_prompt_open = true;
        let n = active.len();
        let message = format!("有 {n} 个传输任务尚未完成。");
        let rx = window.prompt(
            PromptLevel::Warning,
            &message,
            Some("暂停并退出会保存队列，下次启动后继续；立即退出会丢弃未完成任务。"),
            &[
                PromptButton::ok("暂停并退出"),
                PromptButton::cancel("取消"),
                PromptButton::new("立即退出"),
            ],
            cx,
        );
        let engine = Arc::clone(&self.engine);
        let services = Arc::clone(&self.services);
        cx.spawn(async move |this, cx| {
            let answer = match rx.await {
                Ok(i) => i,
                Err(_) => {
                    this.update(cx, |this, _| this.quit_prompt_open = false)
                        .ok();
                    return;
                }
            };
            match answer {
                0 => {
                    engine.suspend_all();
                    let items = persistable_from_snapshot(&engine.snapshot());
                    let services = Arc::clone(&services);
                    let result = cx
                        .background_executor()
                        .spawn(async move { services.replace_transfers(&items) })
                        .await;
                    match result {
                        Ok(()) => {
                            cx.update(|cx| cx.quit());
                        }
                        Err(e) => {
                            this.update(cx, |this, cx| {
                                this.quit_prompt_open = false;
                                this.download_message = Some(DownloadMessage {
                                    is_error: true,
                                    text: format!("保存传输队列失败，未退出：{e}"),
                                });
                                cx.notify();
                            })
                            .ok();
                        }
                    }
                }
                2 => {
                    let services = Arc::clone(&services);
                    let result = cx
                        .background_executor()
                        .spawn(async move { services.clear_transfers() })
                        .await;
                    match result {
                        Ok(()) => {
                            cx.update(|cx| cx.quit());
                        }
                        Err(e) => {
                            this.update(cx, |this, cx| {
                                this.quit_prompt_open = false;
                                this.download_message = Some(DownloadMessage {
                                    is_error: true,
                                    text: format!("清除已保存队列失败，未退出：{e}"),
                                });
                                cx.notify();
                            })
                            .ok();
                        }
                    }
                }
                _ => {
                    this.update(cx, |this, _| this.quit_prompt_open = false)
                        .ok();
                }
            }
        })
        .detach();
    }

    pub(super) fn handle_close_window(
        &mut self,
        _: &CloseWindow,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        // 关闭窗口（macOS 15 上的绕行方案，升级 gpui-pre 0.3.4 后仍保留；
        // 实测记录见 docs/notes/gpui-api-notes.md）：
        //
        // 根因：macOS 15 上 NSWindow close 默认带窗口动画，而 gpui 的 MacWindow::drop 会在
        // close 后毫秒级 autorelease（dealloc），把动画中途杀死，窗口卡在可见状态——表现为
        // close() 永远关不掉窗口。
        //
        // 修复：先禁用 close 动画（NSWindowAnimationBehaviorNone），再 remove_window()
        // （注册表清理 → drop → gpui 内部 close 任务）。无动画的 [super close] 退化为
        // 纯 orderOut，窗口正常消失，进程与 gpui 窗口注册表状态一致。
        use objc::{msg_send, sel, sel_impl};
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let raw = match window.window_handle() {
            Ok(h) => h.as_raw(),
            Err(_) => return,
        };
        let ns_view = match raw {
            RawWindowHandle::AppKit(h) => h.ns_view.as_ptr(),
            _ => return, // macOS 上不可能走到
        };
        unsafe {
            let view = ns_view as *mut objc::runtime::Object;
            let win: *mut objc::runtime::Object = msg_send![view, window];
            if !win.is_null() {
                let _: () = msg_send![win, setAnimationBehavior: 1i64];
            }
        }
        window.remove_window();
    }
}

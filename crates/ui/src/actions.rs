//! 应用级 Action 与快捷键定义。
//!
//! 规范 §11/§22：同一 Action 在 菜单 / 快捷键 / 工具栏 / 右键菜单 中共享。
//! 本模块是 Action 的唯一注册点；键位在这里统一绑定（⌘ 符号写法按规范 §26）。

use gpui::{App, KeyBinding, actions};

actions!(
    cloud_storage,
    [Quit, CloseWindow, ToggleSidebar, ToggleInspector,]
);

// Edit 菜单专用：通过 `MenuItem::os_action` 触发 macOS 原生编辑行为。
//
// 注意：不绑定全局按键——文本输入场景的 ⌘X/⌘C/⌘V 由输入组件与系统响应链处理，
// 全局绑定会吞掉按键、破坏原生行为。
actions!(cloud_storage, [Undo, Redo, Cut, Copy, Paste, SelectAll]);

/// 注册全局快捷键。
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        // 规范 §7：Sidebar ⌘⌥S、Inspector ⌘⌥I（菜单显示 ⌘ 符号，不是 "Cmd"）
        KeyBinding::new("cmd-alt-s", ToggleSidebar, None),
        KeyBinding::new("cmd-alt-i", ToggleInspector, None),
    ]);
}

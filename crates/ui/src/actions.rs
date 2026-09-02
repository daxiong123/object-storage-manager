//! 应用级 Action 与快捷键定义。
//!
//! 规范 §11/§22：同一 Action 在 菜单 / 快捷键 / 工具栏 / 右键菜单 中共享。
//! 本模块是 Action 的唯一注册点；键位在这里统一绑定（⌘ 符号写法按规范 §26）。

use gpui::{App, KeyBinding, actions};

actions!(
    cloud_storage,
    [
        Quit,
        CloseWindow,
        ToggleSidebar,
        ToggleInspector,
        OpenCommandPalette,
        // 添加账号：侧栏「+ 添加账号」入口与命令面板共享（规范 §11/§22）
        AddAccount,
        // 下载选中对象：Inspector 按钮 / 「对象」菜单 / 命令面板三入口共享
        DownloadObject,
    ]
);

// 命令面板（⌘K，规范 §22）内部导航：仅通过 context "Palette" 生效（见 bind_keys），
// 避免与 Input / List 等组件的同键位 Action 冲突。
actions!(
    cloud_storage,
    [PaletteClose, PaletteSelectPrev, PaletteSelectNext,]
);

// 自建模态（添加账号等）的关闭：仅通过 context "AccountModal" 生效。
// 输入框未处理 Esc 时会 propagate 到这里（与命令面板同一机制）。
actions!(cloud_storage, [DismissModal]);

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
        // 命令面板：⌘K 全局打开；↑↓/Esc 收窄到 context "Palette"——
        // 无 context 的绑定按 keymap 深度规则会压过组件（如 Input）的同键绑定。
        // 方向键能用的前提：单行 Input 只在 multi_line 下注册 MoveUp/MoveDown，
        // 未处理时 keymap 会沿绑定列表落到这里的 PaletteSelectPrev/Next。
        KeyBinding::new("cmd-k", OpenCommandPalette, None),
        KeyBinding::new("escape", PaletteClose, Some("Palette")),
        KeyBinding::new("up", PaletteSelectPrev, Some("Palette")),
        KeyBinding::new("down", PaletteSelectNext, Some("Palette")),
        KeyBinding::new("escape", DismissModal, Some("AccountModal")),
    ]);
}

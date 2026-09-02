//! 应用级 Action 与快捷键定义。
//!
//! 规范 §11/§22：同一 Action 在 菜单 / 快捷键 / 工具栏 / 右键菜单 中共享。
//! 本模块是 Action 的唯一注册点；键位在这里统一绑定（⌘ 符号写法按规范 §26）。

use gpui::{App, KeyBinding, actions};

/// 带数据的 Action：跳转到指定 Bucket（命令面板动态命令用）。
/// `no_json`：仅进程内分发，不需要 schema 反序列化。
#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = cloud_storage, no_json)]
pub struct SelectBucketByName(pub String);

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
        // 上传本地文件到当前空间：⌘U / 「对象」菜单 / 命令面板 / Inspector
        UploadFiles,
        // 上传本地目录（递归文件入队）：菜单 / 命令面板 / Inspector
        UploadFolder,
        // 刷新当前视图：有空间则重载对象列表，否则刷新空间/账号（规范 ⌘R）
        Refresh,
        // 删除选中远端对象：⌘⌫ / 「对象」菜单 / 命令面板 / Inspector，必须确认
        DeleteObject,
        // 预览选中对象：Space / 命令面板
        PreviewObject,
        // 复制选中对象的签名下载链接：菜单 / 命令面板 / Inspector
        CopyObjectUrl,
        // 保存文本编辑并覆盖上传：Inspector「保存并上传」/ ⌘S（Workspace 上下文）
        SaveTextObject,
        // 全选当前对象列表：⌘A（仅 Workspace 上下文，不吞文本输入的原生响应链）
        SelectObjectAll,
        // 行内重命名：Return（仅 Workspace 上下文，Finder 式，不弹 Dialog）
        RenameObject,
        // 过滤当前对象列表：⌘F（仅 Workspace 上下文；再按 ⌘F / Esc 关闭）
        ToggleObjectFilter,
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

// 行内重命名的取消（Esc）：仅通过 context "Renaming" 生效。rename 输入框
// 未设 clean_on_escape，Esc 由 Input escape() propagate 到这里。
actions!(cloud_storage, [DismissRename]);

// 对象列表过滤的关闭（Esc）：仅通过 context "ObjectFilter" 生效。
actions!(cloud_storage, [DismissFilter]);

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
        KeyBinding::new("cmd-u", UploadFiles, None),
        KeyBinding::new("cmd-r", Refresh, None),
        // 规范 §43：⌘⌫ 删除；不绑 Delete，避免误触。面板打开时 handler 直接 return。
        KeyBinding::new("cmd-backspace", DeleteObject, None),
        KeyBinding::new("space", PreviewObject, Some("Workspace")),
        KeyBinding::new("cmd-s", SaveTextObject, Some("Workspace")),
        // 规范 §7：⌘A 全选当前对象列表。绑定在 Workspace context（非全局），
        // 命令面板/输入框聚焦时按键由组件原生响应链处理，不会被吞。
        KeyBinding::new("cmd-a", SelectObjectAll, Some("Workspace")),
        // 规范 §42：Return 进 Inline Rename（Finder 式）。绑定 Workspace context，
        // 命令面板输入框聚焦时 Return 由面板自己的 PressEnter 处理，不受影响。
        KeyBinding::new("enter", RenameObject, Some("Workspace")),
        // 规范 ⌘F：过滤当前对象列表。Workspace context 绑定，输入框聚焦时
        // 不触发（输入组件原生响应链优先）。
        KeyBinding::new("cmd-f", ToggleObjectFilter, Some("Workspace")),
        KeyBinding::new("cmd-k", OpenCommandPalette, None),
        KeyBinding::new("escape", PaletteClose, Some("Palette")),
        KeyBinding::new("up", PaletteSelectPrev, Some("Palette")),
        KeyBinding::new("down", PaletteSelectNext, Some("Palette")),
        KeyBinding::new("escape", DismissModal, Some("AccountModal")),
        KeyBinding::new("escape", DismissRename, Some("Renaming")),
        KeyBinding::new("escape", DismissFilter, Some("ObjectFilter")),
    ]);
}

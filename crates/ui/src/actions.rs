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
        OpenCommandPalette,
        // 添加账号：侧栏「+ 添加账号」入口与命令面板共享（规范 §11/§22）
        AddAccount,
        // 下载选中对象：「对象」菜单 / 命令面板入口共享
        DownloadObject,
        // 上传本地文件到当前空间：⌘U / 「对象」菜单 / 命令面板
        UploadFiles,
        // 上传本地目录（递归文件入队）：菜单 / 命令面板
        UploadFolder,
        // 刷新当前视图：有空间则重载对象列表，否则刷新空间/账号（规范 ⌘R）
        Refresh,
        // 删除选中远端对象：⌘⌫ / 「对象」菜单 / 命令面板，必须确认
        DeleteObject,
        // 预览选中对象：Space / 命令面板
        PreviewObject,
        // 复制选中对象的签名下载链接：菜单 / 命令面板
        CopyObjectUrl,
        // 保存文本编辑并覆盖上传：⌘S（Workspace 上下文）
        SaveTextObject,
        // 全选当前对象列表：⌘A（仅 Workspace 上下文，不吞文本输入的原生响应链）
        SelectObjectAll,
        // 重命名：Return（仅 Workspace 上下文）
        RenameObject,
        // 过滤当前对象列表：⌘F（仅 Workspace 上下文；再按 ⌘F / Esc 关闭）
        ToggleObjectFilter,
        // 打开设置：⌘,（菜单 / 快捷键共享）
        OpenSettings,
        // 打开关于弹窗：菜单（设置上方）/ 命令面板共享
        OpenAbout,
        // 导航历史（spec §11）：⌘[ 回退 / ⌘] 前进（桶内前缀栈，浏览器语义）
        NavigateBack,
        NavigateForward,
        // ⌘L 路径跳转（spec §11 Focus Path）：打开路径输入框，回车跳转
        FocusPath,
        // 用默认应用打开选中对象（spec §14：下载到临时目录 → NSWorkspace open）
        OpenObject,
        // 在 Finder 中显示选中对象的本地副本（spec §16）
        RevealInFinder,
    ]
);

// 自建模态（添加账号等）的关闭：仅通过 context "AccountModal" 生效。
// 输入框未处理 Esc 时会 propagate 到这里（与命令面板同一机制）。
actions!(cloud_storage, [DismissModal]);

// 命令面板必须一按 Esc 就关闭，覆盖 Command 默认的「有查询时先清空」。
actions!(cloud_storage, [DismissCommandPalette]);

// 重命名弹窗的取消（Esc）：仅通过 context "Renaming" 生效。rename 输入框
// 未设 clean_on_escape，Esc 由 Input escape() propagate 到这里。
actions!(cloud_storage, [DismissRename]);

// 对象列表过滤的关闭（Esc）：仅通过 context "ObjectFilter" 生效。
actions!(cloud_storage, [DismissFilter]);

// 无文本输入弹层（对象详情/预览）的统一关闭（Esc）：仅通过 context
// "Overlay" 生效。这些弹层没有 Input 组件，Esc 不会被输入组件拦截，
// 直接落到此绑定（弹层卡片上 on_action 注册 handler）。
actions!(cloud_storage, [UnifiedDismiss]);

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
        // 规范 §7：Sidebar ⌘⌥S（菜单显示 ⌘ 符号，不是 "Cmd"）
        KeyBinding::new("cmd-alt-s", ToggleSidebar, None),
        // 添加账号：仅 Workspace context 生效，避免模态/Input 聚焦时误触。
        KeyBinding::new("cmd-n", AddAccount, Some("Workspace")),
        // 命令面板内部过滤、↑↓/Enter 由 GPUI Kit Command 负责；Esc 见下方覆盖绑定。
        KeyBinding::new("cmd-u", UploadFiles, None),
        KeyBinding::new("cmd-r", Refresh, None),
        // 规范 §43：⌘⌫ 删除；不绑 Delete，避免误触。面板打开时 handler 直接 return。
        KeyBinding::new("cmd-backspace", DeleteObject, None),
        KeyBinding::new("space", PreviewObject, Some("Workspace")),
        KeyBinding::new("cmd-s", SaveTextObject, Some("Workspace")),
        // 规范 §7：⌘A 全选当前对象列表。绑定在 Workspace context（非全局），
        // 命令面板/输入框聚焦时按键由组件原生响应链处理，不会被吞。
        KeyBinding::new("cmd-a", SelectObjectAll, Some("Workspace")),
        // 规范 §42：Return 进重命名。绑定 Workspace context，
        // 命令面板输入框聚焦时 Return 由面板自己的 PressEnter 处理，不受影响。
        KeyBinding::new("enter", RenameObject, Some("Workspace")),
        // 规范 ⌘F：过滤当前对象列表。Workspace context 绑定，输入框聚焦时
        // 不触发（输入组件原生响应链优先）。
        KeyBinding::new("cmd-f", ToggleObjectFilter, Some("Workspace")),
        // 规范 §11：⌘[ 回退 / ⌘] 前进（桶内导航历史）。
        KeyBinding::new("cmd-[", NavigateBack, None),
        KeyBinding::new("cmd-]", NavigateForward, None),
        // 规范 §11：⌘L 聚焦路径跳转输入框。
        KeyBinding::new("cmd-l", FocusPath, None),
        // 规范 ⌘,：设置。
        KeyBinding::new("cmd-,", OpenSettings, None),
        // 规范 ⌘O：用默认应用打开选中对象。
        KeyBinding::new("cmd-o", OpenObject, None),
        KeyBinding::new("cmd-k", OpenCommandPalette, None),
        // ui::init 晚于 gpui_component::init，后注册的同 context 绑定优先。
        KeyBinding::new("escape", DismissCommandPalette, Some("Command")),
        KeyBinding::new("escape", DismissModal, Some("AccountModal")),
        // 设置弹窗与添加账号共用 DismissModal action，context 不同：
        // 保存进行中由 SettingsModal::close 自身拒绝关闭。
        KeyBinding::new("escape", DismissModal, Some("SettingsModal")),
        KeyBinding::new("escape", DismissRename, Some("Renaming")),
        KeyBinding::new("escape", DismissFilter, Some("ObjectFilter")),
        KeyBinding::new("escape", UnifiedDismiss, Some("Overlay")),
    ]);
}

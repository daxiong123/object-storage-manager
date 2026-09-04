//! 主窗口三栏 Workspace。
//!
//! 结构（agents.md §7）：Unified Titlebar + Sidebar(180/220/360) + Content + Inspector(280/320/520)。
//! - Sidebar 折叠为 44px 图标栏（规范硬指标；gpui-component 自带 Sidebar 固定 255px/48px，
//!   无法满足，故自建，用其 Icon/主题 token 保持视觉一致）。
//! - 三栏宽度用 gpui-component Resizable；折叠/展开切换布局变体（不同的 resizable group id），
//!   使每种变体各自记住拖拽后的宽度。
//! - Action 处理见 `crate::actions`：⌘⌥S / ⌘⌥I / ⌘W / ⌘Q 与菜单共享同一 Action。
//!
//! 数据接线（里程碑 c）：Sidebar 渲染真实账号/空间，Content 渲染选中 Bucket 的
//! 对象列表。所有 IO（SQLite/Keychain/网络）经 `AppServices` 的阻塞方法丢进 gpui
//! 后台执行器（`background_executor().spawn`），窗口显示永不被 IO 阻塞；
//! UI 状态更新统一回主线程 `this.update` + `cx.notify()`。
//!
//! 串台防护：每次异步加载携带自增代号（generation）。用户快速切换账号/桶时，
//! 过期任务的结果因代号不匹配被丢弃，不会覆盖新选中项的状态。

// 本文件 handle_close_window 里的 objc msg_send! 宏内部会检查
// `cfg(feature = "cargo-clippy")`（宏兼容性开关），在 rustc 1.80+ 的
// check-cfg 机制下产生已知误报警告；就地静音（先例：gpui 平台层自身
// 也如此封装 objc 调用，文件对话框等直接用 gpui API，不自建 NSPanel）。
#![allow(unexpected_cfgs)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, ExternalPaths, FocusHandle, Img,
    InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent, ObjectFit,
    ParentElement as _, PathPromptOptions, Pixels, PromptButton, PromptLevel, Render, SharedString,
    StatefulInteractiveElement as _, Styled, StyledImage as _, Window, div, hsla, img,
    prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme, Disableable as _, Icon, IconName, Sizable, Size, Theme, TitleBar, button::Button,
    button::ButtonVariants as _, h_flex, input::Input, input::InputEvent, input::InputState,
    progress::Progress, resizable::h_resizable, resizable::resizable_panel,
    scroll::ScrollableElement, spinner::Spinner, v_flex,
};

use object_storage_app::{AppServices, PersistedTransfer};
use object_storage_core::ByteProgress;
use object_storage_domain::{
    Account, Bucket, CloudObject, ListObjectsRequest, ListingEntry, ProviderKind,
};
use object_storage_transfer::{
    TaskRunner, TransferEngine, TransferKind, TransferOp, TransferRequest, TransferState,
    TransferTask,
};

use crate::PaletteCommand;
use crate::account_modal::AddAccountModal;
use crate::actions::{
    AddAccount, CloseWindow, CopyObjectUrl, DeleteObject, DismissFilter, DismissRename,
    DownloadObject, OpenAbout, OpenCommandPalette, OpenObject, OpenSettings, PreviewObject, Quit,
    Refresh, RenameObject, RevealInFinder, SaveTextObject, SelectBucketByName, SelectObjectAll,
    ToggleObjectFilter, ToggleSidebar, UnifiedDismiss, UploadFiles, UploadFolder,
};
use crate::command_palette::CommandPaletteView;
use crate::settings_modal::SettingsModal;
use crate::tokens;

/// 左栏折叠后的图标栏宽度（规范：44px Icon Rail）。
const RAIL_WIDTH: Pixels = px(44.);
/// Sidebar 默认宽度（规范：默认 220，范围 180–360）。
const SIDEBAR_DEFAULT: Pixels = px(220.);
const SIDEBAR_MIN: Pixels = px(180.);
const SIDEBAR_MAX: Pixels = px(360.);
/// 对象列表单页条数（七牛列举单页上限内）。
const OBJECTS_PAGE_LIMIT: u32 = 100;
// 签名链接 TTL / 剪贴板清除秒数不再用编译期常量：运行时取 self.settings
// （settings.json，⌘, 可改；默认值见 object-storage-persistence）。

/// 侧栏/内容区的异步加载状态。`Loaded` 不单独建模——数据非空且 state==Idle 即加载完成。
#[derive(Debug, Clone, PartialEq, Eq)]
enum AsyncState {
    Idle,
    Loading,
    Failed(String),
}

/// Inspector 底部的下载结果提示（成功/失败一次一笇）
#[derive(Debug, Clone, PartialEq, Eq)]
struct DownloadMessage {
    is_error: bool,
    text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CopyObjectUrlRequest {
    account_id: String,
    bucket: String,
    key: String,
    ttl_secs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum InspectorTab {
    Preview,
    Details,
    Metadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreviewKind {
    Image,
    Text,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObjectMenuItem {
    Details,
    CopyUrl,
    Download,
    Rename,
    CopyTo,
    MoveTo,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TopMoreMenuItem {
    CopyTo,
    MoveTo,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CopyMoveMode {
    Copy,
    Move,
}

struct CopyMoveState {
    mode: CopyMoveMode,
    source_keys: Vec<String>,
    target_prefix: String,
    filter: Entity<InputState>,
    entries: Vec<ListingEntry>,
    state: AsyncState,
}

fn object_menu_items() -> Vec<ObjectMenuItem> {
    vec![
        ObjectMenuItem::Details,
        ObjectMenuItem::CopyUrl,
        ObjectMenuItem::Download,
        ObjectMenuItem::Rename,
        ObjectMenuItem::CopyTo,
        ObjectMenuItem::MoveTo,
        ObjectMenuItem::Delete,
    ]
}

fn top_more_menu_items() -> Vec<TopMoreMenuItem> {
    vec![
        TopMoreMenuItem::CopyTo,
        TopMoreMenuItem::MoveTo,
        TopMoreMenuItem::Delete,
    ]
}

fn object_menu_item_label(item: ObjectMenuItem) -> &'static str {
    match item {
        ObjectMenuItem::Details => "详情",
        ObjectMenuItem::CopyUrl => "获取地址",
        ObjectMenuItem::Download => "下载",
        ObjectMenuItem::Rename => "重命名",
        ObjectMenuItem::CopyTo => "复制到",
        ObjectMenuItem::MoveTo => "移动到",
        ObjectMenuItem::Delete => "删除",
    }
}

fn top_more_menu_item_label(item: TopMoreMenuItem) -> &'static str {
    match item {
        TopMoreMenuItem::CopyTo => "复制到",
        TopMoreMenuItem::MoveTo => "移动到",
        TopMoreMenuItem::Delete => "删除",
    }
}

fn object_menu_item_icon(item: ObjectMenuItem) -> IconName {
    match item {
        ObjectMenuItem::Details => IconName::Info,
        ObjectMenuItem::CopyUrl => IconName::ExternalLink,
        ObjectMenuItem::Download => IconName::ArrowDown,
        ObjectMenuItem::Rename => IconName::Replace,
        ObjectMenuItem::CopyTo => IconName::Copy,
        ObjectMenuItem::MoveTo => IconName::Folder,
        ObjectMenuItem::Delete => IconName::Delete,
    }
}

pub struct WorkspaceView {
    focus_handle: FocusHandle,
    sidebar_collapsed: bool,
    #[allow(dead_code)]
    inspector_tab: InspectorTab,
    /// 当前打开的命令面板（⌘K）。Some 时在根容器上渲染模态遮罩层；
    /// 面板关闭（open=false）后由此处置 None 并归还焦点。
    palette: Option<Entity<CommandPaletteView>>,
    /// 当前打开的「添加账号」模态（overlay 与命令面板同机制）。
    add_modal: Option<Entity<AddAccountModal>>,

    /// 组装好的应用服务（SQLite + Keychain + tokio 运行时），后台任务共享。
    services: Arc<AppServices>,

    // ---- 账号（Sidebar 上段） ----
    accounts: Vec<Account>,
    accounts_state: AsyncState,
    selected_account_id: Option<String>,

    // ---- 空间（Sidebar 下段；跟随选中账号异步加载） ----
    buckets: Vec<Bucket>,
    buckets_state: AsyncState,
    selected_bucket: Option<String>,
    /// RAM 子账号无 ListBuckets 时，手动输入空间名
    manual_bucket_input: Option<Entity<InputState>>,

    // ---- 对象列表（Content；跟随选中桶异步加载，支持翻页与前缀下钻） ----
    entries: Vec<ListingEntry>,
    objects_state: AsyncState,
    /// 「加载更多」进行中（不影响整表状态，避免整表闪回加载态）
    loading_more: bool,
    /// 下一页标记；None 或空 = 没有更多
    next_marker: Option<String>,
    /// 当前浏览的目录前缀（None = 根目录），以 `/` 结尾
    current_prefix: Option<String>,
    /// 检查器选中的对象 Key（entries 内查找）
    selected_object_key: Option<String>,
    /// 多选集合（规范 §7：Click/⌘Click/⇧Click/⌘A）。有序去重；
    /// `selected_object_key` 始终是其中最后一项（主选），供 Inspector/预览。
    selected_object_keys: indexmap::IndexSet<String>,
    /// 范围选择锚点（对象序号；上次普通/⌘点击的对象，不含目录前缀）。
    selection_anchor: Option<usize>,
    /// 重命名弹窗进行中：(对象 key，输入框)。
    renaming: Option<(String, Entity<InputState>)>,
    /// rename 后台执行中（防重入）。
    renaming_busy: bool,
    /// ⌘F 过滤：Some = 过滤开启（查询词在输入框实体里）。
    object_filter: Option<Entity<InputState>>,
    /// 过滤命中缓存（render 时由 filter_entries 计算；None = 未开启过滤）。
    filtered_ix: Option<Vec<usize>>,
    /// 应用设置（settings.json 快照；⌘, 可改）。
    settings: object_storage_persistence::Settings,
    /// settings.json 路径（模态展示与保存用）。
    settings_path: PathBuf,
    /// 设置模态（⌘,）。Some 时渲染遮罩。
    settings_modal: Option<Entity<SettingsModal>>,
    /// capture 清空前的选择快照：(集合, 锚点)。行 mouse-down 处理器
    /// 消费（⌘/⇧ 基线）；普通点击不消费（下一次快照覆盖）。
    selection_before_capture: Option<(indexmap::IndexSet<String>, Option<usize>)>,
    /// 本帧渲染的对象行 bounds（paint 阶段写入；窗口级 mouse-down 钩子
    /// 据此做命中检测：点击不在任何行/按钮 bounds 内 = 空白 → 清空）。
    row_bounds: std::cell::RefCell<Vec<gpui::Bounds<gpui::Pixels>>>,
    /// 内容区 bounds（paint 阶段写入；钩子只处理内容区内的点击）。
    content_bounds: std::cell::RefCell<Option<gpui::Bounds<gpui::Pixels>>>,
    /// 对象下载进行中（Inspector 按钮置灰防重入）
    downloading: bool,
    /// 上传选文件面板打开中（防重入）
    uploading: bool,
    /// 远端删除进行中
    deleting: bool,
    /// 预览对象下载/打开进行中
    previewing: bool,
    /// 已下载到本地缓存、供 GPUI img 直接渲染的预览路径
    preview_path: Option<PathBuf>,
    /// 文本预览内容；编辑器使用 GPUI InputState，不自建 WebView
    preview_text: Option<String>,
    text_editor: Option<Entity<InputState>>,
    /// Space 触发预览时，系统格式下载完成后自动打开 Quick Look
    preview_open_quicklook: bool,
    /// 文件名/预览按钮触发的应用内预览弹层。
    preview_overlay_open: bool,
    /// 当前打开对象菜单的 object key。
    object_menu_open: Option<String>,
    /// 顶部「更多」菜单是否打开。
    top_more_open: bool,
    /// 当前是否显示对象详情弹层。
    details_overlay_open: bool,
    /// 当前是否显示「关于」弹层（独立于设置模态）。
    about_overlay_open: bool,
    /// 删除确认 sheet 已弹出（gpui 禁止重入 prompt）
    delete_prompt_open: bool,
    /// 文本保存覆盖确认 sheet 已弹出
    save_prompt_open: bool,
    /// 正在生成并复制签名链接
    copying_url: bool,
    /// 新建目录弹窗输入。
    create_folder_input: Option<Entity<InputState>>,
    /// 新建目录上传占位对象中。
    creating_folder: bool,
    /// 复制/移动目标选择弹窗。
    copy_move: Option<CopyMoveState>,
    /// 复制/移动执行中。
    copy_move_busy: bool,
    copy_move_gen: u64,
    /// 最近一次下载结果提示（入队确认/失败；失败用 danger 色）
    download_message: Option<DownloadMessage>,
    /// 传输引擎：下载入队，状态经 watch 令牌事件驱动回填 `transfers`（不轮询）
    engine: Arc<TransferEngine>,
    /// 引擎任务快照（watch 订阅任务回填；取消/继续/重试直接作用于引擎）
    transfers: Vec<TransferTask>,
    /// ⌘Q 确认面板已弹出：gpui 禁止重入 `window.prompt`，二次 ⌘Q 直接忽略
    quit_prompt_open: bool,

    // ---- 串台防护：账号/对象各自的自增代号 ----
    bucket_gen: u64,
    object_gen: u64,
}

/// 对象多选语义（规范 §7：Click / ⌘Click / ⇧Click / ⌘A）的纯决策逻辑。
///
/// 独立成自由函数以便单测：输入当前选中集合、按键修饰符与点击位置，
/// 输出新选中集合与是否触发预览（仅普通 Click 主选行为触发预览）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObjectSelectionIntent {
    /// ⌘Click：切换点击项的选中态（不改变其它项）。
    pub command: bool,
    /// ⇧Click：从锚点到点击项的范围选择（⌘⇧ 同理，增量）。
    pub shift: bool,
    /// ⌘A：全选当前列表可见对象。
    pub select_all: bool,
    /// 是否点了空白处（清空选择；列表空白处点击才传 true）。
    pub clicked_empty: bool,
    /// 点击对象在 ordered_keys（不含目录前缀）中的下标；⌘A / 空白点击时为 None。
    pub clicked_index: Option<usize>,
}

/// 点击命中的条目类型：对象参与多选，目录前缀不参与（点击即下钻）。
pub(crate) enum ClickedEntry {
    Object(String),
    /// 目录前缀点击：命中项不进选择集合；载荷 key 仅为语义完整性保留。
    CommonPrefix(#[allow(dead_code)] String),
    None,
}

/// 计算点击后的选中集合。`ordered_keys` 是当前列表中全部对象 key
/// （按展示顺序，不含目录前缀）；`selection` 是当前选中集合（有序）；
/// `anchor` 是范围选择的起点（上次普通/⌘点击的对象下标）。
pub(crate) fn apply_object_selection(
    intent: ObjectSelectionIntent,
    ordered_keys: &[String],
    selection: &indexmap::IndexSet<String>,
    anchor: Option<usize>,
    clicked: ClickedEntry,
) -> (indexmap::IndexSet<String>, Option<usize>, bool) {
    // 返回 (新选中集合, 新锚点, 是否触发预览)
    if intent.select_all {
        let all: indexmap::IndexSet<String> = ordered_keys.iter().cloned().collect();
        return (all, anchor, false);
    }
    if intent.clicked_empty {
        return (indexmap::IndexSet::new(), None, false);
    }
    match clicked {
        ClickedEntry::CommonPrefix(_) => (selection.clone(), anchor, false),
        ClickedEntry::None => (indexmap::IndexSet::new(), None, false),
        ClickedEntry::Object(key) => {
            let Some(ix) = intent.clicked_index else {
                return (selection.clone(), anchor, false);
            };
            if intent.shift {
                // ⇧Click：锚点→点击项范围；⌘⇧ 增量（保留原选择），纯 ⇧ 重置为范围
                let start = anchor.unwrap_or(ix).min(ix);
                let end = anchor.unwrap_or(ix).max(ix);
                let mut next = if intent.command {
                    selection.clone()
                } else {
                    indexmap::IndexSet::new()
                };
                for key in ordered_keys[start..=end].iter() {
                    next.insert(key.clone());
                }
                return (next, anchor, false);
            }
            if intent.command {
                // ⌘Click：切换；锚点更新为点击项（Finder 语义）
                let mut next = selection.clone();
                if next.shift_remove(&key) {
                    // 取消选中：锚点仍指向点击项
                    return (next, Some(ix), false);
                }
                next.insert(key);
                return (next, Some(ix), false);
            }
            // 普通 Click：单选主选，触发预览
            let mut next = indexmap::IndexSet::new();
            next.insert(key);
            (next, Some(ix), true)
        }
    }
}

/// 由当前 key 和新名字推导 rename 目标 key：只替换最后一段（`/` 后的
/// 文件名部分），保持目录前缀不变。名字含 `/` 视为非法（不允许借
/// rename 移动目录，防误操作把对象搬进意外前缀）。
pub(crate) fn rename_target_key(current_key: &str, new_name: &str) -> Result<String, String> {
    let name = new_name.trim();
    if name.is_empty() {
        return Err("名称不能为空".into());
    }
    if name.contains('/') {
        return Err("名称不能包含 /".into());
    }
    if name == "." || name == ".." {
        return Err("名称不能是 . 或 ..".into());
    }
    match current_key.rsplit_once('/') {
        Some((prefix, _)) => Ok(format!("{prefix}/{name}")),
        None => Ok(name.to_string()),
    }
}

/// ⌘F 过滤：大小写不敏感子串匹配。命中对象 key 或目录前缀名任意即保留。
/// `None` query = 不过滤。返回保留项下标（指向 entries）。
pub(crate) fn filter_entries(entries: &[ListingEntry], query: Option<&str>) -> Vec<usize> {
    let Some(q) = query.map(str::trim).filter(|q| !q.is_empty()) else {
        return (0..entries.len()).collect();
    };
    let needle = q.to_lowercase();
    entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| match entry {
            ListingEntry::Object(o) => o.key.to_lowercase().contains(&needle),
            ListingEntry::CommonPrefix(p) => p.to_lowercase().contains(&needle),
        })
        .map(|(ix, _)| ix)
        .collect()
}

fn object_selection_ix(entries: &[ListingEntry], key: &str) -> Option<usize> {
    entries
        .iter()
        .filter_map(object_key)
        .position(|object_key| object_key == key)
}

fn object_keys(entries: &[ListingEntry]) -> Vec<String> {
    entries
        .iter()
        .filter_map(object_key)
        .map(str::to_string)
        .collect()
}

fn object_key_exists(entries: &[ListingEntry], key: &str) -> bool {
    entries
        .iter()
        .filter_map(object_key)
        .any(|object_key| object_key == key)
}

fn common_prefix_exists(entries: &[ListingEntry], prefix: &str) -> bool {
    entries.iter().any(|entry| match entry {
        ListingEntry::CommonPrefix(existing) => existing == prefix,
        ListingEntry::Object(_) => false,
    })
}

fn create_folder_target_key(
    current_prefix: Option<&str>,
    folder_name: &str,
) -> Result<String, String> {
    let name = folder_name.trim();
    if name.is_empty() {
        return Err("目录名不能为空".into());
    }
    if name.contains('/') {
        return Err("目录名不能包含 /".into());
    }
    if name == "." || name == ".." {
        return Err("目录名不能是 . 或 ..".into());
    }
    Ok(format!("{}{}{}", current_prefix.unwrap_or(""), name, "/"))
}

fn create_folder_validation_message(
    current_prefix: Option<&str>,
    folder_name: &str,
    entries: &[ListingEntry],
) -> Option<String> {
    let key = match create_folder_target_key(current_prefix, folder_name) {
        Ok(key) => key,
        Err(message) => return Some(message),
    };
    if common_prefix_exists(entries, &key) || object_key_exists(entries, &key) {
        return Some(format!(
            "目录已存在：{}",
            display_name(key.trim_end_matches('/'))
        ));
    }
    None
}

fn breadcrumb_prefixes(prefix: Option<&str>) -> Vec<(String, String)> {
    let Some(prefix) = prefix else {
        return Vec::new();
    };
    let mut path = String::new();
    prefix
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            path.push_str(segment);
            path.push('/');
            (segment.to_string(), path.clone())
        })
        .collect()
}

/// 长路径折叠：中段收起为 `…`（点击直达被收起的最深一层），保留首段与
/// 最后 2 段（父目录 + 当前目录）。折叠后可见项数与 BREADCRUMB_MAX_VISIBLE
/// 一致（首段 + … + 尾 2 段 = 4）。折叠决策在纯函数（单测锁死），渲染层
/// 只消费结果。
///
/// 返回 `Option`：`None` = 未折叠（渲染完整路径）；
/// `Some((collapsed_prefix, tail))` = 首段后插入省略项，点击直达
/// `collapsed_prefix`（被收起段中最深一层的前缀），`tail` 为保留的尾段。
const BREADCRUMB_MAX_VISIBLE: usize = 4;

fn collapse_breadcrumb(segments: &[(String, String)]) -> Option<(String, Vec<(String, String)>)> {
    if segments.len() <= BREADCRUMB_MAX_VISIBLE {
        return None;
    }
    // 折叠区为 [1, len-2)；最深被收起段是尾段前两段，其前缀即 `…` 的跳转目标
    let collapsed_prefix = segments[segments.len() - 3].1.clone();
    let tail = segments[segments.len() - 2..].to_vec();
    Some((collapsed_prefix, tail))
}

fn rename_validation_message(
    current_key: &str,
    new_name: &str,
    entries: &[ListingEntry],
) -> Option<String> {
    let new_key = match rename_target_key(current_key, new_name) {
        Ok(key) => key,
        Err(message) => return Some(message),
    };
    if new_key == current_key {
        return Some("请输入一个不同的新名称".into());
    }
    if object_key_exists(entries, &new_key) {
        return Some(format!(
            "目标名称已存在：{}，请换一个名字",
            display_name(&new_key)
        ));
    }
    None
}

fn normalize_copy_move_target_prefix(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if trimmed.starts_with('/') {
        return Err("目标目录不能以 / 开头".into());
    }
    if trimmed.split('/').any(|segment| segment == "..") {
        return Err("目标目录不能包含 ..".into());
    }
    Ok(if trimmed.ends_with('/') {
        trimmed.to_string()
    } else {
        format!("{trimmed}/")
    })
}

fn copy_move_target_key(source_key: &str, target_prefix: &str) -> String {
    format!("{}{}", target_prefix, display_name(source_key))
}

fn copy_move_target_keys(
    source_keys: &[String],
    target_prefix: &str,
) -> Result<Vec<(String, String)>, String> {
    let prefix = normalize_copy_move_target_prefix(target_prefix)?;
    let targets: Vec<(String, String)> = source_keys
        .iter()
        .map(|source| (source.clone(), copy_move_target_key(source, &prefix)))
        .collect();
    if let Some((source, _)) = targets.iter().find(|(source, target)| source == target) {
        return Err(format!("目标路径与源路径相同：{source}"));
    }
    let mut seen = std::collections::HashSet::new();
    if let Some((_, target)) = targets
        .iter()
        .find(|(_, target)| !seen.insert(target.as_str()))
    {
        return Err(format!(
            "多个源对象会写入同一目标名称：{}",
            display_name(target)
        ));
    }
    Ok(targets)
}

fn prepare_copy_move_directory_load(
    target_prefix: &mut String,
    entries: &mut Vec<ListingEntry>,
    state: &mut AsyncState,
    next_prefix: String,
) {
    *target_prefix = next_prefix;
    entries.clear();
    *state = AsyncState::Loading;
}

fn can_commit_copy_move(
    busy: bool,
    directory_state: &AsyncState,
    validation: Option<&str>,
) -> bool {
    !busy && *directory_state != AsyncState::Loading && validation.is_none()
}

fn copy_move_summary(mode: CopyMoveMode, success: usize, failures: &[(String, String)]) -> String {
    let action = match mode {
        CopyMoveMode::Copy => "复制",
        CopyMoveMode::Move => "移动",
    };
    if failures.is_empty() {
        return format!("已{action} {success} 个对象");
    }
    let detail = failures
        .iter()
        .take(3)
        .map(|(key, error)| format!("{}：{}", display_name(key), error))
        .collect::<Vec<_>>()
        .join("；");
    format!(
        "{action}完成 {success} 个，失败 {} 个：{detail}",
        failures.len()
    )
}

/// 默认下载目录有效性：设置值存在且是目录才可用。
/// 无效值（未设置/已被删除/指向文件）一律返回 None，由调用方退回面板默认位置。
fn effective_default_download_dir(dir: Option<&Path>) -> Option<PathBuf> {
    dir.filter(|path| path.is_dir()).map(Path::to_path_buf)
}

/// 下载目标路径：目标目录 + 云端 Key 的末段文件名。
fn download_dest_path(dest_dir: &Path, object_key: &str) -> PathBuf {
    dest_dir.join(display_name(object_key))
}

/// 单文件下载确认 sheet 文案（与批量下载同一交互结构：使用默认目录/另存为…/取消）。
fn single_download_confirm_texts(object_key: &str, default_dir: &Path) -> (String, String) {
    (
        format!("将「{}」下载到默认目录。", display_name(object_key)),
        format!(
            "{}\n\n选择「另存为…」可保存到其他位置。",
            default_dir.display()
        ),
    )
}

/// 批量下载确认 sheet 文案：说明 GPUI 0.2.2 目录选择器无初始目录参数的限制。
fn batch_download_confirm_texts(count: usize, default_dir: &Path) -> (String, String) {
    (
        format!("将 {count} 个对象下载到默认目录。"),
        format!(
            "{}\n\nGPUI 0.2.2 的目录选择器不支持设置初始目录；如需选择其他位置，将打开系统目录面板。",
            default_dir.display()
        ),
    )
}

/// 弹层规范锚点（详见 agents.md「弹层规范」行），回答「这套阻断配置下
/// 卡片内的点击/滚动事件是否会冒泡误关弹层」——返回 `true` 即不合规。
/// - `card_stops_mouse_down=false`：点卡片任意事件直接冒泡到遮罩 → 误关；
/// - 含滚动列表（预览/复制移动）的弹层还必须阻断 `mouse_up`：仅阻断 down
///   时，列表上的 mouse up 仍会走遮罩链路引发误关；
/// - `busy` 与阻断无关（busy 由各 close handler 自行拒绝）。
/// 仅测试消费（规范判据），生产路径按 agents.md 约定直接实现。
#[allow(dead_code)]
fn overlay_scroll_dismisses_modal(
    busy: bool,
    card_stops_mouse_down: bool,
    card_stops_mouse_up: bool,
    has_scrollable_list: bool,
) -> bool {
    let _ = busy;
    if !card_stops_mouse_down {
        return true;
    }
    has_scrollable_list && !card_stops_mouse_up
}

fn provider_url_scheme(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::Aliyun => "oss",
        ProviderKind::Qiniu => "kodo",
    }
}

fn object_key(entry: &ListingEntry) -> Option<&str> {
    match entry {
        ListingEntry::Object(object) => Some(object.key.as_str()),
        ListingEntry::CommonPrefix(_) => None,
    }
}

/// 判断本地缓存文件是否对应当前选中的对象（缓存复用判据）：
/// 缓存文件名形如 `{nanos}-{display_name}`，只需后缀匹配 display_name。
/// `None` path / 无 key 一律不复用（保守：多下载一次好过开错文件）。
pub(crate) fn cached_copy_matches(path: Option<&std::path::Path>, object_key: &str) -> bool {
    let Some(path) = path else {
        return false;
    };
    if object_key.is_empty() {
        return false;
    }
    let name = display_name(object_key);
    if name.is_empty() {
        return false;
    }
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(name) && n.len() > name.len())
}

impl WorkspaceView {
    pub fn new(services: Arc<AppServices>, cx: &mut Context<Self>) -> Self {
        // 设置（settings.json）：损坏必须显式报错退出（Fail Fast），
        // 与数据库打不开同级——不能静默重置吞掉用户的自定义配置。
        let (settings, settings_path) = {
            let path = object_storage_persistence::settings_path()
                .expect("无法定位设置文件目录（Application Support）");
            let settings = object_storage_persistence::Settings::load_at(path.clone())
                .unwrap_or_else(|error| panic!("设置文件损坏或不可读（{path:?}）：{error}"));
            (settings, path)
        };
        crate::theme::apply_settings(&settings, None, cx);
        // 传输引擎：任务执行体注入 AppServices 下载（provider 构建即锁即放），
        // 引擎把 future spawn 到 AppServices 的 tokio 运行时上（abort 即断流）。
        let runner: TaskRunner = {
            let services = Arc::clone(&services);
            Arc::new(move |request: TransferRequest| {
                let services = Arc::clone(&services);
                Box::pin(async move {
                    let (_, provider) = services
                        .build_provider(&request.account_id)
                        .map_err(|e| e.to_string())?;
                    let progress = request.progress.clone();
                    let cb: ByteProgress =
                        Arc::new(move |done, total| progress.report(done, total));
                    match request.op {
                        TransferOp::Download => provider
                            .download_object_to_file(
                                &request.bucket,
                                &request.key,
                                &request.dest,
                                Some(cb),
                            )
                            .await
                            .map_err(|e| e.to_string()),
                        TransferOp::Upload => provider
                            .upload_object_from_file(
                                &request.bucket,
                                &request.key,
                                &request.dest,
                                Some(cb),
                            )
                            .await
                            .map_err(|e| e.to_string()),
                    }
                })
            })
        };
        let engine = Arc::new(TransferEngine::new(
            services.runtime_handle(),
            runner,
            settings.transfer_concurrency as usize,
        ));

        // 系统事件 → 传输引擎（spec §25/§26，P0）。
        // - 睡眠（NSWorkspaceWillSleepNotification）→ suspend_all
        // - 网络断开（NWPathMonitor 非 satisfied）→ suspend_all
        // - 网络恢复（satisfied）→ resume_all
        // - 唤醒（didWake）故意不动：等网络满意事件再恢复，避免唤醒瞬间
        //   网络未就绪把重排队任务打成 Failed（P0 场景，见 macos 模块文档）
        // 回调只碰引擎（Send+Sync，无 gpui 实体），UI 经 watch 订阅自动刷新。
        // 约束：WorkspaceView 单窗口一次性创建，重复 new 会叠加监视器。
        let engine_sleep = Arc::clone(&engine);
        let engine_down = Arc::clone(&engine);
        let engine_up = Arc::clone(&engine);
        object_storage_macos::start_sleep_wake_monitor(
            Box::new(move || engine_sleep.suspend_all()),
            Box::new(|| {}), // 唤醒不直接恢复，理由见上
        );
        object_storage_macos::start_network_monitor(
            Box::new(move || engine_down.suspend_all()),
            Box::new(move || engine_up.resume_all()),
        );

        let mut this = Self {
            focus_handle: cx.focus_handle(),
            sidebar_collapsed: false,
            inspector_tab: InspectorTab::Preview,
            palette: None,
            add_modal: None,
            services,
            accounts: Vec::new(),
            accounts_state: AsyncState::Idle,
            selected_account_id: None,
            buckets: Vec::new(),
            buckets_state: AsyncState::Idle,
            manual_bucket_input: None,
            selected_bucket: None,
            entries: Vec::new(),
            objects_state: AsyncState::Idle,
            loading_more: false,
            next_marker: None,
            current_prefix: None,
            selected_object_key: None,
            selected_object_keys: indexmap::IndexSet::new(),
            selection_anchor: None,
            renaming: None,
            renaming_busy: false,
            object_filter: None,
            filtered_ix: None,
            settings,
            settings_path,
            settings_modal: None,
            selection_before_capture: None,
            row_bounds: std::cell::RefCell::new(Vec::new()),
            content_bounds: std::cell::RefCell::new(None),
            downloading: false,
            uploading: false,
            deleting: false,
            previewing: false,
            preview_path: None,
            preview_text: None,
            text_editor: None,
            preview_open_quicklook: false,
            preview_overlay_open: false,
            object_menu_open: None,
            top_more_open: false,
            details_overlay_open: false,
            about_overlay_open: false,
            delete_prompt_open: false,
            save_prompt_open: false,
            copying_url: false,
            create_folder_input: None,
            creating_folder: false,
            copy_move: None,
            copy_move_busy: false,
            copy_move_gen: 0,
            download_message: None,
            engine: Arc::clone(&engine),
            transfers: Vec::new(),
            quit_prompt_open: false,
            bucket_gen: 0,
            object_gen: 0,
        };
        this.load_accounts(cx);
        this.restore_persisted_transfers(cx);
        Self::subscribe_transfers(engine, cx);
        this
    }

    /// 启动时取出上次 ⌘Q「暂停并退出」留下的队列，入引擎后按原状态恢复。
    /// SQLite 走后台执行器；入队发生在首帧前后的短窗口，用户来不及抢点下载。
    fn restore_persisted_transfers(&mut self, cx: &mut Context<Self>) {
        let services = Arc::clone(&self.services);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { services.take_transfers() })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(items) if !items.is_empty() => {
                        // 不 suspend/resume：尊重网络监视器当前挂起标志。
                        // 引擎已挂起时入队停在 Queued；未挂起则按并发上限启动。
                        for item in items {
                            let local = PathBuf::from(item.dest);
                            let id = if item.kind == "upload" {
                                this.engine.enqueue_upload(
                                    item.account_id,
                                    item.bucket,
                                    item.object_key,
                                    local,
                                    item.display_name,
                                )
                            } else {
                                this.engine.enqueue_download(
                                    item.account_id,
                                    item.bucket,
                                    item.object_key,
                                    local,
                                    item.display_name,
                                )
                            };
                            if item.state == "paused" {
                                this.engine.pause(id);
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        this.download_message = Some(DownloadMessage {
                            is_error: true,
                            text: format!("读取已保存的传输队列失败：{e}"),
                        });
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// 订阅引擎状态变更令牌：任何任务状态突变 → 取快照回填 → notify。
    /// 常驻 async 任务，纯事件驱动（规范禁轮询）。引擎/视图任一消亡即退出。
    fn subscribe_transfers(engine: Arc<TransferEngine>, cx: &mut Context<Self>) {
        let mut changes = engine.subscribe();
        cx.spawn(async move |this, cx| {
            loop {
                if changes.changed().await.is_err() {
                    break; // 引擎已销毁（应用退出路径）
                }
                let snapshot = engine.snapshot();
                let alive = this
                    .update(cx, |this, cx| {
                        this.transfers = snapshot;
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    break; // 视图已释放
                }
            }
        })
        .detach();
    }

    // ---- 异步数据加载（全部经 AppServices 走后台执行器） ----

    /// 拉取账号列表（SQLite，快）。启动时与添加账号成功后调用。
    fn load_accounts(&mut self, cx: &mut Context<Self>) {
        self.accounts_state = AsyncState::Loading;
        let services = Arc::clone(&self.services);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { services.list_accounts() })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(accounts) => {
                        this.accounts = accounts;
                        this.accounts_state = AsyncState::Idle;
                    }
                    Err(e) => this.accounts_state = AsyncState::Failed(e.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// 选中账号并异步加载其空间列表。重复点击同一账号不重复请求（重试走 retry_buckets）。
    fn select_account(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.selected_account_id.as_deref() == Some(id) {
            return;
        }
        self.selected_account_id = Some(id.to_string());
        self.buckets.clear();
        self.buckets_state = AsyncState::Loading;
        self.clear_bucket_selection();
        cx.notify();
        self.start_bucket_load(cx);
    }

    /// 空间列表加载失败后的重试入口（保持当前选中账号重新请求）。
    fn add_manual_bucket(&mut self, cx: &mut Context<Self>) {
        let Some(input) = self.manual_bucket_input.clone() else {
            return;
        };
        let name = input.read(cx).value().trim().to_string();
        if name.is_empty() {
            return;
        }
        let kind = self
            .accounts
            .iter()
            .find(|account| Some(account.id.as_str()) == self.selected_account_id.as_deref())
            .map(|account| account.provider)
            .unwrap_or(object_storage_domain::ProviderKind::Aliyun);
        if !self.buckets.iter().any(|bucket| bucket.name == name) {
            self.buckets.push(Bucket {
                name: name.clone(),
                kind,
                region: None,
            });
        }
        self.buckets_state = AsyncState::Idle;
        self.selected_bucket = Some(name);
        self.selected_object_key = None;
        self.reload_objects(cx);
        cx.notify();
    }

    fn retry_buckets(&mut self, cx: &mut Context<Self>) {
        if self.selected_account_id.is_none() || self.buckets_state == AsyncState::Loading {
            return;
        }
        self.buckets_state = AsyncState::Loading;
        cx.notify();
        self.start_bucket_load(cx);
    }

    fn start_bucket_load(&mut self, cx: &mut Context<Self>) {
        self.bucket_gen += 1;
        let generation = self.bucket_gen;
        let Some(account_id) = self.selected_account_id.clone() else {
            return;
        };
        let services = Arc::clone(&self.services);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { services.list_buckets(&account_id) })
                .await;
            this.update(cx, |this, cx| {
                if this.bucket_gen != generation {
                    return; // 已切到别的账号，丢弃过期结果
                }
                match result {
                    Ok(buckets) => {
                        this.buckets = buckets;
                        this.buckets_state = AsyncState::Idle;
                    }
                    Err(e) => this.buckets_state = AsyncState::Failed(e.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// 选中桶并从根目录开始加载对象。重复点击同一桶不重复请求。
    fn select_bucket(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.selected_bucket.as_deref() == Some(name) {
            return;
        }
        self.selected_bucket = Some(name.to_string());
        self.current_prefix = None;
        // 跳桶 = 重上下文切换：关闭过滤条（与 Finder 语义一致；
        // ⌘R 刷新/翻页保留过滤词——只关这里，不动 reload_objects）
        self.object_filter = None;
        self.filtered_ix = None;
        self.reload_objects(cx);
    }

    /// 清空对象区（切换账号/桶时）。
    fn clear_bucket_selection(&mut self) {
        self.entries.clear();
        self.objects_state = AsyncState::Idle;
        self.loading_more = false;
        self.next_marker = None;
        self.current_prefix = None;
        self.clear_object_selection();
        // 过滤命中缓存基于 entries，数据已换直接作废缓存与过滤条
        // （跳桶是重上下文切换，Finder 同样不保留过滤）。
        self.object_filter = None;
        self.filtered_ix = None;
        self.download_message = None;
    }

    /// 从头（当前前缀的第一页）重新加载对象。
    fn reload_objects(&mut self, cx: &mut Context<Self>) {
        self.entries.clear();
        self.next_marker = None;
        self.clear_object_selection();
        // entries 将重建：作废过滤命中缓存（过滤条保留，加载完成后
        // refresh_filter 会按新数据重算；用同一词继续过滤是用户预期）
        self.filtered_ix = None;
        self.download_message = None;
        self.objects_state = AsyncState::Loading;
        cx.notify();
        self.request_objects(None, cx);
    }

    /// 清空多选与主选（切桶/翻页/删除后）。
    fn clear_object_selection(&mut self) {
        self.selected_object_key = None;
        self.selected_object_keys.clear();
        self.selection_anchor = None;
        self.renaming = None;
        self.object_menu_open = None;
        self.top_more_open = false;
        self.details_overlay_open = false;
    }

    /// 下钻到某个目录前缀。
    fn open_prefix(&mut self, prefix: String, cx: &mut Context<Self>) {
        // 目录切换 = 重上下文切换：关闭过滤条（与跳桶同理）
        self.object_filter = None;
        self.filtered_ix = None;
        self.current_prefix = Some(prefix);
        self.reload_objects(cx);
    }

    fn open_bucket_root(&mut self, cx: &mut Context<Self>) {
        if self.current_prefix.is_none() {
            return;
        }
        self.object_filter = None;
        self.filtered_ix = None;
        self.current_prefix = None;
        self.reload_objects(cx);
    }

    /// 返回上一级目录；已在根目录则无操作。
    fn go_up(&mut self, cx: &mut Context<Self>) {
        let Some(prefix) = self.current_prefix.clone() else {
            return;
        };
        self.current_prefix = parent_prefix(&prefix).map(str::to_string);
        self.reload_objects(cx);
    }

    /// 「加载更多」：带 marker 请求下一页并追加（错误时保留已加载内容）。
    fn load_more(&mut self, cx: &mut Context<Self>) {
        if self.loading_more || self.next_marker.is_none() {
            return;
        }
        self.request_objects(self.next_marker.clone(), cx);
    }

    /// 对象列表请求核心：携带代号发起后台加载；marker 非空表示翻页追加。
    fn request_objects(&mut self, marker: Option<String>, cx: &mut Context<Self>) {
        let is_more = marker.is_some();
        if is_more {
            self.loading_more = true;
        }
        self.object_gen += 1;
        let generation = self.object_gen;

        let Some(account_id) = self.selected_account_id.clone() else {
            return;
        };
        let Some(bucket) = self.selected_bucket.clone() else {
            return;
        };
        let region = self
            .buckets
            .iter()
            .find(|b| b.name == bucket)
            .and_then(|b| b.region.clone());
        let prefix = self.current_prefix.clone();
        let services = Arc::clone(&self.services);

        cx.spawn(async move |this, cx| {
            let request = ListObjectsRequest {
                bucket,
                prefix,
                delimiter: Some("/".into()),
                marker,
                limit: OBJECTS_PAGE_LIMIT,
                region,
            };
            let result = cx
                .background_executor()
                .spawn(async move { services.list_objects(&account_id, request) })
                .await;
            this.update(cx, |this, cx| {
                if this.object_gen != generation {
                    return; // 已切换桶/前缀，丢弃过期结果
                }
                this.loading_more = false;
                match result {
                    Ok(page) => {
                        let marker = if page.has_more() {
                            page.next_marker.clone()
                        } else {
                            None
                        };
                        if is_more {
                            this.entries.extend(page.entries);
                        } else {
                            this.entries = page.entries;
                        }
                        this.next_marker = marker;
                        this.objects_state = AsyncState::Idle;
                        // 过滤条开着时按新 entries 重算命中（refresh_filter
                        // 对未开启过滤是 no-op）
                        this.refresh_filter(cx);
                    }
                    Err(e) => {
                        // 整页失败清空；翻页失败保留已加载数据（见 render_objects）
                        if !is_more {
                            this.entries.clear();
                        }
                        this.objects_state = AsyncState::Failed(e.to_string());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn selected_cloud_object(&self) -> Option<&CloudObject> {
        let key = self.selected_object_key.as_ref()?;
        self.entries.iter().find_map(|e| match e {
            ListingEntry::Object(o) if o.key == *key => Some(o),
            _ => None,
        })
    }

    fn selected_provider_kind(&self) -> Option<ProviderKind> {
        let account_id = self.selected_account_id.as_ref()?;
        self.accounts
            .iter()
            .find(|account| account.id == *account_id)
            .map(|account| account.provider)
    }

    fn selected_object_keys_vec(&self) -> Vec<String> {
        if self.selected_object_keys.is_empty() {
            self.selected_cloud_object()
                .map(|object| vec![object.key.clone()])
                .unwrap_or_default()
        } else {
            self.selected_object_keys.iter().cloned().collect()
        }
    }

    /// 行级操作按钮以“该行对象”为作用域，避免当前多选集合导致误批量操作。
    fn select_object_for_row_action(&mut self, key: &str) {
        let anchor = object_selection_ix(&self.entries, key);
        self.selected_object_keys.clear();
        self.selected_object_keys.insert(key.to_string());
        self.selected_object_key = Some(key.to_string());
        self.selection_anchor = anchor;
        self.renaming = None;
    }

    fn open_object_menu(&mut self, key: &str, cx: &mut Context<Self>) {
        self.select_object_for_row_action(key);
        self.object_menu_open = Some(key.to_string());
        self.preview_overlay_open = false;
        self.details_overlay_open = false;
        cx.notify();
    }

    fn toggle_object_menu(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.object_menu_open.as_deref() == Some(key) {
            self.object_menu_open = None;
            cx.notify();
        } else {
            self.open_object_menu(key, cx);
        }
    }

    fn toggle_top_more_menu(&mut self, cx: &mut Context<Self>) {
        self.top_more_open = !self.top_more_open;
        self.object_menu_open = None;
        cx.notify();
    }

    fn close_top_more_menu(&mut self) {
        self.top_more_open = false;
    }

    fn open_details_overlay(&mut self, cx: &mut Context<Self>) {
        if self.selected_cloud_object().is_none() {
            return;
        }
        self.object_menu_open = None;
        self.preview_overlay_open = false;
        self.details_overlay_open = true;
        cx.notify();
    }

    fn close_details_overlay(&mut self, cx: &mut Context<Self>) {
        self.details_overlay_open = false;
        cx.notify();
    }

    fn handle_object_menu_item(
        &mut self,
        item: ObjectMenuItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.object_menu_open = None;
        match item {
            ObjectMenuItem::Details => self.open_details_overlay(cx),
            ObjectMenuItem::CopyUrl => self.copy_object_url(cx),
            ObjectMenuItem::Download => self.start_object_download(window, cx),
            ObjectMenuItem::Rename => self.handle_rename_object(&RenameObject, window, cx),
            ObjectMenuItem::CopyTo => self.open_copy_move_overlay(CopyMoveMode::Copy, window, cx),
            ObjectMenuItem::MoveTo => self.open_copy_move_overlay(CopyMoveMode::Move, window, cx),
            ObjectMenuItem::Delete => self.confirm_and_delete_object(window, cx),
        }
        cx.notify();
    }

    fn handle_top_more_menu_item(
        &mut self,
        item: TopMoreMenuItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_top_more_menu();
        match item {
            TopMoreMenuItem::CopyTo => self.open_copy_move_overlay(CopyMoveMode::Copy, window, cx),
            TopMoreMenuItem::MoveTo => self.open_copy_move_overlay(CopyMoveMode::Move, window, cx),
            TopMoreMenuItem::Delete => self.confirm_and_delete_object(window, cx),
        }
        cx.notify();
    }

    /// 下载选中对象：与批量下载一致的确认交互——
    /// - 已设置有效默认目录：先弹确认 sheet（使用默认目录 / 另存为… / 取消）
    /// - 未设置：直接打开保存面板（初始目录 = HOME）
    /// 最终都经 gpui 平台 API（`cx.prompt_for_new_path`，异步回调）拿目标路径。
    /// 用户取消 = 无操作。
    ///
    /// 为什么必须用 gpui 平台 API、不能在事件处理器里同步 `runModal`：
    /// 模态循环期间 AppKit 事件会重入 gpui（borrow App RefCell），而外层处理器
    /// 还持有借用 → "RefCell already borrowed" panic 闪退。gpui 自带的面板从
    /// foreground executor 任务发起 `beginWithCompletionHandler:`，结果经
    /// oneshot 回传，天生规避重入。详见 docs/notes/gpui-api-notes.md「文件对话框」。
    fn start_object_download(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.downloading {
            return; // 防重入
        }
        // 多选（≥2）走批量目录流程；单选维持原保存面板。
        if self.selected_object_keys.len() > 1 {
            self.start_batch_download(window, cx);
            return;
        }
        let Some(key) = self.selected_cloud_object().map(|o| o.key.clone()) else {
            return; // 未选中对象：无操作（按钮本应置灰）
        };
        let Some(account_id) = self.selected_account_id.clone() else {
            return;
        };
        let Some(bucket) = self.selected_bucket.clone() else {
            return;
        };

        self.downloading = true;
        self.download_message = None;
        cx.notify();

        // 默认目录确认 sheet：与批量下载交互对齐（使用默认目录/另存为…/取消）
        if let Some(default_dir) =
            effective_default_download_dir(self.settings.default_download_dir.as_deref())
        {
            let (message, detail) = single_download_confirm_texts(&key, &default_dir);
            let receiver = window.prompt(
                PromptLevel::Info,
                &message,
                Some(&detail),
                &[
                    PromptButton::ok("使用默认目录"),
                    PromptButton::new("另存为…"),
                    PromptButton::cancel("取消"),
                ],
                cx,
            );
            cx.spawn(async move |this, cx| {
                let answer = match receiver.await {
                    Ok(answer) => answer,
                    Err(_) => {
                        this.update(cx, |this, cx| {
                            this.downloading = false;
                            cx.notify();
                        })
                        .ok();
                        return;
                    }
                };
                this.update(cx, |this, cx| match answer {
                    0 => this.enqueue_single_download(&account_id, &bucket, &key, cx),
                    1 => {
                        let directory = this.download_initial_directory();
                        this.prompt_for_single_download_destination(
                            account_id, bucket, key, directory, cx,
                        )
                    }
                    _ => {
                        this.downloading = false;
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
            return;
        }

        // 无默认目录：保存面板初始目录 = HOME（仅影响初始位置）
        let directory = self.download_initial_directory();
        self.prompt_for_single_download_destination(account_id, bucket, key, directory, cx);
    }

    /// 单文件下载入队（默认目录路径）。
    fn enqueue_single_download(
        &mut self,
        account_id: &str,
        bucket: &str,
        key: &str,
        cx: &mut Context<Self>,
    ) {
        let dest = download_dest_path(&self.download_initial_directory(), key);
        self.downloading = false;
        self.engine
            .enqueue_download(account_id, bucket, key, dest, display_name(key).to_string());
        self.download_message = Some(DownloadMessage {
            is_error: false,
            text: format!("已加入传输队列：{}", display_name(key)),
        });
        cx.notify();
    }

    /// 打开保存面板拿单文件目标路径（异步回调，规避模态重入）。
    fn prompt_for_single_download_destination(
        &mut self,
        account_id: String,
        bucket: String,
        key: String,
        directory: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let suggested_name = display_name(&key).to_string();
        let receiver = cx.prompt_for_new_path(&directory, Some(&*suggested_name));

        cx.spawn(async move |this, cx| {
            // 面板结果在无借用作用域内先归一化，再一次性回主线程提交
            enum PanelOutcome {
                Picked(PathBuf),
                Cancelled,
                Failed(String),
            }
            let outcome = match receiver.await {
                Ok(Ok(Some(dest))) => PanelOutcome::Picked(dest),
                Ok(Ok(None)) => PanelOutcome::Cancelled,
                Ok(Err(e)) => PanelOutcome::Failed(format!("无法打开存储面板：{e}")),
                Err(_) => PanelOutcome::Failed("存储面板结果通道已关闭".into()),
            };

            let dest = match outcome {
                PanelOutcome::Picked(dest) => dest,
                PanelOutcome::Cancelled => {
                    // 用户取消：正常流程，静默复位
                    this.update(cx, |this, cx| {
                        this.downloading = false;
                        cx.notify();
                    })
                    .ok();
                    return;
                }
                PanelOutcome::Failed(text) => {
                    // 面板层异常：明确呈现，不静默（Fail Fast 的 UI 面）
                    this.update(cx, |this, cx| {
                        this.downloading = false;
                        this.download_message = Some(DownloadMessage {
                            is_error: true,
                            text,
                        });
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };

            // 入队即返回：排队/进度/结果全部由传输列表呈现（事件驱动）；
            // 下载从零重传（File::create 截断旧残留），断流续传在传输引擎后续里程碑
            this.update(cx, |this, cx| {
                this.downloading = false;
                this.engine.enqueue_download(
                    account_id.as_str(),
                    bucket.as_str(),
                    key.as_str(),
                    dest,
                    display_name(&key).to_string(),
                );
                this.download_message = Some(DownloadMessage {
                    is_error: false,
                    text: format!("已加入传输队列：{}", display_name(&key)),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn download_initial_directory(&self) -> PathBuf {
        effective_default_download_dir(self.settings.default_download_dir.as_deref())
            .or_else(std::env::home_dir)
            .unwrap_or_else(|| PathBuf::from("/"))
    }

    /// 批量下载（多选 ≥2）：先选目标目录（gpui `prompt_for_paths`
    /// `directories: true, multiple: false`）→ 逐项入队到该目录。
    /// 目标文件名 = display_name(key)；重名由传输引擎按路径覆盖写（File::create）。
    fn start_batch_download(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.downloading {
            return;
        }
        let keys: Vec<String> = self.selected_object_keys.iter().cloned().collect();
        if keys.len() < 2 {
            return;
        }
        let Some(account_id) = self.selected_account_id.clone() else {
            return;
        };
        let Some(bucket) = self.selected_bucket.clone() else {
            return;
        };
        self.downloading = true;
        self.download_message = None;
        cx.notify();

        if let Some(default_dir) =
            effective_default_download_dir(self.settings.default_download_dir.as_deref())
        {
            let (message, detail) = batch_download_confirm_texts(keys.len(), &default_dir);
            let receiver = window.prompt(
                PromptLevel::Info,
                &message,
                Some(&detail),
                &[
                    PromptButton::ok("使用默认目录"),
                    PromptButton::new("另选目录…"),
                    PromptButton::cancel("取消"),
                ],
                cx,
            );
            cx.spawn(async move |this, cx| {
                let answer = match receiver.await {
                    Ok(answer) => answer,
                    Err(_) => {
                        this.update(cx, |this, cx| {
                            this.downloading = false;
                            cx.notify();
                        })
                        .ok();
                        return;
                    }
                };
                this.update(cx, |this, cx| match answer {
                    0 => this.enqueue_batch_downloads(&account_id, &bucket, &keys, default_dir, cx),
                    1 => this.prompt_for_batch_download_directory(account_id, bucket, keys, cx),
                    _ => {
                        this.downloading = false;
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
            return;
        }

        self.prompt_for_batch_download_directory(account_id, bucket, keys, cx);
    }

    fn prompt_for_batch_download_directory(
        &mut self,
        account_id: String,
        bucket: String,
        keys: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择下载目录".into()),
        });

        cx.spawn(async move |this, cx| {
            enum PanelOutcome {
                Picked(PathBuf),
                Cancelled,
                Failed(String),
            }
            let outcome = match receiver.await {
                Ok(Ok(Some(paths))) => match paths.into_iter().next() {
                    Some(dir) => PanelOutcome::Picked(dir),
                    None => PanelOutcome::Cancelled,
                },
                Ok(Ok(None)) => PanelOutcome::Cancelled,
                Ok(Err(e)) => PanelOutcome::Failed(format!("无法打开目录面板：{e}")),
                Err(_) => PanelOutcome::Failed("目录面板结果通道已关闭".into()),
            };

            let dest_dir = match outcome {
                PanelOutcome::Picked(dir) => dir,
                PanelOutcome::Cancelled => {
                    this.update(cx, |this, cx| {
                        this.downloading = false;
                        cx.notify();
                    })
                    .ok();
                    return;
                }
                PanelOutcome::Failed(text) => {
                    this.update(cx, |this, cx| {
                        this.downloading = false;
                        this.download_message = Some(DownloadMessage {
                            is_error: true,
                            text,
                        });
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };

            this.update(cx, |this, cx| {
                this.enqueue_batch_downloads(&account_id, &bucket, &keys, dest_dir, cx)
            })
            .ok();
        })
        .detach();
    }

    fn enqueue_batch_downloads(
        &mut self,
        account_id: &str,
        bucket: &str,
        keys: &[String],
        dest_dir: PathBuf,
        cx: &mut Context<Self>,
    ) {
        self.downloading = false;
        for key in keys {
            let name = display_name(key).to_string();
            self.engine.enqueue_download(
                account_id,
                bucket,
                key.as_str(),
                download_dest_path(&dest_dir, key),
                name,
            );
        }
        self.download_message = Some(DownloadMessage {
            is_error: false,
            text: format!(
                "已加入传输队列：{} 个对象 → {}",
                keys.len(),
                dest_dir.display()
            ),
        });
        cx.notify();
    }

    fn start_object_preview(&mut self, cx: &mut Context<Self>) {
        if self.previewing {
            return;
        }
        let Some(key) = self
            .selected_cloud_object()
            .map(|object| object.key.clone())
        else {
            return;
        };
        let Some(account_id) = self.selected_account_id.clone() else {
            return;
        };
        let Some(bucket) = self.selected_bucket.clone() else {
            return;
        };
        let name = display_name(&key).to_string();
        let mut path = std::env::temp_dir();
        path.push("CloudStorage");
        path.push("preview");
        path.push(format!(
            "{}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("系统时间早于 Unix epoch")
                .as_nanos(),
            name
        ));
        self.previewing = true;
        self.preview_path = None;
        self.preview_text = None;
        self.text_editor = None;
        self.download_message = None;
        cx.notify();
        let services = Arc::clone(&self.services);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    std::fs::create_dir_all(path.parent().expect("预览路径必须有父目录"))
                        .map_err(|e| format!("创建预览缓存目录失败：{e}"))?;
                    services
                        .download_object(&account_id, &bucket, &key, &path)
                        .map_err(|e| {
                            preview_download_error_message(&bucket, &key, &e.to_string())
                        })?;
                    let text = if is_text_object(&key) {
                        let metadata = std::fs::metadata(&path)
                            .map_err(|e| format!("读取预览文件信息失败：{e}"))?;
                        if metadata.len() > 2 * 1024 * 1024 {
                            return Err("文本对象超过 2 MiB，暂不在编辑器中打开".into());
                        }
                        Some(
                            std::fs::read_to_string(&path)
                                .map_err(|e| format!("文本对象不是有效 UTF-8：{e}"))?,
                        )
                    } else {
                        None
                    };
                    Ok::<_, String>((path, text))
                })
                .await;
            this.update(cx, |this, cx| {
                this.previewing = false;
                match result {
                    Ok((path, text)) => {
                        this.preview_path = Some(path.clone());
                        this.preview_text = text;
                        if this.preview_open_quicklook {
                            this.preview_open_quicklook = false;
                            if let Some(key) = this.selected_object_key.as_deref() {
                                if preview_kind(key) == PreviewKind::System {
                                    if let Err(error) = object_storage_macos::quick_look(&path) {
                                        this.download_message = Some(DownloadMessage {
                                            is_error: true,
                                            text: format!("打开 Quick Look 失败：{error}"),
                                        });
                                    }
                                }
                            }
                        }
                    }
                    Err(error) => {
                        this.download_message = Some(DownloadMessage {
                            is_error: true,
                            text: error,
                        });
                    }
                }
                cx.notify();
            })
            .expect("预览结果回传 UI 失败");
        })
        .detach();
    }

    /// 确保选中对象有本地副本：`preview_path` 已有（同 key）→ 直接复用；
    /// 否则下载到临时目录（与预览同一缓存位置）。返回 (本地路径, key)。
    /// 异步；结果经 `then` 回调（在 UI 线程执行）。
    fn ensure_local_copy(
        &mut self,
        then: impl FnOnce(&mut Self, PathBuf, String, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = self
            .selected_cloud_object()
            .map(|object| object.key.clone())
        else {
            return;
        };
        // 已有本地副本且对应当前对象：直接用
        let current = self.selected_object_key.clone().unwrap_or_default();
        if cached_copy_matches(self.preview_path.as_deref(), &current) {
            let path = self.preview_path.clone().expect("刚检查过");
            then(self, path, key, cx);
            return;
        }
        if self.previewing {
            return; // 已有下载在进行：防重入
        }
        let Some(account_id) = self.selected_account_id.clone() else {
            return;
        };
        let Some(bucket) = self.selected_bucket.clone() else {
            return;
        };
        let name = display_name(&key).to_string();
        let mut path = std::env::temp_dir();
        path.push("CloudStorage");
        path.push("preview");
        path.push(format!(
            "{}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("系统时间早于 Unix epoch")
                .as_nanos(),
            name
        ));
        self.previewing = true;
        self.download_message = None;
        cx.notify();
        let services = Arc::clone(&self.services);
        let key_for_result = key.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    std::fs::create_dir_all(path.parent().expect("预览路径必须有父目录"))
                        .map_err(|e| format!("创建缓存目录失败：{e}"))?;
                    services
                        .download_object(&account_id, &bucket, &key, &path)
                        .map_err(|e| {
                            preview_download_error_message(&bucket, &key, &e.to_string())
                        })?;
                    Ok::<_, String>(path)
                })
                .await;
            this.update(cx, |this, cx| {
                this.previewing = false;
                match result {
                    Ok(path) => {
                        this.preview_path = Some(path.clone());
                        then(this, path, key_for_result, cx);
                    }
                    Err(error) => {
                        this.download_message = Some(DownloadMessage {
                            is_error: true,
                            text: error,
                        });
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// ⌘O / Inspector「打开」：用默认应用打开选中对象的本地副本
    /// （spec §14：下载到 Temporary Directory → NSWorkspace open）。
    fn handle_open_object(&mut self, _: &OpenObject, _window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() || self.add_modal.is_some() || self.settings_modal.is_some() {
            return;
        }
        self.ensure_local_copy(
            |this, path, _key, cx| {
                if let Err(error) = object_storage_macos::open_with_default_app(&path) {
                    this.download_message = Some(DownloadMessage {
                        is_error: true,
                        text: format!("打开失败：{error}"),
                    });
                    cx.notify();
                } else {
                    this.download_message = Some(DownloadMessage {
                        is_error: false,
                        text: format!(
                            "已用默认应用打开：{}",
                            display_name(&path.display().to_string())
                        ),
                    });
                    cx.notify();
                }
            },
            cx,
        );
    }

    /// Inspector「在 Finder 中显示」（spec §16）：gpui reveal_path。
    fn handle_reveal_in_finder(
        &mut self,
        _: &RevealInFinder,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.is_some() || self.add_modal.is_some() || self.settings_modal.is_some() {
            return;
        }
        self.ensure_local_copy(
            |this, path, _key, cx| {
                cx.reveal_path(&path);
                this.download_message = Some(DownloadMessage {
                    is_error: false,
                    text: format!(
                        "已在 Finder 中显示：{}",
                        display_name(&path.display().to_string())
                    ),
                });
                cx.notify();
            },
            cx,
        );
    }

    fn start_text_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self.preview_text.clone() else {
            return;
        };
        let language = self
            .selected_cloud_object()
            .map(|object| syntax_language(&object.key))
            .unwrap_or("text");
        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor(language)
                .default_value(text)
        });
        self.text_editor = Some(editor);
        cx.notify();
    }

    fn ensure_preview_text_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.text_editor.is_some() {
            return;
        }
        if !self.preview_overlay_open {
            return;
        }
        let Some(text) = self.preview_text.clone() else {
            return;
        };
        let language = self
            .selected_cloud_object()
            .map(|object| syntax_language(&object.key))
            .unwrap_or("text");
        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor(language)
                .default_value(text)
        });
        self.text_editor = Some(editor);
    }

    fn save_text_edit(&mut self, cx: &mut Context<Self>) {
        let (Some(editor), Some(path), Some(account_id), Some(bucket), Some(object)) = (
            self.text_editor.clone(),
            self.preview_path.clone(),
            self.selected_account_id.clone(),
            self.selected_bucket.clone(),
            self.selected_cloud_object()
                .map(|object| object.key.clone()),
        ) else {
            return;
        };
        let text = editor.read(cx).value().to_string();
        if let Err(error) = std::fs::write(&path, text.as_bytes()) {
            self.download_message = Some(DownloadMessage {
                is_error: true,
                text: format!("写入编辑内容失败：{error}"),
            });
            cx.notify();
            return;
        }
        self.preview_text = Some(text.clone());
        self.engine.enqueue_upload(
            &account_id,
            &bucket,
            &object,
            path,
            display_name(&object).to_string(),
        );
        self.download_message = Some(DownloadMessage {
            is_error: false,
            text: format!("已加入覆盖上传队列：{}", display_name(&object)),
        });
        cx.notify();
    }

    fn confirm_and_save_text_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.text_editor.is_none() {
            return;
        }
        if self.palette.is_some() || self.add_modal.is_some() {
            return;
        }
        if self.save_prompt_open || self.delete_prompt_open || self.quit_prompt_open {
            return;
        }
        let name = self
            .selected_cloud_object()
            .map(|object| display_name(&object.key).to_string())
            .unwrap_or_else(|| "对象".into());
        self.save_prompt_open = true;
        let message = format!("覆盖“{name}”？");
        let rx = window.prompt(
            PromptLevel::Warning,
            &message,
            Some("编辑后的内容将上传并覆盖远端对象，无法撤销。"),
            &[PromptButton::ok("保存并上传"), PromptButton::cancel("取消")],
            cx,
        );
        cx.spawn(async move |this, cx| {
            let answer = match rx.await {
                Ok(i) => i,
                Err(_) => {
                    this.update(cx, |this, _| this.save_prompt_open = false)
                        .ok();
                    return;
                }
            };
            this.update(cx, |this, cx| {
                this.save_prompt_open = false;
                if answer == 0 {
                    this.save_text_edit(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn copy_object_url(&mut self, cx: &mut Context<Self>) {
        if self.palette.is_some() || self.add_modal.is_some() {
            return;
        }
        if self.copying_url {
            return;
        }
        let request = match copy_object_url_request(
            self.selected_account_id.as_deref(),
            self.selected_bucket.as_deref(),
            self.selected_cloud_object(),
            self.settings.signed_url_ttl_secs,
        ) {
            Ok(request) => request,
            Err(message) => {
                self.download_message = Some(message);
                cx.notify();
                return;
            }
        };
        let clear_secs = self.settings.clipboard_clear_secs;
        self.copying_url = true;
        self.download_message = None;
        cx.notify();
        let services = Arc::clone(&self.services);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    services.signed_get_url(
                        &request.account_id,
                        &request.bucket,
                        &request.key,
                        request.ttl_secs,
                    )
                })
                .await;
            this.update(cx, |this, cx| {
                this.copying_url = false;
                match result {
                    Ok(url) => match object_storage_macos::copy_to_clipboard(&url) {
                        Ok(()) => {
                            let clear_note = if clear_secs > 0 {
                                format!("（{clear_secs} 秒后自动从剪贴板清除）")
                            } else {
                                String::new()
                            };
                            this.download_message = Some(DownloadMessage {
                                is_error: false,
                                text: format!("已复制签名链接{clear_note}"),
                            });
                            if clear_secs > 0 {
                                std::thread::spawn(move || {
                                    std::thread::sleep(std::time::Duration::from_secs(clear_secs));
                                    if let Err(error) =
                                        object_storage_macos::clear_clipboard_if_equals(&url)
                                    {
                                        eprintln!("[clipboard] 自动清除失败：{error}");
                                    }
                                });
                            }
                        }
                        Err(error) => {
                            this.download_message = Some(DownloadMessage {
                                is_error: true,
                                text: format!("写入剪贴板失败：{error}"),
                            });
                        }
                    },
                    Err(error) => {
                        this.download_message = Some(DownloadMessage {
                            is_error: true,
                            text: format!("复制链接失败：{error}"),
                        });
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn handle_copy_object_url(
        &mut self,
        _: &CopyObjectUrl,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_object_url(cx);
    }

    fn handle_save_text_object(
        &mut self,
        _: &SaveTextObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_and_save_text_edit(window, cx);
    }

    fn open_system_preview(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.preview_path.clone() else {
            return;
        };
        if let Err(error) = object_storage_macos::quick_look(&path) {
            self.download_message = Some(DownloadMessage {
                is_error: true,
                text: format!("打开 Quick Look 失败：{error}"),
            });
            cx.notify();
        }
    }

    fn open_preview_overlay(&mut self, cx: &mut Context<Self>) {
        self.preview_overlay_open = true;
        self.object_menu_open = None;
        self.details_overlay_open = false;
        self.preview_open_quicklook = false;
        self.start_object_preview(cx);
    }

    fn close_preview_overlay(&mut self, cx: &mut Context<Self>) {
        self.preview_overlay_open = false;
        cx.notify();
    }

    fn handle_preview_object(
        &mut self,
        _: &PreviewObject,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_preview_overlay(cx);
    }

    fn handle_download_object(
        &mut self,
        _: &DownloadObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_object_download(window, cx);
    }

    /// 对象行点击 → 多选语义（规范 §7）。纯决策在 `apply_object_selection`，
    /// 这里只负责取上下文 + 回写状态；预览由文件名/预览按钮显式触发。
    fn handle_object_row_click(
        &mut self,
        ix: usize,
        clicked: ClickedEntry,
        modifiers: gpui::Modifiers,
        cx: &mut Context<Self>,
    ) {
        // 直接用当前选中集合做基线（行点击不会触发空白清空钩子——几何
        // 命中检测保证两者互斥，集合在行处理时是完整的点击前状态）
        let (baseline, baseline_anchor) =
            (self.selected_object_keys.clone(), self.selection_anchor);
        let ordered_keys = object_keys(&self.entries);
        let clicked_index = match &clicked {
            ClickedEntry::Object(key) => {
                ordered_keys.iter().position(|object_key| object_key == key)
            }
            ClickedEntry::CommonPrefix(_) => Some(ix),
            ClickedEntry::None => None,
        };
        let intent = ObjectSelectionIntent {
            command: modifiers.platform,
            shift: modifiers.shift,
            select_all: false,
            clicked_empty: false,
            clicked_index,
        };
        let (next, anchor, _) = apply_object_selection(
            intent,
            &ordered_keys,
            &baseline,
            // ⇧ 基线锚点优先（capture 保留了旧锚点）；否则用当前
            if baseline_anchor.is_some() {
                baseline_anchor
            } else {
                self.selection_anchor
            },
            clicked,
        );
        self.selected_object_keys = next;
        self.selected_object_key = self.selected_object_keys.last().cloned();
        self.selection_anchor = anchor;
        self.object_menu_open = None;
        self.top_more_open = false;
        cx.notify();
    }

    /// ⌘A：全选当前列表中的对象（不含目录前缀）。命令面板/模态打开时忽略。
    fn handle_select_all(
        &mut self,
        _: &SelectObjectAll,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.is_some() || self.add_modal.is_some() {
            return;
        }
        if self.selected_bucket.is_none() {
            return;
        }
        let intent = ObjectSelectionIntent {
            command: false,
            shift: false,
            select_all: true,
            clicked_empty: false,
            clicked_index: None,
        };
        let ordered_keys = object_keys(&self.entries);
        let (next, anchor, _) = apply_object_selection(
            intent,
            &ordered_keys,
            &self.selected_object_keys,
            self.selection_anchor,
            ClickedEntry::None,
        );
        self.selected_object_keys = next;
        self.selected_object_key = self.selected_object_keys.last().cloned();
        self.selection_anchor = anchor;
        cx.notify();
    }

    /// Return：打开重命名弹窗。多选（≠1）时忽略——批量改名语义不明确，不做。
    fn handle_rename_object(
        &mut self,
        _: &RenameObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.is_some() || self.add_modal.is_some() {
            return;
        }
        if self.renaming.is_some() || self.renaming_busy {
            return;
        }
        if self.selected_object_keys.len() > 1 {
            self.download_message = Some(DownloadMessage {
                is_error: true,
                text: "多选状态下不支持重命名，请只选中一个对象".into(),
            });
            cx.notify();
            return;
        }
        let Some(object) = self.selected_cloud_object() else {
            return;
        };
        let key = object.key.clone();
        let initial = display_name(&key).to_string();
        let editor = cx.new(|cx| InputState::new(window, cx).default_value(initial));
        // Return 提交：单行 Input 对 Enter emit PressEnter 后 propagate（不会
        // 二次触发 RenameObject——Workspace context 的 enter 绑定在 keymap 里
        // 已被 Input context 的绑定先消费）。
        cx.subscribe_in(&editor, window, |this, _, event: &InputEvent, _, cx| {
            if let InputEvent::PressEnter { .. } = event {
                this.commit_rename(cx);
            }
        })
        .detach();
        let focus_editor = editor.clone();
        focus_editor.update(cx, |state, cx| state.focus(window, cx));
        self.renaming = Some((key, editor));
        cx.notify();
    }

    /// 提交重命名：读取输入 → 校验目标 key → 后台
    /// 下载到临时文件 → 上传新 key → 删旧 key。失败不静默。
    fn commit_rename(&mut self, cx: &mut Context<Self>) {
        let Some((old_key, editor)) = self.renaming.clone() else {
            return;
        };
        if self.renaming_busy {
            return;
        }
        let new_name = editor.read(cx).value().to_string();
        // 无变化：直接退出编辑态（Finder 行为）
        if new_name == display_name(&old_key) {
            cx.notify();
            return;
        }
        if let Some(message) = rename_validation_message(&old_key, &new_name, &self.entries) {
            self.download_message = Some(DownloadMessage {
                is_error: true,
                text: message,
            });
            cx.notify();
            return;
        }
        let new_key = match rename_target_key(&old_key, &new_name) {
            Ok(key) => key,
            Err(message) => {
                self.download_message = Some(DownloadMessage {
                    is_error: true,
                    text: message,
                });
                cx.notify();
                return;
            }
        };
        // 目标 key 已存在：明确提示冲突（云端 GET 不报错即存在，用列举
        // 结果判断——entries 里查即可，覆盖当前列表可见范围；翻页场景
        // 由上传侧 File::create 语义兜底？不，远端覆盖。用 provider 上传
        // 是覆盖语义，所以必须先查；entries 不可靠，直接调 head？OSS 无
        // head 封装。妥协：上传前查 entries + 明确告知覆盖风险）。
        if object_key_exists(&self.entries, &new_key) {
            cx.notify();
            return;
        }
        let Some(account_id) = self.selected_account_id.clone() else {
            return;
        };
        let Some(bucket) = self.selected_bucket.clone() else {
            return;
        };
        let services = Arc::clone(&self.services);
        self.renaming_busy = true;
        self.renaming = None;
        self.download_message = None;
        cx.notify();

        let name = display_name(&old_key).to_string();
        let new_key_display = display_name(&new_key).to_string();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(
                    async move { services.rename_object(&account_id, &bucket, &old_key, &new_key) },
                )
                .await;
            this.update(cx, |this, cx| {
                this.renaming_busy = false;
                match result {
                    Ok(()) => {
                        this.reload_objects(cx);
                        this.download_message = Some(DownloadMessage {
                            is_error: false,
                            text: format!("已重命名：{name} → {new_key_display}"),
                        });
                    }
                    Err(error) => {
                        this.download_message = Some(DownloadMessage {
                            is_error: true,
                            text: format!("重命名失败：{error}"),
                        });
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// 取消重命名（Esc：Input escape() propagate → context "Renaming"）。
    fn handle_dismiss_rename(
        &mut self,
        _: &DismissRename,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((key, _)) = &self.renaming {
            eprintln!("[rename] cancelled key={key}");
        }
        if self.create_folder_input.is_some() {
            self.close_create_folder_overlay(cx);
            return;
        }
        if self.copy_move.is_some() {
            self.close_copy_move_overlay(cx);
            return;
        }
        self.cancel_rename(cx);
    }

    /// 取消重命名。
    fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        if self.renaming.take().is_some() {
            cx.notify();
        }
    }

    fn open_create_folder_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_bucket.is_none()
            || self.create_folder_input.is_some()
            || self.creating_folder
            || self.palette.is_some()
            || self.add_modal.is_some()
        {
            return;
        }
        let editor = cx.new(|cx| InputState::new(window, cx).default_value("新建目录"));
        cx.subscribe_in(&editor, window, |this, _, event: &InputEvent, _, cx| {
            if let InputEvent::PressEnter { .. } = event {
                this.commit_create_folder(cx);
            }
        })
        .detach();
        editor.update(cx, |state, cx| state.focus(window, cx));
        self.object_menu_open = None;
        self.create_folder_input = Some(editor);
        cx.notify();
    }

    fn close_create_folder_overlay(&mut self, cx: &mut Context<Self>) {
        if self.creating_folder {
            return;
        }
        if self.create_folder_input.take().is_some() {
            cx.notify();
        }
    }

    fn commit_create_folder(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.create_folder_input.clone() else {
            return;
        };
        if self.creating_folder {
            return;
        }
        let Some(account_id) = self.selected_account_id.clone() else {
            return;
        };
        let Some(bucket) = self.selected_bucket.clone() else {
            return;
        };
        let folder_name = editor.read(cx).value().to_string();
        if let Some(message) = create_folder_validation_message(
            self.current_prefix.as_deref(),
            &folder_name,
            &self.entries,
        ) {
            self.download_message = Some(DownloadMessage {
                is_error: true,
                text: message,
            });
            cx.notify();
            return;
        }
        let key = match create_folder_target_key(self.current_prefix.as_deref(), &folder_name) {
            Ok(key) => key,
            Err(message) => {
                self.download_message = Some(DownloadMessage {
                    is_error: true,
                    text: message,
                });
                cx.notify();
                return;
            }
        };
        let services = Arc::clone(&self.services);
        let display_key = key.clone();
        self.creating_folder = true;
        self.download_message = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let tmp = std::env::temp_dir().join(format!(
                        "cloudstorage-empty-folder-{}-{}",
                        std::process::id(),
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_nanos())
                            .unwrap_or_default()
                    ));
                    std::fs::File::create(&tmp).map_err(|e| format!("创建临时空文件失败：{e}"))?;
                    let upload = services.upload_object(&account_id, &bucket, &key, &tmp);
                    let cleanup = std::fs::remove_file(&tmp);
                    if let Err(e) = cleanup {
                        eprintln!(
                            "[create_folder] 临时文件清理失败（不影响结果）：{}：{e}",
                            tmp.display()
                        );
                    }
                    upload.map_err(|e| e.to_string())
                })
                .await;
            this.update(cx, |this, cx| {
                this.creating_folder = false;
                match result {
                    Ok(_) => {
                        this.create_folder_input = None;
                        this.download_message = Some(DownloadMessage {
                            is_error: false,
                            text: format!("已新建目录：{}", display_key),
                        });
                        this.reload_objects(cx);
                    }
                    Err(error) => {
                        this.download_message = Some(DownloadMessage {
                            is_error: true,
                            text: format!("新建目录失败：{error}"),
                        });
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn open_copy_move_overlay(
        &mut self,
        mode: CopyMoveMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.is_some()
            || self.add_modal.is_some()
            || self.copy_move_busy
            || self.selected_bucket.is_none()
        {
            return;
        }
        let source_keys = self.selected_object_keys_vec();
        if source_keys.is_empty() {
            self.download_message = Some(DownloadMessage {
                is_error: true,
                text: "请先选中对象".into(),
            });
            cx.notify();
            return;
        }
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("输入或搜索目标目录…"));
        cx.subscribe_in(&filter, window, |this, _, event: &InputEvent, _, cx| {
            if let InputEvent::Change = event {
                cx.notify();
            }
            if let InputEvent::PressEnter { .. } = event {
                this.commit_copy_move(cx);
            }
        })
        .detach();
        filter.update(cx, |state, cx| state.focus(window, cx));
        self.object_menu_open = None;
        self.top_more_open = false;
        self.copy_move = Some(CopyMoveState {
            mode,
            source_keys,
            target_prefix: self.current_prefix.clone().unwrap_or_default(),
            filter,
            entries: Vec::new(),
            state: AsyncState::Loading,
        });
        cx.notify();
        self.load_copy_move_entries(cx);
    }

    fn load_copy_move_entries(&mut self, cx: &mut Context<Self>) {
        self.copy_move_gen += 1;
        let generation = self.copy_move_gen;

        let Some(account_id) = self.selected_account_id.clone() else {
            return;
        };
        let Some(bucket) = self.selected_bucket.clone() else {
            return;
        };
        let Some(prefix) = self
            .copy_move
            .as_ref()
            .map(|state| state.target_prefix.clone())
        else {
            return;
        };
        let region = self
            .buckets
            .iter()
            .find(|b| b.name == bucket)
            .and_then(|b| b.region.clone());
        let services = Arc::clone(&self.services);

        if let Some(state) = &mut self.copy_move {
            prepare_copy_move_directory_load(
                &mut state.target_prefix,
                &mut state.entries,
                &mut state.state,
                prefix.clone(),
            );
        }
        cx.notify();

        cx.spawn(async move |this, cx| {
            let request = ListObjectsRequest {
                bucket,
                prefix: if prefix.is_empty() {
                    None
                } else {
                    Some(prefix)
                },
                delimiter: Some("/".into()),
                marker: None,
                limit: OBJECTS_PAGE_LIMIT,
                region,
            };
            let result = cx
                .background_executor()
                .spawn(async move { services.list_objects(&account_id, request) })
                .await;
            this.update(cx, |this, cx| {
                if this.copy_move_gen != generation {
                    return;
                }
                let Some(state) = &mut this.copy_move else {
                    return;
                };
                match result {
                    Ok(page) => {
                        state.entries = page.entries;
                        state.state = AsyncState::Idle;
                    }
                    Err(e) => {
                        state.entries.clear();
                        state.state = AsyncState::Failed(e.to_string());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn close_copy_move_overlay(&mut self, cx: &mut Context<Self>) {
        if self.copy_move_busy {
            return;
        }
        if self.copy_move.take().is_some() {
            cx.notify();
        }
    }

    fn copy_move_target_prefix(&self, cx: &mut Context<Self>) -> Result<String, String> {
        let Some(state) = &self.copy_move else {
            return Err("复制/移动弹窗未打开".into());
        };
        let input = state.filter.read(cx).value().to_string();
        if input.trim().is_empty() {
            return Ok(state.target_prefix.clone());
        }
        normalize_copy_move_target_prefix(&input)
    }

    fn copy_move_validation_message(&self, cx: &mut Context<Self>) -> Option<String> {
        let Some(state) = &self.copy_move else {
            return None;
        };
        let target_prefix = match self.copy_move_target_prefix(cx) {
            Ok(prefix) => prefix,
            Err(message) => return Some(message),
        };
        let targets = match copy_move_target_keys(&state.source_keys, &target_prefix) {
            Ok(targets) => targets,
            Err(message) => return Some(message),
        };
        if target_prefix == state.target_prefix
            && let Some((_, target)) = targets
                .iter()
                .find(|(_, target)| object_key_exists(&state.entries, target))
        {
            return Some(format!("目标对象已存在：{target}"));
        }
        None
    }

    fn enter_copy_move_prefix(
        &mut self,
        prefix: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(state) = &mut self.copy_move {
            prepare_copy_move_directory_load(
                &mut state.target_prefix,
                &mut state.entries,
                &mut state.state,
                prefix.clone(),
            );
            state
                .filter
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.load_copy_move_entries(cx);
    }

    fn go_up_copy_move_prefix(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(state) = &mut self.copy_move {
            let prefix = parent_prefix(&state.target_prefix)
                .map(str::to_string)
                .unwrap_or_default();
            prepare_copy_move_directory_load(
                &mut state.target_prefix,
                &mut state.entries,
                &mut state.state,
                prefix,
            );
            state
                .filter
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.load_copy_move_entries(cx);
    }

    fn commit_copy_move(&mut self, cx: &mut Context<Self>) {
        if self.copy_move_busy {
            return;
        }
        let Some(state) = &self.copy_move else {
            return;
        };
        if !can_commit_copy_move(self.copy_move_busy, &state.state, None) {
            return;
        }
        if let Some(message) = self.copy_move_validation_message(cx) {
            self.download_message = Some(DownloadMessage {
                is_error: true,
                text: message,
            });
            cx.notify();
            return;
        }
        let Some(account_id) = self.selected_account_id.clone() else {
            return;
        };
        let Some(bucket) = self.selected_bucket.clone() else {
            return;
        };
        let mode = state.mode;
        let target_prefix = match self.copy_move_target_prefix(cx) {
            Ok(prefix) => prefix,
            Err(message) => {
                self.download_message = Some(DownloadMessage {
                    is_error: true,
                    text: message,
                });
                cx.notify();
                return;
            }
        };
        let targets = match copy_move_target_keys(&state.source_keys, &target_prefix) {
            Ok(targets) => targets,
            Err(message) => {
                self.download_message = Some(DownloadMessage {
                    is_error: true,
                    text: message,
                });
                cx.notify();
                return;
            }
        };
        let services = Arc::clone(&self.services);
        self.copy_move_busy = true;
        self.download_message = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut success = 0usize;
                    let mut failures = Vec::new();
                    for (source, target) in targets {
                        let result = match mode {
                            CopyMoveMode::Copy => {
                                services.copy_object(&account_id, &bucket, &source, &target)
                            }
                            CopyMoveMode::Move => {
                                services.move_object(&account_id, &bucket, &source, &target)
                            }
                        };
                        match result {
                            Ok(()) => success += 1,
                            Err(error) => failures.push((source, error.to_string())),
                        }
                    }
                    (success, failures)
                })
                .await;
            this.update(cx, |this, cx| {
                this.copy_move_busy = false;
                this.copy_move = None;
                let (success, failures) = result;
                this.download_message = Some(DownloadMessage {
                    is_error: !failures.is_empty(),
                    text: copy_move_summary(mode, success, &failures),
                });
                this.reload_objects(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// ⌘F：开/关对象列表过滤。开启时焦点入过滤框；关闭时清空查询。
    fn handle_toggle_object_filter(
        &mut self,
        _: &ToggleObjectFilter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.is_some() || self.add_modal.is_some() {
            return;
        }
        if self.selected_bucket.is_none() {
            return;
        }
        if self.object_filter.is_some() {
            // 已开启 → 关闭（Esc 走 DismissFilter 也到这）
            self.close_object_filter(window, cx);
            return;
        }
        let editor = cx.new(|cx| InputState::new(window, cx).placeholder("过滤当前列表…"));
        cx.subscribe_in(&editor, window, |this, _, event: &InputEvent, _, cx| {
            if let InputEvent::Change = event {
                this.refresh_filter(cx);
            }
        })
        .detach();
        let focus_editor = editor.clone();
        focus_editor.update(cx, |state, cx| state.focus(window, cx));
        self.object_filter = Some(editor);
        self.refresh_filter(cx);
        cx.notify();
    }

    /// 关闭过滤：清空查询与命中缓存，焦点归还 Workspace。
    fn close_object_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.object_filter.take().is_some() {
            self.filtered_ix = None;
            self.focus_handle.focus(window);
            cx.notify();
        }
    }

    /// 依据输入框当前值重算过滤命中缓存。
    fn refresh_filter(&mut self, cx: &mut Context<Self>) {
        let query = self
            .object_filter
            .as_ref()
            .map(|editor| editor.read(cx).value().to_string());
        self.filtered_ix = query
            .as_deref()
            .map(|q| filter_entries(&self.entries, Some(q)));
        cx.notify();
    }

    /// Esc 关闭过滤（context "ObjectFilter"）。
    fn handle_dismiss_filter(
        &mut self,
        _: &DismissFilter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.object_filter.is_some() {
            self.close_object_filter(window, cx);
        }
    }

    fn handle_upload_files(
        &mut self,
        _: &UploadFiles,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_files_upload(cx);
    }

    fn handle_upload_folder(
        &mut self,
        _: &UploadFolder,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_folder_upload(cx);
    }

    fn handle_delete_object(
        &mut self,
        _: &DeleteObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_and_delete_object(window, cx);
    }

    /// 远端删除必须确认（规范 §43，无废纸篓）。⌘⌫ / 菜单 / Inspector 共用。
    /// 支持多选：选中多项时逐项删除，失败逐项可见（不静默）。
    fn confirm_and_delete_object(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() || self.add_modal.is_some() {
            return;
        }
        if self.deleting || self.delete_prompt_open || self.quit_prompt_open {
            return;
        }
        // 多选集合（主选兼容：单选时两者一致）
        let keys: Vec<String> = if self.selected_object_keys.is_empty() {
            self.selected_cloud_object()
                .map(|o| vec![o.key.clone()])
                .unwrap_or_default()
        } else {
            self.selected_object_keys.iter().cloned().collect()
        };
        if keys.is_empty() {
            self.download_message = Some(DownloadMessage {
                is_error: true,
                text: "请先选中一个对象再删除".into(),
            });
            cx.notify();
            return;
        }
        let Some(account_id) = self.selected_account_id.clone() else {
            return;
        };
        let Some(bucket) = self.selected_bucket.clone() else {
            return;
        };
        let count = keys.len();
        let summary = delete_summary(&keys);
        self.delete_prompt_open = true;
        let message = if count == 1 {
            format!("删除“{}”？", display_name(&keys[0]))
        } else {
            format!("删除 {count} 个对象？")
        };
        let detail = format!("将从空间 {bucket} 永久删除{summary}，无法撤销。");
        let rx = window.prompt(
            PromptLevel::Warning,
            &message,
            Some(&detail),
            &[PromptButton::ok("删除"), PromptButton::cancel("取消")],
            cx,
        );
        let services = Arc::clone(&self.services);
        cx.spawn(async move |this, cx| {
            let answer = match rx.await {
                Ok(i) => i,
                Err(_) => {
                    this.update(cx, |this, _| this.delete_prompt_open = false)
                        .ok();
                    return;
                }
            };
            if answer != 0 {
                this.update(cx, |this, _| this.delete_prompt_open = false)
                    .ok();
                return;
            }
            this.update(cx, |this, cx| {
                this.delete_prompt_open = false;
                this.deleting = true;
                this.download_message = None;
                cx.notify();
            })
            .ok();
            // 逐项删除（多选批量 = 循环单删）：任一失败不中断剩余项，
            // 结束后汇总成功/失败明细，失败逐项可见（不静默）。
            let results = cx
                .background_executor()
                .spawn(async move {
                    let mut ok = Vec::new();
                    let mut failed = Vec::new();
                    for key in keys {
                        match services.delete_object(&account_id, &bucket, &key) {
                            Ok(()) => ok.push(key),
                            Err(e) => failed.push((key, e.to_string())),
                        }
                    }
                    (ok, failed)
                })
                .await;
            this.update(cx, |this, cx| {
                this.deleting = false;
                let (ok, failed) = results;
                if !ok.is_empty() {
                    this.clear_object_selection();
                    this.reload_objects(cx);
                }
                if failed.is_empty() {
                    let text = if ok.len() == 1 {
                        format!("已删除 {}", display_name(&ok[0]))
                    } else {
                        format!("已删除 {} 个对象", ok.len())
                    };
                    this.download_message = Some(DownloadMessage {
                        is_error: false,
                        text,
                    });
                } else {
                    let failed_text = failed
                        .iter()
                        .map(|(key, err)| format!("{}：{err}", display_name(key)))
                        .collect::<Vec<_>>()
                        .join("；");
                    let text = if ok.is_empty() {
                        format!("删除失败：{failed_text}")
                    } else {
                        format!(
                            "已删除 {} 个，失败 {} 个：{}",
                            ok.len(),
                            failed.len(),
                            failed_text
                        )
                    };
                    this.download_message = Some(DownloadMessage {
                        is_error: true,
                        text,
                    });
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// ⌘R：有选中空间则重载当前前缀的对象列表；否则刷新空间/账号。
    fn handle_refresh(&mut self, _: &Refresh, _window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_bucket.is_some() {
            self.reload_objects(cx);
        } else if self.selected_account_id.is_some() {
            self.buckets_state = AsyncState::Loading;
            cx.notify();
            self.start_bucket_load(cx);
        } else {
            self.load_accounts(cx);
        }
    }

    /// 上传所需的账号 / 空间 / 当前前缀。缺一则提示并返回 None。
    fn upload_target(&mut self, cx: &mut Context<Self>) -> Option<(String, String, String)> {
        let Some(account_id) = self.selected_account_id.clone() else {
            self.download_message = Some(DownloadMessage {
                is_error: true,
                text: "请先选中一个账号和空间再上传".into(),
            });
            cx.notify();
            return None;
        };
        let Some(bucket) = self.selected_bucket.clone() else {
            self.download_message = Some(DownloadMessage {
                is_error: true,
                text: "请先选中一个空间再上传".into(),
            });
            cx.notify();
            return None;
        };
        let prefix = self.current_prefix.clone().unwrap_or_default();
        Some((account_id, bucket, prefix))
    }

    /// 上传本地文件：gpui `prompt_for_paths`（多选，只要文件）→ 入队。
    /// 云端 key = 当前前缀 + 文件名。
    fn start_files_upload(&mut self, cx: &mut Context<Self>) {
        if self.uploading {
            return;
        }
        let Some((account_id, bucket, prefix)) = self.upload_target(cx) else {
            return;
        };
        self.uploading = true;
        self.download_message = None;
        cx.notify();

        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("上传文件".into()),
        });

        cx.spawn(async move |this, cx| {
            enum PanelOutcome {
                Picked(Vec<PathBuf>),
                Cancelled,
                Failed(String),
            }
            let outcome = match receiver.await {
                Ok(Ok(Some(paths))) if !paths.is_empty() => PanelOutcome::Picked(paths),
                Ok(Ok(Some(_))) | Ok(Ok(None)) => PanelOutcome::Cancelled,
                Ok(Err(e)) => PanelOutcome::Failed(format!("无法打开文件面板：{e}")),
                Err(_) => PanelOutcome::Failed("文件面板结果通道已关闭".into()),
            };

            this.update(cx, |this, cx| {
                this.uploading = false;
                match outcome {
                    PanelOutcome::Cancelled => {}
                    PanelOutcome::Failed(text) => {
                        this.download_message = Some(DownloadMessage {
                            is_error: true,
                            text,
                        });
                    }
                    PanelOutcome::Picked(paths) => {
                        let mut names = Vec::new();
                        for path in paths {
                            let name = path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("file")
                                .to_string();
                            let key = format!("{prefix}{name}");
                            this.engine.enqueue_upload(
                                account_id.as_str(),
                                bucket.as_str(),
                                key.as_str(),
                                path,
                                name.clone(),
                            );
                            names.push(name);
                        }
                        this.download_message = Some(DownloadMessage {
                            is_error: false,
                            text: format!("已加入传输队列：{}", names.join("、")),
                        });
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// 上传本地目录：`prompt_for_paths`（只要目录）→ 后台递归列举文件 → 入队。
    /// 云端 key = 当前前缀 + 目录名 + 相对路径（`/` 分隔，含顶层目录名）。
    fn start_folder_upload(&mut self, cx: &mut Context<Self>) {
        if self.uploading {
            return;
        }
        let Some((account_id, bucket, prefix)) = self.upload_target(cx) else {
            return;
        };
        self.uploading = true;
        self.download_message = None;
        cx.notify();

        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("上传文件夹".into()),
        });

        cx.spawn(async move |this, cx| {
            enum PanelOutcome {
                Picked(Vec<PathBuf>),
                Cancelled,
                Failed(String),
            }
            let outcome = match receiver.await {
                Ok(Ok(Some(paths))) if !paths.is_empty() => PanelOutcome::Picked(paths),
                Ok(Ok(Some(_))) | Ok(Ok(None)) => PanelOutcome::Cancelled,
                Ok(Err(e)) => PanelOutcome::Failed(format!("无法打开目录面板：{e}")),
                Err(_) => PanelOutcome::Failed("目录面板结果通道已关闭".into()),
            };

            let roots = match outcome {
                PanelOutcome::Picked(paths) => paths,
                PanelOutcome::Cancelled => {
                    this.update(cx, |this, cx| {
                        this.uploading = false;
                        cx.notify();
                    })
                    .ok();
                    return;
                }
                PanelOutcome::Failed(text) => {
                    this.update(cx, |this, cx| {
                        this.uploading = false;
                        this.download_message = Some(DownloadMessage {
                            is_error: true,
                            text,
                        });
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };

            let collected = cx
                .background_executor()
                .spawn(async move {
                    let mut files = Vec::new();
                    let mut errors = Vec::new();
                    for root in &roots {
                        match collect_folder_uploads(root) {
                            Ok(entries) => files.extend(entries),
                            Err(e) => errors.push(e),
                        }
                    }
                    (files, errors)
                })
                .await;

            this.update(cx, |this, cx| {
                this.uploading = false;
                let (files, errors) = collected;
                if files.is_empty() {
                    let text = if errors.is_empty() {
                        "目录为空，没有可上传的文件".into()
                    } else {
                        errors.join("；")
                    };
                    this.download_message = Some(DownloadMessage {
                        is_error: true,
                        text,
                    });
                } else {
                    let n = files.len();
                    for entry in files {
                        let key = format!("{prefix}{}", entry.relative_key);
                        this.engine.enqueue_upload(
                            account_id.as_str(),
                            bucket.as_str(),
                            key.as_str(),
                            entry.source,
                            entry.display_name,
                        );
                    }
                    let mut text = format!("已加入传输队列：{n} 个文件");
                    if !errors.is_empty() {
                        text.push_str("；部分目录失败：");
                        text.push_str(&errors.join("；"));
                    }
                    this.download_message = Some(DownloadMessage {
                        is_error: !errors.is_empty(),
                        text,
                    });
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Finder / 其它 App 拖入文件：对象浏览区是投放目标（规范 §15）。
    fn with_file_drop(&self, el: gpui::Div, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        // gpui 只在「已有 hover 样式 / 正在拖」时注册 mousemove→notify。
        // 系统文件拖入前一帧 active_drag 为空，不注册监听 → drag_over 永不刷新。
        // 空 hover() 让监听常驻；透明 2px 边框避免拖入时布局跳动。
        el.id("object-browser-drop")
            .border_2()
            .border_color(hsla(0., 0., 0., 0.))
            .hover(|style| style)
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.handle_dropped_paths(paths.paths(), cx);
            }))
            .drag_over::<ExternalPaths>(|style, _, _, cx| {
                style
                    .border_color(cx.theme().accent)
                    .bg(cx.theme().accent.opacity(0.18))
            })
    }

    fn enqueue_file_uploads(
        &mut self,
        account_id: &str,
        bucket: &str,
        prefix: &str,
        paths: Vec<PathBuf>,
    ) -> usize {
        let n = paths.len();
        for path in paths {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file")
                .to_string();
            let key = format!("{prefix}{name}");
            self.engine
                .enqueue_upload(account_id, bucket, key.as_str(), path, name);
        }
        n
    }

    fn handle_dropped_paths(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        if paths.is_empty() {
            return;
        }
        let Some((account_id, bucket, prefix)) = self.upload_target(cx) else {
            return;
        };
        let mut files = Vec::new();
        let mut dirs = Vec::new();
        for path in paths {
            let meta = match std::fs::symlink_metadata(path) {
                Ok(m) => m,
                Err(e) => {
                    self.download_message = Some(DownloadMessage {
                        is_error: true,
                        text: format!("无法读取 {}：{e}", path.display()),
                    });
                    cx.notify();
                    return;
                }
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                dirs.push(path.clone());
            } else if meta.is_file() {
                files.push(path.clone());
            }
        }
        if dirs.is_empty() {
            if files.is_empty() {
                self.download_message = Some(DownloadMessage {
                    is_error: true,
                    text: "没有可上传的文件（已跳过符号链接）".into(),
                });
            } else {
                let n = self.enqueue_file_uploads(&account_id, &bucket, &prefix, files);
                self.download_message = Some(DownloadMessage {
                    is_error: false,
                    text: format!("已加入传输队列：{n} 个文件"),
                });
            }
            cx.notify();
            return;
        }

        cx.spawn(async move |this, cx| {
            let walked = cx
                .background_executor()
                .spawn(async move {
                    let mut entries = Vec::new();
                    let mut errors = Vec::new();
                    for dir in &dirs {
                        match collect_folder_uploads(dir) {
                            Ok(found) => entries.extend(found),
                            Err(e) => errors.push(e),
                        }
                    }
                    (entries, errors)
                })
                .await;
            this.update(cx, |this, cx| {
                let n_files = this.enqueue_file_uploads(&account_id, &bucket, &prefix, files);
                let (entries, errors) = walked;
                let n_folder = entries.len();
                for entry in entries {
                    let key = format!("{prefix}{}", entry.relative_key);
                    this.engine.enqueue_upload(
                        account_id.as_str(),
                        bucket.as_str(),
                        key.as_str(),
                        entry.source,
                        entry.display_name,
                    );
                }
                let n = n_files + n_folder;
                if n == 0 {
                    let text = if errors.is_empty() {
                        "没有可上传的文件".into()
                    } else {
                        errors.join("；")
                    };
                    this.download_message = Some(DownloadMessage {
                        is_error: true,
                        text,
                    });
                } else {
                    let mut text = format!("已加入传输队列：{n} 个文件");
                    if !errors.is_empty() {
                        text.push_str("；部分目录失败：");
                        text.push_str(&errors.join("；"));
                    }
                    this.download_message = Some(DownloadMessage {
                        is_error: !errors.is_empty(),
                        text,
                    });
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // ---- 模态：添加账号 ----

    fn handle_open_add_modal(
        &mut self,
        _: &AddAccount,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.add_modal.is_some() {
            return;
        }
        let services = Arc::clone(&self.services);
        let modal = cx.new(|cx| AddAccountModal::new(services, window, cx));
        cx.observe_in(&modal, window, Self::handle_add_modal_changed)
            .detach();
        modal.update(cx, |modal, cx| modal.focus_first(window, cx));
        self.add_modal = Some(modal);
        cx.notify();
    }

    /// 模态观察：置 done（保存成功 → 刷新账号列表）或 closed（取消）后丢弃实体。
    fn handle_add_modal_changed(
        &mut self,
        modal: Entity<AddAccountModal>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (closed, done) = {
            let m = modal.read(cx);
            (m.closed(), m.done())
        };
        if !closed && !done {
            return; // saving/error 等常规通知，不处理
        }
        self.add_modal = None;
        window.focus(&self.focus_handle);
        if done {
            self.load_accounts(cx);
        }
        cx.notify();
    }

    /// 「添加账号」模态遮罩：点击空白处请求关闭（保存中拒绝）；卡片内点击
    /// 已被卡片的 stop_propagation 挡住。
    fn render_add_modal_overlay(
        &self,
        modal: &Entity<AddAccountModal>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.overlay)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                    let Some(modal) = &this.add_modal else {
                        return;
                    };
                    if !modal.read(cx).saving() {
                        modal.update(cx, AddAccountModal::close);
                    }
                }),
            )
            .child(modal.clone())
    }

    /// 设置模态遮罩（结构与添加账号一致）。
    fn render_settings_modal_overlay(
        &self,
        modal: &Entity<SettingsModal>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.overlay)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                    let Some(modal) = &this.settings_modal else {
                        return;
                    };
                    if !modal.read(cx).saving() {
                        modal.update(cx, SettingsModal::close);
                    }
                }),
            )
            .child(modal.clone())
    }

    fn render_preview_overlay(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(object) = self.selected_cloud_object() else {
            return div().into_any_element();
        };
        let name = display_name(&object.key).to_string();
        let kind = preview_kind(&object.key);
        let file_type = object.mime_type.clone().unwrap_or_else(|| match kind {
            PreviewKind::Image => "image/*".into(),
            PreviewKind::Text => format!("text/{}", syntax_language(&object.key)),
            PreviewKind::System => "未知类型".into(),
        });
        let error = self
            .download_message
            .as_ref()
            .filter(|message| message.is_error)
            .map(|message| message.text.clone());

        let preview_content = if self.previewing {
            v_flex()
                .size_full()
                .w_full()
                .items_center()
                .justify_center()
                .gap_3()
                .child(Spinner::new())
                .child(
                    div()
                        .text_size(tokens::text(13.))
                        .text_color(theme.muted_foreground)
                        .child("正在准备预览…"),
                )
                .into_any_element()
        } else if let Some(error) = error {
            v_flex()
                .size_full()
                .w_full()
                .items_center()
                .justify_center()
                .gap_3()
                .p_6()
                .child(Icon::new(IconName::TriangleAlert).text_color(theme.danger))
                .child(
                    div()
                        .max_w(px(520.))
                        .text_size(tokens::text(13.))
                        .text_color(theme.muted_foreground)
                        .child(error),
                )
                .into_any_element()
        } else if let Some(editor) = self.text_editor.clone() {
            v_flex()
                .size_full()
                .w_full()
                .gap_2()
                .child(
                    div()
                        .text_size(tokens::text(11.))
                        .text_color(theme.muted_foreground)
                        .child(format!("{} · 可编辑文本", syntax_language(&object.key))),
                )
                .child(
                    Input::new(&editor)
                        .flex_1()
                        .w_full()
                        .font_family(theme.mono_font_family.clone())
                        .text_size(theme.mono_font_size),
                )
                .into_any_element()
        } else if let Some(text) = self.preview_text.clone() {
            v_flex()
                .size_full()
                .w_full()
                .gap_2()
                .child(
                    div()
                        .text_size(tokens::text(11.))
                        .text_color(theme.muted_foreground)
                        .child(format!("{} · UTF-8", syntax_language(&object.key))),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .w_full()
                        .overflow_y_scrollbar()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.sidebar)
                        .p_3()
                        .font_family(theme.mono_font_family.clone())
                        .text_size(theme.mono_font_size)
                        .child(text),
                )
                .into_any_element()
        } else if kind == PreviewKind::Image {
            match self.preview_path.clone() {
                Some(path) => img(path)
                    .size_full()
                    .object_fit(ObjectFit::Contain)
                    .into_any_element(),
                None => div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(Icon::new(IconName::File).text_color(theme.muted_foreground))
                    .into_any_element(),
            }
        } else {
            v_flex()
                .size_full()
                .w_full()
                .items_center()
                .justify_center()
                .gap_3()
                .p_6()
                .child(Icon::new(IconName::Eye).text_color(theme.muted_foreground))
                .child(
                    div()
                        .text_size(tokens::text(15.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child("暂不支持应用内直接预览"),
                )
                .child(
                    div()
                        .max_w(px(420.))
                        .text_size(tokens::text(12.))
                        .text_color(theme.muted_foreground)
                        .child("此文件可下载到本机后使用系统应用打开；PDF、Office、视频等格式建议使用系统 Quick Look。"),
                )
                .into_any_element()
        };

        let can_open_system = kind == PreviewKind::System && self.preview_path.is_some();
        let meta_label = |label: &'static str| {
            div()
                .w(px(110.))
                .flex_shrink_0()
                .text_size(tokens::text(12.))
                .text_color(theme.muted_foreground)
                .child(label)
        };
        let meta_value = |value: String| {
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(tokens::text(12.))
                .child(value)
        };
        div()
            .absolute()
            .inset_0()
            .occlude()
            .key_context("Overlay")
            .on_action(
                cx.listener(|this, _: &UnifiedDismiss, _, cx| this.close_preview_overlay(cx)),
            )
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.overlay)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _window, cx| this.close_preview_overlay(cx)),
            )
            .child(
                v_flex()
                    .w(px(800.))
                    .bg(theme.background)
                    .border_1()
                    .border_color(theme.border)
                    .rounded(px(10.))
                    .shadow_lg()
                    .overflow_hidden()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .gap_3()
                            .px_4()
                            .pt_4()
                            .pb_3()
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(tokens::text(16.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(name.clone()),
                            )
                            .child(
                                Button::new("close-preview-overlay")
                                    .icon(Icon::new(IconName::Close))
                                    .ghost()
                                    .with_size(Size::Small)
                                    .tooltip("关闭")
                                    .on_click(
                                        cx.listener(|this, _, _, cx| {
                                            this.close_preview_overlay(cx)
                                        }),
                                    ),
                            ),
                    )
                    .child(
                        div().w_full().px_4().child(
                            div()
                                .h(px(350.))
                                .w_full()
                                .overflow_hidden()
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.sidebar)
                                .child(preview_content),
                        ),
                    )
                    .child(
                        h_flex()
                            .mx_4()
                            .px_4()
                            .py_3()
                            .gap_5()
                            .border_l_1()
                            .border_r_1()
                            .border_b_1()
                            .border_color(theme.border)
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(meta_label("文件大小"))
                                    .child(meta_value(format!(
                                        "{} ({} B)",
                                        format_size(object.size),
                                        format_integer_grouped(object.size)
                                    ))),
                            )
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(meta_label("文件类型"))
                                    .child(meta_value(file_type)),
                            ),
                    )
                    .child(
                        h_flex()
                            .mx_4()
                            .my_3()
                            .px_4()
                            .py_3()
                            .gap_3()
                            .border_1()
                            .border_color(theme.border)
                            .child(meta_label("原始文件URL"))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(tokens::text(12.))
                                    .child(object.key.clone()),
                            )
                            .when(self.preview_text.is_some(), |row| {
                                if self.text_editor.is_some() {
                                    row.child(
                                        Button::new("preview-overlay-save-text")
                                            .label("保存并上传")
                                            .primary()
                                            .with_size(Size::Small)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.confirm_and_save_text_edit(window, cx)
                                            })),
                                    )
                                } else {
                                    row.child(
                                        Button::new("preview-overlay-edit-text")
                                            .label("编辑")
                                            .with_size(Size::Small)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.start_text_edit(window, cx)
                                            })),
                                    )
                                }
                            })
                            .when(can_open_system, |row| {
                                row.child(
                                    Button::new("preview-overlay-quicklook")
                                        .label("系统预览")
                                        .with_size(Size::Small)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.open_system_preview(cx)
                                        })),
                                )
                            })
                            .child(
                                Button::new("preview-overlay-download")
                                    .label("下载")
                                    .ghost()
                                    .with_size(Size::Small)
                                    .disabled(self.downloading)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.start_object_download(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("preview-overlay-copy-url")
                                    .label("复制")
                                    .ghost()
                                    .with_size(Size::Small)
                                    .disabled(self.copying_url)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.copy_object_url(cx)),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_object_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let mut menu = v_flex()
            .w(px(154.))
            .py_2()
            .rounded(px(8.))
            .border_1()
            .border_color(theme.border)
            .bg(theme.background)
            .occlude()
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation());

        for (item_ix, item) in object_menu_items().into_iter().enumerate() {
            let color = if item == ObjectMenuItem::Delete {
                theme.danger
            } else {
                theme.foreground
            };
            menu = menu.child(
                h_flex()
                    .id(("object-menu-item", item_ix))
                    .mx_1()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .rounded(px(6.))
                    .text_size(tokens::text(13.))
                    .text_color(color)
                    .hover(|row| row.bg(theme.accent))
                    .child(Icon::new(object_menu_item_icon(item)).text_color(color))
                    .child(object_menu_item_label(item))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.handle_object_menu_item(item, window, cx)
                    })),
            );
        }
        menu.into_any_element()
    }

    fn render_top_more_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let has_selection = !self.selected_object_keys_vec().is_empty();
        let mut menu = v_flex()
            .w(px(120.))
            .py_2()
            .rounded(px(8.))
            .border_1()
            .border_color(theme.border)
            .bg(theme.background)
            .occlude()
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation());

        for (item_ix, item) in top_more_menu_items().into_iter().enumerate() {
            let disabled = !has_selection
                && matches!(
                    item,
                    TopMoreMenuItem::CopyTo | TopMoreMenuItem::MoveTo | TopMoreMenuItem::Delete
                );
            let color = if disabled {
                theme.muted_foreground
            } else if item == TopMoreMenuItem::Delete {
                theme.danger
            } else {
                theme.foreground
            };
            menu = menu.child(
                h_flex()
                    .id(("top-more-menu-item", item_ix))
                    .mx_1()
                    .px_3()
                    .py_2()
                    .rounded(px(6.))
                    .text_size(tokens::text(13.))
                    .text_color(color)
                    .when(!disabled, |row| row.hover(|row| row.bg(theme.accent)))
                    .child(top_more_menu_item_label(item))
                    .when(!disabled, |row| {
                        row.on_click(cx.listener(move |this, _, window, cx| {
                            this.handle_top_more_menu_item(item, window, cx)
                        }))
                    }),
            );
        }
        menu.into_any_element()
    }

    fn render_details_overlay(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(object) = self.selected_cloud_object() else {
            return div().into_any_element();
        };
        let rows = [
            ("名称", display_name(&object.key).to_string(), false),
            ("Key", object.key.clone(), true),
            ("大小", format_size(object.size), false),
            (
                "文件类型",
                object
                    .mime_type
                    .clone()
                    .unwrap_or_else(|| "未知类型".into()),
                false,
            ),
            (
                "ETag",
                object.etag.clone().unwrap_or_else(|| "-".into()),
                true,
            ),
            ("上传时间", format_time(object.put_time_millis), false),
        ];

        let mut content = v_flex()
            .mx_4()
            .mb_4()
            .border_1()
            .border_color(theme.border)
            .rounded(px(8.));
        for (label, value, mono) in rows {
            let mut value_el = div().flex_1().min_w_0().truncate().child(value);
            if mono {
                value_el = value_el
                    .font_family(theme.mono_font_family.clone())
                    .text_size(theme.mono_font_size);
            }
            content = content.child(
                h_flex()
                    .px_4()
                    .py_2()
                    .gap_4()
                    .border_b_1()
                    .border_color(theme.border)
                    .text_size(tokens::text(13.))
                    .child(
                        div()
                            .w(px(96.))
                            .flex_shrink_0()
                            .text_color(theme.muted_foreground)
                            .child(label),
                    )
                    .child(value_el),
            );
        }

        div()
            .absolute()
            .inset_0()
            .occlude()
            .key_context("Overlay")
            .on_action(
                cx.listener(|this, _: &UnifiedDismiss, _, cx| this.close_details_overlay(cx)),
            )
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.overlay)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _window, cx| this.close_details_overlay(cx)),
            )
            .child(
                v_flex()
                    .w(px(560.))
                    .bg(theme.background)
                    .border_1()
                    .border_color(theme.border)
                    .rounded(px(10.))
                    .shadow_lg()
                    .overflow_hidden()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .gap_3()
                            .px_4()
                            .py_3()
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(tokens::text(16.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child("对象详情"),
                            )
                            .child(
                                Button::new("close-details-overlay")
                                    .icon(Icon::new(IconName::Close))
                                    .ghost()
                                    .with_size(Size::Small)
                                    .tooltip("关闭")
                                    .on_click(
                                        cx.listener(|this, _, _, cx| {
                                            this.close_details_overlay(cx)
                                        }),
                                    ),
                            ),
                    )
                    .child(content),
            )
            .into_any_element()
    }

    /// 「关于」弹层：无输入框，按弹层规范走 Overlay context + UnifiedDismiss，
    /// 遮罩点击关，卡片阻断冒泡。
    fn render_about_overlay(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .absolute()
            .inset_0()
            .occlude()
            .key_context("Overlay")
            .on_action(cx.listener(|this, _: &UnifiedDismiss, _, cx| this.close_about_overlay(cx)))
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.overlay)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _window, cx| this.close_about_overlay(cx)),
            )
            .child(
                v_flex()
                    .w(px(420.))
                    .bg(theme.background)
                    .border_1()
                    .border_color(theme.border)
                    .rounded(px(10.))
                    .shadow_lg()
                    .overflow_hidden()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    // 标题栏：标题 + 关闭按钮（规范：标题栏右侧 ✕）
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .items_center()
                            .px_4()
                            .py_3()
                            .child(
                                div()
                                    .text_size(tokens::text(16.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child("关于 CloudStorage"),
                            )
                            .child(
                                Button::new("close-about-overlay")
                                    .icon(Icon::new(IconName::Close))
                                    .ghost()
                                    .with_size(Size::Small)
                                    .tooltip("关闭")
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.close_about_overlay(cx)),
                                    ),
                            ),
                    )
                    .child(
                        // 内容：应用标识 + 信息卡
                        v_flex()
                            .px_4()
                            .pb_4()
                            .gap_3()
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        div()
                                            .size(px(56.))
                                            .rounded(px(14.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .overflow_hidden()
                                            .shadow_sm()
                                            .child(
                                                img(Arc::new(gpui::Image::from_bytes(
                                                    gpui::ImageFormat::Png,
                                                    crate::APP_ICON_PNG.to_vec(),
                                                )))
                                                .size(px(56.))
                                                .object_fit(ObjectFit::Cover),
                                            ),
                                    )
                                    .child(
                                        v_flex()
                                            .gap_0p5()
                                            .child(
                                                div()
                                                    .text_size(tokens::text(18.))
                                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                                    .text_color(theme.foreground)
                                                    .child("CloudStorage"),
                                            )
                                            .child(
                                                div()
                                                    .text_size(tokens::text(12.))
                                                    .text_color(theme.muted_foreground)
                                                    .child(format!(
                                                        "版本 {} · macOS 14+",
                                                        env!("CARGO_PKG_VERSION")
                                                    )),
                                            ),
                                    ),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .rounded(px(10.))
                                    .border_1()
                                    .border_color(theme.border)
                                    .bg(theme.sidebar)
                                    .p_3()
                                    .child(
                                        v_flex()
                                            .gap_2()
                                            .child(about_kv(
                                                "应用定位",
                                                "高性能三栏 Workspace，键盘优先。",
                                                theme,
                                            ))
                                            .child(about_kv(
                                                "技术栈",
                                                "Rust · GPUI · Tokio · SQLite · Keychain",
                                                theme,
                                            ))
                                            .child(about_kv(
                                                "数据安全",
                                                "Secret 只存 macOS Keychain，不落盘。",
                                                theme,
                                            ))
                                            .child(about_kv(
                                                "支持服务商",
                                                "Qiniu Kodo · Aliyun OSS",
                                                theme,
                                            )),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn close_about_overlay(&mut self, cx: &mut Context<Self>) {
        self.about_overlay_open = false;
        cx.notify();
    }

    fn render_rename_overlay(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some((old_key, editor)) = self.renaming.as_ref() else {
            return div().into_any_element();
        };
        let current_name = editor.read(cx).value().to_string();
        let validation_message = rename_validation_message(old_key, &current_name, &self.entries);
        let can_confirm = !self.renaming_busy && validation_message.is_none();

        div()
            .absolute()
            .inset_0()
            .occlude()
            .key_context("Renaming")
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.overlay)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                    if !this.renaming_busy {
                        this.cancel_rename(cx);
                    }
                }),
            )
            .child(
                v_flex()
                    .w(px(480.))
                    .bg(theme.background)
                    .border_1()
                    .border_color(theme.border)
                    .rounded(px(10.))
                    .shadow_lg()
                    .overflow_hidden()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .gap_3()
                            .px_6()
                            .pt_5()
                            .pb_4()
                            .child(
                                div()
                                    .text_size(tokens::text(16.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child("重命名"),
                            )
                            .child(
                                Button::new("close-rename-overlay")
                                    .icon(Icon::new(IconName::Close))
                                    .ghost()
                                    .with_size(Size::Small)
                                    .tooltip("关闭")
                                    .disabled(self.renaming_busy)
                                    .on_click(cx.listener(|this, _, _, cx| this.cancel_rename(cx))),
                            ),
                    )
                    .child(
                        v_flex()
                            .px_6()
                            .gap_3()
                            .child(
                                h_flex()
                                    .gap_4()
                                    .items_center()
                                    .child(
                                        div()
                                            .w(px(74.))
                                            .flex_shrink_0()
                                            .text_size(tokens::text(13.))
                                            .text_color(theme.foreground)
                                            .child("原路径："),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(tokens::text(13.))
                                            .child(old_key.clone()),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_4()
                                    .items_center()
                                    .child(
                                        div()
                                            .w(px(74.))
                                            .flex_shrink_0()
                                            .text_size(tokens::text(13.))
                                            .text_color(theme.foreground)
                                            .child("重命名："),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .child(Input::new(editor).small()),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_4()
                                    .child(div().w(px(74.)).flex_shrink_0())
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(tokens::text(12.))
                                            .text_color(theme.muted_foreground)
                                            .line_height(px(20.))
                                            .child("请注意，若您针对目录重命名则该目录下所有路径名称将一并修改"),
                                    ),
                            )
                            .children(validation_message.map(|message| {
                                h_flex()
                                    .gap_4()
                                    .child(div().w(px(74.)).flex_shrink_0())
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(tokens::text(12.))
                                            .text_color(theme.danger)
                                            .line_height(px(20.))
                                            .child(message),
                                    )
                            })),
                    )
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .px_6()
                            .pt_5()
                            .pb_5()
                            .child(
                                Button::new("cancel-rename")
                                    .label("取消")
                                    .disabled(self.renaming_busy)
                                    .on_click(cx.listener(|this, _, _, cx| this.cancel_rename(cx))),
                            )
                            .child(
                                Button::new("confirm-rename")
                                    .label("确认修改")
                                    .primary()
                                    .disabled(!can_confirm)
                                    .on_click(cx.listener(|this, _, _, cx| this.commit_rename(cx))),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_create_folder_overlay(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(editor) = self.create_folder_input.as_ref() else {
            return div().into_any_element();
        };
        let folder_name = editor.read(cx).value().to_string();
        let validation_message = create_folder_validation_message(
            self.current_prefix.as_deref(),
            &folder_name,
            &self.entries,
        );
        let can_confirm = !self.creating_folder && validation_message.is_none();
        let parent = self.current_prefix.as_deref().unwrap_or("/").to_string();

        div()
            .absolute()
            .inset_0()
            .occlude()
            .key_context("Renaming")
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.overlay)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                    this.close_create_folder_overlay(cx);
                }),
            )
            .child(
                v_flex()
                    .w(px(480.))
                    .bg(theme.background)
                    .border_1()
                    .border_color(theme.border)
                    .rounded(px(10.))
                    .shadow_lg()
                    .overflow_hidden()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .gap_3()
                            .px_6()
                            .pt_5()
                            .pb_4()
                            .child(
                                div()
                                    .text_size(tokens::text(16.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child("新建目录"),
                            )
                            .child(
                                Button::new("close-create-folder-overlay")
                                    .icon(Icon::new(IconName::Close))
                                    .ghost()
                                    .with_size(Size::Small)
                                    .tooltip("关闭")
                                    .disabled(self.creating_folder)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.close_create_folder_overlay(cx)
                                    })),
                            ),
                    )
                    .child(
                        v_flex()
                            .px_6()
                            .gap_3()
                            .child(
                                h_flex()
                                    .gap_4()
                                    .items_center()
                                    .child(
                                        div()
                                            .w(px(74.))
                                            .flex_shrink_0()
                                            .text_size(tokens::text(13.))
                                            .text_color(theme.foreground)
                                            .child("所在目录："),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(tokens::text(13.))
                                            .child(parent),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_4()
                                    .items_center()
                                    .child(
                                        div()
                                            .w(px(74.))
                                            .flex_shrink_0()
                                            .text_size(tokens::text(13.))
                                            .text_color(theme.foreground)
                                            .child("目录名："),
                                    )
                                    .child(
                                        div().flex_1().min_w_0().child(Input::new(editor).small()),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_4()
                                    .child(div().w(px(74.)).flex_shrink_0())
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(tokens::text(12.))
                                            .text_color(theme.muted_foreground)
                                            .line_height(px(20.))
                                            .child("对象存储目录会创建为以 / 结尾的占位对象。"),
                                    ),
                            )
                            .children(validation_message.map(|message| {
                                h_flex()
                                    .gap_4()
                                    .child(div().w(px(74.)).flex_shrink_0())
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(tokens::text(12.))
                                            .text_color(theme.danger)
                                            .line_height(px(20.))
                                            .child(message),
                                    )
                            })),
                    )
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .px_6()
                            .pt_5()
                            .pb_5()
                            .child(
                                Button::new("cancel-create-folder")
                                    .label("取消")
                                    .disabled(self.creating_folder)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.close_create_folder_overlay(cx)
                                    })),
                            )
                            .child(
                                Button::new("confirm-create-folder")
                                    .label(if self.creating_folder {
                                        "创建中…"
                                    } else {
                                        "创建"
                                    })
                                    .primary()
                                    .disabled(!can_confirm)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.commit_create_folder(cx)),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_copy_move_overlay(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(state) = self.copy_move.as_ref() else {
            return div().into_any_element();
        };
        let Some(bucket) = self.selected_bucket.as_ref() else {
            return div().into_any_element();
        };
        let Some(provider) = self.selected_provider_kind() else {
            return div().into_any_element();
        };
        let title = match state.mode {
            CopyMoveMode::Copy => "复制到",
            CopyMoveMode::Move => "移动到",
        };
        let action = match state.mode {
            CopyMoveMode::Copy => "复制",
            CopyMoveMode::Move => "移动",
        };
        let query = state.filter.read(cx).value().to_string();
        let validation_message = self.copy_move_validation_message(cx);
        let is_loading_dirs = state.state == AsyncState::Loading;
        let can_confirm = can_commit_copy_move(
            self.copy_move_busy,
            &state.state,
            validation_message.as_deref(),
        );
        let target_prefix = self
            .copy_move_target_prefix(cx)
            .unwrap_or_else(|_| state.target_prefix.clone());
        let location = format!(
            "{}:// {} / {}",
            provider_url_scheme(provider),
            bucket,
            if target_prefix.is_empty() {
                String::new()
            } else {
                target_prefix.clone()
            }
        );
        let query_lower = query.trim().to_lowercase();
        let directories: Vec<String> = state
            .entries
            .iter()
            .filter_map(|entry| match entry {
                ListingEntry::CommonPrefix(prefix) => Some(prefix.clone()),
                ListingEntry::Object(_) => None,
            })
            .filter(|prefix| {
                query_lower.is_empty()
                    || display_name(prefix)
                        .to_lowercase()
                        .contains(query_lower.as_str())
                    || prefix.to_lowercase().contains(query_lower.as_str())
            })
            .collect();

        let mut list = v_flex()
            .id("copy-move-dir-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        if is_loading_dirs {
            list = list.child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .text_color(theme.muted_foreground)
                    .text_size(tokens::text(13.))
                    .child(Spinner::new())
                    .child("加载目录中…"),
            );
        } else if let AsyncState::Failed(message) = &state.state {
            list = list.child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .text_color(theme.danger)
                    .text_size(tokens::text(13.))
                    .child(Icon::new(IconName::TriangleAlert))
                    .child(message.clone()),
            );
        } else if directories.is_empty() {
            list = list.child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.muted_foreground)
                    .text_size(tokens::text(13.))
                    .child("没有可显示的目录，可直接输入目标目录"),
            );
        }
        for (ix, prefix) in directories.into_iter().enumerate() {
            let target = prefix.clone();
            list = list.child(
                h_flex()
                    .id(("copy-move-dir", ix))
                    .px_4()
                    .py_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .text_size(tokens::text(13.))
                    .hover(|row| row.bg(theme.accent))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            this.enter_copy_move_prefix(target.clone(), window, cx)
                        }),
                    )
                    .child(Icon::new(IconName::Folder).text_color(theme.accent_foreground))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(display_name(&prefix).to_string()),
                    ),
            );
        }

        div()
            .absolute()
            .inset_0()
            .occlude()
            .key_context("Renaming")
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.overlay)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                    this.close_copy_move_overlay(cx);
                }),
            )
            .child(
                v_flex()
                    .w(px(760.))
                    .h(px(560.))
                    .bg(theme.background)
                    .border_1()
                    .border_color(theme.border)
                    .rounded(px(10.))
                    .shadow_lg()
                    .overflow_hidden()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .gap_3()
                            .px_6()
                            .pt_5()
                            .pb_3()
                            .child(
                                div()
                                    .text_size(tokens::text(16.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(title),
                            )
                            .child(
                                Button::new("close-copy-move-overlay")
                                    .icon(Icon::new(IconName::Close))
                                    .ghost()
                                    .with_size(Size::Small)
                                    .tooltip("关闭")
                                    .disabled(self.copy_move_busy)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.close_copy_move_overlay(cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .mx_6()
                            .mb_3()
                            .px_3()
                            .py_2()
                            .rounded(px(4.))
                            .bg(theme.sidebar)
                            .text_size(tokens::text(12.))
                            .text_color(theme.muted_foreground)
                            .child(location),
                    )
                    .child(div().mx_6().mb_3().child(Input::new(&state.filter).small()))
                    .child(
                        h_flex()
                            .mx_6()
                            .px_4()
                            .py_2()
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.sidebar)
                            .text_size(tokens::text(12.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("名称"),
                    )
                    .child(
                        div()
                            .mx_6()
                            .flex_1()
                            .min_h_0()
                            .border_1()
                            .border_t_0()
                            .border_color(theme.border)
                            .child(list),
                    )
                    .children(validation_message.map(|message| {
                        div()
                            .mx_6()
                            .mt_3()
                            .text_size(tokens::text(12.))
                            .text_color(theme.danger)
                            .child(message)
                    }))
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .px_6()
                            .py_4()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .text_size(tokens::text(12.))
                                    .text_color(theme.muted_foreground)
                                    .child(format!("{} 个对象", state.source_keys.len()))
                                    .child("遇到同名文件：询问"),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .children((!state.target_prefix.is_empty()).then(|| {
                                        Button::new("copy-move-go-up")
                                            .label("上一级")
                                            .with_size(Size::Small)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.go_up_copy_move_prefix(window, cx)
                                            }))
                                    }))
                                    .child(
                                        Button::new("cancel-copy-move")
                                            .label("取消")
                                            .with_size(Size::Small)
                                            .disabled(self.copy_move_busy)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.close_copy_move_overlay(cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("confirm-copy-move")
                                            .label(if self.copy_move_busy {
                                                "执行中…"
                                            } else {
                                                action
                                            })
                                            .primary()
                                            .with_size(Size::Small)
                                            .disabled(!can_confirm)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.commit_copy_move(cx)
                                            })),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }

    // ---- Action 处理（与菜单/快捷键共享） ----

    fn handle_toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        cx.notify();
    }

    /// ⌘,：打开设置模态。
    fn handle_open_settings(
        &mut self,
        _: &OpenSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings_modal.is_some() || self.palette.is_some() || self.add_modal.is_some() {
            return;
        }
        let path = self.settings_path.clone();
        let settings = self.settings.clone();
        let modal = cx.new(|cx| SettingsModal::new(settings, path, window, cx));
        cx.observe_in(&modal, window, Self::handle_settings_modal_changed)
            .detach();
        modal.update(cx, |modal, cx| modal.focus_first(window, cx));
        self.settings_modal = Some(modal);
        cx.notify();
    }

    /// 「关于」弹层（菜单在设置上方 / 命令面板共享）。与设置模态互斥。
    fn handle_open_about(&mut self, _: &OpenAbout, _window: &mut Window, cx: &mut Context<Self>) {
        if self.about_overlay_open
            || self.settings_modal.is_some()
            || self.palette.is_some()
            || self.add_modal.is_some()
        {
            return;
        }
        self.about_overlay_open = true;
        cx.notify();
    }

    /// 设置模态观察：每次保存（弹窗保持打开，验收反馈）取出并应用新值；
    /// 仅 closed（取消/Esc/点遮罩）时丢弃实体。
    fn handle_settings_modal_changed(
        &mut self,
        modal: Entity<SettingsModal>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let closed = modal.read(cx).closed();
        // 保存成功（弹窗不关）：就地应用新设置 + 底部提示
        let saved = modal.update(cx, |modal, _| modal.take_saved());
        if let Some((settings, changed)) = saved {
            self.settings = settings;
            crate::theme::apply_settings(&self.settings, Some(window), cx);
            self.engine
                .set_max_parallel(self.settings.transfer_concurrency as usize);
            if changed {
                self.download_message = Some(DownloadMessage {
                    is_error: false,
                    text: "设置已保存，已对后续操作生效".into(),
                });
            }
            cx.notify();
        }
        if !closed {
            return;
        }
        self.settings_modal = None;
        window.focus(&self.focus_handle);
        cx.notify();
    }

    fn handle_quit(&mut self, _: &Quit, window: &mut Window, cx: &mut Context<Self>) {
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
                            let _ = cx.update(|cx| cx.quit());
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
                            let _ = cx.update(|cx| cx.quit());
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

    fn handle_close_window(
        &mut self,
        _: &CloseWindow,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        // 关闭窗口（gpui 0.2.2 在 macOS 15 上的绕行方案，实测记录见 docs/notes/gpui-api-notes.md）：
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

    fn handle_open_command_palette(
        &mut self,
        _: &OpenCommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.is_some() {
            return; // 已打开（⌘K 重复触发为无操作）
        }
        let extra = self.bucket_jump_commands(cx);
        let palette = cx.new(|cx| CommandPaletteView::new(window, cx, extra));
        // 面板关闭（open=false）后由观察者丢弃实体并归还焦点。
        cx.observe_in(&palette, window, Self::handle_palette_changed)
            .detach();
        palette.update(cx, |palette, cx| palette.focus_input(window, cx));
        self.palette = Some(palette);
        cx.notify();
    }

    /// 「跳转到 Bucket」动态命令：当前账号下的每个空间一条，点击即选中
    /// （触发对象列表加载）。命令面板每次打开都重建，此处数据天然最新。
    ///
    /// 为什么不走 Action 派发：`execute_selected` 先 close 面板（焦点立即
    /// 归还 Workspace 根），Handler 里的 `dispatch_action` 是 **deferred**——
    /// 捕获的是派发调用时刻的焦点（已关闭的面板输入框），下一帧按该焦点
    /// 找 dispatch tree 节点落空，Action 静默丢失（E2 验收问题根因）。
    /// 因此直接经 WeakEntity 调用 WorkspaceView 方法，绕开焦点链。
    fn bucket_jump_commands(&self, cx: &Context<Self>) -> Vec<PaletteCommand> {
        let weak = cx.weak_entity();
        self.buckets
            .iter()
            .map(|bucket| {
                let name = bucket.name.clone();
                let weak = weak.clone();
                PaletteCommand::handler(
                    format!("跳转：{name}"),
                    move |_window: &mut gpui::Window, cx: &mut gpui::App| {
                        let _ = weak.update(cx, |this, cx| {
                            this.select_bucket(&name, cx);
                        });
                    },
                )
            })
            .collect()
    }

    /// 命令面板「跳转：Bucket」入口（等价于侧栏点击：换桶 + 清对象区）。
    fn handle_select_bucket_by_name(
        &mut self,
        action: &SelectBucketByName,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_bucket(&action.0, cx);
    }

    /// 面板状态观察：面板自己调用 close() 置 open=false 时，这里收尾——
    /// 丢弃实体（遮罩与卡片随之消失）并把焦点归还 Workspace 根节点。
    /// 在 observe 回调里丢弃面板是安全的：回调参数持有的 Entity 让它
    /// 存活到本次调用结束。
    fn handle_palette_changed(
        &mut self,
        palette: Entity<CommandPaletteView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if palette.read(cx).open() {
            return; // 过滤/选行等常规通知，不处理
        }
        self.palette = None;
        window.focus(&self.focus_handle);
        cx.notify();
    }

    // ---- 渲染 ----

    /// 命令面板遮罩：点击空白处关闭；卡片自身的 on_mouse_down 会阻止
    /// 冒泡，所以点卡片内部不会触发这里。
    fn render_palette_overlay(
        &self,
        palette: &Entity<CommandPaletteView>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .absolute()
            .inset_0()
            .occlude() // 挡住下层元素的鼠标交互
            .bg(theme.overlay)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    if let Some(palette) = &this.palette {
                        palette.update(cx, |palette, cx| palette.close(window, cx));
                    }
                }),
            )
            .child(palette.clone())
    }

    fn render_title_bar(&self, _theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let sidebar_icon = if self.sidebar_collapsed {
            IconName::PanelLeftOpen
        } else {
            IconName::PanelLeftClose
        };
        TitleBar::new().child(
            h_flex()
                .w_full()
                .justify_between()
                // 左：Sidebar 开关
                .child(
                    Button::new("toggle-sidebar")
                        .icon(Icon::new(sidebar_icon))
                        .ghost()
                        .with_size(Size::Small)
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
                )
                // 中：标题
                .child("CloudStorage")
                .child(div().w(px(32.))),
        )
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        cx.notify();
    }

    fn render_body(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let group_id: &'static str = if self.sidebar_collapsed {
            "workspace-layout-content-only"
        } else {
            "workspace-layout-no-inspector"
        };

        let mut body = h_flex().flex_1().min_h_0();
        if self.sidebar_collapsed {
            body = body.child(self.render_sidebar_rail(theme, cx));
        }

        let mut group = h_resizable(group_id);
        if !self.sidebar_collapsed {
            group = group.child(
                resizable_panel()
                    .size(SIDEBAR_DEFAULT)
                    .size_range(SIDEBAR_MIN..SIDEBAR_MAX)
                    .child(self.render_sidebar(theme, cx).into_any_element()),
            );
        }
        group = group.child(self.render_content(theme, cx).into_any_element());

        body.child(
            // ResizablePanelGroup 自身渲染为 size_full 容器，需包一层分配剩余空间。
            div().flex_1().min_w_0().h_full().child(group),
        )
    }

    /// 侧栏分组标题（"账户"/"空间"）。
    fn sidebar_section_label(&self, theme: &Theme, label: &str) -> impl IntoElement {
        div()
            .px_3()
            .pt_2()
            .pb_1()
            .text_size(tokens::text(11.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(theme.muted_foreground)
            .child(label.to_string())
    }

    /// 展开态 Sidebar（自建，宽度由 Resizable 面板控制，内容 w_full 填充）。
    fn render_sidebar(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let mut sidebar = v_flex()
            .h_full()
            .w_full()
            .overflow_hidden()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .border_r_1()
            .border_color(theme.sidebar_border)
            .child(self.sidebar_section_label(theme, "账户"))
            .children(self.render_account_rows(theme, cx))
            .child(self.render_add_account_row(theme, cx));

        if self.selected_account_id.is_some() {
            sidebar = sidebar
                .child(self.sidebar_section_label(theme, "空间"))
                .children(self.render_bucket_rows(theme, cx));
        } else {
            sidebar = sidebar
                .child(self.sidebar_section_label(theme, "空间"))
                .child(
                    div()
                        .px_3()
                        .py_1()
                        .text_size(tokens::text(12.))
                        .text_color(theme.muted_foreground)
                        .child("先选择一个账号"),
                );
        }

        sidebar.child(div().flex_1())
    }

    /// 账户区：加载态 / 错误重试 / 真实账号行（点击选中并加载空间）。
    fn render_account_rows(&self, theme: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        match &self.accounts_state {
            AsyncState::Loading => vec![
                self.sidebar_status_row(theme, "正在加载账号…")
                    .into_any_element(),
            ],
            AsyncState::Failed(msg) => vec![
                self.sidebar_error_row(
                    theme,
                    "sidebar-accounts-error",
                    msg,
                    cx.listener(|this, _, _, cx| this.load_accounts(cx)),
                )
                .into_any_element(),
            ],
            AsyncState::Idle => {
                if self.accounts.is_empty() {
                    return vec![
                        div()
                            .px_3()
                            .py_1()
                            .text_size(tokens::text(12.))
                            .text_color(theme.muted_foreground)
                            .child("还没有账号，点下方添加")
                            .into_any_element(),
                    ];
                }
                self.accounts
                    .iter()
                    .enumerate()
                    .map(|(ix, account)| {
                        let active =
                            self.selected_account_id.as_deref() == Some(account.id.as_str());
                        let id = account.id.clone();
                        self.sidebar_row(
                            theme,
                            SharedString::from(format!("account-row-{ix}")),
                            IconName::User,
                            &account.name,
                            active,
                            cx.listener(move |this, _, _, cx| this.select_account(&id, cx)),
                        )
                        .into_any_element()
                    })
                    .collect()
            }
        }
    }

    /// 「+ 添加账号」入口（与命令面板共享 AddAccount Action）。
    fn render_add_account_row(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("sidebar-add-account")
            .mx_2()
            .mt_1()
            .px_2()
            .py(px(5.))
            .rounded(px(6.))
            .flex()
            .items_center()
            .gap_2()
            .text_size(tokens::text(13.))
            .text_color(theme.muted_foreground)
            .hover(|row| {
                row.bg(theme.sidebar_accent)
                    .text_color(theme.sidebar_accent_foreground)
            })
            .on_click(cx.listener(|this, _, window, cx| {
                this.handle_open_add_modal(&AddAccount, window, cx);
            }))
            .child(Icon::new(IconName::Plus))
            .child("添加账号")
    }

    /// 空间区：加载态 / 错误重试 / 真实桶行（点击选中并加载对象）。
    fn render_bucket_rows(&self, theme: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        match &self.buckets_state {
            AsyncState::Loading => vec![
                self.sidebar_status_row(theme, "正在加载空间…")
                    .into_any_element(),
            ],
            AsyncState::Failed(msg) => {
                let mut rows = vec![
                    self.sidebar_error_row(
                        theme,
                        "sidebar-buckets-error",
                        msg,
                        cx.listener(|this, _, _, cx| this.retry_buckets(cx)),
                    )
                    .into_any_element(),
                ];
                if msg.contains("填写") || msg.contains("Bucket") {
                    if let Some(input) = &self.manual_bucket_input {
                        rows.push(
                            v_flex()
                                .px_2()
                                .pt_1()
                                .gap_1()
                                .child(
                                    div()
                                        .text_size(tokens::text(11.))
                                        .text_color(theme.muted_foreground)
                                        .child("空间名称"),
                                )
                                .child(Input::new(input))
                                .child(
                                    Button::new("add-manual-bucket")
                                        .label("添加空间")
                                        .primary()
                                        .with_size(Size::Small)
                                        .on_click(
                                            cx.listener(|this, _, _, cx| {
                                                this.add_manual_bucket(cx)
                                            }),
                                        ),
                                )
                                .into_any_element(),
                        );
                    } else {
                        rows.push(
                            div()
                                .px_2()
                                .pt_1()
                                .child(
                                    Button::new("open-manual-bucket")
                                        .label("输入 Bucket 名称…")
                                        .primary()
                                        .with_size(Size::Small)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.manual_bucket_input = Some(cx.new(|cx| {
                                                InputState::new(window, cx)
                                                    .placeholder("Bucket 名称")
                                            }));
                                            cx.notify();
                                        })),
                                )
                                .into_any_element(),
                        );
                    }
                }
                rows
            }
            AsyncState::Idle => {
                if self.buckets.is_empty() {
                    return vec![
                        div()
                            .px_3()
                            .py_1()
                            .text_size(tokens::text(12.))
                            .text_color(theme.muted_foreground)
                            .child("（此账号没有空间）")
                            .into_any_element(),
                    ];
                }
                self.buckets
                    .iter()
                    .enumerate()
                    .map(|(ix, bucket)| {
                        let active = self.selected_bucket.as_deref() == Some(bucket.name.as_str());
                        let name = bucket.name.clone();
                        self.sidebar_row(
                            theme,
                            SharedString::from(format!("bucket-row-{ix}")),
                            IconName::Folder,
                            &bucket.name,
                            active,
                            cx.listener(move |this, _, _, cx| this.select_bucket(&name, cx)),
                        )
                        .into_any_element()
                    })
                    .collect()
            }
        }
    }

    /// 非交互状态行（加载中）。
    fn sidebar_status_row(&self, theme: &Theme, label: &'static str) -> impl IntoElement {
        h_flex()
            .mx_2()
            .px_2()
            .py(px(5.))
            .gap_2()
            .text_size(tokens::text(12.))
            .text_color(theme.muted_foreground)
            .child(Spinner::new().with_size(Size::Small))
            .child(label)
    }

    /// 可点击的错误行（点击重试）。窄侧栏里必须换行，不能截成半个词。
    fn sidebar_error_row<F>(
        &self,
        theme: &Theme,
        id: &'static str,
        msg: &str,
        on_click: F,
    ) -> impl IntoElement
    where
        F: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    {
        v_flex()
            .id(id)
            .mx_2()
            .px_2()
            .py_2()
            .gap_1()
            .rounded(px(6.))
            .text_size(tokens::text(12.))
            .text_color(theme.danger)
            .hover(|row| row.bg(theme.sidebar_accent))
            .on_click(on_click)
            .child(
                h_flex()
                    .items_start()
                    .gap_2()
                    .child(Icon::new(IconName::TriangleAlert))
                    .child(div().flex_1().min_w_0().child(msg.to_string())),
            )
            .child(
                div()
                    .pl(px(22.))
                    .text_size(tokens::text(11.))
                    .text_color(theme.muted_foreground)
                    .child("点击重试"),
            )
    }

    /// 通用侧栏行（id 动态：账号/桶行）。
    fn sidebar_row<F>(
        &self,
        theme: &Theme,
        id: SharedString,
        icon: IconName,
        label: &str,
        active: bool,
        on_click: F,
    ) -> impl IntoElement
    where
        F: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    {
        div()
            .id(id)
            .mx_2()
            .px_2()
            .py(px(5.))
            .rounded(px(6.))
            .flex()
            .items_center()
            .gap_2()
            .text_size(tokens::text(13.))
            .when(active, |row| {
                row.bg(theme.sidebar_accent)
                    .text_color(theme.sidebar_accent_foreground)
            })
            .hover(|row| row.bg(theme.sidebar_accent))
            .on_click(on_click)
            .child(Icon::new(icon))
            .child(div().truncate().child(label.to_string()))
    }

    /// 折叠态 44px 图标栏（规范硬指标）。点击顶部按钮恢复展开。
    fn render_sidebar_rail(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w(RAIL_WIDTH)
            .h_full()
            .flex_shrink_0()
            .items_center()
            .pt_2()
            .pb_2()
            .gap_2()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .border_r_1()
            .border_color(theme.sidebar_border)
            .child(
                Button::new("rail-expand-sidebar")
                    .icon(Icon::new(IconName::PanelLeftOpen))
                    .ghost()
                    .with_size(Size::Small)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
            )
            .child(Icon::new(IconName::User))
            .child(Icon::new(IconName::Folder))
    }

    /// 中间内容区：对象列表（选中桶后异步加载，含前缀导航与翻页）。
    fn render_content(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(bucket) = self.selected_bucket.clone() else {
            return self
                .with_file_drop(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .bg(theme.background)
                        .text_color(theme.muted_foreground)
                        .child(Icon::new(IconName::Inbox))
                        .child(if self.selected_account_id.is_some() {
                            "选择一个 Bucket 查看对象列表"
                        } else {
                            "添加并选择账号后开始浏览"
                        }),
                    cx,
                )
                .into_any_element();
        };

        let mut content = self.with_file_drop(
            v_flex()
                .relative()
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_hidden()
                .bg(theme.background)
                .child(self.render_content_toolbar(theme, &bucket, cx)),
            cx,
        );
        // ⌘F 过滤条（开启时出现在 toolbar 与列表之间）
        if let Some(bar) = self.render_filter_bar(theme, cx) {
            content = content.child(bar);
        }
        if self.objects_state == AsyncState::Loading && self.entries.is_empty() {
            content = content.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .text_color(theme.muted_foreground)
                    .child(Spinner::new())
                    .child("加载对象列表中…"),
            );
            return content.into_any_element();
        }

        if let AsyncState::Failed(msg) = &self.objects_state {
            if self.entries.is_empty() {
                content = content.child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .text_color(theme.muted_foreground)
                        .child(Icon::new(IconName::TriangleAlert).text_color(theme.danger))
                        .child(div().max_w(px(480.)).child(msg.clone()))
                        .child(
                            Button::new("objects-retry")
                                .label("重试")
                                .with_size(Size::Small)
                                .on_click(cx.listener(|this, _, _, cx| this.reload_objects(cx))),
                        ),
                );
                return content.into_any_element();
            }
            // 翻页失败但已有数据：保留列表，顶部横幅提示
            content = content.child(
                h_flex()
                    .mx_3()
                    .mt_2()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded(px(6.))
                    .text_color(theme.danger)
                    .text_size(tokens::text(12.))
                    .child(Icon::new(IconName::TriangleAlert))
                    .child(format!("加载更多失败：{msg}")),
            );
        }

        content = content.child(self.render_object_list(theme, cx));
        content = content.child(self.render_blank_clear_layer(cx));
        if self.top_more_open {
            content = content.child(
                div()
                    .absolute()
                    .top(px(42.))
                    .left(px(268.))
                    .occlude()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(self.render_top_more_menu(theme, cx)),
            );
        }
        content.into_any_element()
    }

    /// 空白点击清空（Finder 语义）——canvas + window 级监听 + 几何命中检测。
    /// 不依赖容器事件分发顺序：钩子只处理「坐标在内容区但不在任何行内」
    /// 的点击（行点击由行自己的处理器负责，互不干扰，顺序无关）。
    /// 前两版 capture/bubble 方案因 gpui hit-test 只统计滚动 hitbox 链而
    /// 不可靠，已废弃。
    fn render_blank_clear_layer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.weak_entity();
        gpui::canvas(
            |bounds, window, _cx| {
                // 每帧执行：记录内容区 bounds
                if let Some(Some(view)) = window.root::<crate::WorkspaceView>() {
                    view.update(_cx, |this, _| {
                        *this.content_bounds.borrow_mut() = Some(bounds);
                    });
                }
            },
            move |_bounds, _state, window, _cx| {
                window.on_mouse_event({
                    let weak = weak.clone();
                    move |event: &MouseDownEvent, phase, _window, cx| {
                        // 只在 bubble 阶段处理一次
                        if phase != gpui::DispatchPhase::Bubble {
                            return;
                        }
                        let Some(view) = weak.upgrade() else {
                            return;
                        };
                        view.update(cx, |this, cx| {
                            let Some(content) = *this.content_bounds.borrow() else {
                                return;
                            };
                            if !content.contains(&event.position) {
                                return;
                            }
                            if event.modifiers.platform || event.modifiers.shift {
                                return;
                            }
                            let hit_row = this
                                .row_bounds
                                .borrow()
                                .iter()
                                .any(|b| b.contains(&event.position));
                            if hit_row {
                                return;
                            }
                            // 空白点击：有选择则清空（含修饰键——Finder 语义：
                            // 空白点击永远清空，⌘ 只对行点击有意义）
                            if this.selected_object_keys.is_empty()
                                && this.selected_object_key.is_none()
                            {
                                return;
                            }
                            this.clear_object_selection();
                            this.selection_before_capture = None;
                            cx.notify();
                        });
                    }
                });
            },
        )
    }

    /// 行 bounds 记录器：透明 canvas 包住一行，paint 阶段把自身 bounds
    /// 写入 row_bounds（供空白点击命中检测）。不拦截任何事件。
    fn row_bounds_recorder(&self) -> gpui::Canvas<()> {
        gpui::canvas(
            |bounds, window, _cx| {
                if let Some(Some(view)) = window.root::<crate::WorkspaceView>() {
                    view.update(_cx, |this, _| {
                        this.row_bounds.borrow_mut().push(bounds);
                    });
                }
            },
            |_bounds, _state, _window, _cx| {},
        )
    }

    /// ⌘F 过滤条：输入框 + 命中统计。context "ObjectFilter" 接 Esc 关闭。
    fn render_filter_bar(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let editor = self.object_filter.as_ref()?.clone();
        let total = self.entries.len();
        let matched = self.filtered_ix.as_ref().map(Vec::len).unwrap_or(total);
        let stats = if matched == total {
            format!("共 {total} 项")
        } else {
            format!("匹配 {matched} / 共 {total} 项")
        };
        Some(
            h_flex()
                .key_context("ObjectFilter")
                .w_full()
                .px_3()
                .py_1()
                .gap_2()
                .border_b_1()
                .border_color(theme.border)
                .child(div().flex_1().min_w_0().child(Input::new(&editor).small()))
                .child(
                    div()
                        .text_size(tokens::text(12.))
                        .text_color(theme.muted_foreground)
                        .child(stats),
                )
                .child(
                    Button::new("filter-close")
                        .icon(Icon::new(IconName::Close))
                        .ghost()
                        .with_size(Size::Small)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.close_object_filter(window, cx);
                        })),
                ),
        )
    }

    /// 内容区工具行：桶名 + 当前前缀 + 返回上一级 / 刷新。
    fn render_content_toolbar(
        &self,
        theme: &Theme,
        bucket: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let has_selection = !self.selected_object_keys.is_empty();
        v_flex()
            .w_full()
            .border_b_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .text_size(tokens::text(13.))
                    .child(
                        Button::new("toolbar-upload-files")
                            .label(if self.uploading {
                                "选择文件…"
                            } else {
                                "上传"
                            })
                            .primary()
                            .with_size(Size::Small)
                            .disabled(self.uploading)
                            .on_click(cx.listener(|this, _, _, cx| this.start_files_upload(cx))),
                    )
                    .child(
                        Button::new("toolbar-upload-folder")
                            .label("上传文件夹")
                            .with_size(Size::Small)
                            .disabled(self.uploading)
                            .on_click(cx.listener(|this, _, _, cx| this.start_folder_upload(cx))),
                    )
                    .child(
                        Button::new("toolbar-create-folder")
                            .label(if self.creating_folder {
                                "创建中…"
                            } else {
                                "新建目录"
                            })
                            .with_size(Size::Small)
                            .disabled(self.creating_folder)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_create_folder_overlay(window, cx)
                            })),
                    )
                    .child(
                        Button::new("toolbar-more")
                            .label("更多")
                            .with_size(Size::Small)
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_top_more_menu(cx))),
                    )
                    .child(
                        Button::new("toolbar-file-fragments")
                            .label("文件碎片")
                            .with_size(Size::Small)
                            .disabled(true),
                    )
                    .child(div().flex_1())
                    .children(has_selection.then(|| {
                        div()
                            .flex_shrink_0()
                            .text_size(tokens::text(12.))
                            .text_color(theme.muted_foreground)
                            .child(format!("已选择 {} 个对象", self.selected_object_keys.len()))
                    }))
                    .child(
                        Button::new("toolbar-download")
                            .label("下载")
                            .with_size(Size::Small)
                            .disabled(!has_selection || self.downloading)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start_object_download(window, cx)
                            })),
                    )
                    .child(
                        Button::new("toolbar-clear-selection")
                            .label("取消选择")
                            .ghost()
                            .with_size(Size::Small)
                            .disabled(!has_selection)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.clear_object_selection();
                                cx.notify();
                            })),
                    )
                    .children(self.current_prefix.is_some().then(|| {
                        Button::new("objects-go-up")
                            .icon(Icon::new(IconName::ArrowLeft))
                            .ghost()
                            .with_size(Size::Small)
                            .on_click(cx.listener(|this, _, _, cx| this.go_up(cx)))
                    }))
                    .child(
                        Button::new("objects-refresh")
                            .label("刷新")
                            .ghost()
                            .with_size(Size::Small)
                            .on_click(cx.listener(|this, _, _, cx| this.reload_objects(cx))),
                    ),
            )
            .child({
                let mut path = h_flex()
                    .w_full()
                    .px_3()
                    .pb_2()
                    .gap_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_size(tokens::text(12.))
                    .child(
                        div()
                            .px_1()
                            .rounded(px(4.))
                            .flex_shrink_0()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .hover(|el| el.bg(theme.accent))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| this.open_bucket_root(cx)),
                            )
                            .child(bucket.to_string()),
                    );
                let segments = breadcrumb_prefixes(self.current_prefix.as_deref());
                // 长路径折叠：首段后插入 `…`（点击直达被收起的最后一层），
                // 只保留首段 + 最后两段。空路径（仅省略号）不可能出现：
                // 折叠要求段数 > BREADCRUMB_MAX_VISIBLE（≥ 5）。
                let (collapsed, segments) = match collapse_breadcrumb(&segments) {
                    Some((collapsed_prefix, tail)) => {
                        let first = segments[0].clone();
                        let first_target = first.1.clone();
                        path = path
                            .child(div().text_color(theme.muted_foreground).child("/"))
                            .child(
                                div()
                                    .px_1()
                                    .rounded(px(4.))
                                    .min_w_0()
                                    .truncate()
                                    .text_color(theme.muted_foreground)
                                    .hover(|el| el.bg(theme.accent).text_color(theme.foreground))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, _, _, cx| {
                                            this.open_prefix(first_target.clone(), cx)
                                        }),
                                    )
                                    .child(first.0),
                            )
                            .child(div().text_color(theme.muted_foreground).child("/"))
                            .child(
                                div()
                                    .px_1()
                                    .rounded(px(4.))
                                    .text_color(theme.muted_foreground)
                                    .hover(|el| el.bg(theme.accent).text_color(theme.foreground))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, _, _, cx| {
                                            this.open_prefix(collapsed_prefix.clone(), cx)
                                        }),
                                    )
                                    .child("…"),
                            );
                        (true, tail)
                    }
                    None => (false, segments),
                };
                let _ = collapsed;
                for (label, prefix) in segments {
                    let target_prefix = prefix.clone();
                    path = path
                        .child(
                            div()
                                .text_color(theme.muted_foreground)
                                .flex_shrink_0()
                                .child("/"),
                        )
                        .child(
                            div()
                                .px_1()
                                .rounded(px(4.))
                                .min_w_0()
                                .truncate()
                                .text_color(theme.muted_foreground)
                                .hover(|el| el.bg(theme.accent).text_color(theme.foreground))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        this.open_prefix(target_prefix.clone(), cx)
                                    }),
                                )
                                .child(label),
                        );
                }
                path
            })
    }

    /// 对象列表本体：表格列布局 + 行级操作列。
    /// 行 bounds 记入 row_bounds（paint 阶段），供空白点击命中检测。
    fn render_object_list(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        // 每帧重建（paint 阶段逐行写入）
        self.row_bounds.borrow_mut().clear();
        let mut list = v_flex()
            .id("object-list")
            .relative()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .bg(theme.background);

        if self.entries.is_empty() && self.objects_state == AsyncState::Idle {
            list = list.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .text_color(theme.muted_foreground)
                    .child(Icon::new(IconName::Inbox))
                    .child("此目录为空"),
            );
        }

        // ⌘F 过滤开启时只渲染命中项；命中为空给出明确空态。
        // 过滤只影响展示：entries 全集与选择集合不动（Finder 语义）。
        let visible: Vec<(usize, &ListingEntry)> = match &self.filtered_ix {
            Some(ix) => ix
                .iter()
                .filter_map(|&ix| self.entries.get(ix).map(|e| (ix, e)))
                .collect(),
            None => self.entries.iter().enumerate().collect(),
        };
        let open_menu_top = self.object_menu_open.as_deref().and_then(|open_key| {
            visible
                .iter()
                .position(|(_, entry)| matches!(entry, ListingEntry::Object(object) if object.key == open_key))
                .map(|row| px(34. + row as f32 * 40. + 28.))
        });
        if self.filtered_ix.is_some() && visible.is_empty() {
            list = list.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .text_color(theme.muted_foreground)
                    .child(Icon::new(IconName::Search))
                    .child("没有匹配的对象"),
            );
            return list.into_any_element();
        }

        if !visible.is_empty() {
            list = list.child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .bg(theme.sidebar)
                    .text_size(tokens::text(12.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.muted_foreground)
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .child(div().flex_1().min_w_0().child("名称"))
                            .child(div().w(px(96.)).flex_shrink_0().child("大小"))
                            .child(div().w(px(104.)).flex_shrink_0().child("存储类型"))
                            .child(div().w(px(148.)).flex_shrink_0().child("最新修改时间")),
                    )
                    .child(div().w(px(64.)).flex_shrink_0().child("操作")),
            );
        }

        for (ix, entry) in visible {
            match entry {
                ListingEntry::CommonPrefix(prefix) => {
                    let label = display_name(prefix).to_string();
                    let prefix_sel = prefix.clone();
                    let prefix_nav = prefix.clone();
                    let prefix_action = prefix.clone();
                    list = list.child(
                        h_flex()
                            .id(("object-row", ix))
                            .relative()
                            .w_full()
                            .px_3()
                            .py(px(6.))
                            .gap_2()
                            .border_b_1()
                            .border_color(theme.border)
                            .text_size(tokens::text(13.))
                            .hover(|row| row.bg(theme.accent))
                            .child(self.row_bounds_recorder().absolute().inset_0())
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_2()
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(
                                            move |this, event: &MouseDownEvent, _window, cx| {
                                                // 目录选择在下钻前应用（capture 清空之后）
                                                this.handle_object_row_click(
                                                    ix,
                                                    ClickedEntry::CommonPrefix(prefix_sel.clone()),
                                                    event.modifiers,
                                                    cx,
                                                );
                                                this.open_prefix(prefix_nav.clone(), cx);
                                            },
                                        ),
                                    )
                                    .child(
                                        h_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_2()
                                            .child(
                                                Icon::new(IconName::Folder)
                                                    .text_color(theme.accent_foreground),
                                            )
                                            .child(div().min_w_0().truncate().child(label)),
                                    )
                                    .child(
                                        div()
                                            .w(px(96.))
                                            .flex_shrink_0()
                                            .text_color(theme.muted_foreground)
                                            .child("-"),
                                    )
                                    .child(
                                        div()
                                            .w(px(104.))
                                            .flex_shrink_0()
                                            .text_color(theme.muted_foreground)
                                            .child("-"),
                                    )
                                    .child(
                                        div()
                                            .w(px(148.))
                                            .flex_shrink_0()
                                            .text_color(theme.muted_foreground)
                                            .child("-"),
                                    ),
                            )
                            .child(
                                h_flex().w(px(64.)).flex_shrink_0().gap_1().child(
                                    Button::new(("open-prefix", ix))
                                        .label("进入")
                                        .ghost()
                                        .with_size(Size::Small)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.open_prefix(prefix_action.clone(), cx)
                                        })),
                                ),
                            ),
                    );
                }
                ListingEntry::Object(object) => {
                    let selected = self.selected_object_keys.contains(&object.key);
                    let key = object.key.clone();
                    let name_key = object.key.clone();
                    let menu_key = object.key.clone();
                    let size = format_size(object.size);
                    let time = format_time(object.put_time_millis);
                    list = list.child(
                        h_flex()
                            .id(("object-row", ix))
                            .relative()
                            .w_full()
                            .px_3()
                            .py(px(6.))
                            .gap_2()
                            .border_b_1()
                            .border_color(theme.border)
                            .text_size(tokens::text(13.))
                            // selection ≠ primary（agents.md §7）：选中用 list_active，
                            // hover 是可交互反馈用 accent
                            .when(selected, |row| row.bg(theme.list_active))
                            .hover(|row| row.bg(theme.accent))
                            .child(self.row_bounds_recorder().absolute().inset_0())
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                    this.handle_object_row_click(
                                        ix,
                                        ClickedEntry::Object(key.clone()),
                                        event.modifiers,
                                        cx,
                                    );
                                }),
                            )
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_2()
                                    .child(
                                        h_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_2()
                                            .child(
                                                Icon::new(IconName::File)
                                                    .text_color(theme.muted_foreground),
                                            )
                                            .child(
                                                div()
                                                    .min_w_0()
                                                    .truncate()
                                                    .hover(|name| name.text_color(theme.foreground))
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(move |this, _, _, cx| {
                                                            this.select_object_for_row_action(
                                                                &name_key,
                                                            );
                                                            this.open_preview_overlay(cx);
                                                        }),
                                                    )
                                                    .child(display_name(&object.key).to_string()),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .w(px(96.))
                                            .flex_shrink_0()
                                            .text_color(theme.muted_foreground)
                                            .text_size(tokens::text(12.))
                                            .child(size),
                                    )
                                    .child(
                                        div()
                                            .w(px(104.))
                                            .flex_shrink_0()
                                            .text_color(theme.muted_foreground)
                                            .text_size(tokens::text(12.))
                                            .child("标准存储"),
                                    )
                                    .child(
                                        div()
                                            .w(px(148.))
                                            .flex_shrink_0()
                                            .text_color(theme.muted_foreground)
                                            .text_size(tokens::text(12.))
                                            .child(time),
                                    ),
                            )
                            .child(
                                div().w(px(64.)).flex_shrink_0().child(
                                    Button::new(("object-menu-row", ix))
                                        .icon(Icon::new(IconName::EllipsisVertical))
                                        .ghost()
                                        .with_size(Size::Small)
                                        .tooltip("更多操作")
                                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                            cx.stop_propagation()
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.toggle_object_menu(&menu_key, cx);
                                        })),
                                ),
                            ),
                    );
                }
            }
        }

        if self.entries.is_empty() {
            return list.into_any_element();
        }

        // 底部：统计 + 翻页
        let mut footer = h_flex()
            .px_3()
            .py_2()
            .gap_3()
            .border_t_1()
            .border_color(theme.border)
            .text_size(tokens::text(12.))
            .text_color(theme.muted_foreground)
            .child(format!("共 {} 项", self.entries.len()));
        if self.next_marker.is_some() {
            footer = footer.child(
                Button::new("objects-load-more")
                    .label(if self.loading_more {
                        "加载中…"
                    } else {
                        "加载更多"
                    })
                    .loading(self.loading_more)
                    .disabled(self.loading_more)
                    .ghost()
                    .with_size(Size::Small)
                    .on_click(cx.listener(|this, _, _, cx| this.load_more(cx))),
            );
        }
        list = list.child(footer);
        if let Some(top) = open_menu_top {
            list = list.child(
                div()
                    .absolute()
                    .top(top)
                    .right(px(12.))
                    .occlude()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(self.render_object_menu(theme, cx)),
            );
        }
        list.into_any_element()
    }

    /// 右侧 Inspector：选中对象的元数据；未选中时显示占位破折号。
    #[allow(dead_code)]
    fn render_inspector(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<(&'static str, String)> = match self.selected_cloud_object() {
            Some(object) => vec![
                ("名称", display_name(&object.key).to_string()),
                ("Key", object.key.clone()),
                ("大小", format_size(object.size)),
                (
                    "类型",
                    object.mime_type.clone().unwrap_or_else(|| "—".into()),
                ),
                ("ETag", object.etag.clone().unwrap_or_else(|| "—".into())),
                ("上传时间", format_time(object.put_time_millis)),
            ],
            None => vec![
                ("名称", "—".into()),
                ("大小", "—".into()),
                ("类型", "—".into()),
                ("修改时间", "—".into()),
            ],
        };

        let selected = self.selected_cloud_object();
        let preview_path = self.preview_path.clone();
        let mut panel = v_flex()
            .h_full()
            .w_full()
            .overflow_hidden()
            .bg(theme.background)
            .border_l_1()
            .border_color(theme.border)
            .child(
                div()
                    .px_3()
                    .py_2()
                    .text_size(tokens::text(13.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        selected
                            .map(|object| display_name(&object.key).to_string())
                            .unwrap_or_else(|| "检查器".into()),
                    ),
            )
            .child(
                h_flex()
                    .px_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .id("inspector-tab-preview")
                            .flex_1()
                            .px_2()
                            .py_2()
                            .text_size(tokens::text(12.))
                            .text_color(if self.inspector_tab == InspectorTab::Preview {
                                theme.foreground
                            } else {
                                theme.muted_foreground
                            })
                            .hover(|tab| tab.bg(theme.sidebar_accent))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.inspector_tab = InspectorTab::Preview;
                                cx.notify();
                            }))
                            .child("预览"),
                    )
                    .child(
                        div()
                            .id("inspector-tab-details")
                            .flex_1()
                            .px_2()
                            .py_2()
                            .text_size(tokens::text(12.))
                            .text_color(if self.inspector_tab == InspectorTab::Details {
                                theme.foreground
                            } else {
                                theme.muted_foreground
                            })
                            .hover(|tab| tab.bg(theme.sidebar_accent))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.inspector_tab = InspectorTab::Details;
                                cx.notify();
                            }))
                            .child("详情"),
                    )
                    .child(
                        div()
                            .id("inspector-tab-metadata")
                            .flex_1()
                            .px_2()
                            .py_2()
                            .text_size(tokens::text(12.))
                            .text_color(if self.inspector_tab == InspectorTab::Metadata {
                                theme.foreground
                            } else {
                                theme.muted_foreground
                            })
                            .hover(|tab| tab.bg(theme.sidebar_accent))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.inspector_tab = InspectorTab::Metadata;
                                cx.notify();
                            }))
                            .child("元数据"),
                    ),
            );

        if self.inspector_tab == InspectorTab::Preview {
            if let Some(object) = selected {
                let kind = preview_kind(&object.key);
                let preview_content = if let Some(editor) = self.text_editor.clone() {
                    Input::new(&editor)
                        .h(px(220.))
                        .font_family(theme.mono_font_family.clone())
                        .text_size(theme.mono_font_size)
                        .into_any_element()
                } else if let Some(text) = self.preview_text.clone() {
                    div()
                        .w_full()
                        .h(px(220.))
                        .overflow_hidden()
                        .p_2()
                        .font_family(theme.mono_font_family.clone())
                        .text_size(theme.mono_font_size)
                        .child(text)
                        .into_any_element()
                } else if kind == PreviewKind::Image {
                    if let Some(path) = preview_path {
                        img(path)
                            .w_full()
                            .h(px(220.))
                            .object_fit(ObjectFit::Contain)
                            .into_any_element()
                    } else {
                        div()
                            .h(px(220.))
                            .w_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                Icon::new(IconName::File)
                                    .text_color(theme.muted_foreground)
                                    .text_size(tokens::text(42.)),
                            )
                            .into_any_element()
                    }
                } else if kind == PreviewKind::System {
                    div()
                        .h(px(220.))
                        .w_full()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .child(
                            Icon::new(IconName::Eye)
                                .text_color(theme.muted_foreground)
                                .text_size(tokens::text(32.)),
                        )
                        .child(
                            div()
                                .text_size(tokens::text(12.))
                                .text_color(theme.muted_foreground)
                                .child("此格式使用系统 Quick Look 预览"),
                        )
                        .into_any_element()
                } else {
                    div()
                        .h(px(220.))
                        .w_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            Icon::new(IconName::File)
                                .text_color(theme.muted_foreground)
                                .text_size(tokens::text(42.)),
                        )
                        .into_any_element()
                };
                panel = panel.child(
                    v_flex()
                        .mx_3()
                        .mt_3()
                        .gap_2()
                        .items_center()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.sidebar)
                        .p_3()
                        .child(preview_content)
                        .child(
                            div()
                                .text_size(tokens::text(13.))
                                .child(display_name(&object.key).to_string()),
                        )
                        .child(
                            div()
                                .text_size(tokens::text(11.))
                                .text_color(theme.muted_foreground)
                                .child(
                                    object
                                        .mime_type
                                        .clone()
                                        .unwrap_or_else(|| "未知类型".into()),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Button::new("preview-object-inspector")
                                        .label(if self.previewing {
                                            "准备预览…"
                                        } else {
                                            "预览"
                                        })
                                        .disabled(self.previewing)
                                        .with_size(Size::Small)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.start_object_preview(cx)
                                        })),
                                )
                                .when(
                                    kind == PreviewKind::System && self.preview_path.is_some(),
                                    |row| {
                                        row.child(
                                            Button::new("quicklook-object")
                                                .label("系统预览")
                                                .primary()
                                                .with_size(Size::Small)
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.open_system_preview(cx)
                                                })),
                                        )
                                    },
                                )
                                .when(self.preview_text.is_some(), |row| {
                                    if self.text_editor.is_some() {
                                        row.child(
                                            Button::new("save-text-object")
                                                .label("保存并上传")
                                                .primary()
                                                .with_size(Size::Small)
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.confirm_and_save_text_edit(window, cx)
                                                })),
                                        )
                                    } else {
                                        row.child(
                                            Button::new("edit-text-object")
                                                .label("编辑…")
                                                .with_size(Size::Small)
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.start_text_edit(window, cx)
                                                })),
                                        )
                                    }
                                }),
                        ),
                );
            }
        }
        if self.inspector_tab == InspectorTab::Details {
            for (label, value) in rows {
                panel = panel.child(
                    h_flex()
                        .px_3()
                        .py_1()
                        .justify_between()
                        .gap_2()
                        .text_size(tokens::text(12.))
                        .child(div().text_color(theme.muted_foreground).child(label))
                        .child(div().min_w_0().truncate().child(value)),
                );
            }
        }
        if self.inspector_tab == InspectorTab::Metadata {
            if let Some(object) = selected {
                for (label, value) in [
                    (
                        "Content-Type",
                        object.mime_type.clone().unwrap_or_else(|| "—".into()),
                    ),
                    ("大小（字节）", object.size.to_string()),
                    ("ETag", object.etag.clone().unwrap_or_else(|| "—".into())),
                    ("更新时间", format_time(object.put_time_millis)),
                ] {
                    panel = panel.child(
                        h_flex()
                            .px_3()
                            .py_1()
                            .justify_between()
                            .gap_2()
                            .text_size(tokens::text(12.))
                            .child(div().text_color(theme.muted_foreground).child(label))
                            .child(div().min_w_0().truncate().child(value)),
                    );
                }
            }
        }

        // 下载（选中对象）/ 上传（选中空间）入口 + 最近一次结果提示
        if self.selected_cloud_object().is_some() || self.selected_bucket.is_some() {
            panel = panel.child(
                div()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(
                        h_flex()
                            .gap_2()
                            .when(self.selected_cloud_object().is_some(), |row| {
                                row.child(
                                    Button::new("copy-object-url")
                                        .label(if self.copying_url {
                                            "复制中…"
                                        } else {
                                            "复制链接"
                                        })
                                        .disabled(self.copying_url)
                                        .with_size(Size::Small)
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.copy_object_url(cx)),
                                        ),
                                )
                                .child(
                                    Button::new("open-object")
                                        .label("打开")
                                        .disabled(self.previewing)
                                        .with_size(Size::Small)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.handle_open_object(&OpenObject, window, cx)
                                        })),
                                )
                                .child(
                                    Button::new("reveal-object")
                                        .label("Finder")
                                        .disabled(self.previewing)
                                        .with_size(Size::Small)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.handle_reveal_in_finder(
                                                &RevealInFinder,
                                                window,
                                                cx,
                                            )
                                        })),
                                )
                                .child(
                                    Button::new("download-object")
                                        .label(if self.downloading {
                                            "下载中…"
                                        } else {
                                            "下载…"
                                        })
                                        .disabled(self.downloading)
                                        .with_size(Size::Small)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.start_object_download(window, cx)
                                        })),
                                )
                                .child(
                                    Button::new("delete-object")
                                        .danger()
                                        .label(if self.deleting {
                                            "删除中…"
                                        } else {
                                            "删除…"
                                        })
                                        .disabled(self.deleting || self.delete_prompt_open)
                                        .with_size(Size::Small)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.confirm_and_delete_object(window, cx)
                                        })),
                                )
                            })
                            .when(self.selected_bucket.is_some(), |row| {
                                row.child(
                                    Button::new("upload-files")
                                        .label(if self.uploading {
                                            "选择文件…"
                                        } else {
                                            "上传…"
                                        })
                                        .disabled(self.uploading)
                                        .with_size(Size::Small)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.start_files_upload(cx)
                                        })),
                                )
                                .child(
                                    Button::new("upload-folder")
                                        .label("文件夹…")
                                        .disabled(self.uploading)
                                        .with_size(Size::Small)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.start_folder_upload(cx)
                                        })),
                                )
                            }),
                    ),
            );
        }
        if let Some(message) = &self.download_message {
            let color = if message.is_error {
                theme.danger
            } else {
                theme.muted_foreground
            };
            panel = panel.child(
                div()
                    .px_3()
                    .py_1()
                    .text_size(tokens::text(12.))
                    .text_color(color)
                    .child(message.text.clone()),
            );
        }

        // 传输队列（引擎事件驱动快照；取消/继续/重试直接作用于引擎）
        if !self.transfers.is_empty() {
            let finished_count = self
                .transfers
                .iter()
                .filter(|task| task.state.is_finished())
                .count();
            panel = panel.child(
                div()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(
                        h_flex()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(tokens::text(13.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(format!("传输（{}）", self.transfers.len())),
                            )
                            .children((finished_count > 0).then(|| {
                                Button::new("clear-finished-transfers")
                                    .label("清除已完成")
                                    .with_size(Size::Small)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.engine.clear_finished();
                                        this.transfers = this.engine.snapshot();
                                        cx.notify();
                                    }))
                            })),
                    ),
            );
            for (index, task) in self.transfers.iter().enumerate() {
                let state_color = match task.state {
                    TransferState::Running => theme.foreground,
                    TransferState::Failed => theme.danger,
                    TransferState::Waiting => theme.accent,
                    _ => theme.muted_foreground,
                };
                let mut row = v_flex().px_3().py_1().text_size(tokens::text(12.)).child(
                    h_flex()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .truncate()
                                .child(task.display_name.clone()),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_color(state_color)
                                .child(task.state.label().to_string()),
                        ),
                );
                if task.state == TransferState::Running
                    || (task.bytes_done > 0 && !task.state.is_finished())
                {
                    let (pct, label) = transfer_progress_text(task);
                    row = row.child(
                        v_flex()
                            .gap_1()
                            .child(Progress::new().h(px(4.)).value(pct))
                            .child(
                                h_flex()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_size(tokens::text(11.))
                                            .text_color(theme.muted_foreground)
                                            .child(label),
                                    )
                                    // 百分比只在已知总量时展示（未知总量算不出）
                                    .children(transfer_percent(task).map(|p| {
                                        div()
                                            .text_size(tokens::text(11.))
                                            .text_color(theme.muted_foreground)
                                            .child(format!("{p:.1}%"))
                                    })),
                            ),
                    );
                }
                if let Some(error) = &task.error {
                    // 失败原因完整展示：长错误换行不截断（用户需要据此排查，
                    // 比如签名/权限错误的关键细节都在尾部）
                    row = row.child(
                        div()
                            .text_color(theme.danger)
                            .text_size(tokens::text(11.))
                            .line_height(gpui::DefiniteLength::Fraction(1.4))
                            .child(error.clone()),
                    );
                }
                let actions: Vec<(&'static str, &'static str)> = match task.state {
                    TransferState::Queued | TransferState::Running | TransferState::Waiting => {
                        vec![("cancel", "取消")]
                    }
                    TransferState::Paused => vec![("resume", "继续"), ("cancel", "取消")],
                    TransferState::Failed | TransferState::Cancelled => vec![("resume", "重试")],
                    TransferState::Completed => Vec::new(),
                };
                if !actions.is_empty() {
                    let task_id = task.id;
                    let mut buttons = h_flex().gap_1().pt_1();
                    for (action, label) in actions {
                        let id = SharedString::from(format!("transfer-{action}-{index}"));
                        buttons = buttons.child(
                            Button::new(id)
                                .label(label)
                                .with_size(Size::Small)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    match action {
                                        "cancel" => {
                                            this.engine.cancel(task_id);
                                        }
                                        _ => {
                                            this.engine.resume(task_id);
                                        }
                                    }
                                    this.transfers = this.engine.snapshot();
                                    cx.notify();
                                })),
                        );
                    }
                    row = row.child(buttons);
                }
                panel = panel.child(row);
            }
        }
        panel
    }
}

// 侧栏行点击的 listener 由调用点的 `cx.listener` 适配（on_click 需要
// gpui App 级闭包，helper 拿不到 Context），无需额外适配函数。

// ---- 纯展示工具（单测覆盖） ----

/// 字节数人性化：B 整数展示，KB/MB/GB/TB 保留 1 位小数。
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

fn format_integer_grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// 活动任务 → 持久化行。Running/Waiting/Queued 落成 queued（下次自动继续），
/// 用户暂停保持 paused。终态任务不落盘。
fn persistable_from_snapshot(tasks: &[TransferTask]) -> Vec<PersistedTransfer> {
    tasks
        .iter()
        .filter(|t| t.state.is_active())
        .map(|t| {
            let (kind, account_id, bucket, key, local) = match &t.kind {
                TransferKind::Download {
                    account_id,
                    bucket,
                    key,
                    dest,
                } => (
                    "download",
                    account_id.clone(),
                    bucket.clone(),
                    key.clone(),
                    dest.clone(),
                ),
                TransferKind::Upload {
                    account_id,
                    bucket,
                    key,
                    source,
                } => (
                    "upload",
                    account_id.clone(),
                    bucket.clone(),
                    key.clone(),
                    source.clone(),
                ),
            };
            let state = if t.state == TransferState::Paused {
                "paused"
            } else {
                "queued"
            };
            PersistedTransfer {
                kind: kind.into(),
                account_id,
                bucket,
                object_key: key,
                dest: local.to_string_lossy().into_owned(),
                display_name: t.display_name.clone(),
                state: state.into(),
                enqueued_at_millis: t.enqueued_at_millis as i64,
            }
        })
        .collect()
}

/// 目录上传的一条文件：本地路径 + 云端相对 key（`/` 分隔，含顶层目录名）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct FolderUploadFile {
    source: PathBuf,
    relative_key: String,
    display_name: String,
}

fn skip_folder_entry_name(name: &str) -> bool {
    name == ".DS_Store" || name == ".localized" || name.starts_with("._")
}

/// 递归收集目录内普通文件。不跟随符号链接。相对 key 以 `/` 连接。
fn collect_folder_uploads(root: &std::path::Path) -> Result<Vec<FolderUploadFile>, String> {
    let folder_name = root
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("目录名不是合法 UTF-8：{}", root.display()))?;
    let meta = std::fs::symlink_metadata(root)
        .map_err(|e| format!("读取 {} 失败：{e}", root.display()))?;
    if meta.file_type().is_symlink() {
        return Err(format!("不跟随符号链接：{}", root.display()));
    }
    if !meta.is_dir() {
        return Err(format!("{} 不是目录", root.display()));
    }
    let mut out = Vec::new();
    walk_folder(root, folder_name, &mut out)?;
    Ok(out)
}

fn walk_folder(
    dir: &std::path::Path,
    key_prefix: &str,
    out: &mut Vec<FolderUploadFile>,
) -> Result<(), String> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("读取目录 {} 失败：{e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("读取目录项失败：{e}"))?;
        let name_os = entry.file_name();
        let name = name_os
            .to_str()
            .ok_or_else(|| format!("文件名不是合法 UTF-8：{}", entry.path().display()))?;
        if skip_folder_entry_name(name) {
            continue;
        }
        let ft = entry
            .file_type()
            .map_err(|e| format!("读取 {} 类型失败：{e}", entry.path().display()))?;
        if ft.is_symlink() {
            continue;
        }
        let rel = format!("{key_prefix}/{name}");
        if ft.is_dir() {
            walk_folder(&entry.path(), &rel, out)?;
        } else if ft.is_file() {
            out.push(FolderUploadFile {
                source: entry.path(),
                relative_key: rel,
                display_name: name.to_string(),
            });
        }
    }
    Ok(())
}

/// 云端 Key 的末段展示名（目录前缀先去掉结尾 `/`）。
pub fn display_name(key: &str) -> &str {
    let trimmed = key.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(i) => &trimmed[i + 1..],
        None => trimmed,
    }
}

fn preview_kind(key: &str) -> PreviewKind {
    if is_text_object(key) {
        PreviewKind::Text
    } else if is_image_object(key) {
        PreviewKind::Image
    } else {
        PreviewKind::System
    }
}

fn is_image_object(key: &str) -> bool {
    let Some(ext) = key.rsplit('.').next() else {
        return false;
    };
    let ext = ext.to_ascii_lowercase();
    Img::extensions().iter().any(|candidate| *candidate == ext)
}

fn syntax_language(key: &str) -> &'static str {
    match key
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("json") => "json",
        Some("js") => "javascript",
        Some("ts") => "typescript",
        Some("html" | "htm") => "html",
        Some("css") => "css",
        Some("md") => "markdown",
        Some("xml") => "xml",
        Some("yaml" | "yml") => "yaml",
        Some("rs") => "rust",
        Some("csv") | Some("txt") | None => "text",
        _ => "text",
    }
}

fn is_text_object(key: &str) -> bool {
    matches!(
        key.rsplit('.')
            .next()
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some(
            "txt"
                | "json"
                | "md"
                | "html"
                | "htm"
                | "css"
                | "js"
                | "ts"
                | "xml"
                | "yaml"
                | "yml"
                | "csv"
                | "toml"
                | "rs"
        )
    )
}

/// 目录前缀的上一级（保持结尾 `/`）；已是根级返回 None。
pub fn parent_prefix(prefix: &str) -> Option<&str> {
    let trimmed = prefix.trim_end_matches('/');
    let idx = trimmed.rfind('/')?;
    Some(&prefix[..=idx])
}

/// epoch 毫秒 → 本地时间 "YYYY-MM-DD HH:MM"。非法时间戳原样输出数字（不静默美化）。
pub fn format_time(millis: i64) -> String {
    chrono::DateTime::from_timestamp_millis(millis)
        .map(|utc| {
            utc.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| millis.to_string())
}

/// 「关于」弹层信息行：小标签 + 说明文字。
fn about_kv(label: &'static str, value: &'static str, theme: &Theme) -> gpui::Div {
    v_flex()
        .gap_0p5()
        .child(
            div()
                .text_size(tokens::text(11.))
                .text_color(theme.muted_foreground)
                .child(label),
        )
        .child(
            div()
                .text_size(tokens::text(13.))
                .text_color(theme.foreground)
                .child(value),
        )
}

impl gpui::Focusable for WorkspaceView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for WorkspaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_preview_text_editor(window, cx);
        let theme = cx.theme().clone();
        let mut root = v_flex()
            .id("workspace")
            .relative() // 模态遮罩层的定位基准
            .size_full()
            .key_context("Workspace")
            .track_focus(&self.focus_handle)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                    if this.top_more_open {
                        this.top_more_open = false;
                        cx.notify();
                    }
                }),
            )
            .on_action(cx.listener(Self::handle_toggle_sidebar))
            .on_action(cx.listener(Self::handle_quit))
            .on_action(cx.listener(Self::handle_close_window))
            .on_action(cx.listener(Self::handle_open_command_palette))
            .on_action(cx.listener(Self::handle_open_add_modal))
            .on_action(cx.listener(Self::handle_download_object))
            .on_action(cx.listener(Self::handle_preview_object))
            .on_action(cx.listener(Self::handle_upload_files))
            .on_action(cx.listener(Self::handle_upload_folder))
            .on_action(cx.listener(Self::handle_refresh))
            .on_action(cx.listener(Self::handle_delete_object))
            .on_action(cx.listener(Self::handle_copy_object_url))
            .on_action(cx.listener(Self::handle_save_text_object))
            .on_action(cx.listener(Self::handle_select_all))
            .on_action(cx.listener(Self::handle_rename_object))
            .on_action(cx.listener(Self::handle_dismiss_rename))
            .on_action(cx.listener(Self::handle_toggle_object_filter))
            .on_action(cx.listener(Self::handle_dismiss_filter))
            .on_action(cx.listener(Self::handle_select_bucket_by_name))
            .on_action(cx.listener(Self::handle_open_settings))
            .on_action(cx.listener(Self::handle_open_about))
            .on_action(cx.listener(Self::handle_open_object))
            .on_action(cx.listener(Self::handle_reveal_in_finder))
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(self.render_title_bar(&theme, cx))
            .child(self.render_body(&theme, cx));

        // 模态遮罩层（先渲染 → 在下层），命令面板后渲染盖在其上。
        if let Some(modal) = self.add_modal.clone() {
            root = root.child(self.render_add_modal_overlay(&modal, &theme, cx));
        }
        if let Some(modal) = self.settings_modal.clone() {
            root = root.child(self.render_settings_modal_overlay(&modal, &theme, cx));
        }
        if self.about_overlay_open {
            root = root.child(self.render_about_overlay(&theme, cx));
        }
        if self.details_overlay_open {
            root = root.child(self.render_details_overlay(&theme, cx));
        }
        if self.preview_overlay_open {
            root = root.child(self.render_preview_overlay(&theme, cx));
        }
        if self.renaming.is_some() {
            root = root.child(self.render_rename_overlay(&theme, cx));
        }
        if self.create_folder_input.is_some() {
            root = root.child(self.render_create_folder_overlay(&theme, cx));
        }
        if self.copy_move.is_some() {
            root = root.child(self.render_copy_move_overlay(&theme, cx));
        }
        if let Some(palette) = self.palette.clone() {
            root = root.child(self.render_palette_overlay(&palette, &theme, cx));
        }
        root
    }
}

/// 传输进度条文本：已知总量 → "已完成 / 总量"；未知但有字节 → 字节数。
#[allow(dead_code)]
fn transfer_progress_text(task: &TransferTask) -> (f32, String) {
    let pct = transfer_percent(task).unwrap_or(0.0);
    let label = match task.bytes_total {
        Some(total) => format!("{} / {}", format_size(task.bytes_done), format_size(total)),
        None if task.bytes_done > 0 => format_size(task.bytes_done),
        None => String::new(),
    };
    (pct, label)
}

/// 传输完成百分比（0..100）；总量未知或为 0 时返回 None（不算百分比）。
#[allow(dead_code)]
fn transfer_percent(task: &TransferTask) -> Option<f32> {
    let total = task.bytes_total?;
    if total == 0 {
        return None;
    }
    Some((task.bytes_done as f32 / total as f32) * 100.0)
}

/// 删除确认里的明细摘要：单对象为空串（标题已含名字）；多对象列出
/// 前几个名字，超出截断（确认框不放长列表）。
fn delete_summary(keys: &[String]) -> String {
    const MAX_NAMES: usize = 3;
    if keys.len() <= 1 {
        return String::new();
    }
    let names: Vec<String> = keys
        .iter()
        .take(MAX_NAMES)
        .map(|k| display_name(k).to_string())
        .collect();
    if keys.len() > MAX_NAMES {
        format!("（{} 等 {} 个）", names.join("、"), keys.len())
    } else {
        format!("（{}）", names.join("、"))
    }
}

fn preview_download_error_message(bucket: &str, key: &str, error: &str) -> String {
    let sanitized = sanitize_remote_error(error);
    format!(
        "无法预览：{}\n\n请检查：\n1. 当前账号能否读取 `{}`；\n2. 七牛：Bucket `{bucket}` 的下载域名是否可用；\n3. 阿里云 OSS：Endpoint/区域和 RAM 权限是否正确。",
        sanitized,
        display_name(key)
    )
}

fn sanitize_remote_error(error: &str) -> String {
    let lower = error.to_lowercase();
    if lower.contains("<html") || lower.contains("<!doctype") || lower.contains("403 forbidden") {
        if lower.contains("403") || lower.contains("forbidden") {
            return "远端拒绝访问，通常是权限或下载域名配置问题".into();
        }
        return "远端返回错误页面，无法下载对象内容".into();
    }
    if error.chars().count() > 300 {
        let cut: String = error.chars().take(300).collect();
        format!("{cut}…")
    } else {
        error.to_string()
    }
}

fn copy_object_url_request(
    account_id: Option<&str>,
    bucket: Option<&str>,
    object: Option<&CloudObject>,
    ttl_secs: u64,
) -> Result<CopyObjectUrlRequest, DownloadMessage> {
    let Some(object) = object else {
        return Err(DownloadMessage {
            is_error: true,
            text: "请先选中一个对象再复制链接".into(),
        });
    };
    let (Some(account_id), Some(bucket)) = (account_id, bucket) else {
        return Err(DownloadMessage {
            is_error: true,
            text: "请先选择账号和 Bucket 再复制链接".into(),
        });
    };
    Ok(CopyObjectUrlRequest {
        account_id: account_id.to_string(),
        bucket: bucket.to_string(),
        key: object.key.clone(),
        ttl_secs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_size_human_readable() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(5 * 1024 * 1024 * 1024), "5.0 GB");
        // 超出 TB 封顶：不再升单位
        let tb = 1024.0_f64.powi(4);
        assert_eq!(format_size((tb * 2048.0) as u64), "2048.0 TB");
    }

    #[test]
    fn format_integer_grouped_adds_commas() {
        assert_eq!(format_integer_grouped(0), "0");
        assert_eq!(format_integer_grouped(999), "999");
        assert_eq!(format_integer_grouped(36_648), "36,648");
        assert_eq!(format_integer_grouped(1_234_567), "1,234,567");
    }

    #[test]
    fn display_name_takes_last_segment() {
        assert_eq!(display_name("a/b/c.txt"), "c.txt");
        assert_eq!(display_name("c.txt"), "c.txt");
        assert_eq!(display_name("a/b/"), "b");
        assert_eq!(display_name(""), "");
    }

    #[test]
    fn parent_prefix_walks_up() {
        assert_eq!(parent_prefix("a/"), None);
        assert_eq!(parent_prefix("a/b/"), Some("a/"));
        assert_eq!(parent_prefix("a/b/c/"), Some("a/b/"));
    }

    #[test]
    fn format_time_shapes_local_datetime() {
        // epoch 0 → 本地时区的 "YYYY-MM-DD HH:MM"（16 字符）；跨时区只验证形状
        let s = format_time(0);
        assert_eq!(s.len(), 16, "实际输出：{s}");
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[7..8], "-");
        assert_eq!(&s[10..11], " ");
        assert_eq!(&s[13..14], ":");
        // 非法时间戳：原样输出数字，不静默美化
        assert_eq!(format_time(i64::MIN), i64::MIN.to_string());
    }

    #[test]
    fn collect_folder_uploads_nested_and_skips_junk() {
        let dir = std::env::temp_dir().join(format!(
            "cloudstorage-folder-up-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let nested = dir.join("photos").join("a");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(dir.join("photos").join("root.jpg"), b"r").unwrap();
        std::fs::write(nested.join("cat.jpg"), b"c").unwrap();
        std::fs::write(dir.join("photos").join(".DS_Store"), b"x").unwrap();
        std::fs::write(dir.join("photos").join(".localized"), b"x").unwrap();
        std::fs::write(dir.join("photos").join("._hidden"), b"x").unwrap();
        std::fs::create_dir_all(dir.join("photos").join("empty")).unwrap();

        let mut files = collect_folder_uploads(&dir.join("photos")).unwrap();
        files.sort_by(|a, b| a.relative_key.cmp(&b.relative_key));
        let keys: Vec<_> = files.iter().map(|f| f.relative_key.as_str()).collect();
        assert_eq!(keys, ["photos/a/cat.jpg", "photos/root.jpg"]);
        assert_eq!(files[0].display_name, "cat.jpg");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn collect_folder_uploads_rejects_file() {
        let path =
            std::env::temp_dir().join(format!("cloudstorage-not-dir-{}", std::process::id()));
        std::fs::write(&path, b"x").unwrap();
        let err = collect_folder_uploads(&path).unwrap_err();
        assert!(err.contains("不是目录"), "实际 {err}");
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn preview_kind_classifies_extensions() {
        assert_eq!(preview_kind("logo.png"), PreviewKind::Image);
        assert_eq!(preview_kind("notes.txt"), PreviewKind::Text);
        assert_eq!(preview_kind("config.json"), PreviewKind::Text);
        assert_eq!(preview_kind("specs.pdf"), PreviewKind::System);
        assert_eq!(preview_kind("movie.mp4"), PreviewKind::System);
    }

    #[test]
    fn object_menu_items_keep_product_order() {
        assert_eq!(
            object_menu_items(),
            vec![
                ObjectMenuItem::Details,
                ObjectMenuItem::CopyUrl,
                ObjectMenuItem::Download,
                ObjectMenuItem::Rename,
                ObjectMenuItem::CopyTo,
                ObjectMenuItem::MoveTo,
                ObjectMenuItem::Delete,
            ]
        );
    }

    #[test]
    fn top_more_menu_items_keep_product_order() {
        assert_eq!(
            top_more_menu_items(),
            vec![
                TopMoreMenuItem::CopyTo,
                TopMoreMenuItem::MoveTo,
                TopMoreMenuItem::Delete,
            ]
        );
    }

    #[test]
    fn copy_move_target_prefix_normalizes_and_rejects_invalid_paths() {
        assert_eq!(normalize_copy_move_target_prefix("").unwrap(), "");
        assert_eq!(
            normalize_copy_move_target_prefix("backup").unwrap(),
            "backup/"
        );
        assert_eq!(
            normalize_copy_move_target_prefix(" backup/2026/ ").unwrap(),
            "backup/2026/"
        );
        assert_eq!(
            normalize_copy_move_target_prefix("/absolute").unwrap_err(),
            "目标目录不能以 / 开头"
        );
        assert_eq!(
            normalize_copy_move_target_prefix("a/../b").unwrap_err(),
            "目标目录不能包含 .."
        );
    }

    #[test]
    fn copy_move_target_keys_keep_file_names_and_reject_same_target() {
        let keys = vec!["a/avatar.jpg".to_string(), "b/config.json".to_string()];
        assert_eq!(
            copy_move_target_keys(&keys, "backup").unwrap(),
            vec![
                ("a/avatar.jpg".to_string(), "backup/avatar.jpg".to_string()),
                (
                    "b/config.json".to_string(),
                    "backup/config.json".to_string()
                ),
            ]
        );
        assert!(copy_move_target_keys(&["avatar.jpg".to_string()], "").is_err());
        assert!(
            copy_move_target_keys(&keys, "backup/flat/")
                .expect("different display names are safe")
                .iter()
                .all(|(_, target)| target.starts_with("backup/flat/"))
        );
        assert!(
            copy_move_target_keys(
                &["a/avatar.jpg".to_string(), "b/avatar.jpg".to_string()],
                "backup/"
            )
            .is_err()
        );
    }

    #[test]
    fn copy_move_overlay_navigation_keeps_workspace_prefix_and_enter_blocks_while_loading() {
        let workspace_prefix = Some("main/current/".to_string());
        let mut overlay_prefix = "main/current/".to_string();
        let mut overlay_entries = vec![
            ListingEntry::CommonPrefix("main/current/photos/".into()),
            entry_object("main/current/readme.txt"),
        ];
        let mut overlay_state = AsyncState::Idle;

        prepare_copy_move_directory_load(
            &mut overlay_prefix,
            &mut overlay_entries,
            &mut overlay_state,
            "main/current/photos/".into(),
        );

        assert_eq!(workspace_prefix.as_deref(), Some("main/current/"));
        assert_eq!(overlay_prefix, "main/current/photos/");
        assert!(overlay_entries.is_empty(), "目录切换必须丢弃旧目录项");
        assert_eq!(overlay_state, AsyncState::Loading);
        assert!(
            !can_commit_copy_move(false, &overlay_state, None),
            "Input PressEnter 走 commit_copy_move；目录加载中必须和按钮一样禁止提交"
        );

        overlay_state = AsyncState::Idle;
        overlay_entries = vec![ListingEntry::CommonPrefix(
            "main/current/photos/raw/".into(),
        )];
        let parent = parent_prefix(&overlay_prefix)
            .map(str::to_string)
            .unwrap_or_default();
        prepare_copy_move_directory_load(
            &mut overlay_prefix,
            &mut overlay_entries,
            &mut overlay_state,
            parent,
        );

        assert_eq!(workspace_prefix.as_deref(), Some("main/current/"));
        assert_eq!(overlay_prefix, "main/current/");
        assert!(overlay_entries.is_empty(), "上一级也必须触发重新加载");
        assert_eq!(overlay_state, AsyncState::Loading);

        overlay_state = AsyncState::Idle;
        assert!(can_commit_copy_move(false, &overlay_state, None));
        assert!(!can_commit_copy_move(true, &overlay_state, None));
        assert!(!can_commit_copy_move(
            false,
            &overlay_state,
            Some("目标对象已存在")
        ));
    }

    #[test]
    fn provider_url_scheme_matches_cloud_provider() {
        assert_eq!(provider_url_scheme(ProviderKind::Aliyun), "oss");
        assert_eq!(provider_url_scheme(ProviderKind::Qiniu), "kodo");
    }

    #[test]
    fn effective_default_download_dir_requires_existing_directory() {
        let dir = std::env::temp_dir().join(format!(
            "cloudstorage-dl-dir-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        // 未设置：None
        assert_eq!(effective_default_download_dir(None), None);
        // 目录不存在：None（面板退回 HOME）
        assert_eq!(effective_default_download_dir(Some(dir.as_path())), None);
        // 目录存在：原样返回
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(
            effective_default_download_dir(Some(dir.as_path())),
            Some(dir.clone())
        );
        // 指向文件：None
        let file = dir.join("not-a-dir.txt");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(effective_default_download_dir(Some(file.as_path())), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn download_dest_path_takes_last_segment() {
        let dest_dir = PathBuf::from("/tmp/cloudstorage-test-dest");
        assert_eq!(
            download_dest_path(&dest_dir, "a/b/c/report.pdf"),
            dest_dir.join("report.pdf")
        );
        // 嵌套同名文件不互相覆盖路径的前缀部分：目录名作为整体拼接
        assert_eq!(
            download_dest_path(&dest_dir, "x/avatar.jpg"),
            PathBuf::from("/tmp/cloudstorage-test-dest/avatar.jpg")
        );
        // 扁平化语义：不同目录下同名文件落在同一目标路径。
        // 这是既有行为（传输引擎 File::create 覆盖写），测试锁死以免无意变更。
        assert_eq!(
            download_dest_path(&dest_dir, "a/avatar.jpg"),
            download_dest_path(&dest_dir, "b/avatar.jpg"),
            "同名展平 = 同目标路径（引擎覆盖写语义）"
        );
    }

    #[test]
    fn single_download_confirm_texts_match_batch_structure() {
        let dir = PathBuf::from("/Users/demo/Downloads");
        let (title, detail) = single_download_confirm_texts("a/b/report.pdf", &dir);
        assert_eq!(title, "将「report.pdf」下载到默认目录。");
        assert!(detail.contains("/Users/demo/Downloads"));
        assert!(detail.contains("另存为"));

        let (batch_title, batch_detail) = batch_download_confirm_texts(3, &dir);
        assert_eq!(batch_title, "将 3 个对象下载到默认目录。");
        assert!(batch_detail.contains("/Users/demo/Downloads"));
        assert!(batch_detail.contains("不支持设置初始目录"));
    }

    #[test]
    fn overlay_scroll_dismisses_modal_requires_up_and_down_block() {
        // 预览/复制移动等含滚动列表的弹层：必须同时阻断 down+up（不误关）
        assert!(!overlay_scroll_dismisses_modal(false, true, true, true));
        // 无滚动列表的简单弹层（重命名/新建目录）：阻断 down 即可
        assert!(!overlay_scroll_dismisses_modal(false, true, false, false));
        // 缺少 down 阻断 → 点卡片会误关（不合规）
        assert!(overlay_scroll_dismisses_modal(false, false, true, true));
        assert!(overlay_scroll_dismisses_modal(false, false, false, false));
        // 有滚动列表但缺 up 阻断 → 误关（不合规）
        assert!(overlay_scroll_dismisses_modal(false, true, false, true));
        // busy 不改变阻断要求（busy 只由各 close handler 自行拒绝）
        assert!(!overlay_scroll_dismisses_modal(true, true, true, true));
        assert!(overlay_scroll_dismisses_modal(true, false, true, true));
    }

    #[test]
    fn copy_move_summary_lists_partial_failures() {
        assert_eq!(
            copy_move_summary(CopyMoveMode::Copy, 2, &[]),
            "已复制 2 个对象"
        );
        assert_eq!(
            copy_move_summary(
                CopyMoveMode::Move,
                1,
                &[("a/b.txt".to_string(), "无权限".to_string())]
            ),
            "移动完成 1 个，失败 1 个：b.txt：无权限"
        );
    }

    #[test]
    fn selection_pure_logic_single_click_selects_and_previews() {
        let keys = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let current: indexmap::IndexSet<String> = ["b".to_string()].into_iter().collect();
        let intent = ObjectSelectionIntent {
            command: false,
            shift: false,
            select_all: false,
            clicked_empty: false,
            clicked_index: Some(2),
        };
        let (next, anchor, preview) = apply_object_selection(
            intent,
            &keys,
            &current,
            Some(1),
            ClickedEntry::Object("c".into()),
        );
        assert_eq!(next.len(), 1);
        assert!(next.contains("c"));
        assert_eq!(anchor, Some(2));
        assert!(preview, "普通点击应触发预览");
    }

    #[test]
    fn selection_pure_logic_command_click_toggles() {
        let keys = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let current: indexmap::IndexSet<String> =
            ["a".to_string(), "b".to_string()].into_iter().collect();
        // ⌘Click 已选中的 b → 取消
        let intent = ObjectSelectionIntent {
            command: true,
            shift: false,
            select_all: false,
            clicked_empty: false,
            clicked_index: Some(1),
        };
        let (next, anchor, preview) = apply_object_selection(
            intent,
            &keys,
            &current,
            Some(1),
            ClickedEntry::Object("b".into()),
        );
        assert_eq!(next.len(), 1);
        assert!(next.contains("a"));
        assert_eq!(anchor, Some(1));
        assert!(!preview);

        // ⌘Click 未选中的 c → 追加
        let intent = ObjectSelectionIntent {
            command: true,
            shift: false,
            select_all: false,
            clicked_empty: false,
            clicked_index: Some(2),
        };
        let (next, _, preview) = apply_object_selection(
            intent,
            &keys,
            &current,
            Some(0),
            ClickedEntry::Object("c".into()),
        );
        assert_eq!(next.len(), 3);
        assert!(!preview);
    }

    #[test]
    fn selection_pure_logic_shift_click_range_selects() {
        let keys = (0..5).map(|i| i.to_string()).collect::<Vec<_>>();
        let current: indexmap::IndexSet<String> = ["1".to_string()].into_iter().collect();
        // 锚点 1，⇧Click 3 → 选 1..=3
        let intent = ObjectSelectionIntent {
            command: false,
            shift: true,
            select_all: false,
            clicked_empty: false,
            clicked_index: Some(3),
        };
        let (next, anchor, preview) = apply_object_selection(
            intent,
            &keys,
            &current,
            Some(1),
            ClickedEntry::Object("3".into()),
        );
        assert_eq!(next.len(), 3);
        assert!(next.contains("1") && next.contains("2") && next.contains("3"));
        assert_eq!(anchor, Some(1), "⇧Click 不改变锚点");
        assert!(!preview);

        // ⌘⇧Click：增量（追加范围，不清空原选择）
        let intent = ObjectSelectionIntent {
            command: true,
            shift: true,
            select_all: false,
            clicked_empty: false,
            clicked_index: Some(4),
        };
        let (next, _, _) = apply_object_selection(
            intent,
            &keys,
            &current,
            Some(1),
            ClickedEntry::Object("4".into()),
        );
        assert_eq!(next.len(), 4);
        assert!(next.contains("1"), "原选中保留");
    }

    #[test]
    fn selection_pure_logic_select_all_and_empty_click() {
        let keys = vec!["a".to_string(), "b".to_string()];
        let current: indexmap::IndexSet<String> = ["a".to_string()].into_iter().collect();
        // ⌘A 全选
        let intent = ObjectSelectionIntent {
            command: false,
            shift: false,
            select_all: true,
            clicked_empty: false,
            clicked_index: None,
        };
        let (next, _, preview) =
            apply_object_selection(intent, &keys, &current, None, ClickedEntry::None);
        assert_eq!(next.len(), 2);
        assert!(!preview);

        // 点击空白清空
        let intent = ObjectSelectionIntent {
            command: false,
            shift: false,
            select_all: false,
            clicked_empty: true,
            clicked_index: None,
        };
        let (next, anchor, _) =
            apply_object_selection(intent, &keys, &current, Some(0), ClickedEntry::None);
        assert!(next.is_empty());
        assert_eq!(anchor, None);
    }

    #[test]
    fn selection_pure_logic_prefix_click_keeps_selection() {
        let keys = vec!["a".to_string()];
        let current: indexmap::IndexSet<String> = ["a".to_string()].into_iter().collect();
        let intent = ObjectSelectionIntent {
            command: false,
            shift: false,
            select_all: false,
            clicked_empty: false,
            clicked_index: Some(0),
        };
        // 点目录前缀：不改变选择、不触发预览（目录点击是下钻）
        let (next, anchor, preview) = apply_object_selection(
            intent,
            &keys,
            &current,
            Some(0),
            ClickedEntry::CommonPrefix("dir/".into()),
        );
        assert_eq!(next, current);
        assert_eq!(anchor, Some(0));
        assert!(!preview);
    }

    #[test]
    fn delete_summary_lists_names_for_multiple() {
        assert_eq!(delete_summary(&["a/b.txt".into()]), "");
        assert_eq!(
            delete_summary(&["a/1.txt".into(), "b/2.txt".into()]),
            "（1.txt、2.txt）"
        );
        let many: Vec<String> = ["1", "2", "3", "4", "5"]
            .iter()
            .map(|s| format!("x/{s}.txt"))
            .collect();
        assert_eq!(delete_summary(&many), "（1.txt、2.txt、3.txt 等 5 个）");
    }

    #[test]
    fn delete_summary_handles_missing_extensions() {
        assert_eq!(delete_summary(&["a/b".into(), "c".into()]), "（b、c）");
    }

    #[test]
    fn preview_download_error_message_hides_html_and_lists_checks() {
        let html = r#"API 错误 (HTTP 403): download_object: <html>
<head><title>403 Forbidden</title></head>
<body><center><h1>403 Forbidden</h1></center></body>
</html>"#;
        let message = preview_download_error_message("private-bucket", "report/a.pdf", html);

        assert!(message.contains("无法预览"));
        assert!(message.contains("远端拒绝访问"));
        assert!(message.contains("Bucket `private-bucket`"));
        assert!(message.contains("a.pdf"));
        assert!(message.contains("七牛"));
        assert!(message.contains("下载域名"));
        assert!(message.contains("阿里云 OSS"));
        assert!(message.contains("Endpoint/区域"));
        assert!(message.contains("RAM 权限"));
        assert!(!message.contains("<html>"), "不应暴露原始 HTML");
        assert!(!message.contains("<title>"), "不应暴露原始 HTML");
    }

    #[test]
    fn sanitize_remote_error_truncates_long_plain_text() {
        let error = "x".repeat(400);
        let sanitized = sanitize_remote_error(&error);
        assert!(sanitized.ends_with('…'));
        assert!(sanitized.chars().count() <= 301);
    }

    #[test]
    fn rename_target_key_replaces_last_segment_only() {
        assert_eq!(
            rename_target_key("a/b/c.txt", "d.txt").unwrap(),
            "a/b/d.txt"
        );
        assert_eq!(rename_target_key("top.txt", "new.txt").unwrap(), "new.txt");
        // 无扩展名：替换最后一段
        assert_eq!(rename_target_key("a/b", "c").unwrap(), "a/c");
        // 目录前缀保持（含中文）
        assert_eq!(
            rename_target_key("报告/2024/summary.pdf", "2025.pdf").unwrap(),
            "报告/2024/2025.pdf"
        );
    }

    #[test]
    fn rename_target_key_rejects_invalid_names() {
        assert!(rename_target_key("a/b", "").is_err(), "空名");
        assert!(rename_target_key("a/b", "  ").is_err(), "纯空白");
        assert!(
            rename_target_key("a/b", "x/y").is_err(),
            "含 / 等于移动目录，禁止"
        );
        assert!(rename_target_key("a/b", ".").is_err());
        assert!(rename_target_key("a/b", "..").is_err());
    }

    #[test]
    fn rename_target_key_allows_dotfiles_and_inner_dots() {
        // .gitignore 这类点开头的名字合法；名字中间的点也合法
        assert_eq!(
            rename_target_key("a/b", ".gitignore").unwrap(),
            "a/.gitignore"
        );
        assert_eq!(rename_target_key("a/b.tar.gz", "c.zip").unwrap(), "a/c.zip");
    }

    #[test]
    fn rename_validation_message_reports_modal_errors() {
        let entries = vec![entry_object("dir/existing.jpg")];
        assert_eq!(
            rename_validation_message("dir/avatar.jpg", "avatar.jpg", &entries).as_deref(),
            Some("请输入一个不同的新名称")
        );
        assert_eq!(
            rename_validation_message("dir/avatar.jpg", "bad/name.jpg", &entries).as_deref(),
            Some("名称不能包含 /")
        );
        assert_eq!(
            rename_validation_message("dir/avatar.jpg", "existing.jpg", &entries).as_deref(),
            Some("目标名称已存在：existing.jpg，请换一个名字")
        );
        assert!(rename_validation_message("dir/avatar.jpg", "next.jpg", &entries).is_none());
    }

    #[test]
    fn create_folder_target_key_keeps_current_prefix() {
        assert_eq!(create_folder_target_key(None, "photos").unwrap(), "photos/");
        assert_eq!(
            create_folder_target_key(Some("reports/2026/"), "q1").unwrap(),
            "reports/2026/q1/"
        );
        assert_eq!(
            create_folder_target_key(None, "  photos  ").unwrap(),
            "photos/"
        );
    }

    #[test]
    fn breadcrumb_prefixes_build_click_targets() {
        assert!(breadcrumb_prefixes(None).is_empty());
        assert_eq!(
            breadcrumb_prefixes(Some("firmware/cp-es-c101/")),
            vec![
                ("firmware".to_string(), "firmware/".to_string()),
                ("cp-es-c101".to_string(), "firmware/cp-es-c101/".to_string()),
            ]
        );
        // 多余的 `/` 与重复段照实保留（服务端返回什么就展示什么）
        assert_eq!(
            breadcrumb_prefixes(Some("a//b/")),
            vec![
                ("a".to_string(), "a/".to_string()),
                ("b".to_string(), "a/b/".to_string()),
            ]
        );
        // 无尾斜杠（异常数据防御）：仍能生成段
        assert_eq!(
            breadcrumb_prefixes(Some("orphan")),
            vec![("orphan".to_string(), "orphan/".to_string())]
        );
        // Unicode 目录名按 `/` 正确切分
        assert_eq!(
            breadcrumb_prefixes(Some("报告/2026/")),
            vec![
                ("报告".to_string(), "报告/".to_string()),
                ("2026".to_string(), "报告/2026/".to_string()),
            ]
        );
    }

    #[test]
    fn collapse_breadcrumb_short_paths_keep_all_segments() {
        let segments = breadcrumb_prefixes(Some("a/b/c/"));
        assert!(collapse_breadcrumb(&segments).is_none());
    }

    #[test]
    fn collapse_breadcrumb_long_paths_hide_middle_keep_head_and_tail() {
        let segments = breadcrumb_prefixes(Some("l1/l2/l3/l4/l5/l6/"));
        let (collapsed_prefix, tail) = collapse_breadcrumb(&segments).expect("6 段应触发折叠");
        // 点击 `…` 直达被收起的最深一层（l2/l3/l4 被收起，l4 最深）
        assert_eq!(collapsed_prefix, "l1/l2/l3/l4/");
        // 首段 + 最后两段保留
        assert_eq!(
            tail,
            vec![
                ("l5".to_string(), "l1/l2/l3/l4/l5/".to_string()),
                ("l6".to_string(), "l1/l2/l3/l4/l5/l6/".to_string()),
            ]
        );
    }

    #[test]
    fn create_folder_validation_message_reports_errors() {
        let entries = vec![
            ListingEntry::CommonPrefix("photos/".into()),
            entry_object("reports/"),
        ];
        assert_eq!(
            create_folder_validation_message(None, "", &entries).as_deref(),
            Some("目录名不能为空")
        );
        assert_eq!(
            create_folder_validation_message(None, "a/b", &entries).as_deref(),
            Some("目录名不能包含 /")
        );
        assert_eq!(
            create_folder_validation_message(None, "photos", &entries).as_deref(),
            Some("目录已存在：photos")
        );
        assert_eq!(
            create_folder_validation_message(None, "reports", &entries).as_deref(),
            Some("目录已存在：reports")
        );
        assert!(create_folder_validation_message(None, "next", &entries).is_none());
    }

    fn entry_object(key: &str) -> ListingEntry {
        ListingEntry::Object(CloudObject {
            key: key.into(),
            size: 1,
            mime_type: None,
            etag: None,
            put_time_millis: 0,
        })
    }

    #[test]
    fn object_selection_ix_ignores_directory_prefixes() {
        let entries = vec![
            ListingEntry::CommonPrefix("photos/".into()),
            entry_object("a.txt"),
            ListingEntry::CommonPrefix("reports/".into()),
            entry_object("b.txt"),
        ];
        assert_eq!(object_selection_ix(&entries, "a.txt"), Some(0));
        assert_eq!(object_selection_ix(&entries, "b.txt"), Some(1));
        assert_eq!(object_selection_ix(&entries, "missing.txt"), None);
    }

    #[test]
    fn directory_mixed_keyboard_multi_select_blocks_rename_then_single_rename_keeps_prefix() {
        let entries = vec![
            ListingEntry::CommonPrefix("photos/".into()),
            entry_object("photos/a.jpg"),
            ListingEntry::CommonPrefix("reports/".into()),
            entry_object("reports/b.pdf"),
            entry_object("reports/c.pdf"),
        ];
        let keys = object_keys(&entries);
        assert_eq!(keys, ["photos/a.jpg", "reports/b.pdf", "reports/c.pdf"]);

        // Keyboard path: ⌘A selects objects only; directory prefixes are ignored.
        let select_all = ObjectSelectionIntent {
            command: false,
            shift: false,
            select_all: true,
            clicked_empty: false,
            clicked_index: None,
        };
        let empty = indexmap::IndexSet::new();
        let (multi, anchor, preview) =
            apply_object_selection(select_all, &keys, &empty, None, ClickedEntry::None);
        assert_eq!(multi.len(), 3);
        assert!(multi.contains("photos/a.jpg"));
        assert!(multi.contains("reports/b.pdf"));
        assert!(multi.contains("reports/c.pdf"));
        assert_eq!(anchor, None);
        assert!(!preview);
        assert!(
            multi.len() > 1,
            "Return rename must be blocked for multi-select"
        );

        // Pointer/keyboard recovery path: select one object from the mixed table.
        let b_ix = object_selection_ix(&entries, "reports/b.pdf").expect("b.pdf object index");
        let single_click = ObjectSelectionIntent {
            command: false,
            shift: false,
            select_all: false,
            clicked_empty: false,
            clicked_index: Some(b_ix),
        };
        let (single, anchor, preview) = apply_object_selection(
            single_click,
            &keys,
            &multi,
            anchor,
            ClickedEntry::Object("reports/b.pdf".into()),
        );
        assert_eq!(single.len(), 1);
        assert!(single.contains("reports/b.pdf"));
        assert_eq!(anchor, Some(1));
        assert!(preview);

        // Rename changes only the last segment and detects visible conflicts.
        let renamed = rename_target_key("reports/b.pdf", "renamed.pdf").unwrap();
        assert_eq!(renamed, "reports/renamed.pdf");
        assert!(!object_key_exists(&entries, &renamed));
        let conflict = rename_target_key("reports/b.pdf", "c.pdf").unwrap();
        assert_eq!(conflict, "reports/c.pdf");
        assert!(object_key_exists(&entries, &conflict));
    }

    #[test]
    fn directory_mixed_shift_range_and_command_toggle_use_object_indexes() {
        let entries = vec![
            ListingEntry::CommonPrefix("photos/".into()),
            entry_object("photos/a.jpg"),
            ListingEntry::CommonPrefix("reports/".into()),
            entry_object("reports/b.pdf"),
            ListingEntry::CommonPrefix("archive/".into()),
            entry_object("archive/c.txt"),
            entry_object("archive/d.txt"),
        ];
        let keys = object_keys(&entries);
        assert_eq!(
            keys,
            [
                "photos/a.jpg",
                "reports/b.pdf",
                "archive/c.txt",
                "archive/d.txt"
            ]
        );

        // 普通点击第二个对象建立锚点：entries 下标是 3，对象序号必须是 1。
        let b_ix = object_selection_ix(&entries, "reports/b.pdf").expect("b.pdf object index");
        let single_click = ObjectSelectionIntent {
            command: false,
            shift: false,
            select_all: false,
            clicked_empty: false,
            clicked_index: Some(b_ix),
        };
        let empty = indexmap::IndexSet::new();
        let (selected, anchor, preview) = apply_object_selection(
            single_click,
            &keys,
            &empty,
            None,
            ClickedEntry::Object("reports/b.pdf".into()),
        );
        assert_eq!(
            selected.iter().collect::<Vec<_>>(),
            [&"reports/b.pdf".to_string()]
        );
        assert_eq!(anchor, Some(1));
        assert!(preview);

        // ⇧Click 第四个对象：跨过目录前缀，只选对象序号 1..=3。
        let d_ix = object_selection_ix(&entries, "archive/d.txt").expect("d.txt object index");
        let shift_click = ObjectSelectionIntent {
            command: false,
            shift: true,
            select_all: false,
            clicked_empty: false,
            clicked_index: Some(d_ix),
        };
        let (range, anchor, preview) = apply_object_selection(
            shift_click,
            &keys,
            &selected,
            anchor,
            ClickedEntry::Object("archive/d.txt".into()),
        );
        assert_eq!(range.len(), 3);
        assert!(!range.contains("photos/a.jpg"));
        assert!(range.contains("reports/b.pdf"));
        assert!(range.contains("archive/c.txt"));
        assert!(range.contains("archive/d.txt"));
        assert_eq!(anchor, Some(1), "⇧Click 不改变锚点");
        assert!(!preview);

        // ⌘Click 取消中间对象：集合移除该对象，锚点更新为它的对象序号 2。
        let c_ix = object_selection_ix(&entries, "archive/c.txt").expect("c.txt object index");
        let command_click = ObjectSelectionIntent {
            command: true,
            shift: false,
            select_all: false,
            clicked_empty: false,
            clicked_index: Some(c_ix),
        };
        let (toggled, anchor, preview) = apply_object_selection(
            command_click,
            &keys,
            &range,
            anchor,
            ClickedEntry::Object("archive/c.txt".into()),
        );
        assert_eq!(toggled.len(), 2);
        assert!(toggled.contains("reports/b.pdf"));
        assert!(!toggled.contains("archive/c.txt"));
        assert!(toggled.contains("archive/d.txt"));
        assert_eq!(anchor, Some(2));
        assert!(!preview);
    }

    #[test]
    fn directory_mixed_reverse_shift_range_uses_object_indexes() {
        let entries = vec![
            ListingEntry::CommonPrefix("photos/".into()),
            entry_object("photos/a.jpg"),
            ListingEntry::CommonPrefix("reports/".into()),
            entry_object("reports/b.pdf"),
            ListingEntry::CommonPrefix("archive/".into()),
            entry_object("archive/c.txt"),
            entry_object("archive/d.txt"),
        ];
        let keys = object_keys(&entries);

        // 普通点击最后一个对象建立锚点：entries 下标是 6，对象序号必须是 3。
        let d_ix = object_selection_ix(&entries, "archive/d.txt").expect("d.txt object index");
        let single_click = ObjectSelectionIntent {
            command: false,
            shift: false,
            select_all: false,
            clicked_empty: false,
            clicked_index: Some(d_ix),
        };
        let empty = indexmap::IndexSet::new();
        let (selected, anchor, preview) = apply_object_selection(
            single_click,
            &keys,
            &empty,
            None,
            ClickedEntry::Object("archive/d.txt".into()),
        );
        assert_eq!(
            selected.iter().collect::<Vec<_>>(),
            [&"archive/d.txt".to_string()]
        );
        assert_eq!(anchor, Some(3));
        assert!(preview);

        // ⇧Click 靠前对象：从锚点 3 反向选到对象序号 1，目录前缀不参与范围。
        let b_ix = object_selection_ix(&entries, "reports/b.pdf").expect("b.pdf object index");
        let reverse_shift = ObjectSelectionIntent {
            command: false,
            shift: true,
            select_all: false,
            clicked_empty: false,
            clicked_index: Some(b_ix),
        };
        let (range, anchor, preview) = apply_object_selection(
            reverse_shift,
            &keys,
            &selected,
            anchor,
            ClickedEntry::Object("reports/b.pdf".into()),
        );
        assert_eq!(range.len(), 3);
        assert!(!range.contains("photos/a.jpg"));
        assert!(range.contains("reports/b.pdf"));
        assert!(range.contains("archive/c.txt"));
        assert!(range.contains("archive/d.txt"));
        assert_eq!(anchor, Some(3), "反向 ⇧Click 也不改变锚点");
        assert!(!preview);
    }

    #[test]
    fn filter_entries_none_or_blank_keeps_all() {
        let entries = vec![
            ListingEntry::CommonPrefix("dir/".into()),
            entry_object("a/b.txt"),
        ];
        assert_eq!(filter_entries(&entries, None), vec![0, 1]);
        assert_eq!(filter_entries(&entries, Some("")), vec![0, 1]);
        assert_eq!(filter_entries(&entries, Some("   ")), vec![0, 1]);
    }

    #[test]
    fn filter_entries_matches_key_and_prefix_case_insensitive() {
        let entries = vec![
            ListingEntry::CommonPrefix("Photos/".into()),
            entry_object("photos/2024/a.jpg"),
            entry_object("docs/readme.md"),
        ];
        // 大小写不敏感：photos 同时命中目录前缀与对象 key
        assert_eq!(filter_entries(&entries, Some("photos")), vec![0, 1]);
        // 文件名片段
        assert_eq!(filter_entries(&entries, Some("readme")), vec![2]);
        // 无命中
        assert!(filter_entries(&entries, Some("不存在的词")).is_empty());
    }

    #[test]
    fn filter_entries_keeps_original_order() {
        let entries = vec![
            entry_object("b.txt"),
            ListingEntry::CommonPrefix("a/".into()),
            entry_object("a/c.txt"),
        ];
        assert_eq!(filter_entries(&entries, Some("a")), vec![1, 2]);
    }

    #[test]
    fn cached_copy_matches_requires_suffix_and_prefix() {
        // 缓存文件名 = {nanos}-{display_name}；后缀命中且 nanos 前缀非空
        assert!(cached_copy_matches(
            Some(std::path::Path::new("/tmp/preview/123-a b.png")),
            "x/a b.png"
        ));
        // key 不同 → 不复用
        assert!(!cached_copy_matches(
            Some(std::path::Path::new("/tmp/preview/123-a b.png")),
            "x/other.png"
        ));
        // 纯名字相等（没有 nanos 前缀）不算命中——防止 /tmp/report.pdf 这种
        // 巧合路径被误判为 report.pdf 的缓存
        assert!(!cached_copy_matches(
            Some(std::path::Path::new("/tmp/report.pdf")),
            "report.pdf"
        ));
        // 无缓存 / 空 key
        assert!(!cached_copy_matches(None, "a"));
        assert!(!cached_copy_matches(
            Some(std::path::Path::new("/tmp/1-a")),
            ""
        ));
    }

    #[test]
    fn copy_object_url_request_requires_selected_object() {
        let err =
            copy_object_url_request(Some("account-1"), Some("bucket-1"), None, 3600).unwrap_err();
        assert_eq!(
            err,
            DownloadMessage {
                is_error: true,
                text: "请先选中一个对象再复制链接".into(),
            }
        );
    }

    #[test]
    fn copy_object_url_request_uses_current_selection_and_configured_ttl() {
        let object = CloudObject {
            key: "report/a b.pdf".into(),
            size: 42,
            mime_type: Some("application/pdf".into()),
            etag: Some("etag".into()),
            put_time_millis: 1,
        };
        // TTL 来自设置（⌘, 可改），不再取编译期常量
        let request =
            copy_object_url_request(Some("account-1"), Some("bucket-1"), Some(&object), 600)
                .unwrap();
        assert_eq!(
            request,
            CopyObjectUrlRequest {
                account_id: "account-1".into(),
                bucket: "bucket-1".into(),
                key: "report/a b.pdf".into(),
                ttl_secs: 600,
            }
        );
    }
}

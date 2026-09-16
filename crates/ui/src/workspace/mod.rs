//! 主窗口 Workspace。
//!
//! 结构（agents.md §7）：Unified Titlebar + Sidebar(180/220/360) + Content。
//! - Titlebar 只放**窗口级导航**：侧栏开关、后退/前进、当前位置（面包屑 / ⌘L 路径框）。
//! - 对象区的操作（上传 / 更多 / 过滤）在**表格正上方**的工具栏里（`object_list.rs` 的
//!   `render_object_toolbar`，操作在左、搜索在右）：它们作用于对象列表而非窗口，放在表格
//!   正上方更贴近作用范围，也不必和标题栏的窗口拖拽区抢位置。
//! - Sidebar 折叠为 44px 图标栏（规范硬指标；gpui-component 自带 Sidebar 固定 255px/48px，
//!   无法满足，故自建，用其 Icon/主题 token 保持视觉一致）。
//! - Sidebar 宽度用 gpui-component Resizable；折叠/展开切换布局变体（不同的 resizable group id），
//!   使每种变体各自记住拖拽后的宽度。
//! - Action 处理见 `crate::actions`：⌘⌥S / ⌘W / ⌘Q 与菜单共享同一 Action。
//!
//! 数据接线（里程碑 c）：Sidebar 渲染真实账号/空间，Content 渲染选中 Bucket 的
//! 对象列表。所有 IO（SQLite/Keychain/网络）经 `AppServices` 的阻塞方法丢进 gpui
//! 后台执行器（`background_executor().spawn`），窗口显示永不被 IO 阻塞；
//! UI 状态更新统一回主线程 `this.update` + `cx.notify()`。
//!
//! 串台防护：每次异步加载携带自增代号（generation）。用户快速切换账号/桶时，
//! 过期任务的结果因代号不匹配被丢弃，不会覆盖新选中项的状态。

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{
    Anchor, AnchoredPositionMode, AnyElement, App, AppContext as _, ClickEvent, Context, Entity,
    ExternalPaths, FocusHandle, Img, InteractiveElement as _, IntoElement, MouseButton,
    MouseDownEvent, ObjectFit, ParentElement as _, PathPromptOptions, Pixels, Point, PromptButton,
    PromptLevel, Render, ScrollStrategy, SharedString, StatefulInteractiveElement as _, Styled,
    StyledImage as _, UniformListScrollHandle, Window, anchored, deferred, div, img, point,
    prelude::FluentBuilder, px, uniform_list,
};
use gpui_component::{
    ActiveTheme, Disableable as _, Icon, IconName, Sizable, Size, Theme, TitleBar, button::Button,
    button::ButtonVariants as _, checkbox::Checkbox, h_flex, input::Editor, input::EditorState,
    input::Input, input::InputEvent, input::InputState, progress::Progress, resizable::h_resizable,
    resizable::resizable_panel, scroll::ScrollableElement, spinner::Spinner, v_flex,
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
    DownloadObject, FocusObjectSearch, FocusPath, NavigateBack, NavigateForward, OpenAbout,
    OpenCommandPalette, OpenObject, OpenSettings, PreviewObject, Quit, Refresh, RenameObject,
    RevealInFinder, SaveTextObject, SelectBucketByName, SelectObjectAll, SelectObjectNext,
    SelectObjectNextRange, SelectObjectPrev, SelectObjectPrevRange, ToggleSidebar, UnifiedDismiss,
    UploadFiles, UploadFolder,
};
use crate::command_palette::CommandPaletteView;
use crate::settings_modal::SettingsModal;
use crate::tokens;
use crate::ui;
use crate::ui::overlay;

// 各 feature 模块（对齐 waku 的 src/app/* 分层）。子模块以 `use super::*;` 取用
// 本模块的类型与 import；这里把它们的条目 glob 回来，使整棵模块树看到的名称与拆分前
// 单文件一致，拆分因此是纯搬运而非重写。只含 `impl` 方法的模块不需要 glob——
// 固有方法随类型可见，import 它只会得到一条 unused_imports。

mod accounts;
mod buckets;
mod copy_move;
mod delete;
mod download;
mod folder;
mod format;
mod menus;
mod object_list;
mod objects;
mod palette;
mod preview;
mod quit;
mod rename;
mod selection;
mod settings;
mod sidebar;
mod sort;
mod titlebar;
mod transfers;
mod upload;

#[cfg(test)]
mod tests;

use self::format::*;
use self::objects::*;
use self::preview::*;
use self::rename::*;
use self::selection::*;
use self::sort::*;
use self::transfers::*;

/// 左栏折叠后的图标栏宽度（规范：44px Icon Rail）。
pub(super) const RAIL_WIDTH: Pixels = px(44.);

/// Sidebar 默认宽度（规范：默认 220，范围 180–360）。
pub(super) const SIDEBAR_DEFAULT: Pixels = px(220.);

pub(super) const SIDEBAR_MIN: Pixels = px(180.);

pub(super) const SIDEBAR_MAX: Pixels = px(360.);

/// 对象列表单页条数（七牛列举单页上限内）。列宽在 tokens.rs
/// （`col_size_width` / `col_time_width`，随字号缩放；表头与行共用保证对齐）。
pub(super) const OBJECTS_PAGE_LIMIT: u32 = 100;

/// 「每页条数」可选档位（参照实现底栏右下角那个选择器）。服务端列举本来就是
/// **单页条数上限**语义（`ListObjectsRequest::limit`），所以这一项做得到真值，
/// 不像页码那样只能编。
pub(super) const PAGE_LIMIT_CHOICES: [u32; 4] = [50, 100, 200, 500];

// 签名链接 TTL / 剪贴板清除秒数不再用编译期常量：运行时取 self.settings
// （settings.json，⌘, 可改；默认值见 object-storage-persistence）。
/// 侧栏/内容区的异步加载状态。`Loaded` 不单独建模——数据非空且 state==Idle 即加载完成。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum AsyncState {
    Idle,
    Loading,
    Failed(String),
}

/// 下载结果提示（成功/失败一次一笇）
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DownloadMessage {
    is_error: bool,
    text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CopyObjectUrlRequest {
    account_id: String,
    bucket: String,
    key: String,
    ttl_secs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PreviewKind {
    Image,
    Text,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ObjectMenuItem {
    Details,
    CopyUrl,
    Download,
    Rename,
    CopyTo,
    MoveTo,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TopMoreMenuItem {
    UploadFolder,
    CreateFolder,
    CopyTo,
    MoveTo,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CopyMoveMode {
    Copy,
    Move,
}

pub(super) struct CopyMoveState {
    mode: CopyMoveMode,
    source_keys: Vec<String>,
    target_prefix: String,
    filter: Entity<InputState>,
    entries: Vec<ListingEntry>,
    state: AsyncState,
}

pub struct WorkspaceView {
    focus_handle: FocusHandle,
    /// 无输入框弹层（预览/详情/关于）的焦点句柄：打开时聚焦，Esc 经
    /// context "Overlay" 沿焦点链派发到弹层的 UnifiedDismiss handler，
    /// 关闭后归还 Workspace 根。
    overlay_focus: FocusHandle,
    sidebar_collapsed: bool,
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
    /// 导航历史（规范 ⌘[ ⌘]）：`nav_back` 待回退栈，`nav_forward` 回退后可
    /// 前进栈。条目为历史位置（None = 根目录）。下钻/跳转压 back 清 forward；
    /// 跳桶清空全部（历史只在桶内有意义）。
    nav_back: Vec<Option<String>>,
    nav_forward: Vec<Option<String>>,
    /// 当前预览/详情对象 Key（entries 内查找）
    selected_object_key: Option<String>,
    /// 多选集合（规范 §7：Click/⌘Click/⇧Click/⌘A）。有序去重；
    /// `selected_object_key` 始终是其中最后一项（主选），供预览/详情。
    selected_object_keys: indexmap::IndexSet<String>,
    /// 范围选择锚点（对象序号；上次普通/⌘点击的对象，不含目录前缀）。
    selection_anchor: Option<usize>,
    /// 行内重命名进行中：(对象 key，输入框)。
    renaming: Option<(String, Entity<InputState>)>,
    /// rename 后台执行中（防重入）。
    renaming_busy: bool,
    /// 工具栏的**前缀搜索**框（参照实现叫「文件前缀搜索」）。
    ///
    /// 懒创建：`WorkspaceView::new` 拿不到 `Window`，所以在首次渲染时建（同
    /// `ensure_preview_text_editor`）。它是工具栏的常驻控件，不再是 ⌘F 开关的浮层。
    ///
    /// 语义与服务端一致：提交时把输入值作为 `ListObjectsRequest::prefix` **重新列举**，
    /// 因此能查到「还没加载出来」的对象；本地过滤做不到这点。
    search_input: Option<Entity<InputState>>,
    /// ⌘L 路径跳转输入框（Some = 打开中；回车跳转，Esc 经 DismissFilter 关闭）。
    path_input: Option<Entity<InputState>>,
    /// 对象列表排序方式（工具栏循环切换；Natural = 列举原序）。
    object_sort: ObjectSort,
    /// 当前帧的显示顺序（`entries` 下标序列，已应用排序与过滤）。
    ///
    /// 每帧在 `render_object_list` 里重算一次并存到这里，而不是让行渲染闭包
    /// 各自去算：闭包每被调用一次就重排+重过滤一遍是 O(行数 log 行数)，
    /// 每帧会跑多次。存**下标**而不是克隆条目：既省掉 clone，也让「缓存过期」
    /// 只可能表现为下标越界，由取用处的 `get()` 兜住（不会渲染出错行）。
    display_order: Vec<usize>,
    /// 对象列表的虚拟滚动句柄（`uniform_list` 绑定；键盘导航用它把选中行滚进视野）。
    object_list_scroll: UniformListScrollHandle,
    /// 每页条数（底栏右下角选择器；默认 `OBJECTS_PAGE_LIMIT`）。
    page_limit: u32,
    /// 「每页条数」菜单是否展开，以及触发点的窗口坐标。
    page_limit_menu_open: bool,
    page_limit_menu_at: Option<Point<Pixels>>,
    /// 应用设置（settings.json 快照；⌘, 可改）。
    settings: object_storage_persistence::Settings,
    /// settings.json 路径（模态展示与保存用）。
    settings_path: PathBuf,
    /// 设置模态（⌘,）。Some 时渲染遮罩。
    settings_modal: Option<Entity<SettingsModal>>,
    /// 对象下载进行中（按钮置灰防重入）
    downloading: bool,
    /// 上传选文件面板打开中（防重入）
    uploading: bool,
    /// 远端删除进行中
    deleting: bool,
    /// 预览对象下载/打开进行中
    previewing: bool,
    /// 已下载到本地缓存、供 GPUI img 直接渲染的预览路径
    preview_path: Option<PathBuf>,
    /// 文本预览内容；编辑器使用 GPUI Kit EditorState，不自建 WebView
    preview_text: Option<String>,
    text_editor: Option<Entity<EditorState>>,
    /// Space 触发预览时，系统格式下载完成后自动打开 Quick Look
    preview_open_quicklook: bool,
    /// 文件名/预览按钮触发的应用内预览弹层。
    preview_overlay_open: bool,
    /// 打开预览时置位：焦点必须在弹层元素**渲染挂载后**再设置——
    /// 在 on_mouse_down 处理器里立即 window.focus() 会被同一次点击的
    /// 后续处理覆盖（焦点回到 workspace 根），Esc 派发不到 Overlay context。
    preview_needs_focus: bool,
    /// 当前打开对象菜单的 object key。
    object_menu_open: Option<String>,
    /// 打开对象菜单时右键的**窗口坐标**（`MouseDownEvent.position`）。
    ///
    /// 菜单位置必须按这个点算，不能靠 `anchored()` 的默认锚定（它拿锚定元素
    /// 所在容器的原点当锚点，而行在滚动容器里、坐标空间与视口不一致，菜单会
    /// 跑到侧栏上）。右键位置是唯一不依赖容器/滚动坐标系的锚。
    object_menu_at: Option<Point<Pixels>>,
    /// 顶部「更多」菜单是否打开。
    top_more_open: bool,
    /// 触发「更多」菜单时的**窗口坐标**（点击点）。锚定必须用它，不能靠
    /// 「锚定元素所在容器的原点」——那个原点会随按钮所在的容器变化（按钮从
    /// 标题栏挪到内容区工具栏后就偏了），而窗口坐标与容器无关。
    top_more_menu_at: Option<Point<Pixels>>,
    /// 对象菜单打开时记录的**动作目标集合**（见 `menu_targets_for`）。
    /// 菜单关闭后清空；清空后动作回落为普通选择集。
    object_menu_targets: Vec<String>,
    /// 传输面板是否展开（显示每任务明细）。收起态仅显示一行汇总。
    transfers_expanded: bool,
    /// 当前是否显示对象详情弹层。
    details_overlay_open: bool,
    /// 当前是否显示「关于」弹层（独立于设置模态）。
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

    // ---- 串台防护：账号/对象/预览各自的自增代号 ----
    bucket_gen: u64,
    object_gen: u64,
    preview_gen: u64,
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
    /// 点击对象在 ordered_keys（不含目录前缀）中的下标；⌘A / 无命中项时为 None。
    /// 空白点击清空选择不在这里表达——它由列表容器直接调
    /// `clear_object_selection`（还要一并关菜单/弹层，纯函数表达不了）。
    pub clicked_index: Option<usize>,
}

/// 点击命中的条目类型：对象参与多选，目录前缀不参与（点击即下钻）。
pub(crate) enum ClickedEntry {
    Object(String),
    /// 目录前缀点击：命中项不进选择集合；载荷 key 仅为语义完整性保留。
    CommonPrefix(#[allow(dead_code)] String),
    None,
}

/// 键盘移动方向（对象列表 / 预览）。到边界停止，不循环。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ObjectNavDirection {
    Prev,
    Next,
}

/// 长路径折叠：中段收起为 `…`（点击直达被收起的最深一层），保留首段与
/// 最后 2 段（父目录 + 当前目录）。折叠后可见项数与 BREADCRUMB_MAX_VISIBLE
/// 一致（首段 + … + 尾 2 段 = 4）。折叠决策在纯函数（单测锁死），渲染层
/// 只消费结果。
///
/// 返回 `Option`：`None` = 未折叠（渲染完整路径）；
/// `Some((collapsed_prefix, tail))` = 首段后插入省略项，点击直达
/// `collapsed_prefix`（被收起段中最深一层的前缀），`tail` 为保留的尾段。
pub(super) const BREADCRUMB_MAX_VISIBLE: usize = 4;

/// 目录上传的一条文件：本地路径 + 云端相对 key（`/` 分隔，含顶层目录名）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FolderUploadFile {
    source: PathBuf,
    relative_key: String,
    display_name: String,
}

impl gpui::Focusable for WorkspaceView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for WorkspaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_preview_text_editor(window, cx);
        // 前缀搜索框是工具栏常驻控件：懒创建（new() 拿不到 Window）
        self.ensure_search_input(window, cx);
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
                    if this.top_more_open || this.object_menu_open.is_some() {
                        this.top_more_open = false;
                        this.object_menu_open = None;
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
            .on_action(cx.listener(Self::handle_select_object_prev))
            .on_action(cx.listener(Self::handle_select_object_next))
            .on_action(cx.listener(Self::handle_select_object_prev_range))
            .on_action(cx.listener(Self::handle_select_object_next_range))
            .on_action(cx.listener(Self::handle_rename_object))
            .on_action(cx.listener(Self::handle_dismiss_rename))
            .on_action(cx.listener(Self::handle_focus_object_search))
            .on_action(cx.listener(Self::handle_dismiss_filter))
            .on_action(cx.listener(Self::handle_select_bucket_by_name))
            .on_action(cx.listener(Self::handle_open_settings))
            .on_action(cx.listener(Self::handle_open_about))
            .on_action(cx.listener(Self::handle_open_object))
            .on_action(cx.listener(Self::handle_reveal_in_finder))
            .on_action(cx.listener(Self::handle_navigate_back))
            .on_action(cx.listener(Self::handle_navigate_forward))
            .on_action(cx.listener(Self::handle_focus_path))
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
        if self.details_overlay_open {
            root = root.child(self.render_details_overlay(&theme, cx));
        }
        if self.preview_overlay_open {
            // 焦点在弹层元素挂载后设置（mouse_down 里设置会被同一点击覆盖）
            if self.preview_needs_focus {
                self.preview_needs_focus = false;
                window.focus(&self.overlay_focus, cx);
            }
            root = root.child(self.render_preview_overlay(&theme, cx));
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
            overlay_focus: cx.focus_handle(),
            sidebar_collapsed: false,
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
            nav_back: Vec::new(),
            nav_forward: Vec::new(),
            selected_object_key: None,
            selected_object_keys: indexmap::IndexSet::new(),
            selection_anchor: None,
            renaming: None,
            renaming_busy: false,
            search_input: None,
            path_input: None,
            object_sort: ObjectSort::default(),
            display_order: Vec::new(),
            object_list_scroll: UniformListScrollHandle::new(),
            page_limit: OBJECTS_PAGE_LIMIT,
            page_limit_menu_open: false,
            page_limit_menu_at: None,
            settings,
            settings_path,
            settings_modal: None,
            downloading: false,
            uploading: false,
            deleting: false,
            previewing: false,
            preview_path: None,
            preview_text: None,
            text_editor: None,
            preview_open_quicklook: false,
            preview_overlay_open: false,
            preview_needs_focus: false,
            object_menu_open: None,
            object_menu_at: None,
            top_more_open: false,
            top_more_menu_at: None,
            object_menu_targets: Vec::new(),
            transfers_expanded: false,
            details_overlay_open: false,
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
            preview_gen: 0,
        };
        this.load_accounts(cx);
        this.restore_persisted_transfers(cx);
        Self::subscribe_transfers(engine, cx);
        this
    }

    pub(super) fn render_body(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // 每种 sidebar 布局变体独立 group id：折叠态互不串宽。
        let group_id: &'static str = if self.sidebar_collapsed {
            "workspace-layout-content-only"
        } else {
            "workspace-layout-sidebar-content"
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
            // min_h_0 必须显式：flex 子元素默认 min-height:auto（=内容高度），
            // 列表行数多时会把 content 撑出窗口、状态条被挤出可视区。
            div().flex_1().min_w_0().min_h_0().h_full().child(group),
        )
    }
}

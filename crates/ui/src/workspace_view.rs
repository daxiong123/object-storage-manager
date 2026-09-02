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

use std::path::PathBuf;
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
    progress::Progress, resizable::h_resizable, resizable::resizable_panel, spinner::Spinner,
    v_flex,
};

use object_storage_app::{AppServices, PersistedTransfer};
use object_storage_core::ByteProgress;
use object_storage_domain::{Account, Bucket, CloudObject, ListObjectsRequest, ListingEntry};
use object_storage_transfer::{
    TaskRunner, TransferEngine, TransferKind, TransferOp, TransferRequest, TransferState,
    TransferTask,
};

use crate::PaletteCommand;
use crate::account_modal::AddAccountModal;
use crate::actions::{
    AddAccount, CloseWindow, CopyObjectUrl, DeleteObject, DismissFilter, DismissRename,
    DownloadObject, OpenCommandPalette, OpenSettings, PreviewObject, Quit, Refresh, RenameObject,
    SaveTextObject, SelectBucketByName, SelectObjectAll, ToggleInspector, ToggleObjectFilter,
    ToggleSidebar, UploadFiles, UploadFolder,
};
use crate::command_palette::CommandPaletteView;
use crate::settings_modal::SettingsModal;

/// 左栏折叠后的图标栏宽度（规范：44px Icon Rail）。
const RAIL_WIDTH: Pixels = px(44.);
/// Sidebar 默认宽度（规范：默认 220，范围 180–360）。
const SIDEBAR_DEFAULT: Pixels = px(220.);
const SIDEBAR_MIN: Pixels = px(180.);
const SIDEBAR_MAX: Pixels = px(360.);
/// Inspector 默认宽度（规范：默认 320，范围 280–520）。
const INSPECTOR_DEFAULT: Pixels = px(320.);
const INSPECTOR_MIN: Pixels = px(280.);
const INSPECTOR_MAX: Pixels = px(520.);
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

pub struct WorkspaceView {
    focus_handle: FocusHandle,
    sidebar_collapsed: bool,
    inspector_collapsed: bool,
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
    /// 范围选择锚点（entries 下标；上次普通/⌘点击的对象）。
    selection_anchor: Option<usize>,
    /// 行内重命名进行中：(对象 key，输入框)。Some 时该行渲染为输入框。
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
    inspector_tab: InspectorTab,
    /// 删除确认 sheet 已弹出（gpui 禁止重入 prompt）
    delete_prompt_open: bool,
    /// 文本保存覆盖确认 sheet 已弹出
    save_prompt_open: bool,
    /// 正在生成并复制签名链接
    copying_url: bool,
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
    /// 点击项在 entries（含 CommonPrefixes）中的下标；⌘A / 空白点击时为 None。
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
        let engine = Arc::new(TransferEngine::new(services.runtime_handle(), runner, 2));

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
            inspector_collapsed: false,
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
            downloading: false,
            uploading: false,
            deleting: false,
            previewing: false,
            preview_path: None,
            preview_text: None,
            text_editor: None,
            preview_open_quicklook: false,
            inspector_tab: InspectorTab::Preview,
            delete_prompt_open: false,
            save_prompt_open: false,
            copying_url: false,
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
    }

    /// 下钻到某个目录前缀。
    fn open_prefix(&mut self, prefix: String, cx: &mut Context<Self>) {
        self.current_prefix = Some(prefix);
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

    /// 下载选中对象：gpui 平台保存面板（`cx.prompt_for_new_path`，异步回调）
    /// 拿目标路径 → 后台执行 AppServices 下载（阻塞式，见 agents.md §5 线程模型）。
    /// 用户取消 = 无操作。
    ///
    /// 为什么必须用 gpui 平台 API、不能在事件处理器里同步 `runModal`：
    /// 模态循环期间 AppKit 事件会重入 gpui（borrow App RefCell），而外层处理器
    /// 还持有借用 → "RefCell already borrowed" panic 闪退。gpui 自带的面板从
    /// foreground executor 任务发起 `beginWithCompletionHandler:`，结果经
    /// oneshot 回传，天生规避重入。详见 docs/notes/gpui-api-notes.md「文件对话框」。
    fn start_object_download(&mut self, cx: &mut Context<Self>) {
        if self.downloading {
            return; // 防重入
        }
        // 多选（≥2）走批量目录流程；单选维持原保存面板。
        if self.selected_object_keys.len() > 1 {
            self.start_batch_download(cx);
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

        // 面板初始目录：用户主目录（HOME 缺失时退化到根目录，仅影响初始位置）
        let directory = std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let suggested_name = display_name(&key);
        let receiver = cx.prompt_for_new_path(&directory, Some(suggested_name));

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

    /// 批量下载（多选 ≥2）：先选目标目录（gpui `prompt_for_paths`
    /// `directories: true, multiple: false`）→ 逐项入队到该目录。
    /// 目标文件名 = display_name(key)；重名由传输引擎按路径覆盖写（File::create）。
    fn start_batch_download(&mut self, cx: &mut Context<Self>) {
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
                this.downloading = false;
                for key in &keys {
                    let name = display_name(key).to_string();
                    this.engine.enqueue_download(
                        account_id.as_str(),
                        bucket.as_str(),
                        key.as_str(),
                        dest_dir.join(&name),
                        name,
                    );
                }
                this.download_message = Some(DownloadMessage {
                    is_error: false,
                    text: format!(
                        "已加入传输队列：{} 个对象 → {}",
                        keys.len(),
                        dest_dir.display()
                    ),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
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
        eprintln!("[preview] requested key={key} bucket={bucket}");
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
                        .map_err(|e| format!("下载预览对象失败：{e}"))?;
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
                eprintln!(
                    "[preview] download result: {}",
                    if result.is_ok() { "ok" } else { "error" }
                );
                match result {
                    Ok((path, text)) => {
                        eprintln!("[preview] inline path={}", path.display());
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
        self.engine.enqueue_upload(
            &account_id,
            &bucket,
            &object,
            path,
            display_name(&object).to_string(),
        );
        self.text_editor = None;
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

    fn handle_preview_object(
        &mut self,
        _: &PreviewObject,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(key) = self.selected_object_key.clone() {
            if preview_kind(&key) == PreviewKind::System && self.preview_path.is_some() {
                self.open_system_preview(cx);
                return;
            }
            self.preview_open_quicklook = preview_kind(&key) == PreviewKind::System;
        }
        self.start_object_preview(cx);
    }

    fn handle_download_object(
        &mut self,
        _: &DownloadObject,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_object_download(cx);
    }

    /// 对象行点击 → 多选语义（规范 §7）。纯决策在 `apply_object_selection`，
    /// 这里只负责取上下文 + 回写状态 + 普通点击触发预览。
    fn handle_object_row_click(
        &mut self,
        ix: usize,
        clicked: ClickedEntry,
        modifiers: gpui::Modifiers,
        cx: &mut Context<Self>,
    ) {
        let intent = ObjectSelectionIntent {
            command: modifiers.platform,
            shift: modifiers.shift,
            select_all: false,
            clicked_empty: false,
            clicked_index: Some(ix),
        };
        let ordered_keys: Vec<String> = self
            .entries
            .iter()
            .filter_map(|e| match e {
                ListingEntry::Object(o) => Some(o.key.clone()),
                ListingEntry::CommonPrefix(_) => None,
            })
            .collect();
        let (next, anchor, preview) = apply_object_selection(
            intent,
            &ordered_keys,
            &self.selected_object_keys,
            self.selection_anchor,
            clicked,
        );
        self.selected_object_keys = next;
        self.selected_object_key = self.selected_object_keys.last().cloned();
        self.selection_anchor = anchor;
        if preview {
            self.start_object_preview(cx);
        }
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
        let ordered_keys: Vec<String> = self
            .entries
            .iter()
            .filter_map(|e| match e {
                ListingEntry::Object(o) => Some(o.key.clone()),
                ListingEntry::CommonPrefix(_) => None,
            })
            .collect();
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

    /// Return：进入行内重命名（Finder 式）。多选（≠1）时忽略——批量改名
    /// 语义不明确，不做。已在重命名中则视为提交（输入框 Return 走这里前
    /// 先被 Input 组件消费，此处兜底）。
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

    /// 提交行内重命名：读取输入 → 校验目标 key → 后台
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
            self.renaming = None;
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
        if self
            .entries
            .iter()
            .any(|e| matches!(e, ListingEntry::Object(o) if o.key == new_key))
        {
            self.download_message = Some(DownloadMessage {
                is_error: true,
                text: format!("目标名称已存在：{}，请换一个名字", display_name(&new_key)),
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

    /// 取消行内重命名（Esc：Input escape() propagate → context "Renaming"）。
    fn handle_dismiss_rename(
        &mut self,
        _: &DismissRename,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((key, _)) = &self.renaming {
            eprintln!("[rename] cancelled key={key}");
        }
        self.cancel_rename(cx);
    }

    /// 取消行内重命名。
    fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        if self.renaming.take().is_some() {
            cx.notify();
        }
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

    /// 设置模态观察：saved（保存成功 → 应用并丢弃）/ closed（取消）后丢弃实体。
    fn handle_settings_modal_changed(
        &mut self,
        modal: Entity<SettingsModal>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (closed, saved) = {
            let m = modal.read(cx);
            (m.closed(), m.done().cloned())
        };
        if !closed && saved.is_none() {
            return;
        }
        self.settings_modal = None;
        window.focus(&self.focus_handle);
        if let Some((settings, changed)) = saved {
            self.settings = settings;
            if changed {
                self.download_message = Some(DownloadMessage {
                    is_error: false,
                    text: format!(
                        "设置已保存：签名链接 {} 秒，剪贴板清除 {} 秒（对之后的复制生效）",
                        self.settings.signed_url_ttl_secs, self.settings.clipboard_clear_secs
                    ),
                });
            }
        }
        cx.notify();
    }

    fn handle_toggle_inspector(
        &mut self,
        _: &ToggleInspector,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.inspector_collapsed = !self.inspector_collapsed;
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
        let extra = self.bucket_jump_commands();
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
    /// 分发带数据的 `SelectBucketByName` Action，由 WorkspaceView 统一处理。
    fn bucket_jump_commands(&self) -> Vec<PaletteCommand> {
        self.buckets
            .iter()
            .map(|bucket| {
                let name = bucket.name.clone();
                PaletteCommand::handler(
                    format!("跳转：{name}"),
                    move |_window: &mut gpui::Window, cx: &mut gpui::App| {
                        cx.dispatch_action(&SelectBucketByName(name.clone()));
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
        let inspector_icon = if self.inspector_collapsed {
            IconName::PanelRightOpen
        } else {
            IconName::PanelRightClose
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
                // 右：Inspector 开关
                .child(
                    h_flex().gap_1().child(
                        Button::new("toggle-inspector")
                            .icon(Icon::new(inspector_icon))
                            .ghost()
                            .with_size(Size::Small)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_inspector(cx))),
                    ),
                ),
        )
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        cx.notify();
    }

    fn toggle_inspector(&mut self, cx: &mut Context<Self>) {
        self.inspector_collapsed = !self.inspector_collapsed;
        cx.notify();
    }

    fn render_body(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        // 每种「折叠组合」使用独立的 resizable group id：
        // use_keyed_state 按 id 存宽度，展开/折叠切换互不覆盖，各自记住拖拽尺寸。
        let group_id: &'static str = match (self.sidebar_collapsed, self.inspector_collapsed) {
            (false, false) => "workspace-layout-full",
            (true, false) => "workspace-layout-no-sidebar",
            (false, true) => "workspace-layout-no-inspector",
            (true, true) => "workspace-layout-content-only",
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
        if !self.inspector_collapsed {
            group = group.child(
                resizable_panel()
                    .size(INSPECTOR_DEFAULT)
                    .size_range(INSPECTOR_MIN..INSPECTOR_MAX)
                    .child(self.render_inspector(theme, cx).into_any_element()),
            );
        }

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
            .text_size(px(11.))
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
                        .text_size(px(12.))
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
                            .text_size(px(12.))
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
            .text_size(px(13.))
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
                                        .text_size(px(11.))
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
                            .text_size(px(12.))
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
            .text_size(px(12.))
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
            .text_size(px(12.))
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
                    .text_size(px(11.))
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
            .text_size(px(13.))
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
                    .text_size(px(12.))
                    .child(Icon::new(IconName::TriangleAlert))
                    .child(format!("加载更多失败：{msg}")),
            );
        }

        content = content.child(self.render_object_list(theme, cx));
        content.into_any_element()
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
                        .text_size(px(12.))
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
        h_flex()
            .w_full()
            .px_3()
            .py_2()
            .gap_2()
            .border_b_1()
            .border_color(theme.border)
            .text_size(px(13.))
            .child(
                div()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(bucket.to_string()),
            )
            .children(self.current_prefix.as_ref().map(|prefix| {
                div()
                    .truncate()
                    .text_color(theme.muted_foreground)
                    .child(prefix.clone())
            }))
            .child(div().flex_1())
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
            )
    }

    /// 对象列表本体：目录行（下钻）与对象行（选中进检查器）+ 底部统计与翻页。
    fn render_object_list(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = v_flex()
            .id("object-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .py_1();

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

        for (ix, entry) in visible {
            match entry {
                ListingEntry::CommonPrefix(prefix) => {
                    let label = display_name(prefix).to_string();
                    let prefix = prefix.clone();
                    list = list.child(
                        h_flex()
                            .id(("object-row", ix))
                            .mx_3()
                            .px_2()
                            .py(px(4.))
                            .rounded(px(6.))
                            .gap_2()
                            .text_size(px(13.))
                            .hover(|row| row.bg(theme.accent))
                            .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                                // 目录点击 = 下钻；经统一选择路径保留选中集合
                                this.handle_object_row_click(
                                    ix,
                                    ClickedEntry::CommonPrefix(prefix.clone()),
                                    event.modifiers(),
                                    cx,
                                );
                                this.open_prefix(prefix.clone(), cx);
                            }))
                            .child(Icon::new(IconName::Folder).text_color(theme.accent_foreground))
                            .child(div().truncate().child(label)),
                    );
                }
                ListingEntry::Object(object) => {
                    // 行内重命名进行中：该行渲染为输入框（Finder 式）
                    if self
                        .renaming
                        .as_ref()
                        .is_some_and(|(key, _)| *key == object.key)
                    {
                        let editor = self.renaming.as_ref().expect("刚检查过").1.clone();
                        list = list.child(
                            h_flex()
                                .id(("object-row-rename", ix))
                                // Esc 取消绑定在此 context（Input propagate 后命中）
                                .key_context("Renaming")
                                .mx_3()
                                .px_2()
                                .py(px(2.))
                                .rounded(px(6.))
                                .gap_2()
                                .text_size(px(13.))
                                .bg(theme.list_active)
                                .child(Icon::new(IconName::File).text_color(theme.muted_foreground))
                                .child(div().flex_1().min_w_0().child(Input::new(&editor).small())),
                        );
                        continue;
                    }
                    let selected = self.selected_object_keys.contains(&object.key);
                    let key = object.key.clone();
                    let size = format_size(object.size);
                    let time = format_time(object.put_time_millis);
                    list = list.child(
                        h_flex()
                            .id(("object-row", ix))
                            .mx_3()
                            .px_2()
                            .py(px(4.))
                            .rounded(px(6.))
                            .gap_2()
                            .text_size(px(13.))
                            // selection ≠ primary（agents.md §7）：选中用 list_active，
                            // hover 是可交互反馈用 accent
                            .when(selected, |row| row.bg(theme.list_active))
                            .hover(|row| row.bg(theme.accent))
                            .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                                eprintln!("[preview] selected key={key}");
                                this.handle_object_row_click(
                                    ix,
                                    ClickedEntry::Object(key.clone()),
                                    event.modifiers(),
                                    cx,
                                );
                            }))
                            .child(Icon::new(IconName::File).text_color(theme.muted_foreground))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .child(display_name(&object.key).to_string()),
                            )
                            .child(
                                div()
                                    .text_color(theme.muted_foreground)
                                    .text_size(px(12.))
                                    .child(size),
                            )
                            .child(
                                div()
                                    .text_color(theme.muted_foreground)
                                    .text_size(px(12.))
                                    .child(time),
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
            .text_size(px(12.))
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
        list.into_any_element()
    }

    /// 右侧 Inspector：选中对象的元数据；未选中时显示占位破折号。
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
                    .text_size(px(13.))
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
                            .text_size(px(12.))
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
                            .text_size(px(12.))
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
                            .text_size(px(12.))
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
                    Input::new(&editor).h(px(220.)).into_any_element()
                } else if let Some(text) = self.preview_text.clone() {
                    div()
                        .w_full()
                        .h(px(220.))
                        .overflow_hidden()
                        .p_2()
                        .text_size(px(11.))
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
                                    .text_size(px(42.)),
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
                                .text_size(px(32.)),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
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
                                .text_size(px(42.)),
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
                                .text_size(px(13.))
                                .child(display_name(&object.key).to_string()),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
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
                        .text_size(px(12.))
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
                            .text_size(px(12.))
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
                                    Button::new("download-object")
                                        .label(if self.downloading {
                                            "下载中…"
                                        } else {
                                            "下载…"
                                        })
                                        .disabled(self.downloading)
                                        .with_size(Size::Small)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.start_object_download(cx)
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
                    .text_size(px(12.))
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
                                    .text_size(px(13.))
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
                let mut row = v_flex().px_3().py_1().text_size(px(12.)).child(
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
                                            .text_size(px(11.))
                                            .text_color(theme.muted_foreground)
                                            .child(label),
                                    )
                                    // 百分比只在已知总量时展示（未知总量算不出）
                                    .children(transfer_percent(task).map(|p| {
                                        div()
                                            .text_size(px(11.))
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
                            .text_size(px(11.))
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

impl gpui::Focusable for WorkspaceView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for WorkspaceView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let mut root = v_flex()
            .id("workspace")
            .relative() // 模态遮罩层的定位基准
            .size_full()
            .key_context("Workspace")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::handle_toggle_sidebar))
            .on_action(cx.listener(Self::handle_toggle_inspector))
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
        if let Some(palette) = self.palette.clone() {
            root = root.child(self.render_palette_overlay(&palette, &theme, cx));
        }
        root
    }
}

/// 传输进度条文本：已知总量 → "已完成 / 总量"；未知但有字节 → 字节数。
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

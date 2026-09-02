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
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, ExternalPaths, FocusHandle,
    InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent, ObjectFit,
    ParentElement as _, PathPromptOptions, Pixels, PromptButton, PromptLevel, Render, SharedString,
    StatefulInteractiveElement as _, Styled, StyledImage as _, Window, div, hsla, img,
    prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme, Disableable as _, Icon, IconName, Sizable, Size, Theme, TitleBar, button::Button,
    button::ButtonVariants as _, h_flex, progress::Progress, resizable::h_resizable,
    resizable::resizable_panel, spinner::Spinner, v_flex,
};

use object_storage_app::{AppServices, PersistedTransfer};
use object_storage_core::{ByteProgress, StorageProvider as _};
use object_storage_domain::{Account, Bucket, CloudObject, ListObjectsRequest, ListingEntry};
use object_storage_transfer::{
    TaskRunner, TransferEngine, TransferKind, TransferOp, TransferRequest, TransferState,
    TransferTask,
};

use crate::account_modal::AddAccountModal;
use crate::actions::{
    AddAccount, CloseWindow, DeleteObject, DownloadObject, OpenCommandPalette, PreviewObject, Quit,
    Refresh, ToggleInspector, ToggleSidebar, UploadFiles, UploadFolder,
};
use crate::command_palette::CommandPaletteView;

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

/// 侧栏/内容区的异步加载状态。`Loaded` 不单独建模——数据非空且 state==Idle 即加载完成。
#[derive(Debug, Clone, PartialEq, Eq)]
enum AsyncState {
    Idle,
    Loading,
    Failed(String),
}

/// Inspector 底部的下载结果提示（成功/失败一次一笇）
#[derive(Debug, Clone)]
struct DownloadMessage {
    is_error: bool,
    text: String,
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
    /// 删除确认 sheet 已弹出（gpui 禁止重入 prompt）
    delete_prompt_open: bool,
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

impl WorkspaceView {
    pub fn new(services: Arc<AppServices>, cx: &mut Context<Self>) -> Self {
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
            selected_bucket: None,
            entries: Vec::new(),
            objects_state: AsyncState::Idle,
            loading_more: false,
            next_marker: None,
            current_prefix: None,
            selected_object_key: None,
            downloading: false,
            uploading: false,
            deleting: false,
            previewing: false,
            preview_path: None,
            delete_prompt_open: false,
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
        self.selected_object_key = None;
        self.download_message = None;
    }

    /// 从头（当前前缀的第一页）重新加载对象。
    fn reload_objects(&mut self, cx: &mut Context<Self>) {
        self.entries.clear();
        self.next_marker = None;
        self.selected_object_key = None;
        self.download_message = None;
        self.objects_state = AsyncState::Loading;
        cx.notify();
        self.request_objects(None, cx);
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
        let prefix = self.current_prefix.clone();
        let services = Arc::clone(&self.services);

        cx.spawn(async move |this, cx| {
            let request = ListObjectsRequest {
                bucket,
                prefix,
                delimiter: Some("/".into()),
                marker,
                limit: OBJECTS_PAGE_LIMIT,
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
                    Ok::<_, String>(path)
                })
                .await;
            this.update(cx, |this, cx| {
                this.previewing = false;
                eprintln!(
                    "[preview] download result: {}",
                    if result.is_ok() { "ok" } else { "error" }
                );
                match result {
                    Ok(path) => {
                        eprintln!("[preview] inline path={}", path.display());
                        this.preview_path = Some(path);
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

    fn handle_preview_object(
        &mut self,
        _: &PreviewObject,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
    fn confirm_and_delete_object(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() || self.add_modal.is_some() {
            return;
        }
        if self.deleting || self.delete_prompt_open || self.quit_prompt_open {
            return;
        }
        let Some(object) = self.selected_cloud_object() else {
            self.download_message = Some(DownloadMessage {
                is_error: true,
                text: "请先选中一个对象再删除".into(),
            });
            cx.notify();
            return;
        };
        let Some(account_id) = self.selected_account_id.clone() else {
            return;
        };
        let Some(bucket) = self.selected_bucket.clone() else {
            return;
        };
        let key = object.key.clone();
        let name = display_name(&key).to_string();
        self.delete_prompt_open = true;
        let message = format!("删除“{name}”？");
        let detail = format!("将从空间 {bucket} 永久删除，无法撤销。");
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
            let result = cx
                .background_executor()
                .spawn(async move { services.delete_object(&account_id, &bucket, &key) })
                .await;
            this.update(cx, |this, cx| {
                this.deleting = false;
                match result {
                    Ok(()) => {
                        this.reload_objects(cx);
                        this.download_message = Some(DownloadMessage {
                            is_error: false,
                            text: format!("已删除 {name}"),
                        });
                    }
                    Err(e) => {
                        this.download_message = Some(DownloadMessage {
                            is_error: true,
                            text: format!("删除失败：{e}"),
                        });
                    }
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

    // ---- Action 处理（与菜单/快捷键共享） ----

    fn handle_toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
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
        let palette = cx.new(|cx| CommandPaletteView::new(window, cx));
        // 面板关闭（open=false）后由观察者丢弃实体并归还焦点。
        cx.observe_in(&palette, window, Self::handle_palette_changed)
            .detach();
        palette.update(cx, |palette, cx| palette.focus_input(window, cx));
        self.palette = Some(palette);
        cx.notify();
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
            AsyncState::Failed(msg) => vec![
                self.sidebar_error_row(
                    theme,
                    "sidebar-buckets-error",
                    msg,
                    cx.listener(|this, _, _, cx| this.retry_buckets(cx)),
                )
                .into_any_element(),
            ],
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

    /// 可点击的错误行（点击重试），msg 截断展示。
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
        div()
            .id(id)
            .mx_2()
            .px_2()
            .py(px(5.))
            .rounded(px(6.))
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(12.))
            .text_color(theme.danger)
            .hover(|row| row.bg(theme.sidebar_accent))
            .on_click(on_click)
            .child(Icon::new(IconName::TriangleAlert))
            .child(div().truncate().child(format!("{msg}（点击重试）")))
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

        for (ix, entry) in self.entries.iter().enumerate() {
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
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_prefix(prefix.clone(), cx)
                            }))
                            .child(Icon::new(IconName::Folder).text_color(theme.accent_foreground))
                            .child(div().truncate().child(label)),
                    );
                }
                ListingEntry::Object(object) => {
                    let selected = self.selected_object_key.as_deref() == Some(object.key.as_str());
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
                            .when(selected, |row| row.bg(theme.accent))
                            .hover(|row| row.bg(theme.accent))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                eprintln!("[preview] selected key={key}");
                                this.selected_object_key = Some(key.clone());
                                this.start_object_preview(cx);
                                cx.notify();
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
                            .flex_1()
                            .px_2()
                            .py_2()
                            .text_size(px(12.))
                            .text_color(theme.foreground)
                            .child("预览"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .px_2()
                            .py_2()
                            .text_size(px(12.))
                            .text_color(theme.muted_foreground)
                            .child("详情"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .px_2()
                            .py_2()
                            .text_size(px(12.))
                            .text_color(theme.muted_foreground)
                            .child("元数据"),
                    ),
            );

        if let Some(object) = selected {
            let preview_content = match preview_path {
                Some(path) => img(path)
                    .w_full()
                    .h(px(220.))
                    .object_fit(ObjectFit::Contain)
                    .into_any_element(),
                None => div()
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
                    .into_any_element(),
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
                        Button::new("preview-object-inspector")
                            .label(if self.previewing {
                                "准备预览…"
                            } else {
                                "预览"
                            })
                            .disabled(self.previewing)
                            .with_size(Size::Small)
                            .on_click(cx.listener(|this, _, _, cx| this.start_object_preview(cx))),
                    ),
            );
        }
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
                    let pct = match task.bytes_total {
                        Some(total) if total > 0 => (task.bytes_done as f32 / total as f32) * 100.0,
                        _ => 0.0,
                    };
                    let label = match task.bytes_total {
                        Some(total) => {
                            format!("{} / {}", format_size(task.bytes_done), format_size(total))
                        }
                        None if task.bytes_done > 0 => format_size(task.bytes_done),
                        None => String::new(),
                    };
                    row = row.child(
                        v_flex()
                            .gap_1()
                            .child(Progress::new().h(px(4.)).value(pct))
                            .children((!label.is_empty()).then(|| {
                                div()
                                    .text_size(px(11.))
                                    .text_color(theme.muted_foreground)
                                    .child(label)
                            })),
                    );
                }
                if let Some(error) = &task.error {
                    row = row.child(
                        div()
                            .text_color(theme.danger)
                            .truncate()
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
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(self.render_title_bar(&theme, cx))
            .child(self.render_body(&theme, cx));

        // 模态遮罩层（先渲染 → 在下层），命令面板后渲染盖在其上。
        if let Some(modal) = self.add_modal.clone() {
            root = root.child(self.render_add_modal_overlay(&modal, &theme, cx));
        }
        if let Some(palette) = self.palette.clone() {
            root = root.child(self.render_palette_overlay(&palette, &theme, cx));
        }
        root
    }
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
}

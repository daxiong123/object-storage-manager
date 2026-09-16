# agents.md — 项目操作契约

> 单一事实来源。完整规范见 `docs/spec/macos-platform-spec.md`（冲突时以规范为准）。
> 所有 Agent 在本仓库工作前必须读完本文件。

## 1. 产品定位

**一款专为 macOS 设计的高性能七牛 Kodo / 阿里云 OSS 对象存储工作台。**

> Build the best Qiniu Kodo + Aliyun OSS client for macOS, not the most portable one.

UX 标准：如果 Zed / ChatGPT 团队设计一个 OSS Browser，大概就应该是这个样子。不是 OSSBrowser 换皮，也不是 Web 云控制台塞进桌面客户端。

## 2. 平台

- **macOS Only**。不支持且不考虑 Windows / Linux / Web / iOS / Android。
- Apple Silicon 是 P0 目标（`aarch64-apple-darwin`）。Universal Binary 可选，非 P0。
- 最低 macOS 版本：**已定 macOS 14+**（`.cargo/config.toml` 中 `MACOSX_DEPLOYMENT_TARGET=14.0`；GPUI 0.2.2 对下限无更高要求，取规范建议的新版本档）。
- **禁止**提出"为了以后支持 Windows/Linux 我们应该……"。除非用户主动改变产品目标。

## 3. 技术选型（硬约束）

优先级：`macOS Native Experience > 性能 > 低内存 > 开发效率 > 架构优雅 > 跨平台能力(权重 0)`。

固定栈：

```text
Rust + GPUI + gpui-component   主 UI（Sidebar + Content Workspace / Virtual Table / Command Palette / Keyboard-first）
Tokio                          异步运行时
reqwest + rustls               网络层
SQLite                         持久化（永不存 Secret）
macOS Keychain                 所有 Secret（Security.framework binding）
objc2 / objc2-app-kit / objc2-foundation / objc2-security   系统集成
```

明确禁止引入：

```text
Electron / Chromium / WebView / React / Vue / Node.js Runtime
Slint 切换（除非 GPUI 有无法解决的技术阻碍）
trait GuiBackend 等跨平台 GUI 抽象层
跨平台 Credential abstraction（直接 MacOSCredentialStore，除非抽象有测试价值）
Windows / Linux compatibility code
Lucide + Material + Heroicons 混用
```

原则：**GPUI 负责主 UI，macOS Framework 负责系统级能力**；macOS 已提供的能力（文件选择器、剪贴板、Quick Look、通知等）不要重新实现。为真实需求抽象，不为不存在的平台抽象。

## 4. Workspace 结构

```text
crates/
    desktop/        macOS App 入口
    app/            Application Services
    domain/         Domain Models
    storage-core/   Provider abstraction
    provider-qiniu/ Qiniu Kodo
    provider-aliyun/ Aliyun OSS
    transfer/       Transfer Engine（队列/状态机/watch 事件驱动；runner 闭包由 UI 注入）
    persistence/    SQLite
    macos/          macOS native integration（Keychain/系统事件 NSWorkspace+NWPathMonitor/QuickLook/Clipboard/通知）
    preview/        Preview
    ui/             GPUI views/components
    common/         small shared utilities
```

不要建 `platform/{macos,windows,linux}` 目录。

### 4.1 crates/ui 内部布局（对齐 waku 的 `src/app/*` + `src/ui/*`）

`workspace_view.rs` 曾是 8724 行单文件（占全仓 45%、含 208 个函数与 61 个测试），
任何改动都要在这一个文件里定位。现按 feature 拆分：

```text
crates/ui/src/
    workspace/          # 主窗口：按功能分文件
        mod.rs          #   WorkspaceView 结构体 / 状态类型 / 常量 / new / render / render_body
        accounts.rs buckets.rs sidebar.rs titlebar.rs filter.rs objects.rs
        object_list.rs selection.rs sort.rs download.rs upload.rs delete.rs
        rename.rs folder.rs copy_move.rs preview.rs menus.rs transfers.rs
        settings.rs quit.rs palette.rs format.rs
        tests.rs        #  61 个纯逻辑测试
    ui/                 # 跨视图复用的**基础件**（不放业务视图）
        mod.rs          #  icon_button：紧凑 ghost 图标按钮的唯一形状
        overlay.rs      #  mask / surface / fade_in（浮层规范的可执行版本）
        motion.rs       #  动效时长与曲线常量
        menu.rs         #  popup / item / separator（锚定菜单形状）
    theme.rs tokens.rs actions.rs file_type.rs account_modal.rs settings_modal.rs
    command_palette.rs lib.rs
```

拆分机制（决定了这是**纯搬运而非重写**，也决定了以后新增功能的放置方式）：

- 子模块以 `use super::*;` 取用父模块的类型与 import；父模块把子模块条目
  `use self::xxx::*;` glob 回来，整棵模块树看到的名称与拆分前一致。
- 子模块的 `impl WorkspaceView` 是**固有 impl**，方法随类型可见、不需要 import；
  只含方法的模块因此不参与 glob（否则会得到 unused_imports）。
- 私有字段的可见性是「定义模块及其后代」，所以子模块可直接读写 `WorkspaceView`
  的字段，**不要**为了拆分去加 getter。
- 原先私有的条目标 `pub(super)`（= 本模块子树可见）即可跨子模块调用，不必放宽到
  `pub(crate)`。

新增 UI 的放置判据：**业务视图进 `workspace/*`；两个以上视图重复、且能收成具名
形状的进 `ui/*`**。收「具名形状」（`icon_button` / `popup` / `separator`）而不是
通用抽象（`row(..)` / `card(..)`）——后者要塞进宽度、内边距、圆角、悬停、间距等
一堆参数，读起来不如各视图显式写链式调用。

## 5. 关键技术决策

| 领域 | 决策 |
|---|---|
| 文件选择 | 只用 gpui 平台 API：`cx.prompt_for_new_path`（保存）/ `cx.prompt_for_paths`（打开），结果经 oneshot 异步回传；**禁止在事件处理器里同步 `runModal`**——模态循环重入 gpui `App` RefCell 借用 → "RefCell already borrowed" 闪退（详见 docs/notes/gpui-api-notes.md「文件对话框」；crates/macos 不再封装面板，panel.rs 已删） |
| 剪贴板 | `NSPasteboard`；Signed URL 可配置 N 秒自动清除 |
| Open With / Show in Finder | `NSWorkspace`；远端 Object 下载到临时目录再打开 |
| 预览 | 常见格式应用内；PDF/Office/视频走系统 Quick Look，不自建 Preview Engine |
| Keychain key | `service = com.<company>.<app>.credentials`，`account = <account_uuid>`（不用账号名）；service 名集中在 `crates/macos/src/keychain.rs` 的 `KEYCHAIN_SERVICE`，Bundle ID 定稿后只改一处；实现用 security-framework 3 的 generic password 三函数（`set/get/delete_generic_password`，get 返回 `Vec<u8>`，not-found 用 `err.code() == errSecItemNotFound` 归一化为正常分支） |
| 账号编排 | `AccountService`（crates/app）：Secret 只入 Keychain，元数据（含 AK，AK 非 Secret）只入 SQLite；一致性顺序 —— add 先 Keychain 后 SQLite（失败补偿删 Keychain，补偿再失败报复合错误不吞）；delete 先 SQLite 后 Keychain（幂等）；Keychain 条目缺失报 `MissingSecret` 不静默。本层无状态：`load_secret`/`build_provider(_with_secret)` 分离，Secret 可由调用方提供 |
| SK 会话缓存 | 钥匙串授权弹窗只在「选中账号后的第一次操作」出现：`AppServices.build_provider`（crates/app/src/services.rs）优先用单条会话缓存 `cached_secret`（最近使用账号的 SK，内存驻留、不落盘不进日志），未命中才现取钥匙串并写缓存；切换账号即置换淘汰。账号删除后缓存可能残留，但任何使用都因元数据缺失报 NotFound（不复活）。缓存锁与账号锁永不嵌套 |
| SQLite schema | `accounts` 表列固定为 `id/name/provider/access_key/created_at_millis`；`transfers` 表（⌘Q 暂停并退出）列为 `id/kind/account_id/bucket/object_key/dest/display_name/state/enqueued_at_millis`，kind/state 有 CHECK。两表均有「无 Secret 列」回归测试把守；provider/kind/state 用 CHECK 在 DB 层 Fail Fast |
| 本地路径 | `PathBuf`；Cloud Object Key：`String` + `/`。两者严格区分 |
| Provider trait | `StorageProvider`（`crates/storage-core`）：方法返回 `impl Future + Send`（不用裸 `async fn`，Send 义务显式化，否则无法 spawn 到 tokio/gpui 后台执行器）；非 dyn-safe，上层按服务商 enum 分发 |
| 七牛签名 | V2 请求签名逐字节核对自官方 SDK 源码并内置官方向量测试（V1 hello/world + V2 X-Qiniu-* 规范化排序）；坑：Base64 必须带 padding、签名用实际发送的原始 query 串、X-Qiniu-* 头名规范化为 Title-Case 后排序、putTime 单位 100ns。详见 `docs/notes/qiniu-api-notes.md`，勿凭记忆重写 |
| 七牛区域上传 | 上传 host 按 bucket 经 UC `GET /v4/query?ak=&bucket=` 解析（**公开接口无 Authorization**；官方 Rust SDK `BucketRegionsQueryer` 同构），取 `hosts[0].up.domains[0]`，进程内缓存（host 级 ttl，缺省 86400），失败回退 `upload.qiniup.com`。测试模式（UC 指向 127.0.0.1）直接用注入 up_base，不做真实解析。**断点续传明确不做**（用户决定） |
| Transfer | Sleep/Wake/断网后状态为 `Waiting/Paused` 并恢复，**不得**误标 `Failed`（P0）；事件驱动，不轮询。已落地：`crates/transfer` 引擎（队列/状态机/并发上限）+ 单测锁死 P0 语义；任务执行体由 UI 层注入 `TaskRunner` 闭包（内调 `AppServices::build_provider` 即锁即放），future spawn 到 AppServices 的 tokio 运行时，暂停/挂起/取消 = `JoinHandle::abort()`（future 在 await 点丢弃即断 reqwest 连接）；attempt 代号丢弃过期完成回调；UI 经 `watch` 令牌订阅快照（无定时器）；字节进度经 `ProgressSink` 回写，通知节流 100ms，禁止按块 redraw。系统事件已接线（`crates/macos/src/system_events.rs`）：睡眠 `NSWorkspaceWillSleepNotification` → `suspend_all`；网络 `NWPathMonitor`（C API + block crate，私有 dispatch 队列）非 satisfied → `suspend_all`、satisfied → `resume_all`；**didWake 故意不恢复**——唤醒瞬间网络常未就绪，等 satisfied 事件再 resume，否则重排队任务会变 `Failed`（P0）。监视器在 `WorkspaceView::new` 启动（单窗口一次性创建约束）。上传已落地：⌘U 多选文件；「上传文件夹…」递归入队（key=当前前缀+目录名+相对路径，`/` 分隔；跳过 .DS_Store/`._*`/符号链接/空目录）。七牛表单直传（token 在 multipart，分块读盘不进 `Vec<u8>`）。Finder 拖放：对象浏览区 `on_drop::<ExternalPaths>`（文件立即入队，目录后台递归）；拖入时 accent 浅底提示。区域上传域名、断点续传、拖出到 Finder 在后续里程碑 |
| AppServices 线程模型 | `AccountService` 内含 rusqlite `Connection`（内部 RefCell，非 Sync），直接 `Arc<AppServices>` 进不了 gpui 后台任务（要求 Send+Sync）。连接统一收进 `Mutex<AccountService>`（crates/app/src/services.rs），同一时刻至多一个后台线程用数据库/钥匙串；锁毒化（持锁线程 panic）直接 panic 响报，不静默 |
| UI 异步编排 | UI 层永不直接调 provider/runtime：一律 `cx.spawn` → `background_executor().spawn` 调 AppServices 阻塞方法（内部 `runtime.block_on`）。并发竞态用代数计数（`bucket_gen`/`object_gen`）丢弃过期结果（last-click-wins）；添加账号模态用 `done/closed` 标志 + `observe_in` 由 WorkspaceView 回收，保存中禁止关闭（防丢成功结果） |
| UI 冒烟边界 | 无屏幕录制权限 → `kCGWindowName` 恒为 `"(no title)"`，窗口存在性用 `/tmp/winall.swift`（按 OwnerName 过滤）输出非空判断，不能 grep 窗口名；gpui 不建 AX 树，UI 内部交互无法脚本化，只能人肉验证 |
| ⌘Q vs ⌘W | ⌘W 只关窗口（**必须先禁用 close 动画再 remove_window**，见下行）；⌘Q 有 Transfer 时弹确认，默认 `Pause + Persist` |
| 关窗口实现 | macOS 15 close 动画会被 gpui 立即 teardown 杀死 → 窗口卡死可见。`handle_close_window`：`setAnimationBehavior: None` + `remove_window()`；失败方案与机制详见 `docs/notes/gpui-api-notes.md`「关闭窗口」章节，勿重复试错 |
| 通知 | `UserNotifications.framework`，仅长时间 Transfer 完成/失败、Migration 完成 |
| 字体 | 默认系统 UI 字体 / Menlo（代码），不捆绑 Inter；用户可在设置里配置界面字体族、界面字号缩放、代码字体族与代码字号。字号缩放统一走 `crates/ui/src/tokens.rs` 的**字号阶梯**（`caption`/`label`/`body`/`heading`/`title`…，见下方「设计 token」行），不要新增裸 `text_size(px(...))`，也不要新增阶梯以外的字号档位 |
| 自动更新 | 架构预留 Updater 边界；Check→Download→Verify→Install→Restart，必须验证签名+校验和 |
| Action 注册点 | 所有跨 菜单/快捷键/右键菜单/工具栏 共用的 Action **只**定义在 `crates/ui/src/actions.rs`（`actions!(cloud_storage, …)`），键位在 `bind_keys(cx)` 统一绑定，不得散落各 view |
| 全局键位边界 | 不绑定 ⌘X/⌘C/⌘V/⌘A 全局快捷键（会吞文本输入的原生响应链）；Edit 菜单走 `MenuItem::os_action` 触发系统行为。⌘A 全选走 **Workspace context 绑定**（`SelectObjectAll`），命令面板/输入框聚焦时由组件原生响应链处理 |
| 对象多选 | `selected_object_keys: IndexSet<String>`（有序）+ `selection_anchor`（⇧ 范围起点）+ `selected_object_key`（主选=集合最后一项，预览/详情兼容）。语义决策在纯函数 `apply_object_selection`（workspace/selection.rs，单测锁死）；⇧Click 分支先于 ⌘Click（⌘⇧=增量范围）。批量删除：确认一次（标题报数量，明细列前 3 个名）→ 后台逐项删，失败逐项可见不中断；批量下载（多选≥2）：`prompt_for_paths(directories:true, multiple:false)` 选目标目录 → 逐项入队 |
| Inline Rename | Return 进（Finder 式，仅单选；多选报错），Esc 取消。编辑态该行渲染 Input（context "Renaming" 接 DismissRename；Input 未设 clean_on_escape 才能 propagate Esc）。提交：`rename_target_key` 只改最后一段（含 `/`、`.`、`..` 拒绝，单测锁死）→ `AppServices::rename_object`（下载临时文件→上传新 key→删旧 key；上传失败旧对象保留，删旧失败报复合错误不静默）。预选主名（不含扩展名）待 gpui-component 暴露 selection API 后补 |
| ⌘F 前缀搜索（原「⌘F 过滤」已废弃） | 工具栏右端那个框是**服务端前缀搜索**（对齐 oss-browser2 的「文件前缀搜索」）：回车或点 🔍 **显式提交**（不是边打边筛），提交时把输入值经 `normalize_prefix_query` 规范化后作为 `ListObjectsRequest::prefix` **重新列举**——所以能查到还没加载出来的对象，本地过滤做不到。⌘F 只负责聚焦该框，Esc 清空（框是常驻控件，没有隐藏态）。**已删除本地过滤**（`filter_entries` / `filtered_ix` / `ToggleObjectFilter` / `workspace/filter.rs`）：旧的「本地过滤 + 选择集」组合有过安全隐患——⌘⌫/批量下载取选择**全集**且不含可见性判断，过滤后按 ⌘⌫ 会删掉「看不见的对象」（实测复现过：显示 5 行、实删 45 个）。现在改由**重新列举**代替：`reload_objects` 会清空 entries 与选择，那条隐患从根上消失。跳转 Bucket：带数据 Action `SelectBucketByName(String)`（`#[action(no_json)]`）保留作菜单/快捷键入口；**命令面板动态命令例外**——经 `WeakEntity<WorkspaceView>` 直调 `select_bucket`，避免面板关闭后向失效焦点 deferred 派发 Action |
| 设置 ⌘, | `Settings`（crates/persistence/src/settings.rs）存 `settings.json`（Application Support/CloudStorage/，spec §58；永不存 Secret），损坏显式报错不静默重置（Fail Fast）；新增字段必须 serde default，旧配置缺字段正常补默认。可配：签名链接 TTL、复制后清剪贴板秒数（0=关闭）、外观模式（System/Light/Dark）、界面字体族/字号缩放、代码字体族/字号、传输并发数（1..8）、默认下载目录（单/批量下载交互一致：设置了有效默认目录先弹确认 sheet「使用默认目录/另存为…或另选目录/取消」；单文件另存为面板以默认目录为初始目录；不自动回写）。运行时值在 `WorkspaceView.settings`，改动经 `SettingsModal`（自建 overlay，同 AddAccountModal 机制）保存后即时生效；`copy_object_url_request` 的 TTL 是运行时参数，禁止退回编译期常量。Transfer 列表：进度条 + 百分比 + 字节；失败原因完整换行展示（不 truncate） |
| Open With / Show in Finder | ⌘O / 对象菜单入口：`ensure_local_copy` 复用 `preview_path`（判据 `cached_copy_matches` 纯函数：文件名 = `{nanos}-{display_name}` 后缀匹配且非全等，单测锁死），无副本先下载到临时目录（与预览同缓存位置）→ `object_storage_macos::open_with_default_app`（NSWorkspace）/ gpui `cx.reveal_path`（spec §14/§16） |
| Quit 处理 | 全局 `cx.on_action` 在 **bubble 末尾**（源码 `app.rs:1696`，不是 capture）：有窗口时 `WorkspaceView::handle_quit` 先处理，窗口全关后仍可 ⌘Q。有活动传输时走 gpui `window.prompt`（NSAlert sheet + oneshot，**禁止 runModal**）三按钮：暂停并退出（默认 Return）/ 取消（Esc）/ 立即退出；暂停并退出把活动任务写入 SQLite `transfers` 表（无 Secret 列）后 `cx.quit()`，下次启动 `take_transfers` 入队恢复（paused 保持暂停，其余自动继续）；立即退出 `clear_transfers` 后退出；落盘失败 Fail Fast 不退出 |
| Sidebar | **自建视图**，不用 gpui-component `Sidebar`（组件固定 255px/48px，与规范 180/220/360 + 44px rail 冲突）；可拖拽宽度用 gpui-component `resizable`，按布局变体用不同 group id 保持各自记忆宽度。主界面只保留两列结构，不再提供额外详情列 |
| 文件类型图标 | `crates/ui/src/file_type.rs`：按 Object Key 扩展名选 Lucide SVG（素材 `crates/ui/assets/icons/`，Lucide 官方源码 stroke=currentColor，与 gpui-component 图标同规格；只用 Lucide 一家）。纯函数 `file_icon_kind`（扩展名 → 9 类分组，单测锁死；无扩展名/未知 = Generic）。自有 SVG 用 `include_bytes!` + GPUI Kit 0.6.1 `Icon::data` 直接渲染，不再维护组合 AssetSource/路径注册表；通用图标仍由 `gpui-kit-assets` 提供。着色只用 theme 语义 token（muted/accent），不硬编码色值 |
| 命令面板 | GPUI Kit 0.6.1 `Command` + `CommandState` 负责过滤、虚拟列表、键盘导航、滚动和无障碍语义；应用层只维护命令条目、共享 Action 编排与动态 Bucket Handler。不要恢复手写列表/方向键 Action。`command_palette::tests::native_command_filters_and_confirms_dynamic_items` 用新版 headless test-support 锁死过滤 + 动态确认链路 |
| GPUI API 陷阱 | gpui-pre 0.3.4 / gpui-component 0.6.1 已验证的 API 事实与陷阱清单见 `docs/notes/gpui-api-notes.md`；写 UI 前先查，不凭记忆猜签名。0.6.x 的改名要记住：`Corner`→`Anchor`、`window.focus(&h)`→`(…, cx)`、代码编辑器 `InputState`→`EditorState`、`Progress::new()`→`new(id)` 且 `value` 取值域变为 0..100 |
| 「关于」用系统面板 | 走 `NSApplication.orderFrontStandardAboutPanelWithOptions:`（`crates/macos/src/about.rs` 的 `show_about_panel`，`OpenAbout` Action 入口不变）。**不要**再自建 About 浮层：系统已提供的能力不自建（§3），而且原生面板自动跟随系统语言/外观/无障碍设置。对齐参照实现——oss-browser2 的「关于」就是 Electron 的 `role: 'about'`（主菜单里只有这一项，两份 bundle 里都没有自建对话框）。用 **WithOptions** 而非无参版本：无参版读 bundle 的 Info.plist，开发期裸二进制没有它，会显示可执行文件名与空版本号 |
| 弹层规范（所有自建 overlay 统一） | **Esc 必关**：有输入框的经专属 context 绑定（`Renaming`→DismissRename、`AccountModal`/`SettingsModal`→DismissModal、`ObjectFilter`→DismissFilter），无输入框的（详情/预览）统一 context `Overlay` + `UnifiedDismiss`，卡片上 `on_action` 注册 handler；**遮罩点击关**：遮罩 `on_mouse_down` 调 close，busy 中由 close handler 自行拒绝（保存/创建/复制移动中不可关）；**卡片两相冒泡阻断由构造器保证**：浮层的遮罩与卡片一律经 `crates/ui/src/ui/overlay.rs` 的 `mask()` / `surface()` 构造——`surface()` 内已做 `on_mouse_down` + `on_mouse_up` 双阻断（只阻断 down 时，卡片内滚动列表的滚动手势 up 会冒到遮罩，表现为「滚动一下就关闭」，SettingsModal 曾漏此项）；不要再手写卡片样式链；**入场动画**：浮层一律经 `overlay::fade_in(id, el)` 收口（时长/曲线在 `ui/motion.rs`，180ms opacity 淡入；gpui 无 div 级 transform/blur，故无位移/模糊；退场即时），且必须在链式装配最后一步调用；**卡片宽度必须响应窗口**：写 `w_full().max_w(px(设计宽度))`，**不要**写死 `w(px(N))`（窗口窄于设计宽度时卡片会压出窗口边缘）；**锚定菜单（anchored）必须按触发点的窗口坐标定位**：`position_mode(AnchoredPositionMode::Window).position(<触发点窗口坐标>)`（右键取 `MouseDownEvent.position`），**不要**靠「锚定元素所在容器的原点」相对定位——行在滚动容器里时两者坐标系不一致，菜单会压到侧栏上并纵向偏移（机制见 docs/notes/gpui-api-notes.md「弹出层」）；遮罩自带 `p_6` 内边距，新增遮罩不要再各写 padding；**例外**：命令面板卡片由 GPUI Kit `Command` 自行绝对定位（按窗口宽度手算 `left`），必须用**无内边距**遮罩——绝对定位子元素的包含块是遮罩的 padding box，套 `mask()` 的 `p_6` 会把卡片整体右移 24px 而偏心（见 `render_palette_overlay` 注释）；该手算 `left` 在窗口窄于卡片时会得到负值、卡片压出左边缘（**待修**）；**视觉统一**：卡片圆角 8（= `tokens::radius_lg()`，Linear 风格小圆角）、标题字号 16 SEMIBOLD（`tokens::title()`，**含 AddAccountModal**——它曾是唯一用 `heading()` 的）、标题栏右侧关闭按钮（busy 时 disabled）、底部按钮「取消在左、主操作在右」（macOS HIG 顺序）。新弹层先按本行自查再实现 |
| 标题栏内的交互控件必须保留 `on_mouse_down(stop_propagation)` | 统一标题栏（gpui-component `TitleBar`）在 macOS 上把**双击**当窗口缩放（`on_double_click → window.titlebar_double_click()`）。**拦住它靠的是 mouse_down 的 stop，不是 click 的 stop**：标题栏要触发双击，必须先在**它自己的** mouse_down（bubble 阶段）里置位，才能在 mouse_up 时合成 click；控件在 mouse_down 就 `stop_propagation()` 之后，标题栏那一步永远不会发生。所以**不要**再额外加 `.on_click(stop)`——我一度以为要在过滤框上加它来修「双击选词缩放窗口」，并写了测试；结果测试的**对照组**（去掉 mouse_down 的 stop）显示 click 本来就会漏到标题栏，说明护栏早已生效、那个猜想是错的。两条断言留在 `workspace/titlebar.rs` 的测试里（`click_reaches_the_titlebar_when_the_mouse_down_stop_is_removed` 为对照组），谁误删这条 stop 就会红。**教训**：给「祖先会不会收到事件」下结论前，先写一个能失败的对照组，否则会写出「永远通过」的测试。 |
| 点击空白清空选择（B6，已落地并实测通过） | 清空处理器挂在**对象列表滚动容器自己**身上（`render_object_list` 的 `on_mouse_down`；列表改虚拟化后挂点仍是滚动容器——`UniformList` 自己实现了 `InteractiveElement`，挂不到它的祖先上去），行处理器一律 `cx.stop_propagation()`——能冒泡到容器的一定没命中行，判据不需要几何、也不依赖注册顺序。**不要**回到前几版方案：内容区容器的 `capture_any_mouse_down` 的 `is_hovered` 只统计 `BlockMouseExceptScroll` hitbox 链（非滚动容器收不到事件）；canvas 几何命中版要每帧重建全部行 bounds + 每行挂一个 canvas，为一个交互付 O(行) 代价。注：行处理器 `stop_propagation` 会跳过 WorkspaceView 根节点的菜单兜底关闭，行内已由 `handle_object_row_click` 自己关菜单 |
| B6 复验要点（踩过的两个坑） | ①**空白区只在列表没铺满视口时才存在**：行数 ×行高（`tokens::row_height()`，100% 字号下 34pt）≥ 视口高度时最后一行下方没有空白，点到的是被裁切的行（被 `stop_propagation` 正确挡住，表现为「点了没反应」——这不是 bug，是测试选错了对象，换对象少的桶或把窗口拉高再看）。②**选中底色要按当前外观模式取色**：亮色 (229,232,245)、暗色约 (26,30,44)，系统会在傍晚自动切暗色，按亮色值去找会误判成「没选中」。实测依据：canvas 打印 LIST/WRAPPER bounds 完全一致（1056×710 @222,72，即列表确实铺满包裹层），空白点击时容器 handler 被调用且状态条选中信息与行底色同步清除 |
| UI 基础件（`crates/ui/src/ui/`） | 只放**跨视图复用的基础件**，不放业务视图。判据：两个以上视图重复、且能收成**具名形状**的才进来——`icon_button`（紧凑 ghost 图标按钮的唯一形状：图标 + 幽灵底 + Small + tooltip；工具栏/图标栏/各浮层关闭按钮共用）、`menu::popup`/`menu::item`/`menu::separator`（锚定菜单卡片与条目）、`overlay::mask`/`surface`/`fade_in`、`motion`（动效常量）。**不要**收通用抽象（`row(..)`/`card(..)`）——参数一多就不比各视图显式写链式调用清楚 |
| 动效与 reduce_motion | 时长与曲线只在 `crates/ui/src/ui/motion.rs` 定义，不在调用点写 `180`/`cubic_bezier(...)`。**走 `AnimationExt::with_animation` 的动效不需要自己判 `App::reduce_motion()`**——gpui 已内建（开启时渲染终态且不排帧，见 `gpui-pre/src/elements/animation.rs:407`）；只有装饰性的 `request_animation_frame` 才需要显式判 |
| 视觉风格（`ThemeStyle`） | 两套调色板共用同一份视图代码与同一组语义字段，切换只换颜色、不动布局：`Linear`（冷灰 hue 225 + 低饱和靛蓝，选中为靛蓝淡染）与 `Waku`（无彩灰 + 珊瑚 brand + **蓝色** selection，主 CTA 用单色 inverse）。取值见 `theme.rs` 的 `light_palette`/`dark_palette` 与 `waku_*_palette`（后者逐字段取自 waku 上游 `src/theme.rs`，用 `rgb(0x……)` 换算以便核对）。入口：设置 → 外观 → 视觉风格（持久化）；原型期另有 ⌘⌥T 一键切换（`CycleThemeStyle`，Workspace context）——评估完删掉该 Action + `handle_cycle_theme_style` 即无残留。**已知偏差**：上游在 macOS 上侧栏取 `transparent_black` 让系统 vibrancy 透出，本仓库未做真 vibrancy，故侧栏取与 canvas 同色、观感更平 |
| 色板族纪律（`crates/ui/src/theme.rs`） | 六族：surface / primary / interactive+selection / **status**（唯一允许外来色相的一族）/ 中性面（border+text 与之同族）。规则不是审美口号，由测试把守：`neutral_families_carry_no_hue`、`every_colored_field_matches_a_declared_anchor`（带色相的字段必须对得上 accent/selection/状态色之一）、`status_is_the_only_foreign_hue_family`、`grays_and_colors_are_clearly_separated`、`status_hues_are_mutually_distinguishable`，且对 **2 风格 × 亮暗** 全组合生效。判「是不是灰」用**绝对彩度 ΔRGB**，**不要**用 HSL 的 `s`——接近白/黑时 s 会失真（`rgb(0xF6F5F6)` 只差 1/255 却算出 s=0.053、色相 300°） |
| 可访问性 | 每个鼠标可达控件都必须键盘可达：`track_focus` + `tab_index`/`tab_group`/`tab_stop` 排 Tab 序，焦点态用可见的 `focus_visible`；补齐常规控件键（方向键、Home/End、Enter/Space、Esc）。**不得只靠颜色/悬停/动效表意**——状态色必须与图标或文字配对。命中区域宁可放大控件，不要把字形缩小。装饰性动画判 `cx.reduce_motion()` |
| render 纯净性（性能红线） | `render_*` 及其可达路径**不得**做文件系统、网络、子进程、阻塞锁或同步 IPC：一帧的成本只应与「屏上内容」成正比。需要 IO 就丢 `AppServices` 后台执行器、把结果存到实体上再 `cx.notify()`；未命中即「还不知道」，要优雅降级而不是当场去取（waku 把 render 里可达的 IO 称为 defect，即便它看起来便宜、被缓存或只对少数行发生） |
| 对象列表虚拟化 | 列表用 `gpui::uniform_list`（**不是** gpui-component 的 `List`/`ListState`——后者的 `ListDelegate` 自带选中/搜索语义，会与 `apply_object_selection` 那套被 61 个测试钉死的 Finder 选择语义打架）。只渲染可见区间，行数不再影响每帧成本。三处硬约束：①**行高固定**（见下行），行不能再由内容撑开；②行渲染闭包经 `cx.processor` 拿到 `&mut self`，但它每被调用一次就重排+重过滤一遍是 O(n log n)，所以**显示顺序每帧只算一次**存在 `display_order: Vec<usize>`（存下标而非克隆条目，过期只会表现为下标越界，由取用处 `get()` 兜住）；③**键盘导航要把选中行滚进视野**——虚拟列表里「选中了但看不见」等于没反馈，滚动用 `display_slot_of_key` 求槽位（目录前缀也占槽位，不能拿只含对象的 keys 下标替代） |
| 设计 token（`crates/ui/src/tokens.rs`） | 字号阶梯（`caption`/`label`/`body`/`heading`/`title`/`display`/`icon_sm`/`icon_lg`）、圆角（`radius`/`radius_lg`/`radius_icon`/`radius_nested`）、行距（`row_pad_y`/`row_pad_y_sidebar`/`row_pad_y_transfer`）、**对象列表行高**（`row_height`/`row_line_height`，见下行）与**随字号缩放**的列宽（`col_size_width`/`col_time_width`）都只在这里定义。新增文字/圆角/行距先选现有档位，不要新增第 9 个数字；任何跟随字号的尺寸都必须经 `text()`，固定 `px()` 在 140% 档位会与文字错位（列宽曾如此）。**例外**：行高在「非虚拟列表」里由内容撑开；**对象列表走虚拟列表，行高必须固定**（`uniform_list` 按第 0 行给所有行定高），所以行用 `.h(row_height())` 且必须同时设 `.line_height(row_line_height())`，二者一起保证定高与文字行盒恒等——只固定高度不固定行盒，字号缩放后会裁字或行间留缝。`radius_nested(inset)` = 外圆角 − 内缩（同心圆角规则），嵌套面紧贴时用它推导而不是复用 `radius()`。`theme.rs` 管颜色（六族语义 token，族纪律见上）与组件级 `Theme.radius`，`tokens.rs` 管自绘元素的尺寸 |
| 列表行分隔线 | 行用 `theme.table_row_border`（70% 透明度的发丝线），不要用 `theme.border` 全强度——后者会让整片列表呈网格/尺子感，与 Linear 基调冲突 |
| 列表选中底色 | 用 `theme.selection`，**不要用 `list_active` / `table_active`**：gpui-component 的 `apply_config` 强制把 list_active/table_active 的 alpha 压到 ≤0.2、selection 压到 ≤0.3（`theme/schema.rs` 末尾），而 list_active 源色太淡，压完叠在底色上只差约 2%，选中态肉眼不可见（实测渲染 249,249,252 对白底 255,255,255；换 selection 后 229,232,245）。`theme.rs` 的 `selection_tint_survives_library_alpha_clamp` 回归测试把这条钉住 |

## 6. 目录与数据

```text
~/Library/Application Support/<AppName>/   SQLite / settings / state
~/Library/Caches/<bundle-id>/              Thumbnail / Preview / 临时下载缓存
~/Library/Logs/<AppName>/  或 os_log       日志（Credential 必须 Redact）
/tmp/<app-name>/                           应用临时工作区，退出按策略清理
```

内存红线：大 Bucket 列表不整体驻留、大文件不进 `Vec<u8>`、上传下载全部 Streaming；Thumbnail LRU 上限（Entry 数 + 内存，如 128MB）。下载文件默认不可执行。

## 7. UX 硬标准

- Unified Titlebar（Traffic Lights + **窗口级导航**一体：侧栏开关 / 后退前进 / 当前位置），Sidebar（180/220/360px，折叠 44px Icon Rail，⌘⌥S），Content 主列表占据剩余空间；主界面不再显示额外详情列。Resize 实时 60 FPS。
  **对象区的操作不放标题栏**：上传 / 更多 / 过滤在表格**正上方**的工具栏里（`render_object_toolbar`，操作在左、搜索在右）。分工判据——作用于**窗口**的进 Titlebar，作用于**对象列表**的进内容区工具栏；标题栏属于窗口拖拽区，控件越多越容易和拖拽/双击缩放打架。
- 快捷键一律 Command 系（⌘K/⌘L/⌘F/⌘U/⌘R/⌘,/⌘[/⌘]/⌘A/⌘W/⌘Q）；UI 中只显示 `⌘ ⌥ ⌃ ⇧` 符号，不显示 "Cmd+Shift+P" 文字。
- **列表行一律「单击选择、双击打开」**（Finder 语义，判据为纯函数 `row_activation`，单测锁死）：对象行双击打开预览（先 `select_object_for_row_action` scope 再 `open_preview_overlay`），目录行双击进入（`open_prefix`）；单击只做选择（含 ⌘/⇧ 语义），文件名不是独立点击目标，行内重命名未收尾时不激活。Space 预览（再按 Space/Esc 关闭，方向键切换）；Return 进 Inline Rename（Finder 式，不弹 Dialog）；删除用 `⌘⌫` 且远端删除必须确认（`window.prompt`，无废纸篓）。命令面板/添加账号打开时 ⌘⌫ 不删对象。
- Selection：Click / ⌘Click / ⇧Click / ⌘A，完整 macOS 语义。
- Context Menu 顺序参考 Finder，Delete 放最底。Menu Bar：App/File/Edit/View/Object/Transfer/Window/Help；同一 Action 必须在 Menu / Context Menu / Toolbar / 快捷键 / Command Palette 共用。
- 外观默认跟随 System（监听变化），设置中可手动固定 Light/Dark；低饱和 Accent，自有视觉身份（图标不得拼接七牛+阿里云 Logo）。
- Retina 全适配；Trackpad 滚动平滑（虚拟列表不得丢惯性/跳跃）。
- **可访问性（与视觉同等的要求）**：每个鼠标可达的控件都要键盘可达——`track_focus` + `tab_index`/`tab_group`/`tab_stop` 排 Tab 序、焦点态有可见的 `focus_visible`，并补齐该控件类型的常规键（方向键、Home/End、Enter/Space、Esc）；**不得只靠颜色/悬停/动效表意**，状态色必须与图标或文字配对；命中区域宁可放大控件也不要把字形缩小。装饰性动画必须尊重 `reduce_motion`（`with_animation` 已内建，`request_animation_frame` 需自判）。
- **UI 设计基调（Linear 风格；语义 token 六族沿用 [OpenChamber](https://github.com/openchamber/openchamber) theme-system，已落地 `crates/ui/src/theme.rs`）**：surface（面）/ primary（主 CTA）/ interactive+selection（可交互与选中）/ status（**唯一允许外来色相的一族**，只用于真实反馈）/ 中性（border 与 text 同属中性族）/ inverse。铁律：UI 代码只用 `cx.theme()` 语义字段，禁止硬编码 hex/hsla；hover 只给可交互元素；**selection ≠ primary**；**中性面不得带外来色相**（三条纪律都有测试，见 §5「色板族纪律」行）。Linear 这套的主色为低饱和靛蓝（hue 233，Linear indigo `#5E6AD2` 系），中性面色族统一冷灰（hue 225）+ 发丝级边框，选中态为低饱和 indigo 淡染色；小圆角（Theme radius 5 / lg 8，自绘弹层卡片 rounded 8）。亮/暗两套（`CloudStorage Light/Dark`）经 `Theme::apply_config` 写入全局，`observe_window_appearance` 跟随系统切换。
- **视觉风格可切换（`ThemeStyle`）**：调色板有两套——`Linear`（如上）与 `Waku`（无彩中性灰 + 珊瑚 brand + 蓝色 selection + 单色主 CTA）。两套共用同一份视图代码与语义字段，切换只换颜色、不动布局。**当前处于对照评估阶段**：设置 → 外观 → 视觉风格可切换（持久化），原型期另有 ⌘⌥T 一键 A/B。定稿后应删除未采用的那一套（含 `CycleThemeStyle` Action 与 `handle_cycle_theme_style`），不要长期保留两条调色板路径。

## 8. 性能指标（必须用 Instruments 测量，不许猜）

- 空闲 RSS < 60 MB（上限 80 MB）；Idle CPU ≈ 0%，禁止定时 Poll UI / 持续 redraw / 持续 network polling。
- 冷启动 < 1s：先显示窗口 → 恢复本地状态 → 异步加载云数据。**绝不**等 ListBuckets 才显示主窗口。

## 9. 构建与 CI

- 主 Target：`aarch64-apple-darwin`；`cargo build --release`。
- CI 只跑 macOS ARM64：`cargo fmt --check` → `cargo clippy --all-targets` → `cargo test` → `cargo build --release` → App Bundle → Code Sign → Notarize。
- 发布：标准 `.app` Bundle（Info.plist + `.icns`）→ Developer ID 签名 → Notarize → Staple → DMG；Homebrew Cask 由 `daxiong123/homebrew-tap` 分发。初期不做 App Store Sandbox。
- 开发环境只保证 macOS + Apple Silicon，不为 Linux CI / Windows Developer 加 Workaround。

## 10. 工程纪律

1. **Fail Fast**：不写吞错误的兜底逻辑，错误必须暴露。
2. **Fix the Cause**：定位根因，不打补丁糊症状。
3. **Make It Observable**：关键路径留足日志/可观测性；信息不足就明说，不假装修好。
4. **Traceability**：关键节点可追溯。
5. **Living Documentation**：技术栈或产品方向变更时同步更新本文件与 `docs/spec/`。
6. **Don't Break Mainline**：大规模重构/实验前先切分支。

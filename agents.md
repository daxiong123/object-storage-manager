# agents.md — 项目操作契约

> **单一事实来源**：本文件是操作契约；平台细节以 `docs/spec/macos-platform-spec.md` 为准（冲突时规范优先）。
> 在本仓库动手前读完本文件；写 UI 前先查 `docs/notes/gpui-api-notes.md`，不凭记忆猜 API 签名。

## 0. 现状快照

| 项 | 值 |
|---|---|
| 版本 | `0.4.0`（`Cargo.toml` 的 `[workspace.package] version`，发版时一处改） |
| 平台 | macOS 14+ / Apple Silicon（`aarch64-apple-darwin`） |
| 语言与 UI | Rust 2024 + `gpui-pre`（声明 0.3.1，锁 0.3.4）+ `gpui-component` 0.6.1 + `gpui-kit-assets` 0.6.1 |
| 验证状态 | `cargo fmt --check` 干净、`cargo clippy --all-targets` 无告警、`cargo test --workspace` **235 passed / 2 ignored**（被忽略的是需真实凭证的七牛与腾讯云 COS 联网用例）；无 UI 交互自动化，交互靠人肉验收 |
| 已落地 | 账号与 Keychain、Bucket/对象浏览（前缀下钻/搜索/排序/分页/虚拟列表）、选择语义、上传下载删除重命名复制移动、传输引擎与系统事件、预览与 Quick Look、设置、命令面板、原生菜单 |
| 未落地 | 见 §10（含明确不做的项） |

文档地图：`README.md` 面向使用者与新人；本文件面向在本仓库工作的 Agent；`docs/spec/macos-platform-spec.md` 是 70 节完整规范；`docs/notes/*.md` 是在源码里核实过的 API 事实与踩坑记录。

## 1. 产品定位

**一款专为 macOS 设计的高性能七牛 Kodo / 阿里云 OSS / 腾讯云 COS 对象存储工作台。**

> Build the best Qiniu Kodo + Aliyun OSS + Tencent COS client for macOS, not the most portable one.

UX 标准：如果 Zed / ChatGPT 团队设计一个 OSS Browser，大概就应该是这个样子。不是 OSSBrowser 换皮，也不是 Web 云控制台塞进桌面客户端。

## 2. 平台

- **macOS Only**。不支持且不考虑 Windows / Linux / Web / iOS / Android。
- Apple Silicon 是 P0 目标（`aarch64-apple-darwin`）。Universal Binary 可选，非 P0。
- 最低 macOS 版本：**已定 macOS 14+**（`.cargo/config.toml` 中 `MACOSX_DEPLOYMENT_TARGET=14.0`；`gpui-pre` 0.3.x 对下限无更高要求）。
- **禁止**提出"为了以后支持 Windows/Linux 我们应该……"。除非用户主动改变产品目标。

## 3. 技术选型（硬约束）

优先级：`macOS Native Experience > 性能 > 低内存 > 开发效率 > 架构优雅 > 跨平台能力(权重 0)`。

固定栈（版本以 `Cargo.toml` 为准，此处只是快照）：

```text
Rust 2024 + gpui-pre 0.3.x + gpui-component 0.6.1   主 UI（Sidebar + Content Workspace / Virtual List / Command Palette / Keyboard-first）
Tokio                          异步运行时
reqwest + rustls               网络层（json / http2 / stream / multipart）
SQLite（rusqlite bundled）     持久化（永不存 Secret）
macOS Keychain                 所有 Secret（security-framework 3）
objc2 / objc2-app-kit / objc2-foundation / objc2-security   系统集成
hmac / sha1 / base64           服务商签名
indexmap / dirs / uuid / chrono / thiserror / anyhow   基础能力
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
    desktop/        macOS App 入口（二进制名 CloudStorage；菜单栏定义在主程序里）
    app/            Application Services（账号编排 / Provider 构建 / 传输任务落盘）
    domain/         Domain Models
    storage-core/   Provider abstraction
    provider-qiniu/ Qiniu Kodo
    provider-aliyun/ Aliyun OSS
    provider-tencent/ Tencent Cloud COS
    transfer/       Transfer Engine（队列/状态机/watch 事件驱动；runner 闭包由 UI 注入）
    persistence/    SQLite + settings.json
    macos/          macOS native integration（Keychain/系统事件 NSWorkspace+NWPathMonitor/QuickLook/Clipboard/关于面板）
    preview/        Preview（**占位 crate，尚无实现**）
    ui/             GPUI views/components
    common/         small shared utilities（**占位 crate，尚无实现**）
```

不要建 `platform/{macos,windows,linux}` 目录。

### 4.1 crates/ui 内部布局（对齐 waku 的 `src/app/*` + `src/ui/*`）

`workspace_view.rs` 曾是 8724 行单文件（占全仓 45%、含 208 个函数与 61 个测试），
任何改动都要在这一个文件里定位。现按 feature 拆分：

```text
crates/ui/src/
    workspace/          # 主窗口：按功能分文件（21 个 feature 模块 + tests）
        mod.rs          #   WorkspaceView 结构体 / 状态类型 / 常量 / new / render / render_body
        accounts.rs buckets.rs sidebar.rs titlebar.rs objects.rs
        object_list.rs selection.rs sort.rs download.rs upload.rs delete.rs
        rename.rs folder.rs copy_move.rs preview.rs menus.rs transfers.rs
        settings.rs quit.rs palette.rs format.rs
        tests.rs        #  67 个纯逻辑测试（ui 全 crate 共 100 个）
    ui/                 # 跨视图复用的**基础件**（不放业务视图）
        mod.rs          #  icon_button / compact_search_field（附带大段形状说明）
        overlay.rs      #  mask / surface / fade_in（浮层规范的可执行版本）
        motion.rs       #  动效时长与曲线常量
        menu.rs         #  popup / item / separator（锚定菜单形状）
    theme.rs tokens.rs actions.rs file_type.rs account_modal.rs settings_modal.rs
    command_palette.rs lib.rs
```

> 曾经的 `workspace/filter.rs`（本地过滤）已随「⌘F 改服务端前缀搜索」一并删除，见 §5.6。

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

> 每条都带**根因**：多数是被踩过之后修出来的，删掉理由就会有人再踩一次。
> 按主题分组，组内不排序。

### 5.1 账号、凭据与持久化

| 领域 | 决策 |
|---|---|
| Keychain key | `service = com.example.cloudstorage.credentials`（**占位 bundle id，定稿后统一替换**），`account = <account_uuid>`（不用账号名）；service 名集中在 `crates/macos/src/keychain.rs` 的 `KEYCHAIN_SERVICE`，只改一处；实现用 security-framework 3 的 generic password 三函数（`set/get/delete_generic_password`，get 返回 `Vec<u8>`，not-found 用 `err.code() == errSecItemNotFound` 归一化为正常分支） |
| 账号编排 | `AccountService`（crates/app）：Secret 只入 Keychain，元数据（含 AK，AK 非 Secret）只入 SQLite；一致性顺序 —— add 先 Keychain 后 SQLite（失败补偿删 Keychain，补偿再失败报复合错误不吞）；delete 先 SQLite 后 Keychain（幂等）；Keychain 条目缺失报 `MissingSecret` 不静默。本层无状态：`load_secret`/`build_provider(_with_secret)` 分离，Secret 可由调用方提供 |
| SK 会话缓存 | 钥匙串授权弹窗只在「选中账号后的第一次操作」出现：`AppServices.build_provider`（crates/app/src/services.rs）优先用单条会话缓存 `cached_secret`（最近使用账号的 SK，内存驻留、不落盘不进日志），未命中才现取钥匙串并写缓存；切换账号即置换淘汰。账号删除后缓存可能残留，但任何使用都因元数据缺失报 NotFound（不复活）。缓存锁与账号锁永不嵌套 |
| SQLite schema | `accounts` 表列固定为 `id/name/provider/access_key/created_at_millis`；`transfers` 表（⌘Q 暂停并退出）列为 `id/kind/account_id/bucket/object_key/dest/display_name/state/enqueued_at_millis`，kind/state 有 CHECK。两表均有「无 Secret 列」回归测试把守；provider/kind/state 用 CHECK 在 DB 层 Fail Fast |
| 本地路径 | `PathBuf`；Cloud Object Key：`String` + `/`。两者严格区分 |
| 设置（⌘,） | `Settings`（crates/persistence/src/settings.rs）存 `settings.json`（Application Support/CloudStorage/，spec §58；永不存 Secret），损坏显式报错不静默重置（Fail Fast）；新增字段必须 serde default，旧配置缺字段正常补默认（`load_old_settings_file_fills_new_defaults` 用已删除字段 `theme_style` 把这条兜底变成可证伪的）。可配：签名链接 TTL、复制后清剪贴板秒数（0=关闭）、外观模式（System/Light/Dark）、界面字体族/字号缩放、代码字体族/字号、传输并发数（1..8）、默认下载目录（单/批量下载交互一致：设置了有效默认目录先弹确认 sheet「使用默认目录/另存为…或另选目录/取消」；单文件另存为面板以默认目录为初始目录；不自动回写）、**上传大小上限（MB，0 = 不限制，可设上限 5119 MB——严格小于 5 GB，因为 5 GB 是 COS 简单上传的硬顶，不是用户可调项）**。运行时值在 `WorkspaceView.settings`，改动经 `SettingsModal`（自建 overlay，同 AddAccountModal 机制）保存后即时生效；`copy_object_url_request` 的 TTL 是运行时参数，禁止退回编译期常量。Transfer 列表：进度条 + 百分比 + 字节；失败原因完整换行展示（不 truncate） |
| 上传大小上限的作用面 | **只管本地→云的上传**：⌘U 上传文件、上传文件夹、Finder 拖放、⌘S 保存编辑后的文本。**不拦云端复制/移动/重命名**——它们在本 App 里虽是「下载到临时文件再上传」，但用户心智是「搬运已有对象」而非「上传」，按上限拒掉会像 bug。判据是纯函数 `upload::upload_exceeds_cap`（`cap_mb == 0` = 不限制；`size == cap` **放行**，上限是闭区间上界），四处入口共用，超限**逐文件跳过后在状态条点名**（最多 3 个），不整批拒绝。目录上传的大小在 `walk_folder` 后台递归时随 `FolderUploadFile::size` 采集，避免为了判大小回 UI 线程再 stat 一遍。COS 自己的 5 GB 硬顶（`MAX_SIMPLE_UPLOAD_BYTES`）与用户设置无关，保留作最后一道防线 |

### 5.2 Provider 与网络

| 领域 | 决策 |
|---|---|
| Provider trait | `StorageProvider`（`crates/storage-core`）：方法返回 `impl Future + Send`（不用裸 `async fn`，Send 义务显式化，否则无法 spawn 到 tokio/gpui 后台执行器）；非 dyn-safe，上层按服务商 enum 分发 |
| 七牛签名 | V2 请求签名逐字节核对自官方 SDK 源码并内置官方向量测试（V1 hello/world + V2 X-Qiniu-* 规范化排序）；坑：Base64 必须带 padding、签名用实际发送的原始 query 串、X-Qiniu-* 头名规范化为 Title-Case 后排序、putTime 单位 100ns。详见 `docs/notes/qiniu-api-notes.md`，勿凭记忆重写 |
| 阿里云签名 | V1 签名，逐项核对官方文档并内置测试。坑：对象请求走 virtual-hosted 三级域名（`{bucket}.{location}.aliyuncs.com`，只有本地 mock 用 path-style）；**同时发送 `Date` 与 `x-oss-date`（同一 GMT 串）**——StringToSign 的 Date 行填该时间，并把 `x-oss-date` 列入 CanonicalizedOSSHeaders，Date 留空会 `SignatureDoesNotMatch`；ListObjects 的 CanonicalizedResource 是 `/{bucket}/`（服务端 StringToSign 只认这个）。详见 `docs/notes/aliyun-api-notes.md` |
| 腾讯云 COS 签名 | 签名 v5（`q-sign-algorithm=sha1`），逐字节核对官方文档 + 五个官方 SDK 并内置三条可复现向量（含 Go SDK 的端到端向量）。**腾讯云没有官方 Rust SDK，只能照规范实现**。三个静默签错的坑：① `SignKey` 的**十六进制字符串当文本**用作下一层 HMAC 的 key（不是原始字节）；② `HttpString` 里的 `UriPathname` 必须是**解码后**的 UTF-8 路径（用线上百分号编码形式会得到一个服务端永远算不出的签名）；③ `HttpString` 用 LF 且**结尾换行必须有**，空分量保留空行。只签 `host`（上传加 `content-type`），不签 `date`/`content-length`。详见 `docs/notes/tencent-cos-api-notes.md` |
| 腾讯云 COS 端点与地域 | 列举空间走全局 `https://service.cos.myqcloud.com`（**一次拿到全部地域的桶且带 `<Location>`**，但**它也分页**，容易漏）；对象操作走 `{bucket}.cos.{region}.myqcloud.com`。Bucket 标识是 `<名称>-<APPID>`（`examplebucket-1250000000`），**只写名称会得到 DNS 失败或 404、完全看不出原因**，故联网前用 `bucket_name_error` 挡住。地域缺失时回退查一次全局入口并缓存，仍拿不到就报错——**不蒙默认地域**（地域错了只会回 `SignatureDoesNotMatch`，无从判断）。**地域会话缓存在 AppServices**（`(account_id, bucket) → region`，`list_buckets`/`list_objects` 成功后写入、`build_provider` 回填进新 Tencent 实例）——provider 实例内的缓存活不过单次操作，没有这层的话每次下载/上传都要查一次 service 端点，无 `cos:GetService` 权限的账号（手填 Bucket）会永远卡死；手填空间时 UI 可一并输入地域。限流是 **503 `SlowDown` 而非 429**（与阿里云的映射不同，不能照抄）；错误响应带 `RequestId`，签名类错误必须把它带进报错文案 |
| 腾讯云 COS 上传 | 简单上传（`PUT /{key}`）**上限 5 GB**，超过即在联网前 Fail Fast 报错（**分块上传明确不做**，与七牛断点续传同一决定）；支持 `Transfer-Encoding: chunked`，故流式上传不预先声明 `Content-Length` |
| 七牛区域上传 | 上传 host 按 bucket 经 UC `GET /v4/query?ak=&bucket=` 解析（**公开接口无 Authorization**；官方 Rust SDK `BucketRegionsQueryer` 同构），取 `hosts[0].up.domains[0]`，进程内缓存（host 级 ttl，缺省 86400），失败回退 `upload.qiniup.com`。测试模式（UC 指向 127.0.0.1）直接用注入 up_base，不做真实解析。**断点续传明确不做**（用户决定） |
| 七牛目录占位对象 | key 以 `/` 结尾的占位对象（size=0、mimeType `application/qiniu-object-manager`）不是文件——下载/预览/签名 URL 必 404，目录语义的唯一载体是 `CommonPrefix`；因此在 entries 数据填充点**单点过滤**掉（单一真相源），不在各交互入口打拦截补丁。腾讯云 COS 在控制台建目录同样产生 `<前缀>/` 的空对象，同一条判据覆盖 |
| 新增一家服务商要改的地方 | 判据是「编译器会拦住多少」。`ProviderKind`（domain）加变体后，三处 `match` + `BuiltProvider` 的 7 个臂 + `build_provider_with_secret` 的构造臂会**编译失败**，跟着改即可；`Sidebar 图标` / `URL scheme` / 账号弹层分段控件 / 下载失败排查文案 / 命令面板关键词是**不会编译失败**的几处，必须自己想起来。`accounts.provider` 的 CHECK 约束放宽需要**整表重建迁移**（SQLite 不能 ALTER CHECK），范式抄 `transfers.rs` 的 `migrate_transfers_allow_upload`，并配一条「用真的旧 schema 建库再走 open()」的迁移测试——直接测 `open_in_memory()` 得到的是新 schema，迁移分支根本不会执行 |

### 5.3 传输引擎与生命周期

| 领域 | 决策 |
|---|---|
| Transfer | Sleep/Wake/断网后状态为 `Waiting/Paused` 并恢复，**不得**误标 `Failed`（P0）；事件驱动，不轮询。已落地：`crates/transfer` 引擎（队列/状态机/并发上限）+ 单测锁死 P0 语义；任务执行体由 UI 层注入 `TaskRunner` 闭包（内调 `AppServices::build_provider` 即锁即放），future spawn 到 AppServices 的 tokio 运行时，暂停/挂起/取消 = `JoinHandle::abort()`（future 在 await 点丢弃即断 reqwest 连接）；attempt 代号丢弃过期完成回调；UI 经 `watch` 令牌订阅快照（无定时器）；字节进度经 `ProgressSink` 回写，通知节流 100ms，禁止按块 redraw。系统事件已接线（`crates/macos/src/system_events.rs`）：睡眠 `NSWorkspaceWillSleepNotification` → `suspend_all`；网络 `NWPathMonitor`（C API + block crate，私有 dispatch 队列）非 satisfied → `suspend_all`、satisfied → `resume_all`；**didWake 故意不恢复**——唤醒瞬间网络常未就绪，等 satisfied 事件再 resume，否则重排队任务会变 `Failed`（P0）。监视器在 `WorkspaceView::new` 启动（单窗口一次性创建约束）。上传已落地：⌘U 多选文件；「上传文件夹…」递归入队（key=当前前缀+目录名+相对路径，`/` 分隔；跳过 .DS_Store/`._*`/符号链接/空目录）。七牛表单直传（token 在 multipart，分块读盘不进 `Vec<u8>`）。Finder 拖放：对象浏览区 `on_drop::<ExternalPaths>`（文件立即入队，目录后台递归）；拖入提示照参照实现是**灰色虚线框、不加底色**（`drag_over` 里 `border_dashed()` + `theme.drag_border`，实测参照实现为 #757575 虚线；**不要**再加 `bg(drop_target)` 或彩色实线）。截图里跟随光标的橙色文件名胶囊**不是应用画的**——那是 Chromium 原生拖拽预览（其包里没有该橙色），macOS 下系统会自己画拖拽图，所以应用侧只有那圈虚线。区域上传域名已落地，断点续传/拖出到 Finder 见 §10 |
| ⌘Q vs ⌘W | ⌘W 只关窗口（**必须先禁用 close 动画再 remove_window**，见下行）；⌘Q 有 Transfer 时弹确认，默认 `Pause + Persist` |
| 关窗口实现 | macOS 15 close 动画会被 gpui 立即 teardown 杀死 → 窗口卡死可见。`handle_close_window`：`setAnimationBehavior: None` + `remove_window()`；失败方案与机制详见 `docs/notes/gpui-api-notes.md`「关闭窗口」章节，勿重复试错 |
| Quit 处理 | 全局 `cx.on_action` 在 **bubble 末尾**（源码 `app.rs:1696`，不是 capture）：有窗口时 `WorkspaceView::handle_quit` 先处理，窗口全关后仍可 ⌘Q。有活动传输时走 gpui `window.prompt`（NSAlert sheet + oneshot，**禁止 runModal**）三按钮：暂停并退出（默认 Return）/ 取消（Esc）/ 立即退出；暂停并退出把活动任务写入 SQLite `transfers` 表（无 Secret 列）后 `cx.quit()`，下次启动 `take_transfers` 入队恢复（paused 保持暂停，其余自动继续）；立即退出 `clear_transfers` 后退出；落盘失败 Fail Fast 不退出 |

### 5.4 macOS 系统能力

| 领域 | 决策 |
|---|---|
| 文件选择 | 只用 gpui 平台 API：`cx.prompt_for_new_path`（保存）/ `cx.prompt_for_paths`（打开），结果经 oneshot 异步回传；**禁止在事件处理器里同步 `runModal`**——模态循环重入 gpui `App` RefCell 借用 → "RefCell already borrowed" 闪退（详见 docs/notes/gpui-api-notes.md「文件对话框」；crates/macos 不再封装面板，panel.rs 已删） |
| 剪贴板 | `NSPasteboard`（`copy_text` / `read_text` / `clear_if_equals`）；Signed URL 可配置 N 秒自动清除（0=关闭） |
| Open With / Show in Finder | ⌘O / 对象菜单入口：`ensure_local_copy` 复用 `preview_path`（判据 `cached_copy_matches` 纯函数：文件名 = `{nanos}-{display_name}` 后缀匹配且非全等，单测锁死），无副本先下载到临时目录（与预览同缓存位置）→ `object_storage_macos::open_with_default_app`（NSWorkspace）/ gpui `cx.reveal_path`（spec §14/§16） |
| 预览 | 常见格式应用内；PDF/Office/视频走系统 Quick Look（`object_storage_macos::quick_look`），不自建 Preview Engine。图片等比完整显示：**不要**用 `img(..).size_full().object_fit(Contain)`——布局阶段按自然尺寸推导会撑出容器被 `overflow_hidden` 裁切，可靠写法是外层 flex 居中 + `overflow_hidden`，img 改 `max_w_full().max_h_full()`，Contain 仅作 paint 兜底。文本用 GPUI Kit `EditorState` 查看与编辑，⌘S 保存并上传（dirty 约束：内容与原文一致时按钮禁用，且 ⌘S 入口在保存函数内加同一检查，快捷键不得绕过按钮语义；编辑器内容变化须 `cx.subscribe_in(.., InputEvent::Change)` + `cx.notify()` 驱动禁用态刷新）。**哪些文件进应用内文本预览由 `preview.rs` 的两张表决定**（`TEXT_EXTENSIONS` 扩展名 → 语言名、`TEXT_FILE_NAMES` 无扩展名/点开头按名识别，如 `Dockerfile`/`Makefile`/`README`/`.env`）：白名单与语言映射是同一张表，不会两半各改一次；语言名在已启用的 grammar 里找不到时高亮自动回落纯文本（`xml` 即此类，只有显示名）。判据只看 key 的最后一段，点开头的名字要连点一起写。**超过 2 MiB 的文本对象不报错**：不读进内存，浮层改成超限提示 + 复用「系统预览」按钮（`can_open_system` 同时认 `preview_oversized`） |
| 「关于」用系统面板 | 走 `NSApplication.orderFrontStandardAboutPanelWithOptions:`（`crates/macos/src/about.rs` 的 `show_about_panel`，`OpenAbout` Action 入口不变）。**不要**再自建 About 浮层：系统已提供的能力不自建（§3），而且原生面板自动跟随系统语言/外观/无障碍设置。对齐参照实现——oss-browser2 的「关于」就是 Electron 的 `role: 'about'`（主菜单里只有这一项，两份 bundle 里都没有自建对话框）。用 **WithOptions** 而非无参版本：无参版读 bundle 的 Info.plist，开发期裸二进制没有它，会显示可执行文件名与空版本号 |

### 5.5 UI 架构与线程模型

| 领域 | 决策 |
|---|---|
| AppServices 线程模型 | `AccountService` 内含 rusqlite `Connection`（内部 RefCell，非 Sync），直接 `Arc<AppServices>` 进不了 gpui 后台任务（要求 Send+Sync）。连接统一收进 `Mutex<AccountService>`（crates/app/src/services.rs），同一时刻至多一个后台线程用数据库/钥匙串；锁毒化（持锁线程 panic）直接 panic 响报，不静默 |
| UI 异步编排 | UI 层永不直接调 provider/runtime：一律 `cx.spawn` → `background_executor().spawn` 调 AppServices 阻塞方法（内部 `runtime.block_on`）。并发竞态用代数计数（`bucket_gen`/`object_gen`/`preview_gen`）丢弃过期结果（last-click-wins）；添加账号模态用 `done/closed` 标志 + `observe_in` 由 WorkspaceView 回收，保存中禁止关闭（防丢成功结果）。GPUI 焦点时序：**不要在 `on_mouse_down` 里立即 `window.focus()`**——会被同一次事件的后续处理覆盖；打开弹层只置 needs-focus 标记，等元素渲染挂载后（render 内）再 focus |
| Action 注册点 | 所有跨 菜单/快捷键/右键菜单/工具栏 共用的 Action **只**定义在 `crates/ui/src/actions.rs`（`actions!(cloud_storage, …)`），键位在 `bind_keys(cx)` 统一绑定，不得散落各 view |
| 全局键位边界 | 不绑定 ⌘X/⌘C/⌘V/⌘A 全局快捷键（会吞文本输入的原生响应链）；Edit 菜单走 `MenuItem::os_action` 触发系统行为。⌘A 全选走 **Workspace context 绑定**（`SelectObjectAll`），命令面板/输入框聚焦时由组件原生响应链处理 |
| 命令面板 | GPUI Kit 0.6.1 `Command` + `CommandState` 负责过滤、虚拟列表、键盘导航、滚动和无障碍语义；应用层只维护命令条目、共享 Action 编排与动态 Bucket Handler。不要恢复手写列表/方向键 Action。`command_palette::tests::native_command_filters_and_confirms_dynamic_items` 用新版 headless test-support 锁死过滤 + 动态确认链路 |
| Sidebar | **自建视图**，不用 gpui-component `Sidebar`（组件固定 255px/48px，与规范 180/220/360 + 44px rail 冲突）；可拖拽宽度用 gpui-component `resizable`，按布局变体用不同 group id 保持各自记忆宽度。主界面只保留两列结构，不再提供额外详情列。底部有**常驻入口**（传输 + 设置，在滚动区**之外**，否则会被账号/空间列表顶走）：传输项带进行中数量角标，点它展开标题栏的传输面板；两者都是 `sidebar_footer_entry` 这一形状 |
| 对象列表虚拟化 | 列表用 `gpui::uniform_list`（**不是** gpui-component 的 `List`/`ListState`——后者的 `ListDelegate` 自带选中/搜索语义，会与 `apply_object_selection` 那套被 61 个测试钉死的 Finder 选择语义打架）。只渲染可见区间，行数不再影响每帧成本。三处硬约束：①**行高固定**（见 §5.7「设计 token」），行不能再由内容撑开；②行渲染闭包经 `cx.processor` 拿到 `&mut self`，但它每被调用一次就重排一遍是 O(n log n)，所以**显示顺序每帧只算一次**存在 `display_order: Vec<usize>`（存下标而非克隆条目，过期只会表现为下标越界，由取用处 `get()` 兜住）；③**键盘导航要把选中行滚进视野**——虚拟列表里「选中了但看不见」等于没反馈，滚动用 `display_slot_of_key` 求槽位（目录前缀也占槽位，不能拿只含对象的 keys 下标替代） |
| UI 基础件（`crates/ui/src/ui/`） | 只放**跨视图复用的基础件**，不放业务视图。判据：两个以上视图重复、且能收成**具名形状**的才进来——`icon_button`（紧凑 ghost 图标按钮的唯一形状：图标 + 幽灵底 + Small + tooltip；工具栏/图标栏/各浮层关闭按钮共用）、`menu::popup`/`menu::item`/`menu::separator`（锚定菜单卡片与条目）、`overlay::mask`/`surface`/`fade_in`、`motion`（动效常量）。**不要**收通用抽象（`row(..)`/`card(..)`）——参数一多就不比各视图显式写链式调用清楚 |
| 动效与 reduce_motion | 时长与曲线只在 `crates/ui/src/ui/motion.rs` 定义，不在调用点写 `180`/`cubic_bezier(...)`。**走 `AnimationExt::with_animation` 的动效不需要自己判 `App::reduce_motion()`**——gpui 已内建（开启时渲染终态且不排帧，见 `gpui-pre/src/elements/animation.rs:407`）；只有装饰性的 `request_animation_frame` 才需要显式判 |
| 文件类型图标 | `crates/ui/src/file_type.rs`：按 Object Key 扩展名选 Lucide SVG（素材 `crates/ui/assets/icons/`，Lucide 官方源码 stroke=currentColor，与 gpui-component 图标同规格；只用 Lucide 一家）。纯函数 `file_icon_kind`（扩展名 → 9 类分组，单测锁死；无扩展名/未知 = Generic）。自有 SVG 用 `include_bytes!` + GPUI Kit 0.6.1 `Icon::data` 直接渲染，不再维护组合 AssetSource/路径注册表；通用图标仍由 `gpui-kit-assets` 提供。着色只用 theme 语义 token（muted/accent），不硬编码色值 |
| render 纯净性（性能红线） | `render_*` 及其可达路径**不得**做文件系统、网络、子进程、阻塞锁或同步 IPC：一帧的成本只应与「屏上内容」成正比。需要 IO 就丢 `AppServices` 后台执行器、把结果存到实体上再 `cx.notify()`；未命中即「还不知道」，要优雅降级而不是当场去取（waku 把 render 里可达的 IO 称为 defect，即便它看起来便宜、被缓存或只对少数行发生） |

### 5.6 内容区与交互语义

| 领域 | 决策 |
|---|---|
| 对象多选 | 三个状态：`selected_object_keys: IndexSet<String>`（**勾选集合**，有序）+ `selection_anchor`（⇧ 范围起点）+ `selected_object_key`（**当前行**）。**勾选集合只能由复选框、⌘Click、⇧Click、⌘A 改变**：不带动词的点击/方向键只移动「当前行」，不动集合——点行看一眼不该顺手把复选框勾上（曾经会替换集合，于是「点几行看一眼再按删除」把它们一起删了）。判据是 `ObjectSelectionIntent::plain_click()`（= `!command && !shift && !select_all`），纯函数与调用方共用它，「未加修饰键」只有一处定义。行高亮两者都算（`checked \|\| active`），复选框只认勾选集合。**行级按钮**（行末 ⋯、行内下载）把作用域收敛到本行 = 清空集合 + 设当前行（`select_object_for_row_action`），**不勾复选框**；读取「当前对象」的路径都回落主选（`selected_cloud_object()`，集合为空时 `selected_object_keys_vec()` 也回落）。语义决策在纯函数 `apply_object_selection`（workspace/selection.rs，单测锁死）；⇧Click 分支先于 ⌘Click（⌘⇧=增量范围）。批量删除：确认一次（标题报数量，明细列前 3 个名）→ 后台逐项删，失败逐项可见不中断；批量下载（多选≥2）：`prompt_for_paths(directories:true, multiple:false)` 选目标目录 → 逐项入队 |
| 行激活 | **列表行一律「单击选择、双击打开」**（Finder 语义，判据为纯函数 `row_activation`，单测锁死）：对象行双击打开预览（先 `select_object_for_row_action` scope 再 `open_preview_overlay`），目录行双击进入（`open_prefix`）；单击只做选择（含 ⌘/⇧ 语义），文件名不是独立点击目标，重命名弹层开着时不激活 |
| ⌘F 前缀搜索（原「⌘F 过滤」已废弃） | 工具栏右端那个框是**服务端前缀搜索**（对齐 oss-browser2 的「文件前缀搜索」）：回车或点 🔍 **显式提交**（不是边打边筛），提交时把输入值经 `normalize_prefix_query` 规范化后作为 `ListObjectsRequest::prefix` **重新列举**——所以能查到还没加载出来的对象，本地过滤做不到。⌘F 只负责聚焦该框，Esc 清空（框是常驻控件，没有隐藏态）。**已删除本地过滤**（`filter_entries` / `filtered_ix` / `ToggleObjectFilter` / `workspace/filter.rs`）：旧的「本地过滤 + 选择集」组合有过安全隐患——⌘⌫/批量下载取选择**全集**且不含可见性判断，过滤后按 ⌘⌫ 会删掉「看不见的对象」（实测复现过：显示 5 行、实删 45 个）。现在改由**重新列举**代替：`reload_objects` 会清空 entries 与选择，那条隐患从根上消失（切桶/下钻/改每页条数同此纪律）。跳转 Bucket：带数据 Action `SelectBucketByName(String)`（`#[action(no_json)]`）保留作菜单/快捷键入口；**命令面板动态命令例外**——经 `WeakEntity<WorkspaceView>` 直调 `select_bucket`，避免面板关闭后向失效焦点 deferred 派发 Action |
| 重命名弹层（`rename.rs`） | Return 打开**弹层**（照参照实现），仅单选；多选不弹、只在状态条报错。弹层内容：标题 + ✕ / 「原路径：」原文件名 / 「重命名：」预填新名的 Input / 目录影响说明 / 底部「取消 + 确认修改」。**校验即时反映在界面上**：`rename_validation_message` 不合法 → 输入框下方红字 + 确认置灰；名称未变（`RENAME_UNCHANGED`）→ 只置灰不报错（用户可能只是打开看一眼）。进行中（`renaming_busy`）禁止关闭（✕/取消/遮罩/Esc 全拒）——后台任务完成时要回写弹层；失败保持弹层打开、错误就地显示（不再丢进底部状态条）。Esc 走 context "Renaming" → `DismissRename`（Input **不设 clean_on_escape**，否则 Esc 被输入框吃掉冒泡不出来）。提交：`rename_target_key` 只改最后一段（含 `/`、`.`、`..` 拒绝，单测锁死）→ `AppServices::rename_object`（下载临时文件→上传新 key→删旧 key；上传失败旧对象保留，删旧失败报复合错误不静默）。**卡片外观在 `pub fn rename_dialog`**（与业务解耦，供离屏预览验收） |
| 复制/移动同名冲突（`copy_move.rs`） | 目标目录已有同名对象时**自动改名后执行**（`name (1).ext` 顺延，批内占名继续顺延），不报错、不覆盖——旧实现只在「浏览到目标目录」时校验同名，手动输入路径会**静默覆盖**远端对象（数据安全隐患，已修）。复制到当前目录（目标 == 源）不再报错，按改名处理＝复制一份语义。检测在 commit 时后台按 `target_prefix + 文件名主干` 平铺列举（`collect_existing_target_keys`，含翻页；源对象若在目标目录内必被收进 existing，因此改名候选永不撞上本批另一源）；列举失败 Fail Fast 整批不执行。纯函数 `resolve_copy_move_name_conflicts` 单测锁死 |
| 排序（`sort.rs`） | **目录恒排在对象前**（Finder 语义，参照实现亦然），组内才谈「原序 / 名称 / 大小 / 时间」。`Natural`（默认）也走这一步分组，只是组内保持 provider 返回顺序——它曾经直接 `return ix`，于是默认视图的顺序完全由 provider 决定（OSS 把对象排在 `CommonPrefix` 之前 → 目录被推到整张表最后），与函数自己的注释和 Finder 行为都矛盾。见 `sort_entries_natural_keeps_listing_order_within_groups_but_puts_dirs_first` |
| 对象列表的列与行内动作 | 列＝勾选 / 名称 / 大小 / 最新修改时间 / **操作**。**表头必须给每一列同样的固定宽度占位**（含末尾「操作」列，`tokens::col_action_width`）——表头少一列时名称列（`flex_1`）会多吸收那部分宽度，后面几列整体右移（实测曾偏 55px）。行内动作＝**下载 + 更多**（参照实现是 ☆ ↻ ⤓ ⋯，我们只做有实际动作的两个）：行内下载先 `select_object_for_row_action` **把选择收敛到本行**再下载，所以未选中任何行时点它也是按本行来；其余动作留在 ⋯ 菜单（与右键菜单共用同一个菜单实体） |
| 工具栏 | 左＝上传（主色 + ▾：上传文件… / 上传文件夹…）· 新建目录 · 下载 · 更多（▾）；右＝搜索（紧凑控件）+ 刷新。下拉菜单的状态是**单个枚举** `toolbar_menu: Option<ToolbarMenu>`（Upload/More），所以不可能两个菜单同时开着；锚定一律用触发点的窗口坐标（`position_mode(Window)` + `position`）。「上传文件夹」已从「更多」移到这里，不再两处重复 |
| 标题栏与地址栏 | 标题栏＝应用图标 + 名称 + 侧栏开关（左）、**聚合传输进度条 + 百分比 + 摘要**（右，仅进行中且有已知总大小时出现；下载的 Content-Length 可能未知故按已知项聚合）。地址栏行＝后退/前进 + 面包屑：`<scheme>://`（`oss`/`kodo`/`cos`，次要色、不可点）→ bucket → 可点前缀段（长路径折叠成 `…`）。**不要**把 scheme 也做成可点目标，它不是导航目标 |
| 紧凑搜索控件（`ui::compact_search_field`） | 输入框 + **接合**在右侧的放大镜按钮，形状照参照实现逐像素量出：两段**无间隙**，接缝那条竖线只由输入框自己的右边框提供（输入框右角方角、按钮左角方圆、按钮**不画左边框**）；未聚焦时两段边框同为 `theme.input`，聚焦时输入框整圈变蓝（`Input` 按 `theme.ring` 画）——**不要覆写 `Input` 的 border_color**，那会把聚焦色一起盖掉。按钮分两层：外壳 div 画外观，里层 ghost `Button` 管交互。原因是库的两个 API 都是 `pub(crate)`：`Button::border_edges` 拿不掉单条边、`ManagedTooltipExt` 让纯自绘 div 挂不上 tooltip，两层各取所需。该控件单独导出（`lib.rs` 的 `pub use`）以便离屏预览渲染生产代码本身。**宽度由调用方给、控件自己 `w_full` 铺满**：输入框是 `flex_1`，作为内容尺寸 flex 行的子项会塌成一条缝（工具栏踩过），预览也必须照抄调用点的父链才验得出来 |
| 弹层内分段控件（服务商选择） | 「添加账号」的服务商用库自带 `ButtonGroup` + 每段 `Button::selected(bool)`，选中底走 `ButtonVariant::Custom` 的 **`active`** 色 = `sidebar_accent`（`ButtonGroup` 的选中样式取 variant 的 `active`，所以 accent 必须放 `active` 而非 `color`），选中项另加对勾图标 + `sidebar_accent_foreground` 文字色（不得只靠颜色表意）。两段等宽 `w(tokens::text(120.))` 且**等宽是必须的**——不然加了对勾会把另一段挤动。**禁止退回「选中 Secondary + 非选中 ghost」**：Secondary 的底 ≈ 面板底色，与 ghost 只差约 1%，选中态等于看不见（用户报过这个问题）；`theme.rs` 的 `selection_tint_is_visible_against_the_surface` 把这条钉住 |
| 点击空白清空选择（B6，已落地并实测通过） | 清空处理器挂在**对象列表滚动容器自己**身上（`render_object_list` 的 `on_mouse_down`；列表改虚拟化后挂点仍是滚动容器——`UniformList` 自己实现了 `InteractiveElement`，挂不到它的祖先上去），行处理器一律 `cx.stop_propagation()`——能冒泡到容器的一定没命中行，判据不需要几何、也不依赖注册顺序。**不要**回到前几版方案：内容区容器的 `capture_any_mouse_down` 的 `is_hovered` 只统计 `BlockMouseExceptScroll` hitbox 链（非滚动容器收不到事件）；canvas 几何命中版要每帧重建全部行 bounds + 每行挂一个 canvas，为一个交互付 O(行) 代价。注：行处理器 `stop_propagation` 会跳过 WorkspaceView 根节点的菜单兜底关闭，行内已由 `handle_object_row_click` 自己关菜单 |
| B6 复验要点（踩过的两个坑） | ①**空白区只在列表没铺满视口时才存在**：行数 ×行高（`tokens::row_height()`）≥ 视口高度时最后一行下方没有空白，点到的是被裁切的行（被 `stop_propagation` 正确挡住，表现为「点了没反应」——这不是 bug，是测试选错了对象，换对象少的桶或把窗口拉高再看）。②**选中底色要按当前外观模式取色**：亮色 (229,232,245)、暗色约 (26,30,44)，系统会在傍晚自动切暗色，按亮色值去找会误判成「没选中」 |
| 标题栏内的交互控件必须保留 `on_mouse_down(stop_propagation)` | 统一标题栏（gpui-component `TitleBar`）在 macOS 上把**双击**当窗口缩放（`on_double_click → window.titlebar_double_click()`）。**拦住它靠的是 mouse_down 的 stop，不是 click 的 stop**：标题栏要触发双击，必须先在**它自己的** mouse_down（bubble 阶段）里置位，才能在 mouse_up 时合成 click；控件在 mouse_down 就 `stop_propagation()` 之后，标题栏那一步永远不会发生。所以**不要**再额外加 `.on_click(stop)`——我一度以为要在过滤框上加它来修「双击选词缩放窗口」，并写了测试；结果测试的**对照组**（去掉 mouse_down 的 stop）显示 click 本来就会漏到标题栏，说明护栏早已生效、那个猜想是错的。两条断言留在 `workspace/titlebar.rs` 的测试里（`click_reaches_the_titlebar_when_the_mouse_down_stop_is_removed` 为对照组），谁误删这条 stop 就会红。**教训**：给「祖先会不会收到事件」下结论前，先写一个能失败的对照组，否则会写出「永远通过」的测试 |
| 弹层内滚动容器 | 滚动容器必须显式 `h_full() + min_h_0()`：flex 子元素默认 `min-height:auto`（=内容高度），容器高度会等于内容高度 → 越过父层底部叠压后续兄弟元素（如状态条），且无溢出 → 滚动条失效；两个症状同根。列表类滚动容器的**常驻元素（表头、状态条）必须放在滚动容器之外**，否则条数少时上浮、条数多时被卷走 |

### 5.7 视觉与设计 token

| 领域 | 决策 |
|---|---|
| 色板取值（参照 oss-browser2） | **只有一套调色板**：`light_palette()`/`dark_palette()`（`crates/ui/src/theme.rs`）逐字段取自参照实现的运行实例与 CSS 变量（Ant Design v4 系）——primary `#0064c8`（暗色 `#1668dc`）、控件边框 `#d9d9d9`（`theme.input` 是**边框**色，不是填充色，复选框「没有边框」就是踩了这个）、内容/表格分隔线 `#f0f0f0`、模态遮罩 `rgba(0,0,0,0.45)`、侧栏与内容**同色**纯白（实测，只靠极淡界线分开）。曾存在过的 `ThemeStyle` 视觉风格切换（Linear / waku 双调色板 + 设置项 + ⌘⌥T）**已删除**：每多一套色板，每个视觉决定都要做两遍且没有产品理由支撑。**不要再加回第二套调色板**；要改观感就改这一套的取值，并同步 `theme.rs` 的纪律测试 |
| 色板族纪律（`crates/ui/src/theme.rs`） | 六族：surface / primary / interactive+selection / **status**（唯一允许外来色相的一族）/ 中性面（border+text 与之同族）/ inverse。规则不是审美口号，由测试把守：`neutral_families_carry_no_hue`、`every_colored_field_matches_a_declared_anchor`（带色相的字段必须对得上 accent/selection/状态色之一）、`status_is_the_only_foreign_hue_family`、`grays_and_colors_are_clearly_separated`、`status_hues_are_mutually_distinguishable`，且对**亮/暗两套**都生效。判「是不是灰」用**绝对彩度 ΔRGB**，**不要**用 HSL 的 `s`——接近白/黑时 s 会失真（`rgb(0xF6F5F6)` 只差 1/255 却算出 s=0.053、色相 300°） |
| 设计 token（`crates/ui/src/tokens.rs`） | 字号阶梯（`caption` 11 / `label` 12 / `body` 12 / `heading` 14 / `title` 14 / `display` 20 / `icon_sm` 28 / `icon_lg` 42，均为 100% 字号下的设计值）、圆角（`radius` 2 / `radius_lg` 4 / `radius_icon` 14 / `radius_nested`）、行距（`row_pad_y` / `row_pad_y_sidebar` / `row_pad_y_transfer`）、**对象列表行高**（`row_height` / `row_line_height`）与**随字号缩放**的列宽（`col_size_width` / `col_time_width` / `col_check_width` / `col_action_width`）都只在这里定义。新增文字/圆角/行距先选现有档位，不要新增第 9 个数字；任何跟随字号的尺寸都必须经 `text()`，固定 `px()` 在 140% 档位会与文字错位（列宽曾如此）。**例外**：行高在「非虚拟列表」里由内容撑开；**对象列表走虚拟列表，行高必须固定**（`uniform_list` 按第 0 行给所有行定高），所以行用 `.h(row_height())` 且必须同时设 `.line_height(row_line_height())`，二者一起保证定高与文字行盒恒等——只固定高度不固定行盒，字号缩放后会裁字或行间留缝。`radius_nested(inset)` = 外圆角 − 内缩（同心圆角规则），嵌套面紧贴时用它推导而不是复用 `radius()`。`theme.rs` 管颜色（六族语义 token）与组件级 `Theme.radius`，`tokens.rs` 管自绘元素的尺寸。**写文档或代码时不要复制这里的数值当常量**——数值会随参照实现调整，唯一事实来源是 `tokens.rs` |
| 字体 | 默认系统 UI 字体；代码/技术字段默认系统等宽（设置里 `code_font_family = None` 即系统默认），不捆绑 Inter；用户可在设置里配置界面字体族、界面字号缩放、代码字体族与代码字号。字号缩放统一走 `tokens.rs` 的**字号阶梯**，不要新增裸 `text_size(px(...))`，也不要新增阶梯以外的字号档位 |
| 列表行分隔线 | 行用 `theme.table_row_border`（70% 透明度的发丝线），不要用 `theme.border` 全强度——后者会让整片列表呈网格/尺子感，与参照实现那种「只靠极淡界线分开」的基调冲突 |
| 列表选中底色 | 用 `theme.selection`，**不要用 `list_active` / `table_active`**：gpui-component 的 `apply_config` 强制把 list_active/table_active 的 alpha 压到 ≤0.2、selection 压到 ≤0.3（`theme/schema.rs` 末尾），而 list_active 源色太淡，压完叠在底色上只差约 2%，选中态肉眼不可见（实测渲染 249,249,252 对白底 255,255,255；换 selection 后 229,232,245）。`theme.rs` 的 `selection_tint_survives_library_alpha_clamp` 回归测试把这条钉住 |
| 弹层规范（所有自建 overlay 统一） | **Esc 必关**：有输入框的经专属 context 绑定（`Renaming`→DismissRename、`AccountModal`/`SettingsModal`→DismissModal、`ObjectFilter`→DismissFilter），无输入框的（详情/预览）统一 context `Overlay` + `UnifiedDismiss`，卡片上 `on_action` 注册 handler；**遮罩点击关**：遮罩 `on_mouse_down` 调 close，busy 中由 close handler 自行拒绝（保存/创建/复制移动中不可关）；**卡片两相冒泡阻断由构造器保证**：浮层的遮罩与卡片一律经 `crates/ui/src/ui/overlay.rs` 的 `mask()` / `surface()` 构造——`surface()` 内已做 `on_mouse_down` + `on_mouse_up` 双阻断（只阻断 down 时，卡片内滚动列表的滚动手势 up 会冒到遮罩，表现为「滚动一下就关闭」，SettingsModal 曾漏此项）；不要再手写卡片样式链；**入场动画**：浮层一律经 `overlay::fade_in(id, el)` 收口（时长/曲线在 `ui/motion.rs`，180ms opacity 淡入；gpui 无 div 级 transform/blur，故无位移/模糊；退场即时），且必须在链式装配最后一步调用；**卡片宽度必须响应窗口**：写 `w_full().max_w(px(设计宽度))`，**不要**写死 `w(px(N))`（窗口窄于设计宽度时卡片会压出窗口边缘）；**锚定菜单（anchored）必须按触发点的窗口坐标定位**：`position_mode(AnchoredPositionMode::Window).position(<触发点窗口坐标>)`（右键取 `MouseDownEvent.position`），**不要**靠「锚定元素所在容器的原点」相对定位——行在滚动容器里时两者坐标系不一致，菜单会压到侧栏上并纵向偏移（机制见 docs/notes/gpui-api-notes.md「弹出层」）；更不要手算像素几何（`6. + row × 40.`）——行高随字号/内容变化，偏移随行号线性漂移；遮罩自带 `p_6` 内边距，新增遮罩不要再各写 padding；**例外**：命令面板卡片由 GPUI Kit `Command` 自行绝对定位（按窗口宽度手算 `left`），必须用**无内边距**遮罩——绝对定位子元素的包含块是遮罩的 padding box，套 `mask()` 的 `p_6` 会把卡片整体右移 24px 而偏心（见 `render_palette_overlay` 注释）；该手算 `left` 在窗口窄于卡片时会得到负值、卡片压出左边缘（**待修**）；**视觉统一**：卡片圆角 `tokens::radius_lg()`、标题字号 `tokens::title()` + SEMIBOLD（**含 AddAccountModal**——它曾是唯一用 `heading()` 的）、标题栏右侧关闭按钮（busy 时 disabled）、底部按钮「取消在左、主操作在右」（macOS HIG 顺序）。新弹层先按本行自查再实现 |

### 5.8 验证手段与工具边界

| 领域 | 决策 |
|---|---|
| UI 冒烟边界 | 无屏幕录制权限 → `kCGWindowName` 恒为 `"(no title)"`，窗口存在性用 `/tmp/winall.swift`（按 OwnerName 过滤）输出非空判断，不能 grep 窗口名；gpui 不建 AX 树，UI 内部**交互**无法脚本化（点/键都别用合成事件——前台是谁不由你决定，实测会打到用户在用的其他应用上），只能人肉验证 |
| 单视图视觉验收 | **单个视图的视觉**可以脚本化：`crates/desktop/examples/view_preview.rs` 把**生产代码本身**的视图放进 `focus: false` 的离屏窗口，再按 pid 取 `CGWindowNumber` 用 `screencapture -x -o -l <号>` 抓图（离屏窗口也能抓到），全程不抢用户焦点。`OSM_PREVIEW_VIEW` = `search`（默认）/ `provider` / `dropframe` / `rename`；`OSM_PREVIEW_DARK=1`、`OSM_PREVIEW_FOCUS=1` 对所有视图生效。色值必须**采样**而非肉眼读（gpui 渲染值与 token 不完全相等，如 `#E6F7FF`→`#E3F6FE`）。详见 `docs/notes/gpui-api-notes.md`「离屏预览 + 截图验收」 |
| GPUI API 陷阱 | gpui-pre 0.3.x / gpui-component 0.6.1 已验证的 API 事实与陷阱清单见 `docs/notes/gpui-api-notes.md`；写 UI 前先查，不凭记忆猜签名。0.6.x 的改名要记住：`Corner`→`Anchor`、`window.focus(&h)`→`(…, cx)`、代码编辑器 `InputState`→`EditorState`、`Progress::new()`→`new(id)` 且 `value` 取值域变为 0..100 |
| 可访问性 | 每个鼠标可达控件都必须键盘可达：`track_focus` + `tab_index`/`tab_group`/`tab_stop` 排 Tab 序，焦点态用可见的 `focus_visible`；补齐常规控件键（方向键、Home/End、Enter/Space、Esc）。**不得只靠颜色/悬停/动效表意**——状态色必须与图标或文字配对。命中区域宁可放大控件，不要把字形缩小。装饰性动画判 `cx.reduce_motion()` |
| 调试方法论 | 静态分析数轮无果后**不再继续猜**：在交互链路关键点（处理器入口 → 浮层打开入口 → 各守卫分支）加 `eprintln!("[模块] …")`，或临时 canvas 打印各层真实 bounds（`eprintln!("[bounds] tag: origin/size")`），一轮拿决定性证据。**假设一旦被证伪立即停手**，升级诊断层级，不要在同一假设上再做第三个补丁。临时诊断改动标 `TODO(diagnose): 修完删除`，确认修复后立即连同随之失效的死代码一起删净 |

## 6. 目录与数据

```text
~/Library/Application Support/CloudStorage/   SQLite / settings.json / state
~/Library/Caches/<bundle-id>/                 Thumbnail / Preview / 临时下载缓存
~/Library/Logs/<AppName>/  或 os_log           日志（Credential 必须 Redact）
/tmp/<app-name>/                              应用临时工作区，退出按策略清理
```

内存红线：大 Bucket 列表不整体驻留、大文件不进 `Vec<u8>`、上传下载全部 Streaming；Thumbnail LRU 上限（Entry 数 + 内存，如 128MB）。下载文件默认不可执行。

## 7. UX 硬标准

- Unified Titlebar（Traffic Lights + **窗口级导航**一体：侧栏开关 / 后退前进 / 当前位置），Sidebar（180/220/360px，折叠 44px Icon Rail，⌘⌥S），Content 主列表占据剩余空间；主界面不再显示额外详情列。Resize 实时 60 FPS。
  **对象区的操作不放标题栏**：上传 / 更多 / 搜索在表格**正上方**的工具栏里（`render_object_toolbar`，操作在左、搜索在右）。分工判据——作用于**窗口**的进 Titlebar，作用于**对象列表**的进内容区工具栏；标题栏属于窗口拖拽区，控件越多越容易和拖拽/双击缩放打架。
- 快捷键一律 Command 系（⌘K/⌘L/⌘F/⌘U/⌘R/⌘,/⌘[/⌘]/⌘A/⌘W/⌘Q）；UI 中只显示 `⌘ ⌥ ⌃ ⇧` 符号，不显示 "Cmd+Shift+P" 文字。
- **列表行一律「单击选择、双击打开」**（Finder 语义，判据为纯函数 `row_activation`，单测锁死）。Space 预览（再按 Space/Esc 关闭，方向键切换）；Return 打开重命名弹层；删除用 `⌘⌫` 且远端删除必须确认（`window.prompt`，无废纸篓）。命令面板/添加账号打开时 ⌘⌫ 不删对象。
- Selection：Click / ⌘Click / ⇧Click / ⌘A，完整 macOS 语义。
- Context Menu 顺序参考 Finder，Delete 放最底。Menu Bar：App/文件/编辑/显示/对象/传输/窗口/帮助；同一 Action 必须在 Menu / Context Menu / Toolbar / 快捷键 / Command Palette 共用。
- 外观默认跟随 System（监听变化），设置中可手动固定 Light/Dark；低饱和 Accent，自有视觉身份（图标不得拼接七牛+阿里云+腾讯云 Logo）。**系统外观变化的订阅必须由 `WorkspaceView` 持有**（`watch_window_appearance` → `appearance_subscription` 字段）：gpui 的 `Subscription` 析构即退订，绑成窗口创建闭包里的局部变量会当场失效——曾因此长期「系统切亮/暗 App 不跟」，回归测试 `watch_window_appearance_is_retained_by_the_view` 把这条钉住。详见 `docs/notes/gpui-api-notes.md`「Subscription 是 RAII」。
- Retina 全适配；Trackpad 滚动平滑（虚拟列表不得丢惯性/跳跃）。
- **可访问性（与视觉同等的要求）**：每个鼠标可达的控件都要键盘可达——`track_focus` + `tab_index`/`tab_group`/`tab_stop` 排 Tab 序、焦点态有可见的 `focus_visible`，并补齐该控件类型的常规键（方向键、Home/End、Enter/Space、Esc）；**不得只靠颜色/悬停/动效表意**，状态色必须与图标或文字配对；命中区域宁可放大控件也不要把字形缩小。装饰性动画必须尊重 `reduce_motion`（`with_animation` 已内建，`request_animation_frame` 需自判）。
- **UI 设计基调（参照 oss-browser2；语义 token 六族沿用 [OpenChamber](https://github.com/openchamber/openchamber) theme-system，已落地 `crates/ui/src/theme.rs`）**：surface（面）/ primary（主 CTA）/ interactive+selection（可交互与选中）/ status（**唯一允许外来色相的一族**，只用于真实反馈）/ 中性（border 与 text 同属中性族）/ inverse。铁律：UI 代码只用 `cx.theme()` 语义字段，禁止硬编码 hex/hsla；hover 只给可交互元素；**selection ≠ primary**；**中性面不得带外来色相**（三条纪律都有测试，见 §5.7「色板族纪律」）。主色为 Ant 蓝 `#0064c8`（暗色 `#1668dc`），中性面统一冷灰 + 发丝级分隔线，选中态为淡蓝染色；小圆角（自绘尺寸档位见 `tokens.rs`，组件级由 `Theme.radius` 写入）。亮/暗两套（`CloudStorage Light/Dark`）经 `Theme::apply_config` 写入全局，`observe_window_appearance` 跟随系统切换。
- 收尾纪律：修复经确认后立即移除临时调试日志与随之失效的死代码；平时 `cargo check -p <crate>` 快速确认，收尾跑全量验证（§9）。

## 8. 性能指标（必须用 Instruments 测量，不许猜）

- 空闲 RSS < 60 MB（上限 80 MB）；Idle CPU ≈ 0%，禁止定时 Poll UI / 持续 redraw / 持续 network polling。
- 冷启动 < 1s：先显示窗口 → 恢复本地状态 → 异步加载云数据。**绝不**等 ListBuckets 才显示主窗口。

## 9. 构建与验证

- 主 Target：`aarch64-apple-darwin`；`cargo build --release`。
- 开发环境只保证 macOS + Apple Silicon，不为 Linux CI / Windows Developer 加 Workaround。
- **验证闸门**（提交前全量跑；仓库**尚未接入 CI**，勿假设有流水线把守）：

  ```bash
  cargo fmt --check
  cargo clippy --all-targets
  cargo test --workspace
  cargo build --release
  ```

  基线：235 passed / 2 ignored（ignored 的是需真实凭证的七牛与腾讯云 COS 联网用例；跑法见 README）、clippy 无告警。
- 打包：`./scripts/build-app.sh` → `.app` Bundle（`scripts/Info.plist.in` + `app-icon.png` 生成的 `.icns`）→ ad-hoc 签名 → `dist/CloudStorage-v<版本>-macos-arm64.zip` + sha256。
- 正式发布流程（尚未走完）：Developer ID 签名 → Notarize → Staple → DMG；Homebrew Cask 由 `daxiong123/homebrew-tap` 分发，仓内定义在 `Casks/cloudstorage.rb`（URL 资产名必须与脚本产物同名）。初期不做 App Store Sandbox。
- UI 改动无法脚本化验证交互：改完用 `cargo run -p object-storage-desktop` 后台启动，请用户复现确认；单视图视觉走 §5.8 的离屏预览。

## 10. 未落地与明确不做

| 项 | 状态 |
|---|---|
| 系统通知（UserNotifications） | **未实现**。`crates/macos` 目前只有 keychain / clipboard / quicklook / system_events / about。规范定位：仅长时间 Transfer 完成/失败、Migration 完成时通知 |
| 自动更新 | **未实现**，只预留 Updater 边界。规范要求 Check→Download→Verify→Install→Restart，且必须验证签名 + 校验和 |
| Developer ID 签名与公证 | 未做，发布包为 ad-hoc 签名；Bundle ID 与 Keychain service 仍是占位值 `com.example.cloudstorage` |
| `crates/preview` / `crates/common` | 占位 crate，尚无实现（预览能力目前落在 `crates/ui/src/workspace/preview.rs` 与 `crates/macos/src/quicklook.rs`） |
| 七牛断点续传 | **明确不做**（用户决定） |
| 腾讯云 COS 分块上传 / STS 临时凭证 | **明确不做**。COS 简单上传上限 5 GB，超过即 Fail Fast 报错；STS（`x-cos-security-token`）未接，账号只支持永久密钥 |
| 对象拖出到 Finder | 未做 |
| 命令面板卡片窄窗口偏心 | 已知缺陷：面板按其自算 `left` 绝对定位，窗口窄于卡片时会压出左边缘（**待修**） |
| CI | 未接入，验证靠本地闸门（§9） |

## 11. 工程纪律

1. **Fail Fast**：不写吞错误的兜底逻辑，错误必须暴露。
2. **Fix the Cause**：定位根因，不打补丁糊症状。
3. **Make It Observable**：关键路径留足日志/可观测性；信息不足就明说，不假装修好。
4. **Traceability**：关键节点可追溯。
5. **Living Documentation**：技术栈或产品方向变更时同步更新本文件与 `docs/spec/`；新踩的 gpui / 服务商坑记入 `docs/notes/`。本文件里的**事实性陈述必须可核对**——写数值（字号、圆角、测试数、版本）前先看代码，宁可只写「见 `tokens.rs`」。
6. **Don't Break Mainline**：大规模重构/实验前先切分支。
7. **验证环节由用户主导**：改完 UI 给出简短分点验证要点，让用户亲自复现确认。

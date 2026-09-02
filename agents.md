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
Rust + GPUI + gpui-component   主 UI（三栏 Workspace / Virtual Table / Command Palette / Keyboard-first）
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
    macos/          macOS native integration（Keychain/NSWorkspace/QuickLook/Clipboard/通知）
    preview/        Preview
    ui/             GPUI views/components
    common/         small shared utilities
```

不要建 `platform/{macos,windows,linux}` 目录。

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
| SQLite schema | `accounts` 表列固定为 `id/name/provider/access_key/created_at_millis`，有 `schema_has_no_secret_column` 回归测试把守：任何人给 SQLite 加 Secret 列都会让测试失败；provider 列用 CHECK 约束在 DB 层 Fail Fast |
| 本地路径 | `PathBuf`；Cloud Object Key：`String` + `/`。两者严格区分 |
| Provider trait | `StorageProvider`（`crates/storage-core`）：方法返回 `impl Future + Send`（不用裸 `async fn`，Send 义务显式化，否则无法 spawn 到 tokio/gpui 后台执行器）；非 dyn-safe，上层按服务商 enum 分发 |
| 七牛签名 | V2 请求签名逐字节核对自官方 SDK 源码并内置官方向量测试（V1 hello/world + V2 X-Qiniu-* 规范化排序）；坑：Base64 必须带 padding、签名用实际发送的原始 query 串、X-Qiniu-* 头名规范化为 Title-Case 后排序、putTime 单位 100ns。详见 `docs/notes/qiniu-api-notes.md`，勿凭记忆重写 |
| Transfer | Sleep/Wake/断网后状态为 `Waiting/Paused` 并恢复，**不得**误标 `Failed`（P0）；事件驱动，不轮询。已落地：`crates/transfer` 引擎（队列/状态机/并发上限）+ 单测锁死 P0 语义；任务执行体由 UI 层注入 `TaskRunner` 闭包（内调 `AppServices::build_provider` 即锁即放），future spawn 到 AppServices 的 tokio 运行时，暂停/挂起/取消 = `JoinHandle::abort()`（future 在 await 点丢弃即断 reqwest 连接）；attempt 代号丢弃过期完成回调；UI 经 `watch` 令牌订阅快照（无定时器）。系统事件（NSWorkspace/NWPathMonitor）接线与上传、断点续传在后续里程碑 |
| AppServices 线程模型 | `AccountService` 内含 rusqlite `Connection`（内部 RefCell，非 Sync），直接 `Arc<AppServices>` 进不了 gpui 后台任务（要求 Send+Sync）。连接统一收进 `Mutex<AccountService>`（crates/app/src/services.rs），同一时刻至多一个后台线程用数据库/钥匙串；锁毒化（持锁线程 panic）直接 panic 响报，不静默 |
| UI 异步编排 | UI 层永不直接调 provider/runtime：一律 `cx.spawn` → `background_executor().spawn` 调 AppServices 阻塞方法（内部 `runtime.block_on`）。并发竞态用代数计数（`bucket_gen`/`object_gen`）丢弃过期结果（last-click-wins）；添加账号模态用 `done/closed` 标志 + `observe_in` 由 WorkspaceView 回收，保存中禁止关闭（防丢成功结果） |
| UI 冒烟边界 | 无屏幕录制权限 → `kCGWindowName` 恒为 `"(no title)"`，窗口存在性用 `/tmp/winall.swift`（按 OwnerName 过滤）输出非空判断，不能 grep 窗口名；gpui 不建 AX 树，UI 内部交互无法脚本化，只能人肉验证 |
| ⌘Q vs ⌘W | ⌘W 只关窗口（**必须先禁用 close 动画再 remove_window**，见下行）；⌘Q 有 Transfer 时弹确认，默认 `Pause + Persist` |
| 关窗口实现 | macOS 15 close 动画会被 gpui 立即 teardown 杀死 → 窗口卡死可见。`handle_close_window`：`setAnimationBehavior: None` + `remove_window()`；失败方案与机制详见 `docs/notes/gpui-api-notes.md`「关闭窗口」章节，勿重复试错 |
| 通知 | `UserNotifications.framework`，仅长时间 Transfer 完成/失败、Migration 完成 |
| 字体 | 系统 SF Pro / SF Mono，不捆绑 Inter |
| 自动更新 | 架构预留 Updater 边界；Check→Download→Verify→Install→Restart，必须验证签名+校验和 |
| Action 注册点 | 所有跨 菜单/快捷键/右键菜单/工具栏 共用的 Action **只**定义在 `crates/ui/src/actions.rs`（`actions!(cloud_storage, …)`），键位在 `bind_keys(cx)` 统一绑定，不得散落各 view |
| 全局键位边界 | 不绑定 ⌘X/⌘C/⌘V/⌘A 全局快捷键（会吞文本输入的原生响应链）；Edit 菜单走 `MenuItem::os_action` 触发系统行为 |
| Quit 处理 | 全局 `cx.on_action`（capture 阶段），不依赖窗口焦点；窗口全关后仍可 ⌘Q。传输引擎已落地（`crates/transfer`，`suspend_all/resume_all` 可用）；⌘Q 退出确认（默认「暂停并持久化」）待接 Quit 流程 |
| Sidebar/Inspector | **自建视图**，不用 gpui-component `Sidebar`（组件固定 255px/48px，与规范 180/220/360 + 44px rail 冲突）；可拖拽宽度用 gpui-component `resizable`，按布局变体用不同 group id 保持各自记忆宽度 |
| GPUI API 陷阱 | gpui 0.2.2 / gpui-component 0.5.1 已验证的 API 事实与陷阱清单见 `docs/notes/gpui-api-notes.md`；写 UI 前先查，不凭记忆猜签名 |

## 6. 目录与数据

```text
~/Library/Application Support/<AppName>/   SQLite / settings / state
~/Library/Caches/<bundle-id>/              Thumbnail / Preview / 临时下载缓存
~/Library/Logs/<AppName>/  或 os_log       日志（Credential 必须 Redact）
/tmp/<app-name>/                           应用临时工作区，退出按策略清理
```

内存红线：大 Bucket 列表不整体驻留、大文件不进 `Vec<u8>`、上传下载全部 Streaming；Thumbnail LRU 上限（Entry 数 + 内存，如 128MB）。下载文件默认不可执行。

## 7. UX 硬标准

- Unified Titlebar（Traffic Lights + 导航 + Toolbar 一体），Sidebar（180/220/360px，折叠 44px Icon Rail，⌘⌥S），Inspector（280/320/520px，⌘⌥I）。Resize 实时 60 FPS。
- 快捷键一律 Command 系（⌘K/⌘L/⌘F/⌘U/⌘R/⌘,/⌘[/⌘]/⌘A/⌘W/⌘Q）；UI 中只显示 `⌘ ⌥ ⌃ ⇧` 符号，不显示 "Cmd+Shift+P" 文字。
- Space 预览（再按 Space/Esc 关闭，方向键切换）；Return 进 Inline Rename（Finder 式，不弹 Dialog）；删除用 `⌘⌫` 且远端删除必须确认。
- Selection：Click / ⌘Click / ⇧Click / ⌘A，完整 macOS 语义。
- Context Menu 顺序参考 Finder，Delete 放最底。Menu Bar：App/File/Edit/View/Object/Transfer/Window/Help；同一 Action 必须在 Menu / Context Menu / Toolbar / 快捷键 / Command Palette 共用。
- 外观跟随 System（监听变化）；低饱和 Accent，自有视觉身份（图标不得拼接七牛+阿里云 Logo）。
- Retina 全适配；Trackpad 滚动平滑（虚拟列表不得丢惯性/跳跃）。

## 8. 性能指标（必须用 Instruments 测量，不许猜）

- 空闲 RSS < 60 MB（上限 80 MB）；Idle CPU ≈ 0%，禁止定时 Poll UI / 持续 redraw / 持续 network polling。
- 冷启动 < 1s：先显示窗口 → 恢复本地状态 → 异步加载云数据。**绝不**等 ListBuckets 才显示主窗口。

## 9. 构建与 CI

- 主 Target：`aarch64-apple-darwin`；`cargo build --release`。
- CI 只跑 macOS ARM64：`cargo fmt --check` → `cargo clippy --all-targets` → `cargo test` → `cargo build --release` → App Bundle → Code Sign → Notarize。
- 发布：标准 `.app` Bundle（Info.plist + `.icns`）→ Developer ID 签名 → Notarize → Staple → DMG。初期不做 App Store Sandbox。
- 开发环境只保证 macOS + Apple Silicon，不为 Linux CI / Windows Developer 加 Workaround。

## 10. 工程纪律

1. **Fail Fast**：不写吞错误的兜底逻辑，错误必须暴露。
2. **Fix the Cause**：定位根因，不打补丁糊症状。
3. **Make It Observable**：关键路径留足日志/可观测性；信息不足就明说，不假装修好。
4. **Traceability**：关键节点可追溯。
5. **Living Documentation**：技术栈或产品方向变更时同步更新本文件与 `docs/spec/`。
6. **Don't Break Mainline**：大规模重构/实验前先切分支。

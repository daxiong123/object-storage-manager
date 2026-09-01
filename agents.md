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
- 最低 macOS 版本：13+ 或 14+，以 GPUI 及依赖的实际要求定，开发前先确认。
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
    transfer/       Transfer Engine
    persistence/    SQLite
    macos/          macOS native integration（Keychain/Panel/Workspace/QuickLook/Clipboard/通知）
    preview/        Preview
    ui/             GPUI views/components
    common/         small shared utilities
```

不要建 `platform/{macos,windows,linux}` 目录。

## 5. 关键技术决策

| 领域 | 决策 |
|---|---|
| 文件选择 | 只用 `NSOpenPanel` / `NSSavePanel`，禁止自造 |
| 剪贴板 | `NSPasteboard`；Signed URL 可配置 N 秒自动清除 |
| Open With / Show in Finder | `NSWorkspace`；远端 Object 下载到临时目录再打开 |
| 预览 | 常见格式应用内；PDF/Office/视频走系统 Quick Look，不自建 Preview Engine |
| Keychain key | `service = com.<company>.<app>.credentials`，`account = <account_uuid>`（不用账号名） |
| 本地路径 | `PathBuf`；Cloud Object Key：`String` + `/`。两者严格区分 |
| Transfer | Sleep/Wake/断网后状态为 `Waiting/Paused` 并恢复，**不得**误标 `Failed`（P0）；事件驱动，不轮询 |
| ⌘Q vs ⌘W | ⌘W 只关窗口；⌘Q 有 Transfer 时弹确认，默认 `Pause + Persist` |
| 通知 | `UserNotifications.framework`，仅长时间 Transfer 完成/失败、Migration 完成 |
| 字体 | 系统 SF Pro / SF Mono，不捆绑 Inter |
| 自动更新 | 架构预留 Updater 边界；Check→Download→Verify→Install→Restart，必须验证签名+校验和 |

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

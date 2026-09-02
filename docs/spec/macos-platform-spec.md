# macOS 平台限定规范

> 本文档是项目最高优先级的产品与技术规范，所有开发 Agent（Codex / Claude Code / Z-Code / Cindy）必须遵守。
> 操作层摘要见仓库根目录 `agents.md`；两者冲突时以本文档为准。

---

# macOS 平台限定补充与修正规范

本项目是一个：

> macOS Only · Native Rust Cloud Object Storage Manager

只支持 macOS。

当前以及可预见的未来，不考虑：

* Windows
* Linux
* Web
* iOS
* Android

不要为了理论上的跨平台能力增加任何额外架构复杂度。

---

# 1. 平台目标

唯一目标平台：

```text
macOS
```

重点支持：

```text
Apple Silicon
```

主要架构：

```text
arm64 / aarch64-apple-darwin
```

最低 macOS 版本应根据 GPUI 和依赖实际兼容情况确定。

优先考虑较新的 macOS 系统能力。

建议面向：

```text
macOS 13+
```

或者根据实际依赖能力选择：

```text
macOS 14+
```

开发前检查 GPUI 当前最低 macOS 支持版本，再做最终决定。

不需要为了 Intel Mac：

```text
x86_64-apple-darwin
```

牺牲架构设计。

如果构建成本较低，可以额外提供 Intel / Universal Binary。

但：

> Apple Silicon 是 P0 目标。

---

# 2. 技术选型原则变化

由于只支持 macOS，技术选型优先级调整为：

```text
macOS Native Experience
>
性能
>
低内存
>
开发效率
>
架构优雅
>
跨平台能力
```

跨平台能力权重为：

```text
0
```

任何第三方库如果：

```text
跨平台更强
```

但：

```text
macOS 体验更差
```

则不要选择。

---

# 3. GUI 技术栈

首选保持：

```text
Rust
+
GPUI
+
gpui-component
```

这是整个项目的默认方案。

原因：

* Rust Native
* GPU Accelerated
* Zed 同源设计方向
* 非 WebView
* 非 Chromium
* 高度可定制
* 非常适合开发工具
* 非常适合三栏 Workspace
* 非常适合大量 Object Table
* 非常适合 Command Palette
* 非常适合 Keyboard-first UI
* 非常适合 macOS

---

# 4. 不再考虑 Slint 作为主要候选

由于现在只支持 macOS：

除非 GPUI 存在明确且无法解决的技术阻碍，否则：

```text
不再切换到 Slint
```

项目一旦基于 GPUI 建立：

```text
Design System
Component System
Window System
Action System
```

就保持 GPUI 技术路线。

不要为了“备用方案”写 GUI abstraction layer。

不要设计：

```rust
trait GuiBackend
```

这种无实际价值的抽象。

---

# 5. 充分利用 macOS 原生能力

允许并鼓励 Rust 直接调用 macOS Framework。

可以根据需求使用：

```text
objc2
objc2-app-kit
objc2-foundation
objc2-security
```

等 Rust / Objective-C Bridge。

也可以使用其他成熟、安全的 macOS Rust Binding。

原则：

> GPUI 负责主 UI，macOS Framework 负责系统级能力。

---

# 6. AppKit 使用范围

需要时直接调用：

```text
AppKit
```

处理：

* NSWindow
* NSSavePanel
* NSOpenPanel
* NSWorkspace
* NSPasteboard
* NSApplication
* NSImage
* NSMenu
* NSStatusItem
* NSDragging
* NSNotificationCenter
* macOS Services

不要因为 Rust 而重新实现 macOS 已经提供的能力。

---

# 7. 窗口体验

主窗口必须具有真正的 macOS 桌面应用体验。

参考：

* ChatGPT macOS
* Zed
* Linear
* Raycast
* Finder
* Xcode

重点实现：

```text
macOS Traffic Lights
```

即：

```text
● ● ●
```

红黄绿窗口按钮。

整个顶部区域应该形成：

```text
Titlebar
+
Navigation
+
Toolbar
```

统一视觉区域。

避免传统：

```text
窗口标题
────────
Toolbar
────────
Content
```

的老式 AppKit 视觉。

---

# 8. Titlebar

尽量采用：

```text
Unified Titlebar
```

主界面顶部可以设计成：

```text
[traffic lights]

◀  ▶

production / images / pets

                    Search

             Upload   ...
```

与主内容自然融合。

视觉参考：

```text
Zed
ChatGPT Desktop
```

---

# 9. Sidebar 体验

左侧 Sidebar 必须高度符合 macOS 工具软件习惯。

支持：

```text
Collapse
Expand
Resize
```

建议：

```text
min: 180px
default: 220px
max: 360px
```

折叠后：

```text
44px 左右 Icon Rail
```

支持：

```text
Cmd + Option + S
```

或其他统一 Action 控制。

Sidebar Resize 时必须：

```text
实时响应
60 FPS
```

不要等待鼠标松开才更新。

---

# 10. Inspector

右侧 Inspector：

```text
min: 280px
default: 320px
max: 520px
```

可以：

```text
Collapse
Expand
Resize
```

可以考虑快捷键：

```text
Cmd + Option + I
```

没有选中 Object 时允许自动折叠。

---

# 11. macOS Keyboard-first

快捷键必须优先使用：

```text
Command
```

而不是 Ctrl。

例如：

```text
⌘K
Command Palette

⌘L
Focus Path

⌘F
Search

⌘U
Upload

⌘R
Refresh

⌘,
Settings

⌘[
Back

⌘]
Forward

⌘A
Select All

⌘C
Copy

⌘V
Paste

⌘W
Close Window

⌘Q
Quit
```

使用 macOS 标准符号展示：

```text
⌘
⌥
⌃
⇧
```

不要在 UI 中展示：

```text
Cmd
Option
Shift
```

作为快捷键 Badge。

例如应该显示：

```text
⌘⇧P
```

而不是：

```text
Cmd + Shift + P
```

---

# 12. Space 快速预览

支持：

```text
Space
```

快速预览 Object。

体验参考：

```text
Finder Quick Look
```

可以有两种实现方式。

第一优先：

```text
应用内 Preview
```

如果格式应用内无法处理：

调用：

```text
Quick Look / macOS system preview
```

不要为了支持所有格式自己实现整个 Preview Engine。

---

# 13. Quick Look

充分利用 macOS：

```text
Quick Look
```

对于：

* PDF
* Office
* 视频
* 特殊格式

等，可以根据实际技术能力使用系统 Quick Look。

这样可以避免：

```text
Chromium
PDF Engine
Video Player Engine
Office Renderer
```

导致应用体积和内存膨胀。

原则：

```text
常见格式
→ App 内预览

复杂格式
→ Quick Look / Open With
```

---

# 14. Open With

右键菜单提供：

```text
Open With
```

调用 macOS：

```text
NSWorkspace
```

例如：

```text
Open With
  Preview
  Xcode
  Visual Studio Code
  TextEdit
  ...
```

对于远端 Object：

可以：

```text
下载到 Temporary Directory
→
打开
```

退出或缓存清理时处理临时文件。

---

# 15. Drag & Drop

macOS 是重点。

必须完整支持：

## 本地 → Cloud

用户从：

```text
Finder
Desktop
其他 App
```

拖动：

```text
File
Files
Folder
```

进入 Object Browser：

直接创建 Upload Task。

---

## Cloud → Finder

长期目标支持：

从 Object Table：

```text
拖动 Object
```

到 Finder。

如果系统 Drag Promise 实现复杂：

可以放 Phase 2。

但架构要考虑。

---

# 16. Finder Integration

未来可考虑：

```text
Show in Finder
```

针对已下载文件。

以及：

```text
Download to...
```

打开：

```text
NSSavePanel
```

目录上传：

```text
NSOpenPanel
```

不要自己做文件选择器。

实现载体：gpui 平台 API `cx.prompt_for_new_path` / `cx.prompt_for_paths`
（内部即原生面板，`beginWithCompletionHandler:` 异步回调）。禁止在事件处理器里
同步 `runModal`：模态循环会重入 gpui `App` 借用，触发
"RefCell already borrowed" 闪退（详见 docs/notes/gpui-api-notes.md「文件对话框」）。

---

# 17. 文件选择器

全部使用 macOS 原生：

```text
NSOpenPanel

NSSavePanel
```

不要使用 GPUI 自己造文件选择器。

上传文件：

```text
NSOpenPanel
allowsMultipleSelection = true
```

上传目录：

```text
canChooseDirectories = true
```

下载：

使用：

```text
NSSavePanel
```

或选择 Download Folder。

下载的实现载体同样是 gpui 平台 API（`cx.prompt_for_new_path`，异步 oneshot 回传），
取消 = `Ok(None)` 正常分支；禁止事件处理器内同步 `runModal`（重入闪退，同上）。

---

# 18. Credential

由于只支持 macOS：

Credential Storage 不再使用跨平台：

```text
keyring abstraction
```

作为核心设计。

直接以：

```text
macOS Keychain
```

为唯一目标。

可使用：

```text
Security.framework
```

对应 Rust Binding。

Secret 必须保存在：

```text
Keychain
```

包括：

```text
Qiniu SecretKey

Aliyun AccessKey Secret

STS Secret

STS Token
```

SQLite 永远不保存 Secret。

---

# 19. Keychain Service Naming

建议统一：

```text
com.<company>.<app>
```

例如：

```text
com.example.cloudstorage
```

Keychain Item：

```text
service:
com.example.cloudstorage.credentials

account:
<account_uuid>
```

Account UUID 而不是账号名称作为 Key。

因为：

```text
账号名称允许修改
```

---

# 20. Touch ID / 系统安全

后续高级功能可以考虑：

```text
Touch ID
```

保护敏感 Credential 管理。

例如：

修改 Secret 前：

```text
Touch ID
```

但正常上传下载不要每次要求 Touch ID。

避免破坏体验。

---

# 21. 剪贴板

使用：

```text
NSPasteboard
```

支持：

```text
Copy Object Name
Copy Key
Copy URL
Copy Signed URL
Copy Bucket
Copy Endpoint
```

Signed URL 如果带签名：

可考虑：

```text
N 秒后从 Clipboard 自动清除
```

做成可配置安全选项。

---

# 22. macOS Menu Bar

必须设计真正的 macOS Menu：

```text
App
File
Edit
View
Object
Transfer
Window
Help
```

例如：

```text
File

Upload Files...
Upload Folder...
New Folder
New Bucket
```

---

```text
View

Toggle Sidebar
Toggle Inspector
Command Palette
Refresh
```

---

```text
Object

Open
Preview
Download
Copy URL
Rename
Delete
```

Action 必须和：

```text
Context Menu
Toolbar
Shortcut
Command Palette
```

共用。

---

# 23. Dock

支持标准 Dock 行为。

点击 Dock：

```text
重新激活主窗口
```

如果所有窗口关闭：

应用可以：

```text
继续运行
```

符合普通 macOS App 行为。

具体行为由产品定义。

---

# 24. Application 生命周期

正确处理：

```text
Cmd + W
```

和：

```text
Cmd + Q
```

区别。

⌘W：

```text
关闭 Window
```

不等同于：

```text
退出 Application
```

⌘Q：

如果存在 Transfer：

弹出：

```text
3 transfers are still running.
```

选项：

```text
Quit and Pause Transfers

Cancel

Quit Immediately
```

推荐：

默认：

```text
Pause + Persist
```

而不是粗暴取消。

---

# 25. Sleep / Wake

MacBook 非常容易：

```text
Close Lid
Sleep
Wake
```

Transfer Engine 必须正确处理：

```text
System Sleep

Network Disconnected

System Wake

Network Reconnected
```

系统 Sleep 之后：

Transfer 不应该被错误标记：

```text
Failed
```

而应该：

```text
Waiting / Paused
```

Wake 后恢复。

这是 P0 场景。

---

# 26. 网络变化

考虑 macOS：

```text
Wi-Fi 切换
VPN
代理
Sleep
热点
```

等网络变化。

Request Layer 必须可以：

```text
重新连接
重新 DNS
重新创建连接
```

而不是永久持有已经不可用的 socket。

---

# 27. Apple Silicon 性能优化

重点针对：

```text
M1
M2
M3
M4
M5+
```

优化。

避免不必要：

```text
Rosetta
x86-only dependencies
```

所有 dependency 优先选择：

```text
Native ARM64
```

---

# 28. 编译目标

开发：

```bash
cargo build
```

Release：

```bash
cargo build --release
```

主 Target：

```text
aarch64-apple-darwin
```

如果最终需要 Universal：

额外：

```text
x86_64-apple-darwin
```

然后使用：

```text
lipo
```

生成 Universal Binary。

但不是 P0。

---

# 29. App Bundle

最终不能只是：

```text
可执行文件
```

必须构建：

```text
CloudStorage.app
```

标准 macOS App Bundle。

包含：

```text
Contents/

    MacOS/

    Resources/

    Info.plist
```

---

# 30. App Icon

必须提供：

```text
.icns
```

或者标准 Asset Pipeline。

需要适配：

```text
16
32
64
128
256
512
1024
```

等 macOS Icon 场景。

设计风格：

```text
简洁
工具型
Cloud/Object Storage
```

不要直接：

```text
七牛 Logo + 阿里云 Logo
```

拼接。

产品应该拥有自己的视觉身份。

---

# 31. Code Signing

Release 必须考虑：

```text
Apple Developer ID
```

使用：

```text
codesign
```

正确签名：

```text
.app
```

所有嵌套 Framework / Binary 必须正确签名。

---

# 32. Notarization

正式发布必须：

```text
Apple Notarization
```

使下载后的 App 不出现：

```text
“无法验证开发者”
```

之类问题。

完整 Release Pipeline：

```text
Build

↓

Create .app

↓

Code Sign

↓

Notarize

↓

Staple

↓

Package
```

---

# 33. 分发格式

优先：

```text
DMG
```

用户打开：

```text
CloudStorage
        ↓
Applications
```

拖动安装。

也可以后续增加：

```text
ZIP
```

用于自动更新。

不优先 Mac App Store。

---

# 34. Sandbox

项目初期建议：

```text
非 Mac App Store Sandbox
```

原因：

本项目涉及：

```text
大量文件读写

任意目录上传

任意目录下载

临时文件

拖拽

网络访问
```

Developer ID 分发更灵活。

如果未来进入 Mac App Store：

再单独评估：

```text
App Sandbox
Security Scoped Bookmark
Entitlements
```

不要现在增加复杂度。

---

# 35. 自动更新

由于是桌面工具：

建议后续支持：

```text
Auto Update
```

体验类似：

```text
Zed
Raycast
```

更新流程：

```text
Check
Download
Verify
Install
Restart
```

必须验证：

```text
Signature
Checksum
```

禁止直接下载不验证的 Binary。

第一版可以不实现。

但项目架构要保留：

```text
Updater
```

边界。

---

# 36. 系统通知

使用：

```text
UserNotifications.framework
```

或者合适的 Rust Binding。

只在：

```text
长时间 Transfer 完成

长时间 Transfer 失败

Migration 完成
```

等情况通知。

不要：

```text
每上传一个文件通知一次
```

---

# 37. Appearance

严格支持：

```text
System
Light
Dark
```

默认：

```text
System
```

监听 macOS Appearance 变化。

例如用户：

```text
日落后系统自动切 Dark
```

应用应自动跟随。

---

# 38. Accent

应用可以有自己的：

```text
Accent Color
```

但不要完全依赖 macOS 系统蓝。

整体还是：

```text
ChatGPT / Zed
```

低饱和设计。

---

# 39. Retina

所有：

```text
Icon
Image
Text
Canvas
Divider
```

必须正确适配 Retina。

不能假设：

```text
1 logical px = 1 physical px
```

---

# 40. Trackpad

重点支持：

```text
Trackpad
```

滚动必须：

```text
平滑
```

特别是：

```text
Object Virtual Table
Sidebar
Inspector
Preview
```

不要因为虚拟列表导致：

```text
滚动跳跃
惯性滚动丢失
```

---

# 41. Context Menu

使用 macOS 风格。

菜单顺序参考 Finder。

文件：

```text
Open

Preview

Download

────────

Copy URL
Copy Key

────────

Rename
Move
Copy

────────

Get Info

────────

Delete
```

危险操作：

```text
Delete
```

放底部。

---

# 42. Rename

支持：

```text
Return
```

进入 Inline Rename。

类似 Finder。

不要每次弹 Dialog。

例如：

选中：

```text
dog.jpg
```

按：

```text
Return
```

变成：

```text
[dog.jpg]
```

编辑状态。

---

# 43. Delete

macOS 键盘推荐：

```text
⌘⌫
```

执行删除。

不要单独绑定普通：

```text
Delete
```

避免误操作。

删除远端 Object 不存在 Trash 的情况下：

必须确认。

---

# 44. Quick Look Shortcut

标准：

```text
Space
```

Preview。

再次 Space：

关闭。

Esc：

关闭。

方向键：

在当前 Selection 中切换 Object。

---

# 45. Selection

完整支持 macOS 习惯：

```text
Click
单选

⌘ Click
不连续多选

⇧ Click
连续选择

⌘A
全选
```

不能套用 Web Table 简化版 Selection。

---

# 46. Finder 风格但不是 Finder UI

交互层面可以学习 Finder：

```text
Selection

Rename

Space Preview

Drag Drop

Context Menu

Keyboard
```

但视觉不能抄 Finder。

视觉仍然：

```text
ChatGPT
Zed
Z-Code
Linear
```

也就是说：

> Finder 的桌面交互习惯 + Zed/ChatGPT 的现代视觉。

---

# 47. 系统字体

macOS 下不要捆绑 Inter 作为默认字体。

默认直接使用：

```text
SF Pro
```

通过：

```text
System Font
```

获取。

代码：

```text
SF Mono
```

或者：

```text
System Monospaced Font
```

避免额外 Font Asset。

减少 App Bundle。

---

# 48. Symbol

优先考虑：

```text
SF Symbols
```

用于真正 macOS Native 的系统级 Icon。

如果 GPUI 当前集成 SF Symbols 不方便：

UI Icon 保持：

```text
Lucide
```

统一即可。

不要：

```text
Lucide + Material + Heroicons
```

混合。

系统菜单可以使用 SF Symbol。

---

# 49. Performance Target

由于只支持 Apple Silicon，可以进一步提高要求。

空闲 RSS 目标：

```text
< 60 MB
```

理想：

```text
40-50 MB
```

如果因为 GPU Framework 基础开销无法达到：

优先保持：

```text
< 80 MB
```

但必须通过 Instrument 测量。

不能凭感觉判断。

---

# 50. 启动目标

Apple Silicon：

冷启动：

```text
< 1s
```

目标。

允许网络数据：

```text
启动后异步加载
```

绝对不能：

```text
等待 ListBuckets 完成
```

之后才显示主窗口。

应该：

```text
App Window
↓
立即显示
↓
恢复 Local State
↓
异步加载 Cloud Data
```

---

# 51. Instruments

性能分析优先使用：

```text
Xcode Instruments
```

至少关注：

```text
Time Profiler

Allocations

Leaks

Network

File Activity
```

性能问题不要：

```text
猜
```

必须 Profile。

---

# 52. Energy

MacBook 应关注：

```text
Energy Impact
```

Idle CPU：

```text
接近 0%
```

不要：

```text
定时 Poll UI
持续 redraw
持续 network polling
```

Activity 更新应该：

```text
Event Driven
```

---

# 53. 图片缓存

macOS 内存很宝贵。

Thumbnail Cache：

```text
LRU
```

必须设置：

```text
最大 Entry
+
最大 Memory
```

例如：

```text
128 MB
```

应视实际 Profile 调整。

Preview 关闭后：

大图应释放。

---

# 54. 内存原则

禁止：

```text
整个大 Bucket ObjectEntry 永久驻留

整个大文件放进 Vec<u8>

整个图片原图长期缓存

整个上传文件一次读进内存
```

所有大数据：

```text
Streaming
```

---

# 55. 下载文件权限

下载完成后：

正确继承 macOS 合理：

```text
File Permission
```

默认文件不应：

```text
Executable
```

除非有明确原因。

---

# 56. Temporary Directory

使用系统：

```text
NSTemporaryDirectory
```

或 Rust 对应：

```text
std::env::temp_dir()
```

但必须建立应用自己的：

```text
temp workspace
```

例如：

```text
/tmp/<app-name>/
```

Preview 临时数据退出后：

按策略清理。

---

# 57. Cache Directory

使用：

```text
~/Library/Caches/<bundle-id>/
```

保存：

```text
Thumbnail

Preview Cache

Temporary Download Cache
```

不要乱写：

```text
~/.app/
```

---

# 58. Application Support

SQLite 等持久数据：

使用：

```text
~/Library/Application Support/<AppName>/
```

例如：

```text
database.sqlite
settings
state
```

符合 macOS 规范。

---

# 59. Logs

日志：

```text
~/Library/Logs/<AppName>/
```

或者通过：

```text
os_log
```

系统日志机制。

Credential 必须 Redact。

---

# 60. macOS Path

所有 macOS 本地路径使用：

```text
PathBuf
```

不要在 Domain 中随意：

```text
String
```

拼路径。

Cloud Object Key 则始终：

```text
String
```

使用：

```text
/
```

两者严格区别。

---

# 61. Architecture Simplification

因为没有跨平台需求：

删除所有：

```text
platform/
    macos
    windows
    linux
```

这种无必要目录。

如果存在少数系统 API：

可以：

```text
crates/platform-macos/
```

统一处理：

```text
Keychain

Notification

Open Panel

Save Panel

Workspace

Quick Look

Clipboard

Application Support
```

---

# 62. 推荐 Workspace

最终推荐：

```text
cloud-storage-manager/

Cargo.toml

crates/

    desktop/
        macOS App 入口

    app/
        Application Services

    domain/
        Domain Models

    storage-core/
        Provider abstraction

    provider-qiniu/
        Qiniu Kodo

    provider-aliyun/
        Aliyun OSS

    transfer/
        Transfer Engine

    persistence/
        SQLite

    macos/
        macOS native integration

    preview/
        Preview

    ui/
        GPUI views/components

    common/
        small shared utilities
```

其中：

```text
macos/
```

可以直接依赖：

```text
AppKit
Foundation
Security
UserNotifications
```

相关 Binding。

---

# 63. 不需要的平台抽象

禁止设计：

```rust
trait CredentialStore {
    fn save(...)
}
```

只是为了未来：

```text
Windows Credential Manager
Linux Secret Service
```

如果抽象本身对测试有价值可以保留。

否则：

直接：

```text
MacOSCredentialStore
```

即可。

原则：

> 为真实需求抽象，不为不存在的平台抽象。

---

# 64. Release Target

CI 不再：

```text
macOS
Windows
Linux
```

三平台 Build。

只需要：

```text
macOS ARM64
```

建议 CI：

```text
cargo fmt --check

cargo clippy --all-targets

cargo test

cargo build --release

App Bundle

Code Sign

Notarize
```

---

# 65. Developer Environment

优先确保：

```text
macOS + Apple Silicon
```

开发体验最好。

例如：

```text
MacBook Pro M 系列
Mac mini M 系列
```

不需要为：

```text
Linux CI
Windows Developer
```

添加兼容 Workaround。

---

# 66. 最终产品定位修正

最终产品不是：

> Cross-platform Object Storage Browser

而是：

> A native, high-performance macOS workspace for Qiniu Kodo and Aliyun OSS.

中文：

> 一款专为 macOS 设计的高性能七牛 Kodo / 阿里云 OSS 对象存储工作台。

产品重点：

```text
Mac-first

Native Rust

Apple Silicon optimized

Keyboard-first

Low Memory

High Performance

Beautiful

Professional
```

---

# 67. 最终 UX 标准

产品应该让用户感觉：

```text
如果 Zed / ChatGPT 团队设计一个 OSS Browser，
大概就应该是这个样子。
```

不是：

```text
OSSBrowser 换了一个皮肤
```

更不是：

```text
把 Web 云控制台塞进桌面客户端
```

---

# 68. 最终技术标准

整个项目严格坚持：

```text
macOS only
Rust native
GPUI
gpui-component
Tokio
reqwest + rustls
SQLite
macOS Keychain
AppKit integration
Apple Silicon first
```

不引入：

```text
Electron
Chromium
WebView
React
Vue
Node.js Runtime
跨平台 GUI abstraction
跨平台 Credential abstraction
Windows compatibility code
Linux compatibility code
```

---

# 69. 开发 Agent 必须遵守

如果你是 Codex / Claude Code / Z-Code：

不要提出：

```text
为了以后支持 Windows，我们应该……
```

不要提出：

```text
为了跨平台，可以改用……
```

除非用户主动改变产品目标。

当前产品需求明确：

```text
macOS Only
```

因此所有架构判断优先服务：

```text
macOS 用户体验
+
性能
+
内存
+
维护成本
```

而不是未来不存在的跨平台需求。

---

# 70. 最终一句话原则

> Build the best Qiniu Kodo + Aliyun OSS client for macOS, not the most portable one.

中文：

> 我们的目标不是做最容易跨平台的对象存储客户端，而是做 macOS 上体验最好、性能最高、最轻量的七牛 Kodo + 阿里云 OSS 管理工具。

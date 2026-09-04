# Object Storage Manager

> 一款专为 macOS 设计的高性能七牛 Kodo / 阿里云 OSS 对象存储工作台。
>
> A native, high-performance macOS workspace for Qiniu Kodo and Aliyun OSS.

Object Storage Manager 不是 Web 控制台外壳，也不是 OSSBrowser 换皮。它的目标是把对象存储管理做成一款真正的 macOS 桌面应用：启动快、内存低、键盘优先、和系统能力深度整合。

UX 标准：Finder 的桌面交互习惯，叠加 Zed / ChatGPT 级别的现代视觉与响应速度。

## 产品定位

- **macOS Only**：最低 macOS 14+，Apple Silicon 是 P0 目标。
- **Native Rust App**：Rust + GPUI + gpui-component，不引入 Electron / Chromium / WebView。
- **Keyboard-first**：命令面板、菜单栏、快捷键、右键菜单共享同一套 Action。
- **Streaming First**：上传、下载、预览、复制/移动对象均避免大文件进内存。
- **Secure by Default**：Secret 只进 macOS Keychain；SQLite 只保存账号元数据和本地状态。
- **System Integrated**：Keychain、Quick Look、NSWorkspace、剪贴板、通知、系统外观、文件面板走 macOS/GPUI 原生能力。

## 当前状态

项目仍在开发中，但核心链路已经可用。

已完成的主要能力：

- **账号管理**：账号元数据写 SQLite，Secret 写 Keychain；新增失败有补偿，删除幂等；选中账号后 SK 会话缓存减少 Keychain 授权弹窗。
- **Provider**：七牛 Kodo 与阿里云 OSS；覆盖 Bucket 列表、对象列表、上传、下载、删除、签名 URL 等核心操作。
- **七牛 Kodo**：V1/V2 签名、官方向量回归测试、表单流式直传、Bucket 区域上传域名查询与缓存。
- **阿里云 OSS**：Signature V1、GetService、GetBucketLocation、三级域名访问、友好的 403 错误映射。
- **对象浏览**：Bucket / Prefix 下钻、Breadcrumb、刷新、过滤、多选、范围选择、全选。
- **对象操作**：上传文件、上传文件夹、Finder 拖放上传、下载、批量下载、删除、Inline Rename、新建目录、复制/移动到当前 Bucket 内路径。
- **预览与打开**：图片应用内预览、文本预览与编辑保存，PDF/Office/视频等走系统 Quick Look；支持 Open With / Show in Finder。
- **传输引擎**：队列、状态机、并发上限、字节进度、100ms UI 进度节流；睡眠/断网挂起，网络恢复后继续，不误标 Failed。
- **退出恢复**：有活动传输时 `⌘Q` 可暂停并持久化，下次启动恢复队列。
- **设置**：签名链接 TTL、复制后清剪贴板、外观模式、界面字体与缩放、代码字体、传输并发数、默认下载目录。
- **主题系统**：语义化 token，支持 Light / Dark / System，selection 与 primary 分离。

## 交互边界

应用遵循 macOS 桌面语义，而不是 Web App 语义。

- `⌘K` 打开命令面板。
- `⌘F` 过滤当前对象列表，不改变原始 entries 与选择集合。
- `Space` 打开/关闭预览，方向键切换对象。
- `Return` 进入 Finder 式重命名。
- `⌘⌫` 删除远端对象，必须确认。
- `⌘U` 上传文件，支持多选。
- `⌘W` 只关窗口，`⌘Q` 处理传输后退出。
- 所有自建 overlay 统一支持 Esc、遮罩点击关闭、卡片阻断冒泡和 macOS HIG 按钮顺序。

## 技术栈

```text
Rust 2024
GPUI 0.2.2 + gpui-component 0.5.1
Tokio
reqwest + rustls
SQLite / rusqlite
macOS Keychain / Security.framework
objc2 / AppKit / Foundation / UserNotifications
```

Workspace 结构：

```text
crates/
  desktop/          macOS App 入口，二进制名 CloudStorage
  ui/               GPUI Workspace、组件、主题、命令与快捷键
  app/              Application Services，账号与 Provider 编排
  domain/           Domain Models
  storage-core/     Provider trait 与共享类型
  provider-qiniu/   Qiniu Kodo 实现
  provider-aliyun/  Aliyun OSS 实现
  transfer/         Transfer Engine
  persistence/      SQLite / settings / state
  macos/            Keychain、系统事件、Quick Look、Clipboard、通知等
  preview/          预览相关能力
  common/           小型共享工具
```

## 本地开发

要求：macOS 14+，Apple Silicon 优先。

```bash
cargo run -p object-storage-desktop
cargo build -p object-storage-desktop
cargo build --release
```

常用验证：

```bash
cargo fmt --check
cargo clippy --all-targets
cargo test
cargo build --release
```

局部验证示例：

```bash
cargo test -p object-storage-ui
cargo test -p object-storage-app
cargo test -p object-storage-transfer
cargo build -p object-storage-desktop
```

## 数据与安全

- SQLite / settings：`~/Library/Application Support/CloudStorage/`
- 缓存与预览：`~/Library/Caches/<bundle-id>/`
- Secret：macOS Keychain，service 常量集中在 `crates/macos/src/keychain.rs`
- SQLite schema 有回归测试保证账号表和传输表不出现 Secret 列
- 日志与错误信息不得输出 Secret

## 设计原则

- **macOS Native Experience 优先**：macOS 已有的系统能力不重新造。
- **Fail Fast**：配置损坏、Keychain 缺失、持久化失败等关键错误必须显式暴露。
- **Fix the Cause**：不通过吞错误或兜底逻辑掩盖根因。
- **No Cross-platform Abstraction**：不为不存在的平台设计 GUI backend 或 credential abstraction。
- **Semantic Theme Tokens Only**：UI 代码只使用语义 token，不硬编码 hex/hsla。
- **Performance by Construction**：事件驱动，不轮询；大文件 streaming；进度通知节流。

## 文档入口

- `agents.md`：项目操作契约，所有开发者与 Agent 必读。
- `docs/spec/macos-platform-spec.md`：macOS 平台限定完整规范，冲突时以规范为准。
- `docs/notes/gpui-api-notes.md`：已验证的 GPUI API 事实与踩坑记录。
- `docs/notes/qiniu-api-notes.md`：七牛签名、上传域名与 API 行为记录。
- `docs/notes/aliyun-api-notes.md`：阿里云 OSS 签名、域名与 API 行为记录。

## License

MIT

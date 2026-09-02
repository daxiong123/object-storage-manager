# object-storage-manager

> 一款专为 macOS 设计的高性能七牛 Kodo / 阿里云 OSS 对象存储工作台。
>
> A native, high-performance macOS workspace for Qiniu Kodo and Aliyun OSS.

## 定位

- **macOS Only** · Apple Silicon 优先（`aarch64-apple-darwin`）
- **Native Rust**：GPUI + gpui-component，非 WebView / 非 Electron / 非 Chromium
- **Keyboard-first**：⌘K Command Palette、完整 macOS 快捷键体系
- **低内存 · 高性能**：空闲 RSS < 60 MB，冷启动 < 1s，全链路 Streaming
- **Native Integration**：Keychain、Quick Look、NSOpenPanel/NSSavePanel、NSPasteboard、NSWorkspace、系统通知

UX 标准：Finder 的桌面交互习惯 + Zed / ChatGPT 的现代视觉。

## 文档

- `agents.md` — 项目操作契约（所有开发者与 Agent 必读）
- `docs/spec/macos-platform-spec.md` — macOS 平台限定完整规范（单一事实来源）

## 技术栈

```text
Rust + GPUI + gpui-component · Tokio · reqwest + rustls
SQLite（~/Library/Application Support/） · macOS Keychain（Secret 专用）
objc2 / AppKit / Foundation / Security / UserNotifications
```

## 构建

```bash
cargo build                    # 开发
cargo build --release          # 主 Target: aarch64-apple-darwin
cargo run                      # 启动 App（二进制名 CloudStorage）
cargo fmt --check && cargo clippy --all-targets   # CI 门槛（agents.md §9）
```

## 状态

🚧 开发中。已就绪：

- **架构**：Cargo workspace（12 crates）；GPUI 窗口（Unified Titlebar），二进制名 `CloudStorage`
- **三栏 Workspace**：Sidebar（180/220/360px 可拖拽，折叠 44px Icon Rail）+ 内容区 + Inspector（280/320/520px 可拖拽），⌘⌥S / ⌘⌥I 切换，四种布局各自记忆拖拽宽度
- **账号**：Keychain 存 Secret + SQLite 存元数据，AccountService 编排一致性；SK 会话缓存（钥匙串授权每账号每会话仅首次操作弹一次）
- **Provider**：七牛 Kodo（V1/V2 签名、表单流式直传、签名下载）+ 阿里云 OSS（Signature V1、三级域名、GetService/GetBucketLocation），均内置官方向量回归测试
- **对象操作**：前缀下钻浏览 / 翻页；下载（NSSavePanel）；上传文件 ⌘U / 上传文件夹 / Finder 拖放入队；删除 ⌘⌫（确认）；预览（图片内联 / 文本编辑器语法高亮 / PDF 等走系统 Quick Look）；文本编辑后覆盖上传；复制签名链接（60s 后自动清剪贴板）
- **传输引擎**：队列 / 状态机 / 并发上限 / 字节进度，事件驱动无轮询；睡眠与断网自动挂起、网络恢复续传（不误标失败）；⌘Q 暂停并持久化，启动恢复
- **命令面板 ⌘K**：与菜单 / 快捷键 / 右键入口共享同一 Action（规范 §11/§22）

技术栈：`gpui 0.2.2` + `gpui-component 0.5.1`（crates.io 正式版，无 git 依赖）。
已验证的 GPUI API 陷阱清单见 `docs/notes/gpui-api-notes.md`。

## UI 设计基调

UI 尽可能参考开源项目 [OpenChamber](https://github.com/openchamber/openchamber) 的设计语言（语义化 token 体系 + Zed 级现代感），核心原则：

- **语义 token，不写死颜色**：分四族——`surface`（background / foreground / muted / elevated）、`interactive`（hover / active / selection / focusRing）、`status`（error / warning / success / info，只用于真实反馈）、`primary`（主 CTA）；映射到 gpui-component Theme，低饱和 Accent 保持自有视觉身份
- **selection ≠ primary**：primary 表示「执行动作」，selection 表示「当前选中」；不用 primary 色标记普通选中行
- **hover 只用于可交互元素**；静态内容不给 hover 反馈
- **共享组件优先**：按钮统一 variant / size（不造 `ButtonSmall` 类包装），图标走统一图标集
- **动画只动 `transform` 和 `opacity`**，且只在传达实时信息时使用

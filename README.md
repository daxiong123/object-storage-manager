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

- Cargo workspace（12 crates）+ GPUI 窗口（Unified Titlebar），二进制名 `CloudStorage`
- **三栏 Workspace**：Sidebar（180/220/360px 可拖拽，折叠 44px Icon Rail）+ 内容区 + Inspector（280/320/520px 可拖拽），⌘⌥S / ⌘⌥I 切换，四种布局各自记忆拖拽宽度
- **Action / 快捷键 / 应用菜单**：Action 集中注册（`crates/ui/src/actions.rs`），⌘Q / ⌘W / ⌘⌥S / ⌘⌥I 已绑定；编辑菜单走系统原生行为；菜单与快捷键共享同一 Action（规范 §11/§22）

技术栈：`gpui 0.2.2` + `gpui-component 0.5.1`（crates.io 正式版，无 git 依赖）。
已验证的 GPUI API 陷阱清单见 `docs/notes/gpui-api-notes.md`。

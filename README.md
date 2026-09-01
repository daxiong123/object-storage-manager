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
cargo build            # 开发
cargo build --release  # 主 Target: aarch64-apple-darwin
```

## 状态

🚧 规划阶段 — 规范已定，工程搭建进行中。

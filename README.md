# CloudStorage — Object Storage Manager for macOS

一款原生、高性能的七牛 Kodo / 阿里云 OSS 对象存储工作台。

CloudStorage 面向 macOS 14+ 与 Apple Silicon，使用 Rust + GPUI 构建。它不是 Web 控制台外壳：主界面、键盘交互、文件面板、Keychain、Quick Look、剪贴板与系统通知均采用原生能力。

> 项目仍在积极开发中，核心的账号管理、对象浏览和文件传输链路已经可用。

## 安装

### Homebrew

```bash
brew install --cask daxiong123/tap/cloudstorage
```

升级到 tap 中发布的新版本：

```bash
brew update
brew upgrade --cask daxiong123/tap/cloudstorage
```

要求 macOS 14+、Apple Silicon。

### 首次启动

当前发布包使用 ad-hoc 签名且尚未经过 Apple 公证。安装完成后，可以用以下命令仅移除 CloudStorage 的 Gatekeeper 隔离属性：

```bash
xattr -dr com.apple.quarantine "/Applications/CloudStorage.app"
```

请仅对通过上述 Homebrew tap 或本项目 GitHub Releases 获取的应用执行该命令。也可以不执行命令，首次启动被拦截时在「系统设置 → 隐私与安全性」中选择「仍要打开」。

也可以从 [GitHub Releases](https://github.com/daxiong123/object-storage-manager/releases) 下载应用压缩包，解压后将 `CloudStorage.app` 移入「应用程序」。

## 核心能力

- **七牛 Kodo / 阿里云 OSS**：浏览 Bucket 与对象，上传、下载、删除、重命名、复制、移动以及生成签名 URL。
- **macOS 风格对象浏览**：Prefix 下钻、Breadcrumb、过滤、排序、单选、多选、范围选择与全选。
- **文件传输**：流式上传下载、并发队列、字节进度；支持文件夹上传、批量下载和 Finder 拖放上传。
- **可靠恢复**：睡眠或断网时挂起传输，网络恢复后继续；退出时可暂停并持久化任务。
- **预览与打开**：应用内预览图片和文本、编辑文本对象；PDF、Office 与视频交给系统 Quick Look。
- **安全凭据**：Secret 仅保存到 macOS Keychain，SQLite 只保存账号元数据与应用状态。
- **原生体验**：命令面板、菜单栏、快捷键和右键菜单共用 Action；支持系统明暗外观、通知、剪贴板和 Finder 集成。
- **可配置**：签名链接有效期、剪贴板自动清除、字体与字号、传输并发数及默认下载目录。

## 常用操作

| 快捷键 | 操作 |
|---|---|
| `⌘K` | 打开命令面板 |
| `⌘F` | 过滤当前对象列表 |
| `⌘U` | 上传文件 |
| `Space` | 打开或关闭预览 |
| `Return` | Finder 式行内重命名 |
| `⌘A` | 全选当前对象列表 |
| `⌘⌫` | 确认后删除远端对象 |
| `⌘W` | 关闭当前窗口 |
| `⌘Q` | 处理活动传输后退出 |

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

主 UI 与文件选择由 GPUI 负责；Keychain、Quick Look、NSWorkspace、系统事件和通知直接使用 macOS Framework。上传与下载采用流式处理，避免把大文件整体载入内存。

## 本地开发

开发环境要求 macOS 14+；Apple Silicon 是当前主目标。

```bash
cargo run -p object-storage-desktop
cargo build -p object-storage-desktop
```

提交前验证：

```bash
cargo fmt --check
cargo clippy --all-targets
cargo test
cargo build --release
```

## Workspace

```text
crates/
  desktop/          macOS App 入口，二进制名 CloudStorage
  ui/               GPUI Workspace、组件、主题与快捷键
  app/              Application Services 与 Provider 编排
  domain/           Domain Models
  storage-core/     Provider trait 与共享类型
  provider-qiniu/   七牛 Kodo 实现
  provider-aliyun/  阿里云 OSS 实现
  transfer/         传输队列与状态机
  persistence/      SQLite、设置与恢复状态
  macos/            Keychain、系统事件、Quick Look 与通知
  preview/          预览能力
  common/           小型共享工具
```

## 数据与安全

- 应用数据：`~/Library/Application Support/CloudStorage/`
- 缓存与临时预览：`~/Library/Caches/<bundle-id>/`
- Secret：macOS Keychain
- SQLite schema 回归测试保证账号表与传输表不包含 Secret 列
- 日志和错误信息不得输出 Secret

## 项目文档

- [`agents.md`](agents.md)：项目操作契约与技术决策
- [`docs/spec/macos-platform-spec.md`](docs/spec/macos-platform-spec.md)：完整 macOS 平台规范
- [`docs/notes/gpui-api-notes.md`](docs/notes/gpui-api-notes.md)：已验证的 GPUI API 与踩坑记录
- [`docs/notes/qiniu-api-notes.md`](docs/notes/qiniu-api-notes.md)：七牛签名、上传域名与 API 行为
- [`docs/notes/aliyun-api-notes.md`](docs/notes/aliyun-api-notes.md)：阿里云 OSS 签名、域名与 API 行为

## License

[MIT](LICENSE)

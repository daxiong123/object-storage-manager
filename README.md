# CloudStorage

**一款专为 macOS 打造的原生对象存储工作台** —— 一个窗口里管好七牛 Kodo、阿里云 OSS 与腾讯云 COS。

用 Rust + GPUI 从头构建，不含 Electron / Chromium / WebView。键盘优先、启动即用、内存占用低，把 Finder 的操作习惯搬到云端对象存储上，其余系统能力（Keychain、Quick Look、剪贴板、系统外观、文件面板）直接交给 macOS。

| | |
|---|---|
| 平台 | macOS 14 (Sonoma) 及以上，Apple Silicon (arm64) |
| 当前版本 | 0.3.0 |
| 支持的服务商 | 七牛 Kodo、阿里云 OSS、腾讯云 COS |
| 技术栈 | Rust 2024 · GPUI (`gpui-pre`) · gpui-component · Tokio · reqwest + rustls · SQLite |

> 项目仍在活跃开发中：账号、浏览、传输、预览、设置等主链路已可用；尚未完成的部分集中列在下面的[已知边界](#已知边界)。

## 安装

### Homebrew（推荐）

```bash
brew install --cask daxiong123/tap/cloudstorage
```

升级：

```bash
brew update
brew upgrade --cask daxiong123/tap/cloudstorage
```

### 手动下载

从 [GitHub Releases](https://github.com/daxiong123/object-storage-manager/releases) 下载 `CloudStorage-v<版本>-macos-arm64.zip`，解压后把 `CloudStorage.app` 拖进「应用程序」。

### 首次启动被 Gatekeeper 拦下

当前发布包是 **ad-hoc 签名、尚未经过 Apple 公证**。两种处理方式，任选其一：

- 在「系统设置 → 隐私与安全性」中点击「仍要打开」；或
- 只移除本应用的隔离属性：

```bash
xattr -dr com.apple.quarantine "/Applications/CloudStorage.app"
```

请只对通过上面两个渠道获取的安装包执行该命令。

### 卸载

```bash
brew uninstall --cask cloudstorage   # 只卸载应用
brew zap cloudstorage                # 连配置与缓存一起清理
```

应用数据默认保留在 `~/Library/Application Support/CloudStorage/`。

## 快速上手

1. 点侧栏「账户」分组里的 **+ 添加账号**（也可 ⌘N 或 ⌘K 搜索「添加账号」），选服务商（七牛 Kodo / 阿里云 OSS / 腾讯云 COS），填入 Access Key 与 Secret Key。
2. 选中账号，侧栏下半段列出空间（Bucket）；选中空间，右侧列出对象。
3. **双击目录下钻、双击文件预览**；⌘U 上传，选中后 ⌘⌫ 删除，Return 重命名。
4. 记不住快捷键就按 ⌘K —— 菜单、工具栏、右键菜单里的动作都能在命令面板里找到。

Secret 只写入 macOS Keychain，SQLite 里只有账号元数据。

## 功能

### 账号与凭据

- 七牛 Kodo、阿里云 OSS 与腾讯云 COS 可混用多账号，侧栏一键切换
- 账号元数据（含 Access Key）存 SQLite，Secret Key 只存 macOS Keychain
- 创建账号时先写 Keychain 再写 SQLite，写库失败会回滚 Keychain 条目；删除幂等
- SK 在会话内缓存，钥匙串授权弹窗只在「选中账号后的第一次操作」出现
- 子账号没有 `ListBuckets` 权限时，可手动输入空间名进入

### 浏览对象

- 前缀下钻 + 面包屑导航，长路径中段自动折叠成 `…`
- 桶内前进 / 后退（⌘[ / ⌘]）；⌘L 直接输入路径跳转
- **服务端前缀搜索**：回车或点 🔍 提交，把输入当作前缀重新列举 —— 因此能查到「还没加载出来」的对象
- 排序：原序 / 名称 / 大小 / 修改时间，**目录恒排在文件前**（Finder 语义）
- 分页：每页 50 / 100 / 200 / 500 条可选，底部「加载更多」续取
- 虚拟列表：只渲染可见区间，几十万对象的 Bucket 也不会把整表驻留内存

### 选择与交互

- 完整的 macOS 选择语义：单击、⌘Click 增删、⇧Click 范围、⌘A 全选
- **点行只移动「当前行」，不会顺手勾上复选框** —— 勾选集合只由复选框、⌘Click、⇧Click、⌘A 改变
- 方向键移动当前行并自动滚进视野；⇧ + 方向键范围选择
- 行内动作：下载、更多（⋯）；右键菜单与行末菜单共用同一份动作
- 单击空白处清空选择

### 对象操作

- 上传文件（⌘U，可多选）、上传文件夹（递归入队）
- 从 Finder 拖入文件或目录直接上传（拖入时显示虚线框提示）
- 下载：单选走「另存为」，多选先选目标目录再逐项入队
- 重命名：Return 打开弹层，只改最后一段路径，校验即时反馈
- 复制 / 移动：选择目标前缀后在本桶内完成
- 新建目录、删除（⌘⌫ 确认后执行；批量删除逐项报告结果，失败不中断）
- 复制签名下载链接（有效期可配，可设置复制后自动清空剪贴板）

### 传输

- 流式上传下载，字节级进度；并发数可在设置里调整（1–8）
- 传输面板集中在标题栏与侧栏底部（带进行中数量角标），可取消、重试（暂停或失败的任务）、清除已完成
- **睡眠或断网时挂起，恢复后继续**，不会误报失败
- ⌘Q 时若仍有任务在跑，可选择「暂停并退出」——任务落盘，下次启动自动恢复

### 预览与打开

- 图片在应用内预览，按窗口尺寸等比完整显示
- 文本在应用内查看与编辑，⌘S 保存并上传；内容无改动时保存按钮禁用
- PDF、Office、视频等交给系统 Quick Look
- ⌘O 用默认应用打开、在 Finder 中显示（需要时先下载到缓存目录）
- 对象详情：大小、类型、ETag、上传时间

### 设置（⌘,）

签名链接有效期、剪贴板自动清除秒数、外观模式（跟随系统 / 浅色 / 深色）、界面字体族与字号缩放、代码字体族与字号、传输并发数、默认下载目录、上传大小上限。

**上传大小上限**限制的是**本地文件上传**（⌘U 上传文件、上传文件夹、Finder 拖放、⌘S 保存编辑后的文本）：超过上限的文件不会被加入队列，状态条会点名是哪些文件。`0` 表示不限制（默认）。云端复制 / 移动 / 重命名不受此限制。可设置的最大值是 **5119 MB**——必须小于 5 GB，因为 5 GB 是腾讯云 COS 简单上传的硬性上限。

设置写入 `~/Library/Application Support/CloudStorage/settings.json`；文件损坏会显式报错，不会静默重置。

### 原生集成

- 统一标题栏：侧栏开关、前进后退、当前位置、聚合传输进度
- 命令面板、菜单栏、右键菜单、工具栏、快捷键共享同一套 Action
- 原生文件选择面板、Quick Look、`NSWorkspace` 打开与 Finder 显示
- 外观跟随系统，可手动固定；窗口缩放实时 60 FPS

## 快捷键

| 快捷键 | 操作 |
|---|---|
| `⌘K` | 命令面板 |
| `⌘N` | 添加账号 |
| `⌘F` | 聚焦前缀搜索框（回车提交，`Esc` 清空） |
| `⌘L` | 输入路径跳转 |
| `⌘U` | 上传文件… |
| `⌘S` | 保存并上传（文本预览编辑中） |
| `⌘O` | 用默认应用打开选中对象 |
| `⌘A` | 全选当前对象列表 |
| `⌘⌫` | 删除选中对象（需确认） |
| `⌘R` | 刷新当前视图 |
| `⌘[` / `⌘]` | 后退 / 前进 |
| `⌘⌥S` | 切换侧栏 |
| `⌘,` | 设置 |
| `⌘W` / `⌘Q` | 关闭窗口 / 退出应用 |
| `Space` | 预览选中对象（再按一次或 `Esc` 关闭） |
| `Return` | 重命名选中对象 |
| `↑` `↓` `←` `→` | 移动当前行（`⇧` + 方向键为范围选择） |
| 双击行 | 打开文件 / 进入目录 |

## 已知边界

- **签名与公证**：发布包目前是 ad-hoc 签名，尚未用 Developer ID 签名与公证
- **Bundle ID 仍是占位值** `com.example.cloudstorage`（Keychain service 名同源，定稿后统一替换）
- 系统通知（UserNotifications）与自动更新尚未接入；更新流程只预留了架构边界
- `crates/preview` 与 `crates/common` 目前是占位 crate，尚无实现
- 七牛：**断点续传明确不做**；上传域名已按 Bucket 解析；拖出到 Finder 未做
- 腾讯云 COS：**分块上传明确不做**，单个对象超过 5 GB（简单上传上限）会直接报错；暂不支持 STS 临时凭证
- 仓库暂无 CI 工作流，验证在本地执行（见「本地开发」）

## 项目结构

```text
crates/
  desktop/          macOS App 入口（二进制名 CloudStorage，含菜单栏定义）
  ui/               GPUI 视图、设计 token、主题、Action 与快捷键
  app/              Application Services：账号编排与 Provider 构建
  domain/           Domain Models（Account / Bucket / CloudObject / ListingEntry）
  storage-core/     StorageProvider trait 与共享类型
  provider-qiniu/   七牛 Kodo 实现（V2 签名、区域上传域名解析）
  provider-aliyun/  阿里云 OSS 实现（V1 签名）
  provider-tencent/ 腾讯云 COS 实现（签名 v5）
  transfer/         传输引擎：队列、状态机、并发上限、watch 事件订阅
  persistence/      SQLite（账号 / 传输任务）与 settings.json
  macos/            Keychain、系统事件、Quick Look、剪贴板、关于面板
  preview/          预览能力（占位）
  common/           小型共享工具（占位）

docs/
  spec/macos-platform-spec.md   完整平台规范（70 节）
  notes/gpui-api-notes.md       已在源码核实过的 GPUI API 事实与陷阱
  notes/qiniu-api-notes.md      七牛签名、上传域名与 API 行为
  notes/aliyun-api-notes.md     阿里云 OSS 签名、域名与 API 行为
  notes/tencent-cos-api-notes.md 腾讯云 COS 签名 v5、端点、错误码与目录语义

scripts/build-app.sh            构建 + 打包 .app + 生成 icns + 签名 + 出 zip
Casks/cloudstorage.rb           Homebrew cask 定义
```

主 UI 用 GPUI 与 gpui-component；Keychain、Quick Look、`NSWorkspace`、系统事件等系统级能力直接调用 macOS 框架，不重复实现。上传下载全程流式，大文件不进 `Vec<u8>`。

## 本地开发

需要 macOS 14+ 与 Apple Silicon（主目标 `aarch64-apple-darwin`，最低系统版本在 `.cargo/config.toml` 固定为 14.0）。

```bash
cargo run -p object-storage-desktop     # 运行应用
cargo build --release                   # 发布构建
```

提交前跑完整验证闸门（CI 尚未接入，目前靠本地执行）：

```bash
cargo fmt --check
cargo clippy --all-targets
cargo test --workspace
cargo build --release
```

当前 `cargo test --workspace` 为 **220 passed / 2 ignored**——被忽略的两条是需要真实凭证的联网用例（七牛、腾讯云 COS），可这样单独跑：

```bash
QINIU_ACCESS_KEY=xxx QINIU_SECRET_KEY=yyy \
  cargo test -p object-storage-qiniu -- --ignored --nocapture

TENCENT_SECRET_ID=xxx TENCENT_SECRET_KEY=yyy \
  cargo test -p object-storage-tencent -- --ignored --nocapture
```

UI 的单个视图可以离屏渲染后截图做视觉验收，不影响当前前台应用：

```bash
OSM_PREVIEW_VIEW=search cargo run -p object-storage-desktop --example view_preview
# OSM_PREVIEW_VIEW = search（默认）/ provider / dropframe / rename
# OSM_PREVIEW_DARK=1 切暗色，OSM_PREVIEW_FOCUS=1 让输入框带焦点
```

## 打包与发布

```bash
./scripts/build-app.sh              # release 构建 + 打包
./scripts/build-app.sh --no-build   # 用现有二进制重新打包
```

脚本会组装 `dist/CloudStorage.app`（Info.plist + 由 `app-icon.png` 生成的 `.icns`）、做 ad-hoc 签名、打成 `dist/CloudStorage-v<版本>-macos-arm64.zip`，并输出 sha256 供 cask 使用。cask 定义在 [`Casks/cloudstorage.rb`](Casks/cloudstorage.rb)，通过 `daxiong123/homebrew-tap` 分发。

## 数据与安全

| 内容 | 位置 |
|---|---|
| 账号元数据、传输任务、`settings.json` | `~/Library/Application Support/CloudStorage/` |
| 缩略图、预览与临时下载缓存 | `~/Library/Caches/<bundle-id>/` |
| Secret Key | macOS Keychain（`service = com.example.cloudstorage.credentials`，`account = <账号 UUID>`） |

- SQLite 表里**没有** Secret 列，并有回归测试把守；日志与错误信息不输出 Secret
- Keychain 条目缺失时报错暴露，不静默降级
- 下载下来的文件默认不带可执行权限

## 文档

- [`agents.md`](agents.md)：项目操作契约——平台与技术约束、关键决策及其根因、UX 硬标准、工程纪律
- [`docs/spec/macos-platform-spec.md`](docs/spec/macos-platform-spec.md)：完整 macOS 平台规范（与契约冲突时以规范为准）
- [`docs/notes/gpui-api-notes.md`](docs/notes/gpui-api-notes.md)：GPUI / gpui-component 已验证的 API 事实与踩坑记录
- [`docs/notes/qiniu-api-notes.md`](docs/notes/qiniu-api-notes.md)、[`docs/notes/aliyun-api-notes.md`](docs/notes/aliyun-api-notes.md)、[`docs/notes/tencent-cos-api-notes.md`](docs/notes/tencent-cos-api-notes.md)：三家服务商的签名与接口行为

## License

[MIT](LICENSE)

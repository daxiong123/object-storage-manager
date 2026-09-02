# gpui 0.2.2 / gpui-component 0.5.1 API 笔记（已验证）

> 本文记录**在源码中核实过**的 API 事实与陷阱，供后续开发直接引用，避免凭记忆猜签名。
> 核对基准（本地 registry 源码，grep 不猜）：
> - `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/gpui-0.2.2/`
> - `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/gpui-component-0.5.1/`
>
> 版本固定于 crates.io 正式版，禁止换 git 依赖。升级版本时必须重新核对本文每一条。

## gpui 0.2.2

### 类型与转换
- **没有 blanket `From<E: IntoElement> for AnyElement`**。把具体元素塞进接受 `AnyElement` 的
  参数（如 `resizable_panel().child(...)`）必须显式调用 `.into_any_element()`。
- `.id(...)` 是 `InteractiveElement` 的方法：scope 里需要 `use gpui::InteractiveElement as _`
  （或 `use gpui::*`），否则报 method not found。

### Focusable
- trait 签名：`fn focus_handle(&self, cx: &App) -> FocusHandle`（返回值，不是 `&FocusHandle`）。
- `Entity<V>` 自动转发 `Focusable`（window.rs:445），持有 Entity 的地方可直接调。
- `App::focus_handle()` 在 app.rs:2029（app 级焦点根）。

### Action 与菜单
- 未被处理的 Action 派发是**静默的**（不 panic、不告警）——菜单项挂了 Action 但没人处理时
  不会崩，但也意味着 Action 断链要靠手工验证发现。
- `App::on_action`（app.rs:1696）注册的是 **bubble 末尾** 全局监听（源码注释写明
  `DispatchPhase::Bubble`）：「仅当没有其它 handler，或其它 handler 调用了
  `cx.propagate()` 时才会跑」。适合窗口全关后的 ⌘Q 兜底，**不是** capture。
  窗口内要拦截 Quit（弹确认）应在视图 `.on_action` 处理，不要依赖全局先跑。
- `App::quit()` 在 app.rs:749。
- `Window::remove_window()` 在 window.rs:1375（⌘W 关窗口用）。
- `Window::focus(&FocusHandle)` 在 window.rs:1386；open_window 回调里给根视图设置初始焦点，
  菜单 Action 才能沿焦点链派发到视图。

### 文件对话框：必须走 gpui 平台 API，禁止自建 runModal（重要，有闪退案例）

**症状**：在 gpui 事件处理器（on_click / on_action 监听器）里同步调 NSSavePanel 的
`runModal` → 面板能弹出、也能选目录，但确定/取消瞬间闪退。
日志：`thread 'main' panicked at gpui-0.2.2/src/app.rs:676:39: RefCell already borrowed`
随后 `failed to initiate panic, error 3, aborting`。

**根因**：事件处理器本身运行在 gpui 的 `App` RefCell 借用作用域内；`runModal` 起
嵌套事件循环，模态期间的 AppKit 事件（激活/窗口通知/绘制）回调试图重新进入 gpui
（再 borrow `App`）→ 重入借用冲突 → panic。与「选了什么」无关，面板一关就炸。

**正解**（gpui 自带，平台层实现已规避重入）：
- `App::prompt_for_new_path(directory: &Path, suggested_name: Option<&str>)
  -> oneshot::Receiver<anyhow::Result<Option<PathBuf>>>`（app.rs:1115，保存面板）。
- `App::prompt_for_paths(PathPromptOptions)`（打开/选目录，app.rs:1116 附近）。
- `Context<T>` Deref 到 `App`，实体处理器里直接 `cx.prompt_for_new_path(...)`。
- macOS 实现（platform/mac/platform.rs）：从 **foreground executor 任务**发起
  `beginWithCompletionHandler:`（异步回调，非阻塞 runModal），结果经 oneshot 回传。
  任务轮询不在借用作用域内，模态期间 AppKit 事件可正常借用 gpui，不冲突。
- 结果语义：`Ok(Ok(Some(path)))` 选中；`Ok(Ok(None))` 用户取消（正常流程，静默）；
  `Ok(Err(e))` 面板层错误；`Err(_)` oneshot 关闭（应用退出中等异常时序）。
- 附带修复：macOS 15 Sequoia 保存面板会额外追加扩展名的系统 bug
  （zed#16969）gpui 内部已按 OS 版本打补丁，自建实现则要自己踩一遍。
- 初始目录传用户主目录即可：`std::env::home_dir()`（Rust 1.85+ 已解除废弃）。

**反面教材存档**：本项目 milestone (d) 曾在 `crates/macos/src/panel.rs` 自建
`run_save_panel`（runModal + 手写 delegate），实测选完目录必闪退；已删除，由
gpui 平台 API 取代。教训：**凡是起嵌套 runloop 的东西（模态面板、拖放会话、
上下文菜单跟踪）都不能在事件处理器/更新作用域内同步调用**，要么走 gpui 提供的
异步 API，要么把调用挪进 `cx.spawn` 任务轮询（无借用作用域）。

### 关闭窗口：macOS 15 close 动画陷阱（重要）

**症状**：`Window::remove_window()` → `MacWindow::drop` → gpui 内部 close 任务执行
`[super close]` 之后，NSWindow 在屏幕上**永远不消失**（进程存活、gpui 注册表已清理、
CGWindowList OnScreenOnly 仍列出 layer=0 alpha=1.0 的窗口）。

**根因**：macOS 15 上 NSWindow `close` 默认带窗口动画；而 `MacWindow::drop` 在入队 close
任务的同时会 `window.autorelease()`，close 执行后毫秒级 dealloc，**动画被中途杀死**，
窗口卡在可见状态。这不是 gpui 特有的 bug 路径，而是「close 动画 × 立即 teardown」
的组合，任何 NSWindow 子类都可能踩到。

**修复**（两行，见 `crates/ui/src/workspace_view.rs` `handle_close_window`）：

```rust
// NSWindowAnimationBehaviorNone = 1，禁用 close 动画，[super close] 退化为纯 orderOut
let _: () = msg_send![win, setAnimationBehavior: 1i64];
window.remove_window();
```

实测要点（对比实验保留在 git 历史与本文末尾的实验记录）：
- 只 `remove_window()`（= close-only）→ 永远可见；
- 只 `orderOut`（任意时机）→ 窗口消失，但 gpui 注册表未清理，窗口对象泄漏；
- **setAnimationBehavior(None) + remove_window() → 窗口消失 + 注册表干净 + 进程存活**。

注意：`setAnimationBehavior` 必须在 close 之前设置（handler 里 remove_window 之前即可），
对之后所有的 close 生效。NSWindow 获取方式见下节「raw-window-handle」。

### 关闭窗口：失败方案存档（勿重复尝试）

以下方案全部实测失败，记录以避免踩坑（2025 年 macOS 15 / gpui 0.2.2）：

| # | 方案 | 结果 |
|---|---|---|
| F | remove_window → 延迟 orderOut（retain 保活） | 可见（T2 close 复活窗口） |
| H | remove_window → 200ms 后对裸 NSWindow 指针 orderOut | **段错误**（dealloc 后悬垂指针；延迟消息必须 retain） |
| H' | retain + close 先行 → 200ms 后 orderOut | 可见（close 后窗口对 orderOut 免疫） |
| K | 立即 orderOut → 500ms → remove_window → close | 可见 |
| L/M | remove_window → drop 后立即 orderOut（0ms/200ms 延迟） | 可见（orderOut 后 ~0ms 内 close 会复活窗口） |
| N | 立即 orderOut → 2s → remove_window | 可见（渲染器存活时 orderOut 完全无效，连临时消失都没有） |
| Run B/V1 | vendor 补丁：drop 内 orderOut（+2s 后 close / 不 close） | 隐藏（但需 vendor 补丁，不可接受为正式方案） |

经验教训：
- **对已 dealloc 的 NSWindow 发消息 = 段错误**（不是 NSException）。gpui 的 drop 会
  autorelease；延迟消息必须先 `msg_send![win, retain]`，用完 `release` 平衡。
- **渲染器存活时 orderOut 完全无效**（窗口服务器层面就不消失，非闪现后复活）。
- close 动画被杀死 → 窗口卡死可见，是 macOS 15 特有行为（14 及以下未验证）。
- 排查此类问题用 CGWindowList（`CGWindowListCopyWindowInfo` + `.optionOnScreenOnly`，
  swift 一段脚本即可，无需屏幕录制权限）+ eprintln trace + `exec-launch` 重定向。
- `screencapture` 需要屏幕录制权限（终端宿主常没有），CGWindowList 不需要。
- gpui 0.2.2 无 AX 树，无法用 Accessibility 检查 UI。
- CGEvent `postToPid` 可在无前台权限时向指定进程注入键盘事件（菜单/快捷键自动化测试用）。

### 确认对话框：必须走 `Window::prompt`（NSAlert sheet + oneshot）

`Window::prompt(level, message, detail, answers, cx) -> oneshot::Receiver<usize>`
（window.rs:4141）。macOS 实现（platform/mac/window.rs:1121）用
`beginSheetModalForWindow:completionHandler:`，**不是** `runModal`，不会重入 gpui App RefCell。

- `PromptLevel::{Info, Warning, Critical}`；`PromptButton::{ok, cancel, new}`。
- 返回值是按钮在 `answers` 数组里的**原始下标**（NSAlert 会把 Cancel 视觉上挪到最后，tag 仍是原下标）。
- 第一按钮默认 Return；`PromptButton::Cancel` 绑 Escape。
- **禁止重入**：`cx.prompt_builder.take()`，二次 `prompt` 会 `unreachable!`。⌘Q 这类入口必须用 flag 防抖。
- 调用方 `cx.spawn` 里 `rx.await` 后再 `cx.quit()` / 落盘；不要在事件处理器里同步等。

### raw-window-handle（获取 NSWindow）

- gpui **不重导出** raw-window-handle；需要时自己加依赖 `raw-window-handle = "0.6"`
  （与 gpui 0.2.2 的版本一致，trait 才能对上）。
- gpui 0.2.2 的 `Window` 实现 `HasWindowHandle`（window.rs:4845）——用**新 API**：

```rust
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
let raw = window.window_handle()?.as_raw(); // WindowHandle::as_raw() -> RawWindowHandle (Copy)
let ns_view = match raw {
    RawWindowHandle::AppKit(h) => h.ns_view.as_ptr(), // NonNull<c_void>，即 contentView 所在 NSView
    _ => unreachable!(),
};
let win: *mut Object = msg_send![view, window]; // NSView.window → NSWindow
```

- 旧 API `HasRawWindowHandle` / `window.raw_window_handle()` 在 rwh 0.6 中已 deprecated
  （仍在 crate 内兼容），新代码用 `HasWindowHandle`。

### objc 0.2 消息发送陷阱

- 需要 `use objc::{msg_send, sel, sel_impl};`。
- **没有 `nil` 常量**：用 `std::ptr::null_mut()` 代替。
- selector 冒号语法：`orderOut:` 是对的；`orderOut_:`（尾下划线风格）会
  NSInvalidArgumentException 崩溃。
- 无返回值消息：`let _: () = msg_send![win, orderOut: nil];`（编译器需要类型标注）。
- retain/release：`let _: () = msg_send![win, retain];` / `msg_send![win, release]`。


### 闭包借用
- `bool::then(|| self.render(&mut cx))` 这类写法会触发 E0524（两个闭包同时捕获 `&mut cx`）。
  改用普通 `if` 语句分分支构造。

## gpui-component 0.5.1

### 根重导出缺口
- 根 crate 重导出了 `icon::*` / `styled::*` / `theme::*` / `title_bar::*` / `Root`，
  **但没重导出 `Button` 和 `h_resizable`**。要写完整路径：
  `gpui_component::button::Button`（+ `button::ButtonVariants`）、
  `gpui_component::resizable::h_resizable` / `resizable_panel`。

### Sidebar（本仓库弃用）
- `Sidebar` 是固定 `DEFAULT_WIDTH px(255.)` / `COLLAPSED_WIDTH px(48.)`，构造为
  `Sidebar::new(side)`，实现 `Styled` 但 `refine_style` 在 collapsed 覆盖**之前**应用——
  自定义宽度会被覆盖。与规范 180/220/360 + 44px rail 冲突，故自建视图。
  自建时用它的 primitives（Icon / theme tokens / Button）保持视觉一致。

### Resizable
- `PANEL_MIN_SIZE = px(100.)`；group 容器渲染为 `size_full()`——外面要包一层 `flex_1()`。
- `resize_panel` / `update_panel_size` 是 `pub(crate)`：**没有公开的程序化 resize API**。
- 面板宽度 state 存在 `window.use_keyed_state(group_id)`：同一 group id 共享/记忆宽度。
  需要多套互不干扰的布局（如 边栏开/关 各自记忆宽度）时用**不同 group id**。
- resize handle 挂在每个面板的 LEFT 边，作用目标是 `panel_ix - 1`（即拖左边缘改前一个面板宽）。
  副作用：最右面板（Inspector）的 280..520 范围无法被它自己的把手约束——已知限制，
  待实现真实 Inspector 时解决。

### TitleBar
- `TitleBar` 的 children 只渲染**左侧**（预留 80px macOS padding 给 Traffic Lights）。
  右侧内容用 `h_flex().w_full().justify_between()` 自行布局。

### 主题与图标
- Theme tokens（theme_color.rs:123+）：`sidebar` / `sidebar_foreground` / `sidebar_border` /
  `sidebar_accent` 等可直接用。
- `IconName` 有：PanelLeft/Open/Close、PanelRight/Open/Close、Globe、FolderOpen、Star、
  Settings、Inbox 等；**没有** Cloud / HardDrive / History（需要时从 Lucide 补 SVG，
  只用 Lucide 一家，禁止混用图标集）。
- `Size` 枚举：XSmall / Small / Medium（默认）/ Large；`Sizable::with_size(Size::Small)`。

### 初始化顺序
- `gpui_component::init(cx)` 必须先于其它初始化调用（官方要求）；随后 `ui::init(cx)` 注册键位。

### 杂项
- 查 crates.io API（版本号等）需要带 `User-Agent` 头，否则被拒。

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
- `App::on_action`（app.rs:1696）注册的是 capture 阶段全局监听：适合 `Quit` 这类
  不依赖窗口焦点的应用级行为（最后一个窗口关闭后依然可达）。
- `App::quit()` 在 app.rs:749。
- `Window::remove_window()` 在 window.rs:1375（⌘W 关窗口用）。
- `Window::focus(&FocusHandle)` 在 window.rs:1386；open_window 回调里给根视图设置初始焦点，
  菜单 Action 才能沿焦点链派发到视图。

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

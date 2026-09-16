# gpui-pre 0.3.4 / gpui-component 0.6.1 API 笔记（已验证）

> 本文记录**在源码中核实过**的 API 事实与陷阱，供后续开发直接引用，避免凭记忆猜签名。
> 核对基准（本地 registry 源码，grep 不猜）：
> - `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/gpui-pre-0.3.4/`
> - `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/gpui-component-0.6.1/`
>
> 版本固定于 crates.io 正式版，禁止换 git 依赖。升级版本时必须重新核对本文每一条。

## gpui-pre 0.3.4

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
- `Window::focus(&FocusHandle, cx)`；open_window 回调里给根视图设置初始焦点，
  菜单 Action 才能沿焦点链派发到视图。
- `Application::new()` 已移除；macOS 入口使用 `gpui-pre-platform` 的
  `gpui_platform::application()`，并保留 `font-kit` / `runtime_shaders` feature。
- `AsyncApp::update` 直接返回闭包结果，不再返回 `Result`，调用处不要追加 `?`。
- 浮层锚点枚举由 `Corner` 更名为 `Anchor`。

### 文件对话框：必须走 gpui 平台 API，禁止自建 runModal（重要，有闪退案例）

**症状**：在 gpui 事件处理器（on_click / on_action 监听器）里同步调 NSSavePanel 的
`runModal` → 面板能弹出、也能选目录，但确定/取消瞬间闪退。
历史日志：`thread 'main' panicked at gpui-0.2.2/src/app.rs:676:39: RefCell already borrowed`
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

**修复**（两行，见 `crates/ui/src/workspace/quit.rs` `handle_close_window`）：

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

以下方案全部实测失败，记录以避免踩坑（2025 年 macOS 15 / 当时 gpui 0.2.2；
升级 0.3.4 后仍需保留规避，直到真实运行验证上游已修复）：

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
- gpui-pre 0.3.4 已接入 AccessKit macOS 无障碍支持；是否足以覆盖本项目 UI 自动化需另行实测。
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

### 系统文件拖放：`ExternalPaths` + `drag_over` 必须配空 `hover`

macOS 文件拖入被翻译成内部 drag（window.rs:3622，`FileDropEvent::Entered` → `active_drag = ExternalPaths`）。
投放用 `.on_drop(cx.listener(|_, paths: &ExternalPaths, _, cx| ...))`。

**陷阱**：`div.rs` 只在 `hover_style.is_some() || active_drag.is_some()` 时注册
mousemove→`cx.notify`。系统拖入前一帧 `active_drag` 为空，不注册监听，拖入过程
**不会重绘**，`drag_over` 高亮永远看不见（投放本身仍能触发，因为 MouseUp 走 drop 监听）。
正解：投放目标加空 `.hover(|s| s)` 让监听常驻；高亮用边框（子元素不透明背景会盖住父级 bg）。

### raw-window-handle（获取 NSWindow）

- gpui **不重导出** raw-window-handle；需要时自己加依赖 `raw-window-handle = "0.6"`
  （与 gpui-pre 0.3.4 的版本一致，trait 才能对上）。
- gpui-pre 0.3.4 的 `Window` 实现 `HasWindowHandle`——用**新 API**：

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

## gpui-component 0.6.1

### 导入路径
- `Button` 和 `h_resizable` 继续使用完整路径：
  `gpui_component::button::Button`（+ `button::ButtonVariants`）、
  `gpui_component::resizable::h_resizable` / `resizable_panel`。

### Sidebar（本仓库弃用）
- 本项目需要 180/220/360 + 44px rail，与组件默认布局约束不同，故继续自建视图。
  自建时用它的 primitives（Icon / theme tokens / Button）保持视觉一致。

### Resizable
- `PANEL_MIN_SIZE = px(100.)`；group 容器渲染为 `size_full()`——外面要包一层 `flex_1()`。
- `resize_panel` / `update_panel_size` 是 `pub(crate)`：**没有公开的程序化 resize API**。
- 面板宽度 state 存在 `window.use_keyed_state(group_id)`：同一 group id 共享/记忆宽度。
  需要多套互不干扰的布局（如 边栏开/关 各自记忆宽度）时用**不同 group id**。
- resize handle 挂在每个面板的 LEFT 边，作用目标是 `panel_ix - 1`（即拖左边缘改前一个面板宽）。
  当前只保留左侧栏 + 内容区两列布局；不要再新增额外的详情列 resizable 面板。
- 0.6.1 已修复 resize 时的闪烁，升级即可受益，无需应用层补丁。

### 弹出层（anchored / deferred / 焦点）
- 浮层标准写法（对齐 gpui-component select/popup_menu）：
  `deferred(anchored().anchor(anchor).offset(...).snap_to_window_with_margin(...).child(<菜单卡片>))`，
  锚点类型是 `gpui::Anchor`。
  **不要在 anchored 的 child 里再套 `div().absolute()`**——absolute 与 anchored 锚定机制
  冲突，菜单渲染不可见（曾导致「更多操作」菜单点击后无菜单弹出）。
- 行内弹出菜单锚定：**必须按触发点的窗口坐标绝对定位**，即
  `anchored().anchor(Corner::TopLeft).position_mode(AnchoredPositionMode::Window).position(<触发点窗口坐标>)`。
  右键触发时取 `MouseDownEvent.position`（其文档即「position of the mouse on the window」）。
  **不要靠「锚定元素所在容器的原点」相对定位**（= 不传 `position` 时的默认行为）：菜单挂在
  `w_full()` 的行上时锚点落在行的**左端**，菜单会越过行左边界压到侧栏上；而行身处
  `overflow_y_scroll` 容器时，taffy 给绝对定位元素解出的 `bounds.origin` 与视口坐标系不一致，
  纵向还会整体偏移（对象行右键菜单曾因此两次跑偏）。
- 为什么「按窗口坐标」是安全的（读源码确认的坐标契约）：`Window::layout_bounds` 会
  `bounds.origin += element_offset()`（window.rs）；而 `element.rs` 在每个元素 `prepaint` 前
  用 `with_absolute_element_offset(origin)` **把偏移绝对设成该元素自己的 layout origin**。
  于是 `Anchored::prepaint` 里 `element_offset() == bounds.origin`，再经
  `offset = desired.origin - bounds.origin` + `with_element_offset(offset)`，子元素恰好被画在
  `desired.origin`——与所在容器、与滚动偏移都无关。默认那条「相对 `bounds.origin` 锚定」
  才会跟着容器/滚动跑。
- **禁止手算行 y 偏移**（`row × 40px` 一类）：行高随字号缩放/内容变化，手算值会随行号线性错位。
- **不要在 `on_mouse_down` 处理器里 `window.focus()`**：焦点会被同一次点击的后续处理
  覆盖回原焦点（实测焦点回到 workspace 根），键盘派发（如 Esc → Overlay context）全部
  落空。正确做法：处理器里只置 `needs_focus` 标记，在 `render()` 中弹层元素渲染挂载后
  再 `window.focus()`（参考 WorkspaceView.preview_needs_focus）。
- `InputState` 内部处理 Escape：无 context menu / inline completion / IME 且未设
  `clean_on_escape`（默认 false）时会 `cx.propagate()`，Esc 能继续沿焦点链派发到
  Overlay context——编辑器里的 Esc 关弹层因此可用，无需额外处理。

### Input / Editor
- 0.6.1 将单行 `Input`、多行 `Textarea`、代码 `Editor` 分成独立组件。
- 文本对象预览/编辑使用 `EditorState::new(...).language(...).default_value(...)` +
  `Editor::new(...)`；普通表单继续用 `InputState` + `Input`。
- 当前只启用 `tree-sitter` 基础 feature（保留 JSON 高亮）；没有真实格式需求前不引入整套语法 grammar。

### Command
- 命令面板使用 `Command` + `CommandState`；组件原生提供大小写不敏感过滤、关键词、
  虚拟列表、↑↓/Enter/Esc、滚动定位、无障碍 listbox 语义和 Action 键位提示。
- 本项目命令执行仍由 `on_confirm(IndexPath)` 回调编排：条目本身不直接挂 Action，先关闭面板
  再沿现有焦点链派发共享 Action；动态 Bucket 命令继续经 WeakEntity 直调，避免失效焦点。
- 组件默认 Esc 在查询非空时先清空；项目契约要求一按即关，因此 `ui::init` 在同一个
  `Command` context 后注册 `DismissCommandPalette` 覆盖绑定（GPUI 同深度后注册者优先）。
- UI crate 的 dev-dependency 启用 0.6.1 `test-support`；headless 测试用
  `TestAppContext` + `simulate_keystrokes("enter")` 验证过滤和动态命令确认链路。

### Progress
- `Progress::new(id)` 必须提供稳定 ElementId；`.value(...)` 的范围是 **0..=100**，不是 0..=1。
- 传输进度附带 `accessibility_label`；0.6.1 内建平滑进度过渡，无需手写动画。

### 布局（flex 高度约束）
- **row 容器不拉伸子元素高度**：`div()` 默认 align 非 stretch，column 子容器（如
  滚动列表 `v_flex().overflow_y_scroll()`）必须显式 `.h_full()` 约束高度。缺省时
  滚动容器高度=内容自然高度：内容越过父容器叠在下方元素上，且容器=内容高度
  **无溢出 → overflow_y_scroll 滚动条失效**。两个症状一个根因（曾致状态条与行重叠）。
- **flex 列容器链条上的 `min_h_0()` 要显式补全**：flex 子元素默认 min-height:auto
  （=内容最小高度），任何一层缺了都会把内容高度向上传染，把底部固定元素
  （状态条等）挤出可视区。参考 WorkspaceView render_body → content 链。
- 排查布局用 `gpui::canvas` 打 bounds（paint 阶段回调拿 `Bounds<Pixels>`），
  比 screenshot + 猜测快得多；对照组：滚动区包裹层 / 滚动容器 / 状态条三层。

### TitleBar
- `TitleBar` 的 children 只渲染**左侧**（预留 80px macOS padding 给 Traffic Lights）。
  右侧内容用 `h_flex().w_full().justify_between()` 自行布局。

### 主题与图标
- Theme tokens（theme_color.rs:123+）：`sidebar` / `sidebar_foreground` / `sidebar_border` /
  `sidebar_accent` 等可直接用。
- 完整 Lucide 目录由 `gpui-kit-assets::IconName` 共享；组件兼容枚举仍可用。
- 自有 Lucide SVG 直接 `Icon::default().data(include_bytes!(...))`，不再为路径维护组合
  `AssetSource`；默认图标资产源仍在应用入口设为 `gpui_kit_assets::Assets`。
- `Size` 枚举：XSmall / Small / Medium（默认）/ Large；`Sizable::with_size(Size::Small)`。

### 初始化顺序
- `gpui_component::init(cx)` 必须先于其它初始化调用（官方要求）；随后 `ui::init(cx)` 注册键位。

### 杂项
- 查 crates.io API（版本号等）需要带 `User-Agent` 头，否则被拒。

## 虚拟列表（列表选型，已核实）

三个候选，**选 `gpui::uniform_list`**，理由如下（都是实测过的接口事实）：

- **`gpui::uniform_list`（gpui 本体）实现了 `InteractiveElement`**
  （`elements/uniform_list.rs:714`）→ 可以 `.on_mouse_down(..)` / `.on_click(..)`，
  也就意味着**空白点击这类「挂在滚动容器自己身上」的处理器仍然可用**。
  这是它胜出的决定性原因（见 agents.md 的 B6 规约：处理器必须落在滚动容器上，
  挂到祖先容器历史上失败过）。
- **`v_virtual_list` / `VirtualList`（gpui-base，经 gpui-component 再导出）
  只实现了 `Styled`**（`gpui-base/src/virtual_list.rs:230`），没有
  `InteractiveElement` → **挂不上任何鼠标事件**。若选它，B6 的清空处理器只能挂到
  祖先 div 上，正是踩过坑的那版。
- **`uniform_list` 按第 0 行（`item_to_measure_index`，默认 0）的高度给所有行排版**：
  `content_height = item_height * item_count`、`item_top = item_height * item_index`
  （`uniform_list.rs:371/397/427`）。所以**所有行必须等高**，行不能再由内容撑开——
  行内多挂一行文字（如校验提示）会让该行比定高更高而被裁掉或与邻行重叠，此时应把
  那行文字移出行外（横幅）。
- **`uniform_list` 的行闭包签名是 `Fn(Range<usize>, &mut Window, &mut App)`——拿不到
  `&mut self`**。用 **`Context::processor`**（`app/context.rs:264`）桥接：
  `cx.processor(|this: &mut Self, range, window, cx| ...)` 会捕获 `entity()` 并在调用时
  `view.update(cx, ..)`，把签名适配成元素要的 `'static` 闭包。闭包在 layout/prepaint
  阶段才被调用（render 已返回），因此不会与 render 期间的借用冲突。
- **`UniformListScrollHandle`**（`UniformListScrollHandle::new()`）配 `.track_scroll(&h)`；
  `scroll_to_item(ix, ScrollStrategy::Nearest)` 是**非严格**滚动（已可见则不动），
  适合作「键盘导航把选中行带进视野」，不会把列表来回拽。
- **`v_virtual_list` 若真要用**：`v_virtual_list(view: Entity<V>, id, item_sizes:
  Rc<Vec<Size<Pixels>>>, f: Fn(&mut V, Range<usize>, &mut Window, &mut Context<V>) -> Vec<R>)`
  ——它**给 `&mut V`**（比 uniform_list 方便），但行高要调用方自己给准，且没有交互能力。
- 行内重命名这类「行内要塞控件」的场景注意：`Input::small()` 的高度是库按
  `Size::Small => h_6()` 固定的 **24px**（`gpui-component/src/sizing.rs:265`），
  **不随字号缩放**；定高行必须为它留量，否则字号放大后输入框会被裁。

## 动效（能力边界，已核实）

- **`with_animation` 已内建 `reduce_motion` 支持（勿重复实现）**：`AnimationExt`
  的文档写明「Animations rendered through this trait automatically respect
  `App::reduce_motion`」，实现见 `elements/animation.rs:407`——命中时取最后一个
  动画且 oneshot 直接给 `delta = 1.0`（即渲染**终态**）、`done = true`、不再排帧。
  `window.rs:2522` 同时提醒：**直接**用 `request_animation_frame` 做装饰性动效时
  才需要自己判 `cx.reduce_motion()`。→ 浮层淡入这类 `with_animation` 动效不必再判。
- **`button::ButtonIcon` 对外不可命名**：`button/mod.rs` 是
  `pub(crate) use button_icon::*;`，所以虽然 `pub struct ButtonIcon` 存在，库外
  无法在签名里写它（`Button::icon(impl Into<ButtonIcon>)` 也因此没法被泛型包装）。
  → 想抽一个「图标按钮」helper，形参要取 `gpui_component::Icon`，不能取
  `ButtonIcon`（本仓库 `ui::icon_button` 即如此）。
- **`Div` 没有 `transform` / `scale` / `rotate`**：`Transformation` 只定义在
  `elements/svg.rs`，由 `Svg::with_transformation` 使用；`Styled` 上没有相关方法。
  只有 SVG 能变换（gpui-component 的 `Icon` 会转发 `transform`，所以 Spinner 能转）。
  → 「点按缩放 `scale(0.96)`」「位移进场 `translateY`」在普通 div 上**做不了**。
- **没有元素级 blur**：gpui 里 `blur` 只作为 `BoxShadow.blur_radius` 存在，没有
  `filter` / `BackdropFilter` 样式。→ 淡入只能靠 opacity，没有 `blur(4px)→0`。
- **`Style.opacity` 是组透明度**：Div 的 paint 走 `window.with_element_opacity`
  （`elements/div.rs`），而 `Window` 内部 `element_opacity = previous * opacity`
  相乘下传。→ 淡化最外层 div 即淡化整棵子树（遮罩 + 卡片一起淡入，无需分别处理）。
- **`Animation` 没有完成回调**：`elements/animation.rs` 只有 `new(Duration)` /
  `with_easing` / `repeat`。一次性动画在 `delta > 1.0` 时置 done，并停止
  `request_animation_frame`（自驱帧、无需父级 notify；done 后不再重绘）。
  → **退场动画必须自建「先播动画 + 定时器再真正卸载」**。照抄 gpui-component
  `notification.rs` 时注意它的定时器（0.15s）短于动画（0.25s），退场被截断——
  定时器应 ≥ 动画时长。
- **动画 state 随元素卸载而丢弃**：state 按「元素 id + 渲染树位置」缓存，且帧末只
  保留本帧访问过的 state（`window.rs` 的 `accessed_element_states` 迁移）。
  → 浮层卸载后重开 = 新 state = 动画重放（想要的行为）；反过来，**要在同一元素上
  重放动画必须改 id**（gpui-component 的 switch/checkbox 用
  `ElementId::NamedInteger("move", checked as u64)` 就是为此）。
- **缓动函数**：gpui 导出 `linear` / `quadratic` / `ease_in_out`（都是
  `fn(f32) -> f32`，可直接传）与工厂 `ease_out_quint()` / `bounce(..)` /
  `pulsating_between(..)`（需调用）。**gpui 本体没有 cubic-bezier**，要用
  `gpui_component::animation::cubic_bezier(x1, y1, x2, y2)`。
- **`AnimationExt`（`with_animation` / `with_animations`）不在 prelude**，须显式
  `use gpui::AnimationExt as _;`。blanket impl 在 `IntoElement` 上，animator 签名是
  `Fn(Self, f32) -> Self`，返回 `AnimationElement<Self>`——**调用后不能再接 `Div`
  的方法**（`key_context` / `child` 等都要在传入前接完）。所以动画必须包在链式装配
  的**最后一步**：本仓库统一用 `overlay::fade_in(id, el)`（overlay.rs）。
- `ElementId` 可由 `&'static str` 转换（`impl From<&'static str> for ElementId`）。
- **行号索引（已在 gpui-pre 0.3.4 上逐条复核）**：`Animation` 构造
  `elements/animation.rs:33/44/50/60`（`new`/`repeat`/`repeat_synced`/`with_easing`，
  仍**无**完成回调）；`AnimationExt` blanket impl `elements/animation.rs:158`；
  组透明度 `elements/div.rs:2489` → `window.rs:3786-3798`
  （`element_opacity = previous * opacity`）；动画 state 卸载即丢弃
  `window.rs:1125`（`accessed_element_states` 只迁移本帧访问过的 key）。

## 几何与绘制（已核实）

- **`overflow_hidden()` 只裁矩形**：`ContentMask` 只有 `bounds`，没有圆角。→ 圆角必须
  落在**自己绘制位图/背景的那个元素**上。`Img` 会把自身 `corner_radii` 传给
  `window.paint_image`，所以 `img().rounded(..)` 生效；而外层 `div().rounded(..) +
  overflow_hidden()` 对图片**无效**（位图仍是方角）。
- **`img` 是叶子节点，border 画在位图之上、bounds 内侧**：`Style::paint` 先内容后边框，
  且 taffy 的 border-box 只内缩**子元素**。→ `img().border_1().border_color(..)` 等价于
  CSS `outline: 1px; outline-offset: -1px`，位图不缩小。若把 border 加在外层 Div 上，
  子元素会被内缩（图像变小），语义变成 `border` 而非 `outline`。
- **阴影环只能向外**：`BoxShadow` 没有 `inset` 字段，`spread_radius` 只向外扩张。→ 做不出
  内描边环，只能靠 border。
- **`Button` 覆盖图标尺寸**：Button 渲染时取自己的 `icon_size` 并调
  `icon.with_size(icon_size)`（`button.rs`），且 `Icon::render` 中显式 size 分支在
  `text_size` 分支**之后**生效。→ 给 Button 的 icon 写 `.size_4()` / `.size_3()` 是**死代码**。
- **`Icon` 不写尺寸时取继承的字号**（`window.text_style().font_size`）→ 图标与相邻文字
  同步随字号缩放；写了 `size_4()` 一类固定值就脱离字号缩放（140% 档位下会比正文还小）。
  按 `agents.md` §5，图标优先继承行字号或用 `tokens::text()` 取值。
- **`Theme::apply_config` 会压 alpha**：`list_active` / `table_active` ≤ 0.2，
  `selection` ≤ 0.3（库 `theme/schema.rs` 末尾）。→ 选中底色**不要**用
  `list_active` / `table_active`（源色太淡，压完肉眼不可见），用 `selection`
  （列表行）或不透明的 `sidebar_accent`（侧栏语义面）。
- **`ThemeConfigColors` 是库里的固定字段集，没有扩展位**：想加自定义色无法经
  `ThemeConfig` 下发，只能在 `theme.rs` 另设语义访问器（如 `theme::image_outline`）。
- **行号索引（已在 gpui-pre 0.3.4 / gpui-component 0.6.1 上逐条复核）**：
  `ContentMask` 只有 `bounds` `window.rs:2081-2084`；`Style::paint` 先
  `continuation` 后边框 `style.rs:742` / `:744`（`is_border_visible` 在 `:764`）；
  `Img` 把自身 `corner_radii` 传给 `paint_image` `elements/img.rs:494`；
  `Icon` 尺寸解析 `icon.rs:169-187`（`has_base_size` 为假时取继承字号，
  `self.size` 分支在后覆盖；`Icon::data(&[u8])` 在 `:136`）；
  alpha clamp `theme/schema.rs:1039-1056`（list/table ≤0.2、selection ≤0.3，
  库自带断言在 `:1300`）。

## 离屏预览 + 截图验收（已实测可用）

gpui 不建 AX 树，UI 内部交互没法脚本化；但**单个视图的视觉**可以脚本化验收，
不必人肉看、也不必抢用户焦点：

```bash
# 1) 例子程序把视图放进一个 focus:false 的窗口（crates/desktop/examples/view_preview.rs）
OSM_PREVIEW_VIEW=search nohup ./target/debug/examples/view_preview > /tmp/preview.log 2>&1 &
PID=$!                     # 用 $!，别 pgrep（见下）
# 2) 按 pid 取该窗口的 CGWindowNumber（swift/CGWindowListCopyWindowInfo）
# 3) 按窗口号抓图——离屏窗口也能抓到内容
screencapture -x -o -l <CGWindowNumber> /tmp/view.png
```

**抓图脚本本身踩过的两个坑**（都让人拿到过「看起来对、其实是上一次的」图）：

- **zsh 不对未加引号的变量做分词**：`env $flags nohup app` 会把整串当一个参数**传给程序**，
  而不是当成环境变量赋值（日志里能看到 `view=search OSM_PREVIEW_DARK=1`）。用
  `env "$@" nohup app` 这类显式传参形式，或直接写 `VAR=1 VAR2=2 cmd`。
- **不要用 `pgrep -f <路径>` 取 pid**：外层 shell 的命令行里就含这个路径，`head -1`
  常常命中 shell 而不是目标进程，于是抓到的是别的窗口。用后台启动后的 `$!`。
  另外进程被杀后它的窗口还会在 `CGWindowList` 里停留一会儿，所以**按窗口尺寸筛**
  （例：预览窗口 W=260 还是 W=520）比取第一个更可靠。

踩过的点：

- **窗口坐标会被系统夹回来**：`Bounds { origin: (-10000, -10000) }` 实际得到
  `X=-480`（macOS 不允许窗口完全移出屏幕，会夹到留一条边可抓）。所以不能靠
  「位置 -10000」判断自己的窗口，**按 pid + CGWindowNumber 找**才可靠。
- `focus: false` 生效：预览窗口不会成为前台应用（实测前台仍是用户原应用）。
  绝不要调 `app.activate(true)`（`crates/desktop/src/main.rs:119` 是主程序为
  `cargo run` 直启补的激活，预览里不能抄）。
- **不要用合成鼠标/键盘去「打开」视图**（CGEvent / System Events keystroke）：
  前台是谁不由你决定，实测两次落到用户正在用的应用上（微信被激活、ChaGPT 被误判）。
  AX 菜单项在非前台时 `enabled=false`，也点不动。
- gpui 自带 `VisualTestContext::capture_screenshot`（Metal 纹理直接读回，不需要窗口可见），
  但它要 `Rc<dyn Platform>` 构造 `VisualTestAppContext`，而本工作区的 `gpui-platform`
  是路径依赖、没有对外暴露 platform 构造入口，`TestPlatform` 又是 CPU-only（截图不支持）。
  所以实际可行的是上面这条「外部窗口 + screencapture -l」。

**颜色不要靠肉眼读，要采样**：gpui 渲染出的 RGB 与 token 原值不完全相等
（`#E6F7FF` → 实测 `#E3F6FE`），直接按 token 值搜像素会一无所获。用直方图找实际色值，
再量包围盒拿几何（本次据此确认两段各 120px、无缝接合）。

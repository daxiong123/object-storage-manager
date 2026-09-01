//! 主窗口三栏 Workspace。
//!
//! 结构（agents.md §7）：Unified Titlebar + Sidebar(180/220/360) + Content + Inspector(280/320/520)。
//! - Sidebar 折叠为 44px 图标栏（规范硬指标；gpui-component 自带 Sidebar 固定 255px/48px，
//!   无法满足，故自建，用其 Icon/主题 token 保持视觉一致）。
//! - 三栏宽度用 gpui-component Resizable；折叠/展开切换布局变体（不同的 resizable group id），
//!   使每种变体各自记住拖拽后的宽度。
//! - Action 处理见 `crate::actions`：⌘⌥S / ⌘⌥I / ⌘W / ⌘Q 与菜单共享同一 Action。

use gpui::{
    AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    MouseButton, MouseDownEvent, ParentElement, Pixels, Render, Styled, Window, div,
    prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size, Theme, TitleBar, button::Button,
    button::ButtonVariants as _, h_flex, resizable::h_resizable, resizable::resizable_panel,
    v_flex,
};

use crate::actions::{CloseWindow, OpenCommandPalette, ToggleInspector, ToggleSidebar};
use crate::command_palette::CommandPaletteView;

/// 左栏折叠后的图标栏宽度（规范：44px Icon Rail）。
const RAIL_WIDTH: Pixels = px(44.);
/// Sidebar 默认宽度（规范：默认 220，范围 180–360）。
const SIDEBAR_DEFAULT: Pixels = px(220.);
const SIDEBAR_MIN: Pixels = px(180.);
const SIDEBAR_MAX: Pixels = px(360.);
/// Inspector 默认宽度（规范：默认 320，范围 280–520）。
const INSPECTOR_DEFAULT: Pixels = px(320.);
const INSPECTOR_MIN: Pixels = px(280.);
const INSPECTOR_MAX: Pixels = px(520.);

pub struct WorkspaceView {
    focus_handle: FocusHandle,
    sidebar_collapsed: bool,
    inspector_collapsed: bool,
    /// 当前打开的命令面板（⌘K）。Some 时在根容器上渲染模态遮罩层；
    /// 面板关闭（open=false）后由此处置 None 并归还焦点。
    palette: Option<Entity<CommandPaletteView>>,
}

impl WorkspaceView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            sidebar_collapsed: false,
            inspector_collapsed: false,
            palette: None,
        }
    }

    // ---- Action 处理（与菜单/快捷键共享） ----

    fn handle_toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        cx.notify();
    }

    fn handle_toggle_inspector(
        &mut self,
        _: &ToggleInspector,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.inspector_collapsed = !self.inspector_collapsed;
        cx.notify();
    }

    fn handle_close_window(
        &mut self,
        _: &CloseWindow,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        // 关闭窗口（gpui 0.2.2 在 macOS 15 上的绕行方案，实测记录见 docs/notes/gpui-api-notes.md）：
        //
        // 根因：macOS 15 上 NSWindow close 默认带窗口动画，而 gpui 的 MacWindow::drop 会在
        // close 后毫秒级 autorelease（dealloc），把动画中途杀死，窗口卡在可见状态——表现为
        // close() 永远关不掉窗口。
        //
        // 修复：先禁用 close 动画（NSWindowAnimationBehaviorNone），再 remove_window()
        // （注册表清理 → drop → gpui 内部 close 任务）。无动画的 [super close] 退化为
        // 纯 orderOut，窗口正常消失，进程与 gpui 窗口注册表状态一致。
        use objc::{msg_send, sel, sel_impl};
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let raw = match window.window_handle() {
            Ok(h) => h.as_raw(),
            Err(_) => return,
        };
        let ns_view = match raw {
            RawWindowHandle::AppKit(h) => h.ns_view.as_ptr(),
            _ => return, // macOS 上不可能走到
        };
        unsafe {
            let view = ns_view as *mut objc::runtime::Object;
            let win: *mut objc::runtime::Object = msg_send![view, window];
            if !win.is_null() {
                let _: () = msg_send![win, setAnimationBehavior: 1i64];
            }
        }
        window.remove_window();
    }

    fn handle_open_command_palette(
        &mut self,
        _: &OpenCommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.is_some() {
            return; // 已打开（⌘K 重复触发为无操作）
        }
        let palette = cx.new(|cx| CommandPaletteView::new(window, cx));
        // 面板关闭（open=false）后由观察者丢弃实体并归还焦点。
        cx.observe_in(&palette, window, Self::handle_palette_changed)
            .detach();
        palette.update(cx, |palette, cx| palette.focus_input(window, cx));
        self.palette = Some(palette);
        cx.notify();
    }

    /// 面板状态观察：面板自己调用 close() 置 open=false 时，这里收尾——
    /// 丢弃实体（遮罩与卡片随之消失）并把焦点归还 Workspace 根节点。
    /// 在 observe 回调里丢弃面板是安全的：回调参数持有的 Entity 让它
    /// 存活到本次调用结束。
    fn handle_palette_changed(
        &mut self,
        palette: Entity<CommandPaletteView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if palette.read(cx).open() {
            return; // 过滤/选行等常规通知，不处理
        }
        self.palette = None;
        window.focus(&self.focus_handle);
        cx.notify();
    }

    // ---- 渲染 ----

    /// 命令面板遮罩：点击空白处关闭；卡片自身的 on_mouse_down 会阻止
    /// 冒泡，所以点卡片内部不会触发这里。
    fn render_palette_overlay(
        &self,
        palette: &Entity<CommandPaletteView>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .absolute()
            .inset_0()
            .occlude() // 挡住下层元素的鼠标交互
            .bg(theme.overlay)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    if let Some(palette) = &this.palette {
                        palette.update(cx, |palette, cx| palette.close(window, cx));
                    }
                }),
            )
            .child(palette.clone())
    }

    fn render_title_bar(&self, _theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let sidebar_icon = if self.sidebar_collapsed {
            IconName::PanelLeftOpen
        } else {
            IconName::PanelLeftClose
        };
        let inspector_icon = if self.inspector_collapsed {
            IconName::PanelRightOpen
        } else {
            IconName::PanelRightClose
        };

        TitleBar::new().child(
            h_flex()
                .w_full()
                .justify_between()
                // 左：Sidebar 开关
                .child(
                    Button::new("toggle-sidebar")
                        .icon(Icon::new(sidebar_icon))
                        .ghost()
                        .with_size(Size::Small)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.handle_toggle_sidebar(&ToggleSidebar, window, cx)
                        })),
                )
                // 中：标题
                .child("CloudStorage")
                // 右：Inspector 开关
                .child(
                    h_flex().gap_1().child(
                        Button::new("toggle-inspector")
                            .icon(Icon::new(inspector_icon))
                            .ghost()
                            .with_size(Size::Small)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_inspector(cx))),
                    ),
                ),
        )
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        cx.notify();
    }

    fn toggle_inspector(&mut self, cx: &mut Context<Self>) {
        self.inspector_collapsed = !self.inspector_collapsed;
        cx.notify();
    }

    fn render_body(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        // 每种「折叠组合」使用独立的 resizable group id：
        // use_keyed_state 按 id 存宽度，展开/折叠切换互不覆盖，各自记住拖拽尺寸。
        let group_id: &'static str = match (self.sidebar_collapsed, self.inspector_collapsed) {
            (false, false) => "workspace-layout-full",
            (true, false) => "workspace-layout-no-sidebar",
            (false, true) => "workspace-layout-no-inspector",
            (true, true) => "workspace-layout-content-only",
        };

        let mut body = h_flex().flex_1().min_h_0();
        if self.sidebar_collapsed {
            body = body.child(self.render_sidebar_rail(theme, cx));
        }

        let mut group = h_resizable(group_id);
        if !self.sidebar_collapsed {
            group = group.child(
                resizable_panel()
                    .size(SIDEBAR_DEFAULT)
                    .size_range(SIDEBAR_MIN..SIDEBAR_MAX)
                    .child(self.render_sidebar(theme, cx).into_any_element()),
            );
        }
        group = group.child(self.render_content(theme, cx).into_any_element());
        if !self.inspector_collapsed {
            group = group.child(
                resizable_panel()
                    .size(INSPECTOR_DEFAULT)
                    .size_range(INSPECTOR_MIN..INSPECTOR_MAX)
                    .child(self.render_inspector(theme, cx).into_any_element()),
            );
        }

        body.child(
            // ResizablePanelGroup 自身渲染为 size_full 容器，需包一层分配剩余空间。
            div().flex_1().min_w_0().h_full().child(group),
        )
    }

    /// 展开态 Sidebar（自建，宽度由 Resizable 面板控制，内容 w_full 填充）。
    fn render_sidebar(&self, theme: &Theme, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .h_full()
            .w_full()
            .overflow_hidden()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .border_r_1()
            .border_color(theme.sidebar_border)
            .child(self.sidebar_section_label(theme, "账户"))
            .child(self.sidebar_row(theme, "nav-qiniu", IconName::Globe, "七牛云 Kodo", true))
            .child(self.sidebar_row(theme, "nav-aliyun", IconName::Globe, "阿里云 OSS", false))
            .child(self.sidebar_section_label(theme, "空间"))
            .child(self.sidebar_row(
                theme,
                "nav-buckets",
                IconName::FolderOpen,
                "Bucket 列表",
                false,
            ))
            .child(self.sidebar_row(theme, "nav-starred", IconName::Star, "收藏", false))
            .child(div().flex_1())
            .child(self.sidebar_row(theme, "nav-settings", IconName::Settings, "设置", false))
            .child(div().h_2())
    }

    fn sidebar_section_label(&self, theme: &Theme, label: &'static str) -> impl IntoElement {
        div()
            .px_3()
            .pt_3()
            .pb_1()
            .text_size(px(11.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(theme.muted_foreground)
            .child(label)
    }

    fn sidebar_row(
        &self,
        theme: &Theme,
        id: &'static str,
        icon: IconName,
        label: &'static str,
        active: bool,
    ) -> impl IntoElement {
        div()
            .id(id)
            .mx_2()
            .px_2()
            .py(px(5.))
            .rounded(px(6.))
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(13.))
            .when(active, |row| {
                row.bg(theme.sidebar_accent)
                    .text_color(theme.sidebar_accent_foreground)
            })
            .hover(|row| row.bg(theme.sidebar_accent))
            .child(Icon::new(icon))
            .child(label)
    }

    /// 折叠态 44px 图标栏（规范硬指标）。点击顶部按钮恢复展开。
    fn render_sidebar_rail(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w(RAIL_WIDTH)
            .h_full()
            .flex_shrink_0()
            .items_center()
            .pt_2()
            .pb_2()
            .gap_2()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .border_r_1()
            .border_color(theme.sidebar_border)
            .child(
                Button::new("rail-expand-sidebar")
                    .icon(Icon::new(IconName::PanelLeftOpen))
                    .ghost()
                    .with_size(Size::Small)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
            )
            .child(Icon::new(IconName::Globe))
            .child(Icon::new(IconName::FolderOpen))
            .child(div().flex_1())
            .child(Icon::new(IconName::Settings))
    }

    /// 中间内容区：后续为 Object Table（占位）。
    fn render_content(&self, theme: &Theme, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .items_center()
            .justify_center()
            .gap_2()
            .bg(theme.background)
            .text_color(theme.muted_foreground)
            .child(Icon::new(IconName::Inbox))
            .child("选择一个 Bucket 查看对象列表")
    }

    /// 右侧 Inspector（占位）。
    fn render_inspector(&self, theme: &Theme, _cx: &mut Context<Self>) -> impl IntoElement {
        let mut panel = v_flex()
            .h_full()
            .w_full()
            .overflow_hidden()
            .bg(theme.background)
            .border_l_1()
            .border_color(theme.border)
            .child(
                div()
                    .px_3()
                    .py_2()
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .border_b_1()
                    .border_color(theme.border)
                    .child("检查器"),
            );
        for (label, value) in [
            ("名称", "—"),
            ("大小", "—"),
            ("类型", "—"),
            ("修改时间", "—"),
        ] {
            panel = panel.child(
                h_flex()
                    .px_3()
                    .py_1()
                    .justify_between()
                    .text_size(px(12.))
                    .child(div().text_color(theme.muted_foreground).child(label))
                    .child(div().child(value)),
            );
        }
        panel
    }
}

impl gpui::Focusable for WorkspaceView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for WorkspaceView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let mut root = v_flex()
            .id("workspace")
            .relative() // 命令面板遮罩层的定位基准
            .size_full()
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::handle_toggle_sidebar))
            .on_action(cx.listener(Self::handle_toggle_inspector))
            .on_action(cx.listener(Self::handle_close_window))
            .on_action(cx.listener(Self::handle_open_command_palette))
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(self.render_title_bar(&theme, cx))
            .child(self.render_body(&theme, cx));

        // 命令面板遮罩层：置于根容器之上，挡住下层交互（规范 §22）。
        if let Some(palette) = self.palette.clone() {
            root = root.child(self.render_palette_overlay(&palette, &theme, cx));
        }
        root
    }
}

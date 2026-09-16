//! Unified Titlebar：导航按钮、当前位置（路径输入 / 面包屑）、过滤与上传入口。
//!
//! **标题栏里的交互控件必须保留 `on_mouse_down(stop_propagation)`**。
//!
//! 统一标题栏（gpui-component `TitleBar`）在 macOS 上把**双击**当窗口缩放
//! （`on_double_click → window.titlebar_double_click()`）。要拦住它，靠的是
//! mouse_down 的 stop 而**不是** click 的 stop：标题栏想触发双击，必须先在
//! **它自己的** mouse_down（bubble 阶段）里置位，才能在 mouse_up 时合成 click；
//! 控件在 mouse_down 就 `stop_propagation()` 之后，标题栏那一步永远不会发生。
//! 见本文件测试 `click_reaches_the_titlebar_when_the_mouse_down_stop_is_removed`
//! 与 `click_stops_at_the_filter_wrapper`。

use super::*;

pub(super) fn breadcrumb_prefixes(prefix: Option<&str>) -> Vec<(String, String)> {
    let Some(prefix) = prefix else {
        return Vec::new();
    };
    let mut path = String::new();
    prefix
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            path.push_str(segment);
            path.push('/');
            (segment.to_string(), path.clone())
        })
        .collect()
}

pub(super) fn collapse_breadcrumb(
    segments: &[(String, String)],
) -> Option<(String, Vec<(String, String)>)> {
    if segments.len() <= BREADCRUMB_MAX_VISIBLE {
        return None;
    }
    // 折叠区为 [1, len-2)；最深被收起段是尾段前两段，其前缀即 `…` 的跳转目标
    let collapsed_prefix = segments[segments.len() - 3].1.clone();
    let tail = segments[segments.len() - 2..].to_vec();
    Some((collapsed_prefix, tail))
}

impl WorkspaceView {
    pub(super) fn handle_focus_path(
        &mut self,
        _: &FocusPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.is_some() || self.add_modal.is_some() || self.settings_modal.is_some() {
            return;
        }
        if self.selected_bucket.is_none() {
            self.download_message = Some(DownloadMessage {
                is_error: true,
                text: "请先选择一个 Bucket 再跳转路径".into(),
            });
            cx.notify();
            return;
        }
        // 已有路径框：只聚焦；否则以当前前缀为初值新建（回车跳转，Esc 关闭）
        if let Some(editor) = &self.path_input {
            editor.update(cx, |state, cx| state.focus(window, cx));
            return;
        }
        let initial = self.current_prefix.clone().unwrap_or_default();
        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("输入路径，如 photos/2024/（以 / 结尾为目录）")
                .default_value(initial)
        });
        cx.subscribe_in(
            &editor,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.commit_path_jump(window, cx);
                }
            },
        )
        .detach();
        editor.update(cx, |state, cx| state.focus(window, cx));
        self.path_input = Some(editor);
        cx.notify();
    }

    /// ⌘L 提交：把输入框内容规范化为前缀后跳转（空 = 根目录）。
    /// `/` 分隔、自动补结尾 `/`（目录语义）；拒绝 `/` 开头（绝对路径形态）。
    pub(super) fn commit_path_jump(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.path_input.clone() else {
            return;
        };
        let raw = editor.read(cx).value().to_string();
        self.path_input = None;
        self.focus_handle.focus(window, cx);
        let trimmed = raw.trim();
        let prefix = if trimmed.is_empty() {
            None
        } else if trimmed.starts_with('/') {
            self.download_message = Some(DownloadMessage {
                is_error: true,
                text: "路径不能以 / 开头（相对当前空间）".into(),
            });
            cx.notify();
            return;
        } else {
            Some(format!("{}/", trimmed.trim_end_matches('/')))
        };
        self.object_filter = None;
        self.filtered_ix = None;
        self.push_nav_history();
        self.current_prefix = prefix;
        self.reload_objects(cx);
    }

    pub(super) fn render_title_bar(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let sidebar_icon = if self.sidebar_collapsed {
            IconName::PanelLeftOpen
        } else {
            IconName::PanelLeftClose
        };
        TitleBar::new().child(
            h_flex()
                .w_full()
                .h_full()
                .items_center()
                .gap_1()
                .pr_2()
                .child(
                    Button::new("toggle-sidebar")
                        .icon(Icon::new(sidebar_icon))
                        .ghost()
                        .with_size(Size::Small)
                        .tooltip(if self.sidebar_collapsed {
                            "展开边栏 ⌘⌥S"
                        } else {
                            "折叠边栏 ⌘⌥S"
                        })
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
                )
                .child(
                    Button::new("nav-back")
                        .icon(Icon::new(IconName::ArrowLeft))
                        .ghost()
                        .with_size(Size::Small)
                        .tooltip("后退 ⌘[")
                        .disabled(self.nav_back.is_empty())
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _, _, cx| this.handle_nav_back(cx))),
                )
                .child(
                    Button::new("nav-forward")
                        .icon(Icon::new(IconName::ArrowRight))
                        .ghost()
                        .with_size(Size::Small)
                        .tooltip("前进 ⌘]")
                        .disabled(self.nav_forward.is_empty())
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _, _, cx| this.handle_nav_forward(cx))),
                )
                .child(self.render_title_location(theme, cx)),
        )
    }

    /// Titlebar 中段：⌘L 路径框，否则当前 bucket / prefix 面包屑。
    pub(super) fn render_title_location(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(editor) = &self.path_input {
            return h_flex()
                .id("title-path")
                .key_context("ObjectFilter")
                .flex_1()
                .min_w_0()
                .h_full()
                .items_center()
                .gap_2()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(div().flex_1().min_w_0().child(Input::new(editor).small()))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(tokens::caption())
                        .text_color(theme.muted_foreground)
                        .child("回车跳转"),
                )
                .into_any_element();
        }
        match self.selected_bucket.as_deref() {
            Some(bucket) => self.render_breadcrumb(theme, bucket, cx).into_any_element(),
            None => div()
                .flex_1()
                .min_w_0()
                .text_size(tokens::body())
                .text_color(theme.muted_foreground)
                .child("CloudStorage")
                .into_any_element(),
        }
    }

    /// Titlebar 面包屑：bucket / prefix…（长路径折叠）。
    pub(super) fn render_breadcrumb(
        &self,
        theme: &Theme,
        bucket: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut path = h_flex()
            .id("title-breadcrumb")
            .flex_1()
            .min_w_0()
            .h_full()
            .items_center()
            .gap_1()
            .overflow_hidden()
            .text_size(tokens::label())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .px_1()
                    .rounded(tokens::radius())
                    .flex_shrink_0()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .hover(|el| el.bg(theme.list_hover))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.open_bucket_root(cx)),
                    )
                    .child(bucket.to_string()),
            );
        let segments = breadcrumb_prefixes(self.current_prefix.as_deref());
        let (collapsed, segments) = match collapse_breadcrumb(&segments) {
            Some((collapsed_prefix, tail)) => {
                let first = segments[0].clone();
                let first_target = first.1.clone();
                path = path
                    .child(div().text_color(theme.muted_foreground).child("/"))
                    .child(
                        div()
                            .px_1()
                            .rounded(tokens::radius())
                            .min_w_0()
                            .truncate()
                            .text_color(theme.muted_foreground)
                            .hover(|el| el.bg(theme.list_hover).text_color(theme.foreground))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.open_prefix(first_target.clone(), cx)
                                }),
                            )
                            .child(first.0),
                    )
                    .child(div().text_color(theme.muted_foreground).child("/"))
                    .child(
                        div()
                            .px_1()
                            .rounded(tokens::radius())
                            .text_color(theme.muted_foreground)
                            .hover(|el| el.bg(theme.list_hover).text_color(theme.foreground))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.open_prefix(collapsed_prefix.clone(), cx)
                                }),
                            )
                            .child("…"),
                    );
                (true, tail)
            }
            None => (false, segments),
        };
        let _ = collapsed;
        for (label, prefix) in segments {
            let target_prefix = prefix.clone();
            path = path
                .child(
                    div()
                        .text_color(theme.muted_foreground)
                        .flex_shrink_0()
                        .child("/"),
                )
                .child(
                    div()
                        .px_1()
                        .rounded(tokens::radius())
                        .min_w_0()
                        .truncate()
                        .text_color(theme.muted_foreground)
                        .hover(|el| el.bg(theme.list_hover).text_color(theme.foreground))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.open_prefix(target_prefix.clone(), cx)
                            }),
                        )
                        .child(label),
                );
        }
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Modifiers, TestAppContext};
    use std::cell::Cell;
    use std::rc::Rc;

    /// 复刻「文本输入框挂在统一标题栏里」的结构：把标题栏那个
    /// 「双击 = 缩放窗口」的处理器换成一个可计数的祖先 click 处理器，
    /// 用来断言输入框里的点击有没有漏到标题栏去。
    struct Harness {
        input: Entity<InputState>,
        ancestor_clicks: Rc<Cell<usize>>,
        /// 是否给包裹层保留 `on_mouse_down(stop_propagation)`——即真正的护栏。
        stop_mouse_down: bool,
    }

    impl Render for Harness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let clicks = self.ancestor_clicks.clone();
            let input = self.input.clone();
            let stop = self.stop_mouse_down;
            div()
                .id("titlebar")
                .size_full()
                // 祖先 = 统一标题栏
                .on_click(move |_, _, _| clicks.set(clicks.get() + 1))
                .child(
                    div()
                        .id("title-filter")
                        .size_full()
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            if stop {
                                cx.stop_propagation();
                            }
                        })
                        .child(Input::new(&input)),
                )
        }
    }

    /// 在输入框上点一下，返回「祖先（标题栏）收到的 click 次数」。
    fn ancestor_clicks(cx: &mut TestAppContext, stop_mouse_down: bool) -> usize {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let counter = Rc::new(Cell::new(0));
        let counter_for_view = counter.clone();
        let (_view, cx) = cx.add_window_view(move |window, cx| {
            let input = cx.new(|cx| InputState::new(window, cx).default_value("hello world"));
            input.update(cx, |state, cx| state.focus(window, cx));
            Harness {
                input,
                ancestor_clicks: counter_for_view,
                stop_mouse_down,
            }
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let at = point(px(10.), px(10.));
        cx.simulate_mouse_move(at, None, Modifiers::default());
        cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
        cx.run_until_parked();
        counter.get()
    }

    /// 护栏的**必要性**：去掉 `on_mouse_down(stop_propagation)` 后，输入框里的
    /// 点击会冒泡到标题栏，被它当成「双击 = 缩放窗口」。
    #[gpui::test]
    fn click_reaches_the_titlebar_when_the_mouse_down_stop_is_removed(cx: &mut TestAppContext) {
        assert_eq!(ancestor_clicks(cx, false), 1);
    }

    /// 护栏的**有效性**：保留它，点击就止步于输入框的包裹层。
    ///
    /// 机制（实测确认过，别再按直觉改）：标题栏的 `on_double_click` 想触发，
    /// 必须先在**它自己的** mouse_down（bubble 阶段）里置位，才能在 mouse_up
    /// 时合成 click；而包裹层在 mouse_down 就 `stop_propagation()` 了，标题栏
    /// 那一步永远不会发生。所以**不需要**额外再拦 click——
    /// 「在输入框里双击选词把窗口缩放掉」这个猜想已被证伪（对照组就说明了这点：
    /// 少了 mouse_down 的 stop 才会漏）。
    #[gpui::test]
    fn click_stops_at_the_filter_wrapper(cx: &mut TestAppContext) {
        assert_eq!(ancestor_clicks(cx, true), 0);
    }
}

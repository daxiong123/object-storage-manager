//! Unified Titlebar：侧栏开关、后退/前进、当前位置（路径输入 / 面包屑）。
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

/// 传输摘要按钮的 tooltip：点它展开/收起传输面板。
fn transfer_panel_tooltip(expanded: bool) -> &'static str {
    if expanded {
        "收起传输面板"
    } else {
        "展开传输面板"
    }
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
        self.search_input = None;
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
        // 标题栏是窗口级导航，不是产品名牌，也不是第二条地址栏。
        // 传输只在真有未完成任务时出现——空闲时那句「当前暂无传输任务」是噪音。
        //
        // 判据用 `TransferState::is_active()`（Queued / Running / Waiting / Paused），
        // 与 quit.rs、transfers.rs 保持一致。**不要**手写 matches! 列表并漏掉 Queued：
        // 并发打满或引擎挂起时任务会停在 Queued，漏掉它会在「只剩排队任务」时
        // 把整个传输入口从标题栏抹掉。
        let active_transfers = self
            .transfers
            .iter()
            .filter(|t| t.state.is_active())
            .count();
        let (done, total) = self.transfers.iter().filter(|t| t.state.is_active()).fold(
            (0u64, 0u64),
            |(done, total), task| match task.bytes_total {
                Some(task_total) if task_total > 0 => {
                    (done + task.bytes_done.min(task_total), total + task_total)
                }
                _ => (done, total),
            },
        );
        let progress = (total > 0).then(|| done as f32 / total as f32);

        TitleBar::new().child(
            h_flex()
                .w_full()
                .h_full()
                .items_center()
                .gap_1()
                .px_2()
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
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .flex()
                        .items_center()
                        .child(self.render_title_location(theme, cx)),
                )
                .when(active_transfers > 0, |bar| {
                    // 真按钮而不是带 hover/click 的 div：传输面板入口必须能 Tab 到、
                    // 有可见焦点环、并支持 Enter / Space 激活（§7 可访问性）。
                    bar.child(
                        Button::new("title-transfers")
                            .ghost()
                            .with_size(Size::Small)
                            .tooltip(transfer_panel_tooltip(self.transfers_expanded))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.transfers_expanded = !this.transfers_expanded;
                                cx.notify();
                            }))
                            .child(
                                h_flex()
                                    .flex_shrink_0()
                                    .items_center()
                                    .gap_2()
                                    .text_size(tokens::caption())
                                    .text_color(theme.muted_foreground)
                                    .child(
                                        // 展开态朝上、收起态朝下：图标随面板状态变，
                                        // 不额外加文字（标签已是「N 项传输中」）。
                                        Icon::new(if self.transfers_expanded {
                                            IconName::ChevronUp
                                        } else {
                                            IconName::ChevronDown
                                        })
                                        .text_color(theme.muted_foreground),
                                    )
                                    .child(format!("{active_transfers} 项传输中"))
                                    .when_some(progress, |row, ratio| {
                                        row.child(
                                            div()
                                                .w(tokens::text(72.))
                                                .h(tokens::text(3.))
                                                .rounded(tokens::radius())
                                                .bg(theme.border)
                                                .child(
                                                    div()
                                                        .h_full()
                                                        .rounded(tokens::radius())
                                                        .bg(theme.primary)
                                                        .w(tokens::text(
                                                            72. * ratio.clamp(0.0, 1.0),
                                                        )),
                                                ),
                                        )
                                    }),
                            ),
                    )
                }),
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
                .h_full()
                .flex()
                .items_center()
                .text_size(tokens::body())
                .text_color(theme.muted_foreground)
                .child("选择空间")
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
        // 当前位置就是窗口标题：bucket / 路径。不画 `oss://` 这类协议前缀——
        // 那是控制台地址，不是用户正在看的对象。当前段用前景色，祖先段用次要色。
        //
        // **容器本身不拦鼠标**：`TitleBar` 的拖拽与双击缩放都从它自己的鼠标处理器
        // 开始，整条面包屑 `stop_propagation` 会把标题栏中段整块拖拽区吃掉。拦截只
        // 放在真正可点的段上（bucket / 各路径段 / ⌘L 输入框）；段与段之间的空隙、
        // 以及路径右侧的空白，仍然留给窗口拖拽。
        let segments = breadcrumb_prefixes(self.current_prefix.as_deref());
        let at_root = segments.is_empty();
        // 凹陷槽（Workbench v2）：整条面包屑坐在一条比 chrome 更浅（亮）/更深（暗）
        // 的井里，当前位置读成「嵌在标题栏里的地址槽」，而不是直接浮在 chrome 上的
        // 文字。槽体没有鼠标处理器，不拦截事件——窗口拖拽/双击缩放照旧冒泡到
        // TitleBar（见上方注释），拦截仍只落在可点的段上。
        let mut path = h_flex()
            .id("title-breadcrumb")
            .flex_1()
            .min_w_0()
            .h(tokens::text(28.))
            .px_2()
            .rounded(tokens::radius())
            .bg(crate::theme::inset_background(theme.mode))
            .items_center()
            .gap_1()
            .overflow_hidden()
            .text_size(tokens::body())
            .child(
                div()
                    .px_1()
                    .rounded(tokens::radius())
                    .flex_shrink_0()
                    .when(at_root, |el| {
                        el.text_color(theme.foreground)
                            .font_weight(gpui::FontWeight::MEDIUM)
                    })
                    .when(!at_root, |el| el.text_color(theme.muted_foreground))
                    .hover(|el| el.bg(theme.list_hover).text_color(theme.foreground))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.open_bucket_root(cx)
                        }),
                    )
                    .child(bucket.to_string()),
            );
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
                                    cx.stop_propagation();
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
                                    cx.stop_propagation();
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
        let count = segments.len();
        for (ix, (label, prefix)) in segments.into_iter().enumerate() {
            let current = ix + 1 == count;
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
                        .when(current, |el| {
                            el.text_color(theme.foreground)
                                .font_weight(gpui::FontWeight::MEDIUM)
                        })
                        .when(!current, |el| el.text_color(theme.muted_foreground))
                        .hover(|el| el.bg(theme.list_hover).text_color(theme.foreground))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
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

    /// 面包屑容器**不能**整体 `stop_propagation`。
    ///
    /// 标题栏中段是窗口的主要拖拽 / 双击缩放区。整条面包屑拦截 mouse_down 会让
    /// 这块区域失效，只剩控件周围一圈 padding 可用（PR #4 review 指出）。拦截只
    /// 放在真正可点的段上。这条测试盯源码里的结构约束——比肉眼看渲染可靠。
    #[test]
    fn breadcrumb_container_does_not_block_titlebar_dragging() {
        let source = include_str!("titlebar.rs");
        let container = source
            .split("let mut path = h_flex()")
            .nth(1)
            .and_then(|tail| tail.split(".child(").next())
            .expect("找不到面包屑容器");
        assert!(
            !container.contains("stop_propagation"),
            "面包屑容器不该拦截 mouse_down，否则标题栏中段无法拖拽/双击缩放"
        );
        // 可点的段仍然各自拦截：否则点段会同时被标题栏当成拖拽起点。
        assert!(
            source.matches("cx.stop_propagation();").count() >= 4,
            "各可点面包屑段（bucket / 折叠首段 / … / 路径段）应各自 stop_propagation"
        );
    }

    /// 传输入口在**只剩排队任务**时也必须出现。
    ///
    /// 并发打满或引擎挂起时任务会停在 `Queued`。标题栏若手写 matches! 并漏掉它，
    /// 这个状态下入口会被整个抹掉（PR #4 review 指出）。判据统一走 `is_active()`，
    /// 与 quit.rs / transfers.rs 一致——这里把每个状态的口径逐个钉死。
    #[test]
    fn queued_transfers_count_as_active() {
        use object_storage_transfer::TransferState;
        for state in [
            TransferState::Queued,
            TransferState::Running,
            TransferState::Waiting,
            TransferState::Paused,
        ] {
            assert!(
                state.is_active(),
                "{state:?} 是未完成态，标题栏必须把它算进「N 项传输中」，否则入口会消失"
            );
        }
        for state in [
            TransferState::Completed,
            TransferState::Failed,
            TransferState::Cancelled,
        ] {
            assert!(!state.is_active(), "{state:?} 是终态，不该让入口常驻");
        }
    }

    /// 传输面板入口必须是**可聚焦、可用键盘激活**的控件。
    ///
    /// 带 hover / click 的裸 `h_flex` 键盘用户够不着（PR #4 review 指出）。这里锁住
    /// 两点：一是它由 `Button` 承载（Button 自带 focus ring + Tab + Enter/Space），
    /// 二是它有 tooltip 名称，图标按钮不至于只剩一个图标。
    #[test]
    fn transfer_entry_is_a_real_button() {
        let source = include_str!("titlebar.rs");
        let block = source
            .split("fn render_title_bar")
            .nth(1)
            .and_then(|tail| tail.split("fn render_title_location").next())
            .expect("找不到 render_title_bar");
        assert!(
            block.contains(r#"Button::new("title-transfers")"#),
            "传输入口必须渲染为 Button，才能被键盘聚焦与激活"
        );
        assert!(
            block.contains("transfer_panel_tooltip("),
            "图标+文字的传输入口需要 tooltip 说明其作用"
        );
        assert_eq!(transfer_panel_tooltip(true), "收起传输面板");
        assert_eq!(transfer_panel_tooltip(false), "展开传输面板");
    }
}

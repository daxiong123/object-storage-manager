//! Unified Titlebar：导航按钮、当前位置（路径输入 / 面包屑）、过滤与上传入口。

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
        let has_bucket = self.selected_bucket.is_some();
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
                .child(self.render_title_location(theme, cx))
                .child(self.render_title_trailing(theme, has_bucket, cx)),
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

    pub(super) fn render_title_trailing(
        &self,
        theme: &Theme,
        has_bucket: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        h_flex()
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .when(has_bucket, |row| {
                row.child(self.render_title_filter(theme, cx))
                    .child(
                        Button::new("toolbar-upload-files")
                            .icon(Icon::new(IconName::ArrowUp))
                            .label(if self.uploading {
                                "选择文件…"
                            } else {
                                "上传"
                            })
                            .with_size(Size::Small)
                            .disabled(self.uploading)
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(|this, _, _, cx| this.start_files_upload(cx))),
                    )
                    .child(
                        div()
                            .relative()
                            .child(
                                Button::new("toolbar-more")
                                    .icon(Icon::new(IconName::Ellipsis))
                                    .ghost()
                                    .with_size(Size::Small)
                                    .tooltip("更多操作")
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.toggle_top_more_menu(cx)),
                                    ),
                            )
                            .when(self.top_more_open, |button| {
                                button.child(deferred(
                                    anchored()
                                        .anchor(Anchor::TopRight)
                                        .offset(point(px(0.), px(4.)))
                                        .snap_to_window_with_margin(px(8.))
                                        .child(self.render_top_more_menu(theme, cx)),
                                ))
                            }),
                    )
            })
    }

    pub(super) fn render_title_filter(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        if let Some(editor) = &self.object_filter {
            return h_flex()
                .id("title-filter")
                .key_context("ObjectFilter")
                .w(tokens::text(220.))
                .items_center()
                .gap_1()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(div().flex_1().min_w_0().child(Input::new(editor).small()))
                .child(
                    Button::new("filter-close")
                        .icon(Icon::new(IconName::Close))
                        .ghost()
                        .with_size(Size::Small)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.close_object_filter(window, cx);
                        })),
                )
                .into_any_element();
        }
        let _ = theme;
        Button::new("objects-filter")
            .icon(Icon::new(IconName::Search))
            .ghost()
            .with_size(Size::Small)
            .tooltip("过滤 ⌘F")
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, window, cx| {
                this.handle_toggle_object_filter(&ToggleObjectFilter, window, cx);
            }))
            .into_any_element()
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

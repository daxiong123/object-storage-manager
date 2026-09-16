//! Sidebar：展开态（账号/空间分组）与折叠态 44px 图标栏，以及通用侧栏行。
//!
//! 侧栏行点击的 listener 由调用点的 `cx.listener` 适配（`on_click` 需要 gpui
//! App 级闭包，helper 拿不到 Context），无需额外适配函数。

use super::*;

pub(super) fn provider_icon(kind: ProviderKind) -> IconName {
    match kind {
        ProviderKind::Qiniu => IconName::Globe,
        ProviderKind::Aliyun => IconName::Building2,
    }
}

impl WorkspaceView {
    pub(super) fn handle_toggle_sidebar(
        &mut self,
        _: &ToggleSidebar,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        cx.notify();
    }

    pub(super) fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        cx.notify();
    }

    /// 侧栏分组标题（"账户"/"空间"）。
    pub(super) fn sidebar_section_label(&self, theme: &Theme, label: &str) -> impl IntoElement {
        div()
            .px_3()
            .pt_2()
            .pb_1()
            .text_size(tokens::caption())
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(theme.muted_foreground)
            .child(label.to_string())
    }

    /// 展开态 Sidebar（自建，宽度由 Resizable 面板控制，内容 w_full 填充）。
    pub(super) fn render_sidebar(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .h_full()
            .w_full()
            .overflow_hidden()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .border_r_1()
            .border_color(theme.sidebar_border)
            .child(
                v_flex()
                    .id("sidebar-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(self.sidebar_section_label(theme, "账户"))
                    .children(self.render_account_rows(theme, cx))
                    .child(self.render_add_account_row(theme, cx))
                    .child(self.sidebar_section_label(theme, "空间"))
                    .children(if self.selected_account_id.is_some() {
                        self.render_bucket_rows(theme, cx)
                    } else {
                        vec![
                            div()
                                .px_3()
                                .py_1()
                                .text_size(tokens::label())
                                .text_color(theme.muted_foreground)
                                .child("先选择一个账号")
                                .into_any_element(),
                        ]
                    }),
            )
    }

    /// 账户区：加载态 / 错误重试 / 真实账号行（点击选中并加载空间）。
    pub(super) fn render_account_rows(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        match &self.accounts_state {
            AsyncState::Loading => vec![
                self.sidebar_status_row(theme, "正在加载账号…")
                    .into_any_element(),
            ],
            AsyncState::Failed(msg) => vec![
                self.sidebar_error_row(
                    theme,
                    "sidebar-accounts-error",
                    msg,
                    cx.listener(|this, _, _, cx| this.load_accounts(cx)),
                )
                .into_any_element(),
            ],
            AsyncState::Idle => {
                if self.accounts.is_empty() {
                    return vec![
                        div()
                            .px_3()
                            .py_1()
                            .text_size(tokens::label())
                            .text_color(theme.muted_foreground)
                            .child("还没有账号，点下方添加")
                            .into_any_element(),
                    ];
                }
                self.accounts
                    .iter()
                    .enumerate()
                    .map(|(ix, account)| {
                        let active =
                            self.selected_account_id.as_deref() == Some(account.id.as_str());
                        let id = account.id.clone();
                        self.sidebar_row(
                            theme,
                            SharedString::from(format!("account-row-{ix}")),
                            provider_icon(account.provider),
                            &account.name,
                            active,
                            cx.listener(move |this, _, _, cx| this.select_account(&id, cx)),
                        )
                        .into_any_element()
                    })
                    .collect()
            }
        }
    }

    /// 「+ 添加账号」入口（与命令面板共享 AddAccount Action）。
    pub(super) fn render_add_account_row(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id("sidebar-add-account")
            .mx_2()
            .mt_1()
            .px_2()
            .py(tokens::row_pad_y_sidebar())
            .rounded(tokens::radius())
            .flex()
            .items_center()
            .gap_2()
            .text_size(tokens::body())
            .text_color(theme.muted_foreground)
            .hover(|row| row.bg(theme.list_hover).text_color(theme.foreground))
            .on_click(cx.listener(|this, _, window, cx| {
                this.handle_open_add_modal(&AddAccount, window, cx);
            }))
            .child(Icon::new(IconName::Plus))
            .child("添加账号")
    }

    /// 空间区：加载态 / 错误重试 / 真实桶行（点击选中并加载对象）。
    pub(super) fn render_bucket_rows(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        match &self.buckets_state {
            AsyncState::Loading => vec![
                self.sidebar_status_row(theme, "正在加载空间…")
                    .into_any_element(),
            ],
            AsyncState::Failed(msg) => {
                let mut rows = vec![
                    self.sidebar_error_row(
                        theme,
                        "sidebar-buckets-error",
                        msg,
                        cx.listener(|this, _, _, cx| this.retry_buckets(cx)),
                    )
                    .into_any_element(),
                ];
                if msg.contains("填写") || msg.contains("Bucket") {
                    if let Some(input) = &self.manual_bucket_input {
                        rows.push(
                            v_flex()
                                .px_2()
                                .pt_1()
                                .gap_1()
                                .child(
                                    div()
                                        .text_size(tokens::caption())
                                        .text_color(theme.muted_foreground)
                                        .child("空间名称"),
                                )
                                .child(Input::new(input))
                                .child(
                                    Button::new("add-manual-bucket")
                                        .label("添加空间")
                                        .primary()
                                        .with_size(Size::Small)
                                        .on_click(
                                            cx.listener(|this, _, _, cx| {
                                                this.add_manual_bucket(cx)
                                            }),
                                        ),
                                )
                                .into_any_element(),
                        );
                    } else {
                        rows.push(
                            div()
                                .px_2()
                                .pt_1()
                                .child(
                                    Button::new("open-manual-bucket")
                                        .label("输入 Bucket 名称…")
                                        .with_size(Size::Small)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.manual_bucket_input = Some(cx.new(|cx| {
                                                InputState::new(window, cx)
                                                    .placeholder("Bucket 名称")
                                            }));
                                            cx.notify();
                                        })),
                                )
                                .into_any_element(),
                        );
                    }
                }
                rows
            }
            AsyncState::Idle => {
                if self.buckets.is_empty() {
                    return vec![
                        div()
                            .px_3()
                            .py_1()
                            .text_size(tokens::label())
                            .text_color(theme.muted_foreground)
                            .child("（此账号没有空间）")
                            .into_any_element(),
                    ];
                }
                self.buckets
                    .iter()
                    .enumerate()
                    .map(|(ix, bucket)| {
                        let active = self.selected_bucket.as_deref() == Some(bucket.name.as_str());
                        let name = bucket.name.clone();
                        self.sidebar_row(
                            theme,
                            SharedString::from(format!("bucket-row-{ix}")),
                            IconName::Folder,
                            &bucket.name,
                            active,
                            cx.listener(move |this, _, _, cx| this.select_bucket(&name, cx)),
                        )
                        .into_any_element()
                    })
                    .collect()
            }
        }
    }

    /// 非交互状态行（加载中）。
    pub(super) fn sidebar_status_row(
        &self,
        theme: &Theme,
        label: &'static str,
    ) -> impl IntoElement {
        h_flex()
            .mx_2()
            .px_2()
            .py(tokens::row_pad_y_sidebar())
            .gap_2()
            .text_size(tokens::label())
            .text_color(theme.muted_foreground)
            .child(Spinner::new().with_size(Size::Small))
            .child(label)
    }

    /// 可点击的错误行（点击重试）。窄侧栏里必须换行，不能截成半个词。
    pub(super) fn sidebar_error_row<F>(
        &self,
        theme: &Theme,
        id: &'static str,
        msg: &str,
        on_click: F,
    ) -> impl IntoElement
    where
        F: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    {
        // 图标单独一列、文案（消息 + 「点击重试」）同列堆叠：对齐由结构
        // 决定，不用手算图标宽度做 pl 偏移——那种偏移在图标尺寸或字距
        // 变化后必然错位。
        h_flex()
            .id(id)
            .mx_2()
            .px_2()
            .py_2()
            .gap_2()
            .items_start()
            .rounded(tokens::radius())
            .text_size(tokens::label())
            .text_color(theme.danger)
            .hover(|row| row.bg(theme.list_hover))
            .on_click(on_click)
            .child(Icon::new(IconName::TriangleAlert))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(msg.to_string())
                    .child(
                        div()
                            .text_size(tokens::caption())
                            .text_color(theme.muted_foreground)
                            .child("点击重试"),
                    ),
            )
    }

    /// 通用侧栏行（id 动态：账号/桶行）。
    pub(super) fn sidebar_row<F>(
        &self,
        theme: &Theme,
        id: SharedString,
        icon: IconName,
        label: &str,
        active: bool,
        on_click: F,
    ) -> impl IntoElement
    where
        F: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    {
        div()
            .id(id)
            .mx_2()
            .px_2()
            .py(tokens::row_pad_y_sidebar())
            .rounded(tokens::radius())
            .flex()
            .items_center()
            .gap_2()
            .text_size(tokens::body())
            .when(active, |row| {
                row.bg(theme.sidebar_accent)
                    .text_color(theme.sidebar_accent_foreground)
            })
            .when(!active, |row| row.hover(|row| row.bg(theme.list_hover)))
            .on_click(on_click)
            .child(Icon::new(icon))
            .child(div().truncate().child(label.to_string()))
    }

    /// 折叠态 44px 图标栏（规范硬指标）。顶部展开，下方账号/空间图标入口。
    pub(super) fn render_sidebar_rail(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut rail = v_flex()
            .id("sidebar-rail")
            .w(RAIL_WIDTH)
            .h_full()
            .flex_shrink_0()
            .items_center()
            .pt_2()
            .pb_2()
            .gap_1()
            .overflow_y_scroll()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .border_r_1()
            .border_color(theme.sidebar_border)
            .child(
                Button::new("rail-expand-sidebar")
                    .icon(Icon::new(IconName::PanelLeftOpen))
                    .ghost()
                    .with_size(Size::Small)
                    .tooltip("展开边栏 ⌘⌥S")
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
            );

        for (ix, account) in self.accounts.iter().enumerate() {
            let active = self.selected_account_id.as_deref() == Some(account.id.as_str());
            let id = account.id.clone();
            let name = account.name.clone();
            rail = rail.child(
                Button::new(("rail-account", ix))
                    .icon(Icon::new(provider_icon(account.provider)))
                    .ghost()
                    .with_size(Size::Small)
                    .tooltip(name)
                    .when(active, |btn| {
                        btn.bg(theme.sidebar_accent)
                            .text_color(theme.sidebar_accent_foreground)
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.select_account(&id, cx))),
            );
        }

        if self.selected_account_id.is_some() && !self.buckets.is_empty() {
            rail = rail.child(div().w(px(16.)).h(px(1.)).my_1().bg(theme.sidebar_border));
            for (ix, bucket) in self.buckets.iter().enumerate() {
                let active = self.selected_bucket.as_deref() == Some(bucket.name.as_str());
                let name = bucket.name.clone();
                let tooltip = name.clone();
                rail = rail.child(
                    Button::new(("rail-bucket", ix))
                        .icon(Icon::new(IconName::Folder))
                        .ghost()
                        .with_size(Size::Small)
                        .tooltip(tooltip)
                        .when(active, |btn| {
                            btn.bg(theme.sidebar_accent)
                                .text_color(theme.sidebar_accent_foreground)
                        })
                        .on_click(cx.listener(move |this, _, _, cx| this.select_bucket(&name, cx))),
                );
            }
        }

        rail
    }
}

//! 菜单类浮层：对象右键菜单、Titlebar more 菜单、对象详情、关于。

use super::*;

pub(super) fn object_menu_items() -> Vec<ObjectMenuItem> {
    vec![
        ObjectMenuItem::Details,
        ObjectMenuItem::CopyUrl,
        ObjectMenuItem::Download,
        ObjectMenuItem::Rename,
        ObjectMenuItem::CopyTo,
        ObjectMenuItem::MoveTo,
        ObjectMenuItem::Delete,
    ]
}

pub(super) fn top_more_menu_items() -> Vec<TopMoreMenuItem> {
    vec![
        TopMoreMenuItem::UploadFolder,
        TopMoreMenuItem::CreateFolder,
        TopMoreMenuItem::CopyTo,
        TopMoreMenuItem::MoveTo,
        TopMoreMenuItem::Delete,
    ]
}

pub(super) fn object_menu_item_label(item: ObjectMenuItem) -> &'static str {
    match item {
        ObjectMenuItem::Details => "详情",
        ObjectMenuItem::CopyUrl => "获取地址",
        ObjectMenuItem::Download => "下载",
        ObjectMenuItem::Rename => "重命名",
        ObjectMenuItem::CopyTo => "复制到",
        ObjectMenuItem::MoveTo => "移动到",
        ObjectMenuItem::Delete => "删除",
    }
}

pub(super) fn top_more_menu_item_label(item: TopMoreMenuItem) -> &'static str {
    match item {
        TopMoreMenuItem::UploadFolder => "上传文件夹…",
        TopMoreMenuItem::CreateFolder => "新建目录…",
        TopMoreMenuItem::CopyTo => "复制到…",
        TopMoreMenuItem::MoveTo => "移动到…",
        TopMoreMenuItem::Delete => "删除",
    }
}

pub(super) fn object_menu_item_icon(item: ObjectMenuItem) -> IconName {
    match item {
        ObjectMenuItem::Details => IconName::Info,
        ObjectMenuItem::CopyUrl => IconName::ExternalLink,
        ObjectMenuItem::Download => IconName::ArrowDown,
        ObjectMenuItem::Rename => IconName::Replace,
        ObjectMenuItem::CopyTo => IconName::Copy,
        ObjectMenuItem::MoveTo => IconName::Folder,
        ObjectMenuItem::Delete => IconName::Delete,
    }
}

/// 「关于」弹层信息行：小标签 + 说明文字。
pub(super) fn about_kv(label: &'static str, value: &'static str, theme: &Theme) -> gpui::Div {
    v_flex()
        .gap_0p5()
        .child(
            div()
                .text_size(tokens::caption())
                .text_color(theme.muted_foreground)
                .child(label),
        )
        .child(
            div()
                .text_size(tokens::body())
                .text_color(theme.foreground)
                .child(value),
        )
}

impl WorkspaceView {
    /// 打开对象菜单。`at` 是触发这次打开的窗口坐标（右键位置），菜单以它为锚点。
    pub(super) fn open_object_menu(
        &mut self,
        key: &str,
        at: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if !self.selected_object_keys.contains(key) {
            self.select_object_for_row_action(key);
        }
        self.object_menu_open = Some(key.to_string());
        self.object_menu_at = Some(at);
        self.preview_overlay_open = false;
        self.details_overlay_open = false;
        cx.notify();
    }

    pub(super) fn toggle_object_menu(
        &mut self,
        key: &str,
        at: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.object_menu_open.as_deref() == Some(key) {
            self.object_menu_open = None;
            self.object_menu_at = None;
            cx.notify();
        } else {
            self.open_object_menu(key, at, cx);
        }
    }

    pub(super) fn toggle_top_more_menu(&mut self, cx: &mut Context<Self>) {
        self.top_more_open = !self.top_more_open;
        self.object_menu_open = None;
        cx.notify();
    }

    pub(super) fn close_top_more_menu(&mut self) {
        self.top_more_open = false;
    }

    /// 对象详情弹层：打开聚焦弹层（Esc 经焦点链命中 Overlay context），
    /// 关闭归还 Workspace 根。
    pub(super) fn open_details_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_cloud_object().is_none() {
            return;
        }
        self.object_menu_open = None;
        self.preview_overlay_open = false;
        self.details_overlay_open = true;
        window.focus(&self.overlay_focus, cx);
        cx.notify();
    }

    pub(super) fn close_details_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.details_overlay_open = false;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    pub(super) fn handle_object_menu_item(
        &mut self,
        item: ObjectMenuItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.object_menu_open = None;
        match item {
            ObjectMenuItem::Details => self.open_details_overlay(window, cx),
            ObjectMenuItem::CopyUrl => self.copy_object_url(cx),
            ObjectMenuItem::Download => self.start_object_download(window, cx),
            ObjectMenuItem::Rename => self.handle_rename_object(&RenameObject, window, cx),
            ObjectMenuItem::CopyTo => self.open_copy_move_overlay(CopyMoveMode::Copy, window, cx),
            ObjectMenuItem::MoveTo => self.open_copy_move_overlay(CopyMoveMode::Move, window, cx),
            ObjectMenuItem::Delete => self.confirm_and_delete_object(window, cx),
        }
        cx.notify();
    }

    pub(super) fn handle_top_more_menu_item(
        &mut self,
        item: TopMoreMenuItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_top_more_menu();
        match item {
            TopMoreMenuItem::UploadFolder => self.start_folder_upload(cx),
            TopMoreMenuItem::CreateFolder => self.open_create_folder_overlay(window, cx),
            TopMoreMenuItem::CopyTo => self.open_copy_move_overlay(CopyMoveMode::Copy, window, cx),
            TopMoreMenuItem::MoveTo => self.open_copy_move_overlay(CopyMoveMode::Move, window, cx),
            TopMoreMenuItem::Delete => self.confirm_and_delete_object(window, cx),
        }
        cx.notify();
    }

    pub(super) fn render_object_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let mut menu = ui::menu::popup(theme);

        for (item_ix, item) in object_menu_items().into_iter().enumerate() {
            let color = if item == ObjectMenuItem::Delete {
                theme.danger
            } else {
                theme.foreground
            };
            menu = menu.child(
                ui::menu::item(theme, ("object-menu-item", item_ix), color, true)
                    .child(Icon::new(object_menu_item_icon(item)).text_color(color))
                    .child(object_menu_item_label(item))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.handle_object_menu_item(item, window, cx)
                    })),
            );
        }
        overlay::fade_in("object-context-menu", menu).into_any_element()
    }

    pub(super) fn render_top_more_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let has_selection = !self.selected_object_keys_vec().is_empty();
        let has_bucket = self.selected_bucket.is_some();
        let mut menu = ui::menu::popup(theme);

        for (item_ix, item) in top_more_menu_items().into_iter().enumerate() {
            if item_ix == 2 {
                menu = menu.child(ui::menu::separator(theme));
            }
            let disabled = match item {
                TopMoreMenuItem::UploadFolder => !has_bucket || self.uploading,
                TopMoreMenuItem::CreateFolder => !has_bucket || self.creating_folder,
                TopMoreMenuItem::CopyTo | TopMoreMenuItem::MoveTo | TopMoreMenuItem::Delete => {
                    !has_selection
                }
            };
            let color = if disabled {
                theme.muted_foreground
            } else if item == TopMoreMenuItem::Delete {
                theme.danger
            } else {
                theme.foreground
            };
            menu = menu.child(
                ui::menu::item(theme, ("top-more-menu-item", item_ix), color, !disabled)
                    .child(top_more_menu_item_label(item))
                    .when(!disabled, |row| {
                        row.on_click(cx.listener(move |this, _, window, cx| {
                            this.handle_top_more_menu_item(item, window, cx)
                        }))
                    }),
            );
        }
        overlay::fade_in("top-more-menu", menu).into_any_element()
    }

    pub(super) fn render_details_overlay(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(object) = self.selected_cloud_object() else {
            return div().into_any_element();
        };
        let rows = [
            ("名称", display_name(&object.key).to_string(), false),
            ("Key", object.key.clone(), true),
            ("大小", format_size(object.size), false),
            (
                "文件类型",
                object
                    .mime_type
                    .clone()
                    .unwrap_or_else(|| "未知类型".into()),
                false,
            ),
            (
                "ETag",
                object.etag.clone().unwrap_or_else(|| "-".into()),
                true,
            ),
            ("上传时间", format_time(object.put_time_millis), false),
        ];

        let mut content = v_flex()
            .mx_4()
            .mb_4()
            .border_1()
            .border_color(theme.border)
            .rounded(tokens::radius_lg());
        for (label, value, mono) in rows {
            let mut value_el = div().flex_1().min_w_0().truncate().child(value);
            if mono {
                value_el = value_el
                    .font_family(theme.mono_font_family.clone())
                    .text_size(theme.mono_font_size);
            }
            content = content.child(
                h_flex()
                    .px_4()
                    .py_2()
                    .gap_4()
                    .border_b_1()
                    .border_color(theme.table_row_border)
                    .text_size(tokens::body())
                    .child(
                        div()
                            .w(tokens::text(96.))
                            .flex_shrink_0()
                            .text_color(theme.muted_foreground)
                            .child(label),
                    )
                    .child(value_el),
            );
        }

        let mask = overlay::mask(theme)
            .occlude()
            .key_context("Overlay")
            .track_focus(&self.overlay_focus)
            .on_action(cx.listener(|this, _: &UnifiedDismiss, window, cx| {
                this.close_details_overlay(window, cx)
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    this.close_details_overlay(window, cx)
                }),
            )
            .child(
                overlay::surface(theme)
                    .w_full()
                    .max_w(px(560.))
                    .overflow_hidden()
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .gap_3()
                            .px_4()
                            .py_3()
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(tokens::title())
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child("对象详情"),
                            )
                            .child(
                                ui::icon_button(
                                    "close-details-overlay",
                                    Icon::new(IconName::Close),
                                    "关闭",
                                )
                                .on_click(cx.listener(
                                    |this, _, window, cx| this.close_details_overlay(window, cx),
                                )),
                            ),
                    )
                    .child(content),
            );
        overlay::fade_in("details-overlay", mask).into_any_element()
    }

    /// 「关于」弹层：无输入框，按弹层规范走 Overlay context + UnifiedDismiss，
    /// 遮罩点击关，卡片阻断冒泡。
    pub(super) fn render_about_overlay(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let mask = overlay::mask(theme)
            .occlude()
            .key_context("Overlay")
            .track_focus(&self.overlay_focus)
            .on_action(cx.listener(|this, _: &UnifiedDismiss, window, cx| {
                this.close_about_overlay(window, cx)
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    this.close_about_overlay(window, cx)
                }),
            )
            .child(
                overlay::surface(theme)
                    .w_full()
                    .max_w(px(420.))
                    .overflow_hidden()
                    // 标题栏：标题 + 关闭按钮（规范：标题栏右侧 ✕）
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .items_center()
                            .px_4()
                            .py_3()
                            .child(
                                div()
                                    .text_size(tokens::title())
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child("关于 CloudStorage"),
                            )
                            .child(
                                ui::icon_button(
                                    "close-about-overlay",
                                    Icon::new(IconName::Close),
                                    "关闭",
                                )
                                .on_click(cx.listener(
                                    |this, _, window, cx| this.close_about_overlay(window, cx),
                                )),
                            ),
                    )
                    .child(
                        // 内容：应用标识 + 信息卡
                        v_flex()
                            .px_4()
                            .pb_4()
                            .gap_3()
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        // 圆角与描边必须落在 img 本体上：overflow_hidden
                                        // 只裁矩形（ContentMask 只有 bounds），圆角写在外层
                                        // div 上对位图无效——位图仍是方角。外层 div 只负责
                                        // 抬起感的 shadow_sm（阴影会跟随圆角）。
                                        div()
                                            .rounded(tokens::radius_icon())
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .shadow_sm()
                                            .child(
                                                img(Arc::new(gpui::Image::from_bytes(
                                                    gpui::ImageFormat::Png,
                                                    crate::APP_ICON_PNG.to_vec(),
                                                )))
                                                .size(px(56.))
                                                .rounded(tokens::radius_icon())
                                                .object_fit(ObjectFit::Cover)
                                                // 1px 内描边：img 是叶子节点，border 画在
                                                // 位图之后的自身 bounds 内侧、不内缩位图，
                                                // 等价 outline:1px + outline-offset:-1px。
                                                .border_1()
                                                .border_color(crate::theme::image_outline(
                                                    theme.mode,
                                                )),
                                            ),
                                    )
                                    .child(
                                        v_flex()
                                            .gap_0p5()
                                            .child(
                                                div()
                                                    .text_size(tokens::display())
                                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                                    .text_color(theme.foreground)
                                                    .child("CloudStorage"),
                                            )
                                            .child(
                                                div()
                                                    .text_size(tokens::label())
                                                    .text_color(theme.muted_foreground)
                                                    .child(format!(
                                                        "版本 {} · macOS 14+",
                                                        env!("CARGO_PKG_VERSION")
                                                    )),
                                            ),
                                    ),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .rounded(tokens::radius_lg())
                                    .border_1()
                                    .border_color(theme.border)
                                    .bg(theme.sidebar)
                                    .p_3()
                                    .child(
                                        v_flex()
                                            .gap_2()
                                            .child(about_kv(
                                                "应用定位",
                                                "高性能 Workspace，键盘优先。",
                                                theme,
                                            ))
                                            .child(about_kv(
                                                "技术栈",
                                                "Rust · GPUI · Tokio · SQLite · Keychain",
                                                theme,
                                            ))
                                            .child(about_kv(
                                                "数据安全",
                                                "Secret 只存 macOS Keychain，不落盘。",
                                                theme,
                                            ))
                                            .child(about_kv(
                                                "支持服务商",
                                                "Qiniu Kodo · Aliyun OSS",
                                                theme,
                                            )),
                                    ),
                            ),
                    ),
            );
        overlay::fade_in("about-overlay", mask).into_any_element()
    }

    pub(super) fn close_about_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.about_overlay_open = false;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// 「关于」弹层（菜单在设置上方 / 命令面板共享）。与设置模态互斥。
    pub(super) fn handle_open_about(
        &mut self,
        _: &OpenAbout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.about_overlay_open
            || self.settings_modal.is_some()
            || self.palette.is_some()
            || self.add_modal.is_some()
        {
            return;
        }
        self.about_overlay_open = true;
        window.focus(&self.overlay_focus, cx);
        cx.notify();
    }
}

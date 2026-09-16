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

/// 菜单动作的目标集合（纯函数，单测锁死）。
///
/// 两个入口的语义不同：
/// - **右键菜单**（`keep_selection = true`）：`open_object_menu` 已按 Finder 语义把该行
///   纳入选择，所以目标就是当前选择集。
/// - **⋯ 入口**（`keep_selection = false`）：**不改选择**。点 ⋯ 不该把行选上（用户明确
///   要求），但菜单动作也不能去打「别的已选行」——所以目标是**这一行**；若该行本就在
///   多选里，则整批仍是目标（批量操作语义不变）。
///
/// 为什么必须显式算目标：菜单项的执行体历史上直接读 `selected_object_keys`，一旦入口
/// 不再选中该行，动作就会落到以前的选中集上——那是「删错对象」级别的错。
pub(super) fn menu_targets_for(
    key: &str,
    selection: &indexmap::IndexSet<String>,
    keep_selection: bool,
) -> Vec<String> {
    if keep_selection {
        return selection.iter().cloned().collect();
    }
    if selection.contains(key) {
        return selection.iter().cloned().collect();
    }
    vec![key.to_string()]
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
        self.object_menu_targets = menu_targets_for(key, &self.selected_object_keys, true);
        self.object_menu_open = Some(key.to_string());
        self.object_menu_at = Some(at);
        self.preview_overlay_open = false;
        self.details_overlay_open = false;
        cx.notify();
    }

    /// ⋯ 入口（行末「操作」列）：**不改动选择**，只打开菜单并把动作目标设为该行。
    pub(super) fn open_row_actions_menu(
        &mut self,
        key: &str,
        at: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.object_menu_open.as_deref() == Some(key) {
            self.object_menu_open = None;
            self.object_menu_at = None;
            self.object_menu_targets.clear();
            cx.notify();
            return;
        }
        self.object_menu_targets = menu_targets_for(key, &self.selected_object_keys, false);
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

    pub(super) fn toggle_top_more_menu(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        self.top_more_open = !self.top_more_open;
        self.top_more_menu_at = Some(at);
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

    /// 打开「关于」：调用 **macOS 原生面板**（对齐参照实现的 `role: 'about'`）。
    ///
    /// 自建浮层（含 `about_kv`）已删除：系统已提供的能力不自建（agents.md §3），
    /// 而且原生面板会自动跟随系统语言、外观与无障碍设置，这几点自建版都做不到。
    pub(super) fn handle_open_about(
        &mut self,
        _: &OpenAbout,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 模态开着时不抢焦点（与其它 Action 的守卫一致）
        if self.palette.is_some() || self.add_modal.is_some() || self.settings_modal.is_some() {
            return;
        }
        if let Err(error) =
            object_storage_macos::show_about_panel("CloudStorage", env!("CARGO_PKG_VERSION"))
        {
            self.download_message = Some(DownloadMessage {
                is_error: true,
                text: format!("无法打开「关于」面板：{error}"),
            });
            cx.notify();
        }
    }
}

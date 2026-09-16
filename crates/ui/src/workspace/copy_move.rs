//! 复制到 / 移动到：目录浏览器、目标校验与逐项执行。

use super::*;

pub(super) fn normalize_copy_move_target_prefix(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if trimmed.starts_with('/') {
        return Err("目标目录不能以 / 开头".into());
    }
    if trimmed.split('/').any(|segment| segment == "..") {
        return Err("目标目录不能包含 ..".into());
    }
    Ok(if trimmed.ends_with('/') {
        trimmed.to_string()
    } else {
        format!("{trimmed}/")
    })
}

pub(super) fn copy_move_target_key(source_key: &str, target_prefix: &str) -> String {
    format!("{}{}", target_prefix, display_name(source_key))
}

pub(super) fn copy_move_target_keys(
    source_keys: &[String],
    target_prefix: &str,
) -> Result<Vec<(String, String)>, String> {
    let prefix = normalize_copy_move_target_prefix(target_prefix)?;
    let targets: Vec<(String, String)> = source_keys
        .iter()
        .map(|source| (source.clone(), copy_move_target_key(source, &prefix)))
        .collect();
    if let Some((source, _)) = targets.iter().find(|(source, target)| source == target) {
        return Err(format!("目标路径与源路径相同：{source}"));
    }
    let mut seen = std::collections::HashSet::new();
    if let Some((_, target)) = targets
        .iter()
        .find(|(_, target)| !seen.insert(target.as_str()))
    {
        return Err(format!(
            "多个源对象会写入同一目标名称：{}",
            display_name(target)
        ));
    }
    Ok(targets)
}

pub(super) fn prepare_copy_move_directory_load(
    target_prefix: &mut String,
    entries: &mut Vec<ListingEntry>,
    state: &mut AsyncState,
    next_prefix: String,
) {
    *target_prefix = next_prefix;
    entries.clear();
    *state = AsyncState::Loading;
}

pub(super) fn can_commit_copy_move(
    busy: bool,
    directory_state: &AsyncState,
    validation: Option<&str>,
) -> bool {
    !busy && *directory_state != AsyncState::Loading && validation.is_none()
}

pub(super) fn copy_move_summary(
    mode: CopyMoveMode,
    success: usize,
    failures: &[(String, String)],
) -> String {
    let action = match mode {
        CopyMoveMode::Copy => "复制",
        CopyMoveMode::Move => "移动",
    };
    if failures.is_empty() {
        return format!("已{action} {success} 个对象");
    }
    let detail = failures
        .iter()
        .take(3)
        .map(|(key, error)| format!("{}：{}", display_name(key), error))
        .collect::<Vec<_>>()
        .join("；");
    format!(
        "{action}完成 {success} 个，失败 {} 个：{detail}",
        failures.len()
    )
}

impl WorkspaceView {
    pub(super) fn open_copy_move_overlay(
        &mut self,
        mode: CopyMoveMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.is_some()
            || self.add_modal.is_some()
            || self.copy_move_busy
            || self.selected_bucket.is_none()
        {
            return;
        }
        let source_keys = self.action_target_keys();
        if source_keys.is_empty() {
            self.download_message = Some(DownloadMessage {
                is_error: true,
                text: "请先选中对象".into(),
            });
            cx.notify();
            return;
        }
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("输入或搜索目标目录…"));
        cx.subscribe_in(&filter, window, |this, _, event: &InputEvent, _, cx| {
            if let InputEvent::Change = event {
                cx.notify();
            }
            if let InputEvent::PressEnter { .. } = event {
                this.commit_copy_move(cx);
            }
        })
        .detach();
        filter.update(cx, |state, cx| state.focus(window, cx));
        self.object_menu_open = None;
        self.top_more_open = false;
        self.copy_move = Some(CopyMoveState {
            mode,
            source_keys,
            target_prefix: self.current_prefix.clone().unwrap_or_default(),
            filter,
            entries: Vec::new(),
            state: AsyncState::Loading,
        });
        cx.notify();
        self.load_copy_move_entries(cx);
    }

    pub(super) fn load_copy_move_entries(&mut self, cx: &mut Context<Self>) {
        self.copy_move_gen += 1;
        let generation = self.copy_move_gen;

        let Some(account_id) = self.selected_account_id.clone() else {
            return;
        };
        let Some(bucket) = self.selected_bucket.clone() else {
            return;
        };
        let Some(prefix) = self
            .copy_move
            .as_ref()
            .map(|state| state.target_prefix.clone())
        else {
            return;
        };
        let region = self
            .buckets
            .iter()
            .find(|b| b.name == bucket)
            .and_then(|b| b.region.clone());
        let services = Arc::clone(&self.services);

        if let Some(state) = &mut self.copy_move {
            prepare_copy_move_directory_load(
                &mut state.target_prefix,
                &mut state.entries,
                &mut state.state,
                prefix.clone(),
            );
        }
        cx.notify();

        cx.spawn(async move |this, cx| {
            let request = ListObjectsRequest {
                bucket,
                prefix: if prefix.is_empty() {
                    None
                } else {
                    Some(prefix)
                },
                delimiter: Some("/".into()),
                marker: None,
                limit: OBJECTS_PAGE_LIMIT,
                region,
            };
            let result = cx
                .background_executor()
                .spawn(async move { services.list_objects(&account_id, request) })
                .await;
            this.update(cx, |this, cx| {
                if this.copy_move_gen != generation {
                    return;
                }
                let Some(state) = &mut this.copy_move else {
                    return;
                };
                match result {
                    Ok(page) => {
                        state.entries = page.entries;
                        state.state = AsyncState::Idle;
                    }
                    Err(e) => {
                        state.entries.clear();
                        state.state = AsyncState::Failed(e.to_string());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn close_copy_move_overlay(&mut self, cx: &mut Context<Self>) {
        if self.copy_move_busy {
            return;
        }
        if self.copy_move.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn copy_move_target_prefix(&self, cx: &mut Context<Self>) -> Result<String, String> {
        let Some(state) = &self.copy_move else {
            return Err("复制/移动弹窗未打开".into());
        };
        let input = state.filter.read(cx).value().to_string();
        if input.trim().is_empty() {
            return Ok(state.target_prefix.clone());
        }
        normalize_copy_move_target_prefix(&input)
    }

    pub(super) fn copy_move_validation_message(&self, cx: &mut Context<Self>) -> Option<String> {
        let Some(state) = &self.copy_move else {
            return None;
        };
        let target_prefix = match self.copy_move_target_prefix(cx) {
            Ok(prefix) => prefix,
            Err(message) => return Some(message),
        };
        let targets = match copy_move_target_keys(&state.source_keys, &target_prefix) {
            Ok(targets) => targets,
            Err(message) => return Some(message),
        };
        if target_prefix == state.target_prefix
            && let Some((_, target)) = targets
                .iter()
                .find(|(_, target)| object_key_exists(&state.entries, target))
        {
            return Some(format!("目标对象已存在：{target}"));
        }
        None
    }

    pub(super) fn enter_copy_move_prefix(
        &mut self,
        prefix: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(state) = &mut self.copy_move {
            prepare_copy_move_directory_load(
                &mut state.target_prefix,
                &mut state.entries,
                &mut state.state,
                prefix.clone(),
            );
            state
                .filter
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.load_copy_move_entries(cx);
    }

    pub(super) fn go_up_copy_move_prefix(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(state) = &mut self.copy_move {
            let prefix = parent_prefix(&state.target_prefix)
                .map(str::to_string)
                .unwrap_or_default();
            prepare_copy_move_directory_load(
                &mut state.target_prefix,
                &mut state.entries,
                &mut state.state,
                prefix,
            );
            state
                .filter
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.load_copy_move_entries(cx);
    }

    pub(super) fn commit_copy_move(&mut self, cx: &mut Context<Self>) {
        if self.copy_move_busy {
            return;
        }
        let Some(state) = &self.copy_move else {
            return;
        };
        if !can_commit_copy_move(self.copy_move_busy, &state.state, None) {
            return;
        }
        if let Some(message) = self.copy_move_validation_message(cx) {
            self.download_message = Some(DownloadMessage {
                is_error: true,
                text: message,
            });
            cx.notify();
            return;
        }
        let Some(account_id) = self.selected_account_id.clone() else {
            return;
        };
        let Some(bucket) = self.selected_bucket.clone() else {
            return;
        };
        let mode = state.mode;
        let target_prefix = match self.copy_move_target_prefix(cx) {
            Ok(prefix) => prefix,
            Err(message) => {
                self.download_message = Some(DownloadMessage {
                    is_error: true,
                    text: message,
                });
                cx.notify();
                return;
            }
        };
        let targets = match copy_move_target_keys(&state.source_keys, &target_prefix) {
            Ok(targets) => targets,
            Err(message) => {
                self.download_message = Some(DownloadMessage {
                    is_error: true,
                    text: message,
                });
                cx.notify();
                return;
            }
        };
        let services = Arc::clone(&self.services);
        self.copy_move_busy = true;
        self.download_message = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut success = 0usize;
                    let mut failures = Vec::new();
                    for (source, target) in targets {
                        let result = match mode {
                            CopyMoveMode::Copy => {
                                services.copy_object(&account_id, &bucket, &source, &target)
                            }
                            CopyMoveMode::Move => {
                                services.move_object(&account_id, &bucket, &source, &target)
                            }
                        };
                        match result {
                            Ok(()) => success += 1,
                            Err(error) => failures.push((source, error.to_string())),
                        }
                    }
                    (success, failures)
                })
                .await;
            this.update(cx, |this, cx| {
                this.copy_move_busy = false;
                this.copy_move = None;
                let (success, failures) = result;
                this.download_message = Some(DownloadMessage {
                    is_error: !failures.is_empty(),
                    text: copy_move_summary(mode, success, &failures),
                });
                this.reload_objects(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn render_copy_move_overlay(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(state) = self.copy_move.as_ref() else {
            return div().into_any_element();
        };
        let Some(bucket) = self.selected_bucket.as_ref() else {
            return div().into_any_element();
        };
        let Some(provider) = self.selected_provider_kind() else {
            return div().into_any_element();
        };
        let title = match state.mode {
            CopyMoveMode::Copy => "复制到",
            CopyMoveMode::Move => "移动到",
        };
        let action = match state.mode {
            CopyMoveMode::Copy => "复制",
            CopyMoveMode::Move => "移动",
        };
        let query = state.filter.read(cx).value().to_string();
        let validation_message = self.copy_move_validation_message(cx);
        let is_loading_dirs = state.state == AsyncState::Loading;
        let can_confirm = can_commit_copy_move(
            self.copy_move_busy,
            &state.state,
            validation_message.as_deref(),
        );
        let target_prefix = self
            .copy_move_target_prefix(cx)
            .unwrap_or_else(|_| state.target_prefix.clone());
        let location = format!(
            "{}:// {} / {}",
            provider_url_scheme(provider),
            bucket,
            if target_prefix.is_empty() {
                String::new()
            } else {
                target_prefix.clone()
            }
        );
        let query_lower = query.trim().to_lowercase();
        let directories: Vec<String> = state
            .entries
            .iter()
            .filter_map(|entry| match entry {
                ListingEntry::CommonPrefix(prefix) => Some(prefix.clone()),
                ListingEntry::Object(_) => None,
            })
            .filter(|prefix| {
                query_lower.is_empty()
                    || display_name(prefix)
                        .to_lowercase()
                        .contains(query_lower.as_str())
                    || prefix.to_lowercase().contains(query_lower.as_str())
            })
            .collect();

        let mut list = v_flex()
            .id("copy-move-dir-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        if is_loading_dirs {
            list = list.child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .text_color(theme.muted_foreground)
                    .text_size(tokens::body())
                    .child(Spinner::new())
                    .child("加载目录中…"),
            );
        } else if let AsyncState::Failed(message) = &state.state {
            list = list.child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .text_color(theme.danger)
                    .text_size(tokens::body())
                    .child(Icon::new(IconName::TriangleAlert))
                    .child(message.clone()),
            );
        } else if directories.is_empty() {
            list = list.child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.muted_foreground)
                    .text_size(tokens::body())
                    .child("没有可显示的目录，可直接输入目标目录"),
            );
        }
        for (ix, prefix) in directories.into_iter().enumerate() {
            let target = prefix.clone();
            list = list.child(
                h_flex()
                    .id(("copy-move-dir", ix))
                    .px_4()
                    .py_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(theme.table_row_border)
                    .text_size(tokens::body())
                    .hover(|row| row.bg(theme.list_hover))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            this.enter_copy_move_prefix(target.clone(), window, cx)
                        }),
                    )
                    .child(
                        Icon::new(IconName::Folder)
                            .text_color(crate::theme::file_icon_color(theme.mode)),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(display_name(&prefix).to_string()),
                    ),
            );
        }

        let mask = overlay::mask(theme)
            .occlude()
            .key_context("Renaming")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                    this.close_copy_move_overlay(cx);
                }),
            )
            .child(
                overlay::surface(theme)
                    .w_full()
                    .max_w(px(760.))
                    .h(px(560.))
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
                                    .text_size(tokens::title())
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(title),
                            )
                            .child(
                                ui::icon_button(
                                    "close-copy-move-overlay",
                                    Icon::new(IconName::Close),
                                    "关闭",
                                )
                                .disabled(self.copy_move_busy)
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.close_copy_move_overlay(cx)),
                                ),
                            ),
                    )
                    .child(
                        div()
                            .mx_6()
                            .mb_3()
                            .px_3()
                            .py_2()
                            .rounded(tokens::radius())
                            .bg(theme.sidebar)
                            .text_size(tokens::label())
                            .text_color(theme.muted_foreground)
                            .child(location),
                    )
                    .child(div().mx_6().mb_3().child(Input::new(&state.filter).small()))
                    .child(
                        h_flex()
                            .mx_6()
                            .px_4()
                            .py_2()
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.sidebar)
                            .text_size(tokens::label())
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("名称"),
                    )
                    .child(
                        div()
                            .mx_6()
                            .flex_1()
                            .min_h_0()
                            .border_1()
                            .border_t_0()
                            .border_color(theme.border)
                            .child(list),
                    )
                    .children(validation_message.map(|message| {
                        div()
                            .mx_6()
                            .mt_3()
                            .text_size(tokens::label())
                            .text_color(theme.danger)
                            .child(message)
                    }))
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .px_4()
                            .py_3()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .text_size(tokens::label())
                                    .text_color(theme.muted_foreground)
                                    .child(format!("{} 个对象", state.source_keys.len()))
                                    .child("遇到同名文件：询问"),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .children((!state.target_prefix.is_empty()).then(|| {
                                        Button::new("copy-move-go-up")
                                            .label("上一级")
                                            .with_size(Size::Small)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.go_up_copy_move_prefix(window, cx)
                                            }))
                                    }))
                                    .child(
                                        Button::new("cancel-copy-move")
                                            .label("取消")
                                            .with_size(Size::Small)
                                            .disabled(self.copy_move_busy)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.close_copy_move_overlay(cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("confirm-copy-move")
                                            .label(if self.copy_move_busy {
                                                "执行中…"
                                            } else {
                                                action
                                            })
                                            .primary()
                                            .with_size(Size::Small)
                                            .disabled(!can_confirm)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.commit_copy_move(cx)
                                            })),
                                    ),
                            ),
                    ),
            );
        overlay::fade_in("copy-move-overlay", mask).into_any_element()
    }
}

//! 新建目录：占位对象上传与实时校验。

use super::*;

pub(super) fn common_prefix_exists(entries: &[ListingEntry], prefix: &str) -> bool {
    entries.iter().any(|entry| match entry {
        ListingEntry::CommonPrefix(existing) => existing == prefix,
        ListingEntry::Object(_) => false,
    })
}

pub(super) fn create_folder_target_key(
    current_prefix: Option<&str>,
    folder_name: &str,
) -> Result<String, String> {
    let name = folder_name.trim();
    if name.is_empty() {
        return Err("目录名不能为空".into());
    }
    if name.contains('/') {
        return Err("目录名不能包含 /".into());
    }
    if name == "." || name == ".." {
        return Err("目录名不能是 . 或 ..".into());
    }
    Ok(format!("{}{}{}", current_prefix.unwrap_or(""), name, "/"))
}

pub(super) fn create_folder_validation_message(
    current_prefix: Option<&str>,
    folder_name: &str,
    entries: &[ListingEntry],
) -> Option<String> {
    let key = match create_folder_target_key(current_prefix, folder_name) {
        Ok(key) => key,
        Err(message) => return Some(message),
    };
    if common_prefix_exists(entries, &key) || object_key_exists(entries, &key) {
        return Some(format!(
            "目录已存在：{}",
            display_name(key.trim_end_matches('/'))
        ));
    }
    None
}

impl WorkspaceView {
    pub(super) fn open_create_folder_overlay(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_bucket.is_none()
            || self.create_folder_input.is_some()
            || self.creating_folder
            || self.palette.is_some()
            || self.add_modal.is_some()
        {
            return;
        }
        let editor = cx.new(|cx| InputState::new(window, cx).default_value("新建目录"));
        cx.subscribe_in(&editor, window, |this, _, event: &InputEvent, _, cx| {
            if let InputEvent::PressEnter { .. } = event {
                this.commit_create_folder(cx);
            }
        })
        .detach();
        editor.update(cx, |state, cx| state.focus(window, cx));
        self.object_menu_open = None;
        self.create_folder_input = Some(editor);
        cx.notify();
    }

    pub(super) fn close_create_folder_overlay(&mut self, cx: &mut Context<Self>) {
        if self.creating_folder {
            return;
        }
        if self.create_folder_input.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn commit_create_folder(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.create_folder_input.clone() else {
            return;
        };
        if self.creating_folder {
            return;
        }
        let Some(account_id) = self.selected_account_id.clone() else {
            return;
        };
        let Some(bucket) = self.selected_bucket.clone() else {
            return;
        };
        let folder_name = editor.read(cx).value().to_string();
        if let Some(message) = create_folder_validation_message(
            self.current_prefix.as_deref(),
            &folder_name,
            &self.entries,
        ) {
            self.download_message = Some(DownloadMessage {
                is_error: true,
                text: message,
            });
            cx.notify();
            return;
        }
        let key = match create_folder_target_key(self.current_prefix.as_deref(), &folder_name) {
            Ok(key) => key,
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
        let display_key = key.clone();
        self.creating_folder = true;
        self.download_message = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let tmp = std::env::temp_dir().join(format!(
                        "cloudstorage-empty-folder-{}-{}",
                        std::process::id(),
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_nanos())
                            .unwrap_or_default()
                    ));
                    std::fs::File::create(&tmp).map_err(|e| format!("创建临时空文件失败：{e}"))?;
                    let upload = services.upload_object(&account_id, &bucket, &key, &tmp);
                    let cleanup = std::fs::remove_file(&tmp);
                    if let Err(e) = cleanup {
                        eprintln!(
                            "[create_folder] 临时文件清理失败（不影响结果）：{}：{e}",
                            tmp.display()
                        );
                    }
                    upload.map_err(|e| e.to_string())
                })
                .await;
            this.update(cx, |this, cx| {
                this.creating_folder = false;
                match result {
                    Ok(_) => {
                        this.create_folder_input = None;
                        this.download_message = Some(DownloadMessage {
                            is_error: false,
                            text: format!("已新建目录：{}", display_key),
                        });
                        this.reload_objects(cx);
                    }
                    Err(error) => {
                        this.download_message = Some(DownloadMessage {
                            is_error: true,
                            text: format!("新建目录失败：{error}"),
                        });
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn render_create_folder_overlay(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(editor) = self.create_folder_input.as_ref() else {
            return div().into_any_element();
        };
        let folder_name = editor.read(cx).value().to_string();
        let validation_message = create_folder_validation_message(
            self.current_prefix.as_deref(),
            &folder_name,
            &self.entries,
        );
        let can_confirm = !self.creating_folder && validation_message.is_none();
        let parent = self.current_prefix.as_deref().unwrap_or("/").to_string();

        let mask = overlay::mask(theme)
            .occlude()
            .key_context("Renaming")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                    this.close_create_folder_overlay(cx);
                }),
            )
            .child(
                overlay::surface(theme)
                    .w_full()
                    .max_w(px(480.))
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
                                    .child("新建目录"),
                            )
                            .child(
                                ui::icon_button(
                                    "close-create-folder-overlay",
                                    Icon::new(IconName::Close),
                                    "关闭",
                                )
                                .disabled(self.creating_folder)
                                .on_click(cx.listener(
                                    |this, _, _, cx| this.close_create_folder_overlay(cx),
                                )),
                            ),
                    )
                    .child(
                        v_flex()
                            .px_4()
                            .gap_3()
                            .child(
                                h_flex()
                                    .gap_4()
                                    .items_center()
                                    .child(
                                        div()
                                            .w(tokens::text(74.))
                                            .flex_shrink_0()
                                            .text_size(tokens::body())
                                            .text_color(theme.foreground)
                                            .child("所在目录："),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(tokens::body())
                                            .child(parent),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_4()
                                    .items_center()
                                    .child(
                                        div()
                                            .w(tokens::text(74.))
                                            .flex_shrink_0()
                                            .text_size(tokens::body())
                                            .text_color(theme.foreground)
                                            .child("目录名："),
                                    )
                                    .child(
                                        div().flex_1().min_w_0().child(Input::new(editor).small()),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_4()
                                    .child(div().w(tokens::text(74.)).flex_shrink_0())
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(tokens::label())
                                            .text_color(theme.muted_foreground)
                                            .line_height(px(20.))
                                            .child("对象存储目录会创建为以 / 结尾的占位对象。"),
                                    ),
                            )
                            .children(validation_message.map(|message| {
                                h_flex()
                                    .gap_4()
                                    .child(div().w(tokens::text(74.)).flex_shrink_0())
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(tokens::label())
                                            .text_color(theme.danger)
                                            .line_height(px(20.))
                                            .child(message),
                                    )
                            })),
                    )
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .px_4()
                            .py_3()
                            .child(
                                Button::new("cancel-create-folder")
                                    .label("取消")
                                    .disabled(self.creating_folder)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.close_create_folder_overlay(cx)
                                    })),
                            )
                            .child(
                                Button::new("confirm-create-folder")
                                    .label(if self.creating_folder {
                                        "创建中…"
                                    } else {
                                        "创建"
                                    })
                                    .primary()
                                    .disabled(!can_confirm)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.commit_create_folder(cx)),
                                    ),
                            ),
                    ),
            );
        overlay::fade_in("create-folder-overlay", mask).into_any_element()
    }
}

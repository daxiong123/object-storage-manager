//! 预览与本地副本：应用内预览、文本编辑保存、Open With / Reveal、签名链接。

use super::*;

// 弹层「卡片阻断两相冒泡」这条规范原先是这里的纯函数判据（已删除）：现在由
// `overlay::surface()` 统一提供，构造即合规，不再需要靠测试断言去守。
pub(super) fn provider_url_scheme(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::Aliyun => "oss",
        ProviderKind::Qiniu => "kodo",
    }
}

/// 判断本地缓存文件是否对应当前选中的对象（缓存复用判据）：
/// 缓存文件名形如 `{nanos}-{display_name}`，只需后缀匹配 display_name。
/// `None` path / 无 key 一律不复用（保守：多下载一次好过开错文件）。
pub(crate) fn cached_copy_matches(path: Option<&std::path::Path>, object_key: &str) -> bool {
    let Some(path) = path else {
        return false;
    };
    if object_key.is_empty() {
        return false;
    }
    let name = display_name(object_key);
    if name.is_empty() {
        return false;
    }
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(name) && n.len() > name.len())
}

/// 七牛目录占位对象：key 以 `/` 结尾（size=0，mimeType 为
/// `application/qiniu-object-manager`）。下载必 404，无预览/打开/URL 语义；
/// 交互上应与 CommonPrefix 一致：点击 = 下钻。
pub(super) fn is_directory_object(key: &str) -> bool {
    key.ends_with('/')
}

pub(super) fn preview_kind(key: &str) -> PreviewKind {
    if is_text_object(key) {
        PreviewKind::Text
    } else if is_image_object(key) {
        PreviewKind::Image
    } else {
        PreviewKind::System
    }
}

pub(super) fn is_image_object(key: &str) -> bool {
    let Some(ext) = key.rsplit('.').next() else {
        return false;
    };
    let ext = ext.to_ascii_lowercase();
    Img::extensions().iter().any(|candidate| *candidate == ext)
}

pub(super) fn syntax_language(key: &str) -> &'static str {
    match key
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("json") => "json",
        Some("js") => "javascript",
        Some("ts") => "typescript",
        Some("html" | "htm") => "html",
        Some("css") => "css",
        Some("md") => "markdown",
        Some("xml") => "xml",
        Some("yaml" | "yml") => "yaml",
        Some("rs") => "rust",
        Some("csv") | Some("txt") | None => "text",
        _ => "text",
    }
}

pub(super) fn is_text_object(key: &str) -> bool {
    matches!(
        key.rsplit('.')
            .next()
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some(
            "txt"
                | "json"
                | "md"
                | "html"
                | "htm"
                | "css"
                | "js"
                | "ts"
                | "xml"
                | "yaml"
                | "yml"
                | "csv"
                | "toml"
                | "rs"
        )
    )
}

pub(super) fn preview_download_error_message(bucket: &str, key: &str, error: &str) -> String {
    let sanitized = sanitize_remote_error(error);
    format!(
        "无法预览：{}\n\n请检查：\n1. 当前账号能否读取 `{}`；\n2. 七牛：Bucket `{bucket}` 的下载域名是否可用；\n3. 阿里云 OSS：Endpoint/区域和 RAM 权限是否正确。",
        sanitized,
        display_name(key)
    )
}

pub(super) fn sanitize_remote_error(error: &str) -> String {
    let lower = error.to_lowercase();
    if lower.contains("<html") || lower.contains("<!doctype") || lower.contains("403 forbidden") {
        if lower.contains("403") || lower.contains("forbidden") {
            return "远端拒绝访问，通常是权限或下载域名配置问题".into();
        }
        return "远端返回错误页面，无法下载对象内容".into();
    }
    if error.chars().count() > 300 {
        let cut: String = error.chars().take(300).collect();
        format!("{cut}…")
    } else {
        error.to_string()
    }
}

pub(super) fn copy_object_url_request(
    account_id: Option<&str>,
    bucket: Option<&str>,
    object: Option<&CloudObject>,
    ttl_secs: u64,
) -> Result<CopyObjectUrlRequest, DownloadMessage> {
    let Some(object) = object else {
        return Err(DownloadMessage {
            is_error: true,
            text: "请先选中一个对象再复制链接".into(),
        });
    };
    let (Some(account_id), Some(bucket)) = (account_id, bucket) else {
        return Err(DownloadMessage {
            is_error: true,
            text: "请先选择账号和 Bucket 再复制链接".into(),
        });
    };
    Ok(CopyObjectUrlRequest {
        account_id: account_id.to_string(),
        bucket: bucket.to_string(),
        key: object.key.clone(),
        ttl_secs,
    })
}

impl WorkspaceView {
    pub(super) fn start_object_preview(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self
            .selected_cloud_object()
            .map(|object| object.key.clone())
        else {
            return;
        };
        let Some(account_id) = self.selected_account_id.clone() else {
            return;
        };
        let Some(bucket) = self.selected_bucket.clone() else {
            return;
        };
        self.preview_gen += 1;
        let generation = self.preview_gen;
        let name = display_name(&key).to_string();
        let mut path = std::env::temp_dir();
        path.push("CloudStorage");
        path.push("preview");
        path.push(format!(
            "{}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("系统时间早于 Unix epoch")
                .as_nanos(),
            name
        ));
        self.previewing = true;
        self.preview_path = None;
        self.preview_text = None;
        self.text_editor = None;
        self.download_message = None;
        cx.notify();
        let services = Arc::clone(&self.services);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    std::fs::create_dir_all(path.parent().expect("预览路径必须有父目录"))
                        .map_err(|e| format!("创建预览缓存目录失败：{e}"))?;
                    services
                        .download_object(&account_id, &bucket, &key, &path)
                        .map_err(|e| {
                            preview_download_error_message(&bucket, &key, &e.to_string())
                        })?;
                    let text = if is_text_object(&key) {
                        let metadata = std::fs::metadata(&path)
                            .map_err(|e| format!("读取预览文件信息失败：{e}"))?;
                        if metadata.len() > 2 * 1024 * 1024 {
                            return Err("文本对象超过 2 MiB，暂不在编辑器中打开".into());
                        }
                        Some(
                            std::fs::read_to_string(&path)
                                .map_err(|e| format!("文本对象不是有效 UTF-8：{e}"))?,
                        )
                    } else {
                        None
                    };
                    Ok::<_, String>((path, text))
                })
                .await;
            this.update(cx, |this, cx| {
                if this.preview_gen != generation {
                    return;
                }
                this.previewing = false;
                match result {
                    Ok((path, text)) => {
                        this.preview_path = Some(path.clone());
                        this.preview_text = text;
                        if this.preview_open_quicklook {
                            this.preview_open_quicklook = false;
                            if let Some(key) = this.selected_object_key.as_deref()
                                && preview_kind(key) == PreviewKind::System
                                && let Err(error) = object_storage_macos::quick_look(&path)
                            {
                                this.download_message = Some(DownloadMessage {
                                    is_error: true,
                                    text: format!("打开 Quick Look 失败：{error}"),
                                });
                            }
                        }
                    }
                    Err(error) => {
                        this.download_message = Some(DownloadMessage {
                            is_error: true,
                            text: error,
                        });
                    }
                }
                cx.notify();
            })
            .expect("预览结果回传 UI 失败");
        })
        .detach();
    }

    /// 确保选中对象有本地副本：`preview_path` 已有（同 key）→ 直接复用；
    /// 否则下载到临时目录（与预览同一缓存位置）。返回 (本地路径, key)。
    /// 异步；结果经 `then` 回调（在 UI 线程执行）。
    pub(super) fn ensure_local_copy(
        &mut self,
        then: impl FnOnce(&mut Self, PathBuf, String, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = self
            .selected_cloud_object()
            .map(|object| object.key.clone())
        else {
            return;
        };
        // 已有本地副本且对应当前对象：直接用
        let current = self.selected_object_key.clone().unwrap_or_default();
        if cached_copy_matches(self.preview_path.as_deref(), &current) {
            let path = self.preview_path.clone().expect("刚检查过");
            then(self, path, key, cx);
            return;
        }
        if self.previewing {
            return; // 已有下载在进行：防重入
        }
        let Some(account_id) = self.selected_account_id.clone() else {
            return;
        };
        let Some(bucket) = self.selected_bucket.clone() else {
            return;
        };
        let name = display_name(&key).to_string();
        let mut path = std::env::temp_dir();
        path.push("CloudStorage");
        path.push("preview");
        path.push(format!(
            "{}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("系统时间早于 Unix epoch")
                .as_nanos(),
            name
        ));
        self.previewing = true;
        self.download_message = None;
        cx.notify();
        let services = Arc::clone(&self.services);
        let key_for_result = key.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    std::fs::create_dir_all(path.parent().expect("预览路径必须有父目录"))
                        .map_err(|e| format!("创建缓存目录失败：{e}"))?;
                    services
                        .download_object(&account_id, &bucket, &key, &path)
                        .map_err(|e| {
                            preview_download_error_message(&bucket, &key, &e.to_string())
                        })?;
                    Ok::<_, String>(path)
                })
                .await;
            this.update(cx, |this, cx| {
                this.previewing = false;
                match result {
                    Ok(path) => {
                        this.preview_path = Some(path.clone());
                        then(this, path, key_for_result, cx);
                    }
                    Err(error) => {
                        this.download_message = Some(DownloadMessage {
                            is_error: true,
                            text: error,
                        });
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// ⌘O / 对象菜单「打开」：用默认应用打开选中对象的本地副本
    /// （spec §14：下载到 Temporary Directory → NSWorkspace open）。
    pub(super) fn handle_open_object(
        &mut self,
        _: &OpenObject,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.is_some() || self.add_modal.is_some() || self.settings_modal.is_some() {
            return;
        }
        self.ensure_local_copy(
            |this, path, _key, cx| {
                if let Err(error) = object_storage_macos::open_with_default_app(&path) {
                    this.download_message = Some(DownloadMessage {
                        is_error: true,
                        text: format!("打开失败：{error}"),
                    });
                    cx.notify();
                } else {
                    this.download_message = Some(DownloadMessage {
                        is_error: false,
                        text: format!(
                            "已用默认应用打开：{}",
                            display_name(&path.display().to_string())
                        ),
                    });
                    cx.notify();
                }
            },
            cx,
        );
    }

    /// 对象菜单「在 Finder 中显示」（spec §16）：gpui reveal_path。
    pub(super) fn handle_reveal_in_finder(
        &mut self,
        _: &RevealInFinder,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.is_some() || self.add_modal.is_some() || self.settings_modal.is_some() {
            return;
        }
        self.ensure_local_copy(
            |this, path, _key, cx| {
                cx.reveal_path(&path);
                this.download_message = Some(DownloadMessage {
                    is_error: false,
                    text: format!(
                        "已在 Finder 中显示：{}",
                        display_name(&path.display().to_string())
                    ),
                });
                cx.notify();
            },
            cx,
        );
    }

    pub(super) fn start_text_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self.preview_text.clone() else {
            return;
        };
        let language = self
            .selected_cloud_object()
            .map(|object| syntax_language(&object.key))
            .unwrap_or("text");
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language)
                .default_value(text)
        });
        // 订阅 Change：按键即重渲染，保存按钮的 dirty 禁用态实时刷新
        cx.subscribe_in(&editor, window, |_, _, event: &InputEvent, _, cx| {
            if let InputEvent::Change = event {
                cx.notify();
            }
        })
        .detach();
        self.text_editor = Some(editor);
        cx.notify();
    }

    pub(super) fn ensure_preview_text_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.text_editor.is_some() {
            return;
        }
        if !self.preview_overlay_open {
            return;
        }
        let Some(text) = self.preview_text.clone() else {
            return;
        };
        let language = self
            .selected_cloud_object()
            .map(|object| syntax_language(&object.key))
            .unwrap_or("text");
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language)
                .default_value(text)
        });
        cx.subscribe_in(&editor, window, |_, _, event: &InputEvent, _, cx| {
            if let InputEvent::Change = event {
                cx.notify();
            }
        })
        .detach();
        self.text_editor = Some(editor);
    }

    pub(super) fn save_text_edit(&mut self, cx: &mut Context<Self>) {
        let (Some(editor), Some(path), Some(account_id), Some(bucket), Some(object)) = (
            self.text_editor.clone(),
            self.preview_path.clone(),
            self.selected_account_id.clone(),
            self.selected_bucket.clone(),
            self.selected_cloud_object()
                .map(|object| object.key.clone()),
        ) else {
            return;
        };
        let text = editor.read(cx).value().to_string();
        if let Err(error) = std::fs::write(&path, text.as_bytes()) {
            self.download_message = Some(DownloadMessage {
                is_error: true,
                text: format!("写入编辑内容失败：{error}"),
            });
            cx.notify();
            return;
        }
        self.preview_text = Some(text.clone());
        self.engine.enqueue_upload(
            &account_id,
            &bucket,
            &object,
            path,
            display_name(&object).to_string(),
        );
        self.download_message = Some(DownloadMessage {
            is_error: false,
            text: format!("已加入覆盖上传队列：{}", display_name(&object)),
        });
        cx.notify();
    }

    pub(super) fn confirm_and_save_text_edit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.text_editor.is_none() {
            return;
        }
        // 无改动不保存（与保存按钮禁用态同语义，⌘S 快捷键同此约束）
        if let (Some(editor), Some(original)) =
            (self.text_editor.as_ref(), self.preview_text.as_deref())
            && editor.read(cx).value().as_ref() == original
        {
            return;
        }
        if self.palette.is_some() || self.add_modal.is_some() {
            return;
        }
        if self.save_prompt_open || self.delete_prompt_open || self.quit_prompt_open {
            return;
        }
        let name = self
            .selected_cloud_object()
            .map(|object| display_name(&object.key).to_string())
            .unwrap_or_else(|| "对象".into());
        self.save_prompt_open = true;
        let message = format!("覆盖“{name}”？");
        let rx = window.prompt(
            PromptLevel::Warning,
            &message,
            Some("编辑后的内容将上传并覆盖远端对象，无法撤销。"),
            &[PromptButton::ok("保存并上传"), PromptButton::cancel("取消")],
            cx,
        );
        cx.spawn(async move |this, cx| {
            let answer = match rx.await {
                Ok(i) => i,
                Err(_) => {
                    this.update(cx, |this, _| this.save_prompt_open = false)
                        .ok();
                    return;
                }
            };
            this.update(cx, |this, cx| {
                this.save_prompt_open = false;
                if answer == 0 {
                    this.save_text_edit(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn copy_object_url(&mut self, cx: &mut Context<Self>) {
        if self.palette.is_some() || self.add_modal.is_some() {
            return;
        }
        if self.copying_url {
            return;
        }
        let request = match copy_object_url_request(
            self.selected_account_id.as_deref(),
            self.selected_bucket.as_deref(),
            self.selected_cloud_object(),
            self.settings.signed_url_ttl_secs,
        ) {
            Ok(request) => request,
            Err(message) => {
                self.download_message = Some(message);
                cx.notify();
                return;
            }
        };
        let clear_secs = self.settings.clipboard_clear_secs;
        self.copying_url = true;
        self.download_message = None;
        cx.notify();
        let services = Arc::clone(&self.services);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    services.signed_get_url(
                        &request.account_id,
                        &request.bucket,
                        &request.key,
                        request.ttl_secs,
                    )
                })
                .await;
            this.update(cx, |this, cx| {
                this.copying_url = false;
                match result {
                    Ok(url) => match object_storage_macos::copy_to_clipboard(&url) {
                        Ok(()) => {
                            let clear_note = if clear_secs > 0 {
                                format!("（{clear_secs} 秒后自动从剪贴板清除）")
                            } else {
                                String::new()
                            };
                            this.download_message = Some(DownloadMessage {
                                is_error: false,
                                text: format!("已复制签名链接{clear_note}"),
                            });
                            if clear_secs > 0 {
                                std::thread::spawn(move || {
                                    std::thread::sleep(std::time::Duration::from_secs(clear_secs));
                                    if let Err(error) =
                                        object_storage_macos::clear_clipboard_if_equals(&url)
                                    {
                                        eprintln!("[clipboard] 自动清除失败：{error}");
                                    }
                                });
                            }
                        }
                        Err(error) => {
                            this.download_message = Some(DownloadMessage {
                                is_error: true,
                                text: format!("写入剪贴板失败：{error}"),
                            });
                        }
                    },
                    Err(error) => {
                        this.download_message = Some(DownloadMessage {
                            is_error: true,
                            text: format!("复制链接失败：{error}"),
                        });
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn handle_copy_object_url(
        &mut self,
        _: &CopyObjectUrl,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_object_url(cx);
    }

    pub(super) fn handle_save_text_object(
        &mut self,
        _: &SaveTextObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_and_save_text_edit(window, cx);
    }

    pub(super) fn open_system_preview(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.preview_path.clone() else {
            return;
        };
        if let Err(error) = object_storage_macos::quick_look(&path) {
            self.download_message = Some(DownloadMessage {
                is_error: true,
                text: format!("打开 Quick Look 失败：{error}"),
            });
            cx.notify();
        }
    }

    /// 预览弹层：打开聚焦弹层（Esc 经焦点链命中 Overlay context），
    /// 关闭归还 Workspace 根。
    pub(super) fn open_preview_overlay(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.preview_overlay_open = true;
        // 焦点延迟到渲染挂载后设置（见 preview_needs_focus 注释）
        self.preview_needs_focus = true;
        self.object_menu_open = None;
        self.details_overlay_open = false;
        self.preview_open_quicklook = false;
        self.start_object_preview(cx);
    }

    pub(super) fn close_preview_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.preview_overlay_open = false;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    pub(super) fn handle_preview_object(
        &mut self,
        _: &PreviewObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 规范 §44：Space 预览 / 再按 Space 关闭（toggle 语义；Esc 同效）
        if self.preview_overlay_open {
            self.close_preview_overlay(window, cx);
            return;
        }
        self.open_preview_overlay(window, cx);
    }

    pub(super) fn render_preview_overlay(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(object) = self.selected_cloud_object() else {
            return div().into_any_element();
        };
        let name = display_name(&object.key).to_string();
        let kind = preview_kind(&object.key);
        let file_type = object.mime_type.clone().unwrap_or_else(|| match kind {
            PreviewKind::Image => "image/*".into(),
            PreviewKind::Text => format!("text/{}", syntax_language(&object.key)),
            PreviewKind::System => "未知类型".into(),
        });
        let error = self
            .download_message
            .as_ref()
            .filter(|message| message.is_error)
            .map(|message| message.text.clone());

        let preview_content = if self.previewing {
            v_flex()
                .size_full()
                .w_full()
                .items_center()
                .justify_center()
                .gap_3()
                .child(Spinner::new())
                .child(
                    div()
                        .text_size(tokens::body())
                        .text_color(theme.muted_foreground)
                        .child("正在准备预览…"),
                )
                .into_any_element()
        } else if let Some(error) = error {
            v_flex()
                .size_full()
                .w_full()
                .items_center()
                .justify_center()
                .gap_3()
                .p_6()
                .child(
                    Icon::new(IconName::TriangleAlert)
                        .text_size(tokens::icon_sm())
                        .text_color(theme.danger),
                )
                .child(
                    div()
                        .text_size(tokens::heading())
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(theme.foreground)
                        .child("预览失败"),
                )
                .child(
                    div()
                        .max_w(tokens::text(520.))
                        .text_size(tokens::label())
                        .text_color(theme.muted_foreground)
                        .child(error),
                )
                .into_any_element()
        } else if let Some(editor) = self.text_editor.clone() {
            v_flex()
                .size_full()
                .w_full()
                .gap_2()
                .child(
                    div()
                        .text_size(tokens::caption())
                        .text_color(theme.muted_foreground)
                        .child(format!("{} · 可编辑文本", syntax_language(&object.key))),
                )
                .child(Editor::new(&editor).flex_1().w_full())
                .into_any_element()
        } else if let Some(text) = self.preview_text.clone() {
            v_flex()
                .size_full()
                .w_full()
                .gap_2()
                .child(
                    div()
                        .text_size(tokens::caption())
                        .text_color(theme.muted_foreground)
                        .child(format!("{} · UTF-8", syntax_language(&object.key))),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .w_full()
                        .overflow_y_scrollbar()
                        .rounded(tokens::radius_lg())
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.sidebar)
                        .p_3()
                        .font_family(theme.mono_font_family.clone())
                        .text_size(theme.mono_font_size)
                        .child(text),
                )
                .into_any_element()
        } else if kind == PreviewKind::Image {
            match self.preview_path.clone() {
                Some(path) => div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .overflow_hidden()
                    .child(
                        // 不用 size_full：img 布局阶段的 aspect_ratio/自然尺寸推导
                        // 会把元素撑出固定高容器、被 overflow 裁成"按宽裁切"。
                        // max 约束让元素始终 ≤ 容器，paint 阶段 Contain 兜底等比。
                        // 元素尺寸即图像显示尺寸（元素按固有比例定尺寸，
                        // Contain 不会在元素内再留白），所以描边贴在图像边缘。
                        // 不加圆角：媒体容器本身是直角，img 只是居中在其内部，
                        // 给 img 加圆角没有同心依据（容器那条 theme.border 是
                        // 面板框架，不是图片描边，保留）。
                        img(path)
                            .max_w_full()
                            .max_h_full()
                            .object_fit(ObjectFit::Contain)
                            .border_1()
                            .border_color(crate::theme::image_outline(theme.mode))
                            .into_any_element(),
                    )
                    .into_any_element(),
                None => div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        crate::file_type::file_type_icon(
                            &object.key,
                            theme.muted_foreground,
                            theme.accent,
                        )
                        .text_size(tokens::icon_lg()),
                    )
                    .into_any_element(),
            }
        } else {
            v_flex()
                .size_full()
                .w_full()
                .items_center()
                .justify_center()
                .gap_3()
                .p_6()
                .child(
                    Icon::new(IconName::Eye)
                        .text_size(tokens::icon_sm())
                        .text_color(theme.muted_foreground),
                )
                .child(
                    div()
                        .text_size(tokens::heading())
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child("暂不支持应用内直接预览"),
                )
                .child(
                    div()
                        .max_w(tokens::text(420.))
                        .text_size(tokens::label())
                        .text_color(theme.muted_foreground)
                        .child("此文件可下载到本机后使用系统应用打开；PDF、Office、视频等格式建议使用系统 Quick Look。"),
                )
                .into_any_element()
        };

        let can_open_system = kind == PreviewKind::System && self.preview_path.is_some();
        let meta_label = |label: &'static str| {
            div()
                .w(tokens::text(110.))
                .flex_shrink_0()
                .text_size(tokens::label())
                .text_color(theme.muted_foreground)
                .child(label)
        };
        let meta_value = |value: String| {
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(tokens::label())
                .child(value)
        };
        let mask = overlay::mask(theme)
            .occlude()
            .key_context("Overlay")
            .track_focus(&self.overlay_focus)
            .on_action(cx.listener(|this, _: &UnifiedDismiss, window, cx| {
                this.close_preview_overlay(window, cx)
            }))
            .on_action(cx.listener(Self::handle_select_object_prev))
            .on_action(cx.listener(Self::handle_select_object_next))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    this.close_preview_overlay(window, cx)
                }),
            )
            .child(
                overlay::surface(theme)
                    .w_full()
                    .max_w(px(800.))
                    .overflow_hidden()
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .gap_3()
                            .px_4()
                            .pt_4()
                            .pb_3()
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(tokens::title())
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(name.clone()),
                            )
                            .child(
                                Button::new("close-preview-overlay")
                                    .icon(Icon::new(IconName::Close))
                                    .ghost()
                                    .with_size(Size::Small)
                                    .tooltip("关闭")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.close_preview_overlay(window, cx)
                                    })),
                            ),
                    )
                    .child(
                        div().w_full().px_4().child(
                            div()
                                .h(px(350.))
                                .w_full()
                                .overflow_hidden()
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.sidebar)
                                .child(preview_content),
                        ),
                    )
                    .child(
                        h_flex()
                            .mx_4()
                            .px_4()
                            .py_3()
                            .gap_5()
                            .border_l_1()
                            .border_r_1()
                            .border_b_1()
                            .border_color(theme.border)
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(meta_label("文件大小"))
                                    .child(meta_value(format!(
                                        "{} ({} B)",
                                        format_size(object.size),
                                        format_integer_grouped(object.size)
                                    ))),
                            )
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(meta_label("文件类型"))
                                    .child(meta_value(file_type)),
                            ),
                    )
                    .child(
                        h_flex()
                            .mx_4()
                            .my_3()
                            .px_1()
                            .gap_3()
                            .child(meta_label("对象 Key"))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(tokens::label())
                                    .child(object.key.clone()),
                            )
                            .when(self.preview_text.is_some(), |row| {
                                if self.text_editor.is_some() {
                                    // 内容与原文一致（无改动）时禁用保存：
                                    // 未编辑不产生覆盖上传，避免误触远端覆盖
                                    let text_dirty = self
                                        .text_editor
                                        .as_ref()
                                        .zip(self.preview_text.as_deref())
                                        .is_some_and(|(editor, original)| {
                                            editor.read(cx).value().as_ref() != original
                                        });
                                    row.child(
                                        Button::new("preview-overlay-save-text")
                                            .label("保存并上传")
                                            .primary()
                                            .with_size(Size::Small)
                                            .disabled(!text_dirty)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.confirm_and_save_text_edit(window, cx)
                                            })),
                                    )
                                } else {
                                    row.child(
                                        Button::new("preview-overlay-edit-text")
                                            .label("编辑")
                                            .with_size(Size::Small)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.start_text_edit(window, cx)
                                            })),
                                    )
                                }
                            })
                            .when(can_open_system, |row| {
                                row.child(
                                    Button::new("preview-overlay-quicklook")
                                        .label("系统预览")
                                        .with_size(Size::Small)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.open_system_preview(cx)
                                        })),
                                )
                            })
                            .child(
                                Button::new("preview-overlay-download")
                                    .label("下载")
                                    .ghost()
                                    .with_size(Size::Small)
                                    .disabled(self.downloading)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.start_object_download(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("preview-overlay-copy-url")
                                    .label("复制")
                                    .ghost()
                                    .with_size(Size::Small)
                                    .disabled(self.copying_url)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.copy_object_url(cx)),
                                    ),
                            ),
                    ),
            );
        overlay::fade_in("preview-overlay", mask).into_any_element()
    }
}

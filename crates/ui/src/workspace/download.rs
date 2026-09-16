//! 下载：单文件（另存为 / 默认目录确认）与批量（选目标目录）。

use super::*;

/// 默认下载目录有效性：设置值存在且是目录才可用。
/// 无效值（未设置/已被删除/指向文件）一律返回 None，由调用方退回面板默认位置。
pub(super) fn effective_default_download_dir(dir: Option<&Path>) -> Option<PathBuf> {
    dir.filter(|path| path.is_dir()).map(Path::to_path_buf)
}

/// 下载目标路径：目标目录 + 云端 Key 的末段文件名。
pub(super) fn download_dest_path(dest_dir: &Path, object_key: &str) -> PathBuf {
    dest_dir.join(display_name(object_key))
}

/// 单文件下载确认 sheet 文案（与批量下载同一交互结构：使用默认目录/另存为…/取消）。
pub(super) fn single_download_confirm_texts(
    object_key: &str,
    default_dir: &Path,
) -> (String, String) {
    (
        format!("将「{}」下载到默认目录。", display_name(object_key)),
        format!(
            "{}\n\n选择「另存为…」可保存到其他位置。",
            default_dir.display()
        ),
    )
}

/// 批量下载确认 sheet 文案：说明 GPUI 0.2.2 目录选择器无初始目录参数的限制。
pub(super) fn batch_download_confirm_texts(count: usize, default_dir: &Path) -> (String, String) {
    (
        format!("将 {count} 个对象下载到默认目录。"),
        format!(
            "{}\n\nGPUI 0.2.2 的目录选择器不支持设置初始目录；如需选择其他位置，将打开系统目录面板。",
            default_dir.display()
        ),
    )
}

impl WorkspaceView {
    /// 下载选中对象：与批量下载一致的确认交互——
    /// - 已设置有效默认目录：先弹确认 sheet（使用默认目录 / 另存为… / 取消）
    /// - 未设置：直接打开保存面板（初始目录 = HOME）
    ///   最终都经 gpui 平台 API（`cx.prompt_for_new_path`，异步回调）拿目标路径。
    ///   用户取消 = 无操作。
    ///
    /// 为什么必须用 gpui 平台 API、不能在事件处理器里同步 `runModal`：
    /// 模态循环期间 AppKit 事件会重入 gpui（borrow App RefCell），而外层处理器
    /// 还持有借用 → "RefCell already borrowed" panic 闪退。gpui 自带的面板从
    /// foreground executor 任务发起 `beginWithCompletionHandler:`，结果经
    /// oneshot 回传，天生规避重入。详见 docs/notes/gpui-api-notes.md「文件对话框」。
    pub(super) fn start_object_download(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.downloading {
            return; // 防重入
        }
        // 多选（≥2）走批量目录流程；单选维持原保存面板。
        if self.selected_object_keys.len() > 1 {
            self.start_batch_download(window, cx);
            return;
        }
        let Some(key) = self.selected_cloud_object().map(|o| o.key.clone()) else {
            return; // 未选中对象：无操作（按钮本应置灰）
        };
        let Some(account_id) = self.selected_account_id.clone() else {
            return;
        };
        let Some(bucket) = self.selected_bucket.clone() else {
            return;
        };

        self.downloading = true;
        self.download_message = None;
        cx.notify();

        // 默认目录确认 sheet：与批量下载交互对齐（使用默认目录/另存为…/取消）
        if let Some(default_dir) =
            effective_default_download_dir(self.settings.default_download_dir.as_deref())
        {
            let (message, detail) = single_download_confirm_texts(&key, &default_dir);
            let receiver = window.prompt(
                PromptLevel::Info,
                &message,
                Some(&detail),
                &[
                    PromptButton::ok("使用默认目录"),
                    PromptButton::new("另存为…"),
                    PromptButton::cancel("取消"),
                ],
                cx,
            );
            cx.spawn(async move |this, cx| {
                let answer = match receiver.await {
                    Ok(answer) => answer,
                    Err(_) => {
                        this.update(cx, |this, cx| {
                            this.downloading = false;
                            cx.notify();
                        })
                        .ok();
                        return;
                    }
                };
                this.update(cx, |this, cx| match answer {
                    0 => this.enqueue_single_download(&account_id, &bucket, &key, cx),
                    1 => {
                        let directory = this.download_initial_directory();
                        this.prompt_for_single_download_destination(
                            account_id, bucket, key, directory, cx,
                        )
                    }
                    _ => {
                        this.downloading = false;
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
            return;
        }

        // 无默认目录：保存面板初始目录 = HOME（仅影响初始位置）
        let directory = self.download_initial_directory();
        self.prompt_for_single_download_destination(account_id, bucket, key, directory, cx);
    }

    /// 单文件下载入队（默认目录路径）。
    pub(super) fn enqueue_single_download(
        &mut self,
        account_id: &str,
        bucket: &str,
        key: &str,
        cx: &mut Context<Self>,
    ) {
        let dest = download_dest_path(&self.download_initial_directory(), key);
        self.downloading = false;
        self.engine
            .enqueue_download(account_id, bucket, key, dest, display_name(key).to_string());
        self.download_message = Some(DownloadMessage {
            is_error: false,
            text: format!("已加入传输队列：{}", display_name(key)),
        });
        cx.notify();
    }

    /// 打开保存面板拿单文件目标路径（异步回调，规避模态重入）。
    pub(super) fn prompt_for_single_download_destination(
        &mut self,
        account_id: String,
        bucket: String,
        key: String,
        directory: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let suggested_name = display_name(&key).to_string();
        let receiver = cx.prompt_for_new_path(&directory, Some(&*suggested_name));

        cx.spawn(async move |this, cx| {
            // 面板结果在无借用作用域内先归一化，再一次性回主线程提交
            enum PanelOutcome {
                Picked(PathBuf),
                Cancelled,
                Failed(String),
            }
            let outcome = match receiver.await {
                Ok(Ok(Some(dest))) => PanelOutcome::Picked(dest),
                Ok(Ok(None)) => PanelOutcome::Cancelled,
                Ok(Err(e)) => PanelOutcome::Failed(format!("无法打开存储面板：{e}")),
                Err(_) => PanelOutcome::Failed("存储面板结果通道已关闭".into()),
            };

            let dest = match outcome {
                PanelOutcome::Picked(dest) => dest,
                PanelOutcome::Cancelled => {
                    // 用户取消：正常流程，静默复位
                    this.update(cx, |this, cx| {
                        this.downloading = false;
                        cx.notify();
                    })
                    .ok();
                    return;
                }
                PanelOutcome::Failed(text) => {
                    // 面板层异常：明确呈现，不静默（Fail Fast 的 UI 面）
                    this.update(cx, |this, cx| {
                        this.downloading = false;
                        this.download_message = Some(DownloadMessage {
                            is_error: true,
                            text,
                        });
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };

            // 入队即返回：排队/进度/结果全部由传输列表呈现（事件驱动）；
            // 下载从零重传（File::create 截断旧残留），断流续传在传输引擎后续里程碑
            this.update(cx, |this, cx| {
                this.downloading = false;
                this.engine.enqueue_download(
                    account_id.as_str(),
                    bucket.as_str(),
                    key.as_str(),
                    dest,
                    display_name(&key).to_string(),
                );
                this.download_message = Some(DownloadMessage {
                    is_error: false,
                    text: format!("已加入传输队列：{}", display_name(&key)),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn download_initial_directory(&self) -> PathBuf {
        effective_default_download_dir(self.settings.default_download_dir.as_deref())
            .or_else(std::env::home_dir)
            .unwrap_or_else(|| PathBuf::from("/"))
    }

    /// 批量下载（多选 ≥2）：先选目标目录（gpui `prompt_for_paths`
    /// `directories: true, multiple: false`）→ 逐项入队到该目录。
    /// 目标文件名 = display_name(key)；重名由传输引擎按路径覆盖写（File::create）。
    pub(super) fn start_batch_download(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.downloading {
            return;
        }
        let keys: Vec<String> = self.selected_object_keys.iter().cloned().collect();
        if keys.len() < 2 {
            return;
        }
        let Some(account_id) = self.selected_account_id.clone() else {
            return;
        };
        let Some(bucket) = self.selected_bucket.clone() else {
            return;
        };
        self.downloading = true;
        self.download_message = None;
        cx.notify();

        if let Some(default_dir) =
            effective_default_download_dir(self.settings.default_download_dir.as_deref())
        {
            let (message, detail) = batch_download_confirm_texts(keys.len(), &default_dir);
            let receiver = window.prompt(
                PromptLevel::Info,
                &message,
                Some(&detail),
                &[
                    PromptButton::ok("使用默认目录"),
                    PromptButton::new("另选目录…"),
                    PromptButton::cancel("取消"),
                ],
                cx,
            );
            cx.spawn(async move |this, cx| {
                let answer = match receiver.await {
                    Ok(answer) => answer,
                    Err(_) => {
                        this.update(cx, |this, cx| {
                            this.downloading = false;
                            cx.notify();
                        })
                        .ok();
                        return;
                    }
                };
                this.update(cx, |this, cx| match answer {
                    0 => this.enqueue_batch_downloads(&account_id, &bucket, &keys, default_dir, cx),
                    1 => this.prompt_for_batch_download_directory(account_id, bucket, keys, cx),
                    _ => {
                        this.downloading = false;
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
            return;
        }

        self.prompt_for_batch_download_directory(account_id, bucket, keys, cx);
    }

    pub(super) fn prompt_for_batch_download_directory(
        &mut self,
        account_id: String,
        bucket: String,
        keys: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择下载目录".into()),
        });

        cx.spawn(async move |this, cx| {
            enum PanelOutcome {
                Picked(PathBuf),
                Cancelled,
                Failed(String),
            }
            let outcome = match receiver.await {
                Ok(Ok(Some(paths))) => match paths.into_iter().next() {
                    Some(dir) => PanelOutcome::Picked(dir),
                    None => PanelOutcome::Cancelled,
                },
                Ok(Ok(None)) => PanelOutcome::Cancelled,
                Ok(Err(e)) => PanelOutcome::Failed(format!("无法打开目录面板：{e}")),
                Err(_) => PanelOutcome::Failed("目录面板结果通道已关闭".into()),
            };

            let dest_dir = match outcome {
                PanelOutcome::Picked(dir) => dir,
                PanelOutcome::Cancelled => {
                    this.update(cx, |this, cx| {
                        this.downloading = false;
                        cx.notify();
                    })
                    .ok();
                    return;
                }
                PanelOutcome::Failed(text) => {
                    this.update(cx, |this, cx| {
                        this.downloading = false;
                        this.download_message = Some(DownloadMessage {
                            is_error: true,
                            text,
                        });
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };

            this.update(cx, |this, cx| {
                this.enqueue_batch_downloads(&account_id, &bucket, &keys, dest_dir, cx)
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn enqueue_batch_downloads(
        &mut self,
        account_id: &str,
        bucket: &str,
        keys: &[String],
        dest_dir: PathBuf,
        cx: &mut Context<Self>,
    ) {
        self.downloading = false;
        for key in keys {
            let name = display_name(key).to_string();
            self.engine.enqueue_download(
                account_id,
                bucket,
                key.as_str(),
                download_dest_path(&dest_dir, key),
                name,
            );
        }
        self.download_message = Some(DownloadMessage {
            is_error: false,
            text: format!(
                "已加入传输队列：{} 个对象 → {}",
                keys.len(),
                dest_dir.display()
            ),
        });
        cx.notify();
    }

    pub(super) fn handle_download_object(
        &mut self,
        _: &DownloadObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_object_download(window, cx);
    }
}

//! 上传：文件/文件夹选择、目录递归收集、Finder 拖放。

use super::*;

pub(super) fn skip_folder_entry_name(name: &str) -> bool {
    name == ".DS_Store" || name == ".localized" || name.starts_with("._")
}

/// 上传大小上限判据（纯函数，单测锁死边界）。
///
/// - `cap_mb == 0` = 不限制，任何大小都放行。
/// - `size == cap` 视为**未超限**：「上限」是闭区间上界，正好等于上限的文件应当放行。
pub(super) fn upload_exceeds_cap(size_bytes: u64, cap_mb: u64) -> bool {
    cap_mb > 0 && size_bytes > cap_mb * 1024 * 1024
}

/// 被上限拦下的文件的状态条文案。`rejected` = (显示名, 字节数)。
///
/// 逐个点名（最多 3 个），而不是只说「N 个文件超限」——用户需要知道是哪几个，
/// 否则在大目录里无从下手。
pub(super) fn upload_cap_rejection_message(rejected: &[(String, u64)], cap_mb: u64) -> String {
    let cap = format_size(cap_mb * 1024 * 1024);
    if rejected.len() == 1 {
        let (name, size) = &rejected[0];
        return format!("未上传：{name}（{}）超过上限 {cap}", format_size(*size));
    }
    let shown: Vec<String> = rejected
        .iter()
        .take(3)
        .map(|(name, size)| format!("{name}（{}）", format_size(*size)))
        .collect();
    let tail = if rejected.len() > shown.len() {
        "…"
    } else {
        ""
    };
    format!(
        "未上传 {} 个超过上限 {cap} 的文件：{}{tail}",
        rejected.len(),
        shown.join("、")
    )
}

/// 递归收集目录内普通文件。不跟随符号链接。相对 key 以 `/` 连接。
pub(super) fn collect_folder_uploads(
    root: &std::path::Path,
) -> Result<Vec<FolderUploadFile>, String> {
    let folder_name = root
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("目录名不是合法 UTF-8：{}", root.display()))?;
    let meta = std::fs::symlink_metadata(root)
        .map_err(|e| format!("读取 {} 失败：{e}", root.display()))?;
    if meta.file_type().is_symlink() {
        return Err(format!("不跟随符号链接：{}", root.display()));
    }
    if !meta.is_dir() {
        return Err(format!("{} 不是目录", root.display()));
    }
    let mut out = Vec::new();
    walk_folder(root, folder_name, &mut out)?;
    Ok(out)
}

pub(super) fn walk_folder(
    dir: &std::path::Path,
    key_prefix: &str,
    out: &mut Vec<FolderUploadFile>,
) -> Result<(), String> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("读取目录 {} 失败：{e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("读取目录项失败：{e}"))?;
        let name_os = entry.file_name();
        let name = name_os
            .to_str()
            .ok_or_else(|| format!("文件名不是合法 UTF-8：{}", entry.path().display()))?;
        if skip_folder_entry_name(name) {
            continue;
        }
        let ft = entry
            .file_type()
            .map_err(|e| format!("读取 {} 类型失败：{e}", entry.path().display()))?;
        if ft.is_symlink() {
            continue;
        }
        let rel = format!("{key_prefix}/{name}");
        if ft.is_dir() {
            walk_folder(&entry.path(), &rel, out)?;
        } else if ft.is_file() {
            // 大小在这里顺手取（后台线程），供上传上限在入队前筛选
            let size = entry
                .metadata()
                .map_err(|e| format!("读取 {} 大小失败：{e}", entry.path().display()))?
                .len();
            out.push(FolderUploadFile {
                source: entry.path(),
                relative_key: rel,
                display_name: name.to_string(),
                size,
            });
        }
    }
    Ok(())
}

impl WorkspaceView {
    pub(super) fn handle_upload_files(
        &mut self,
        _: &UploadFiles,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_files_upload(cx);
    }

    pub(super) fn handle_upload_folder(
        &mut self,
        _: &UploadFolder,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_folder_upload(cx);
    }

    /// 上传所需的账号 / 空间 / 当前前缀。缺一则提示并返回 None。
    pub(super) fn upload_target(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<(String, String, String)> {
        let Some(account_id) = self.selected_account_id.clone() else {
            self.download_message = Some(DownloadMessage {
                is_error: true,
                text: "请先选中一个账号和空间再上传".into(),
            });
            cx.notify();
            return None;
        };
        let Some(bucket) = self.selected_bucket.clone() else {
            self.download_message = Some(DownloadMessage {
                is_error: true,
                text: "请先选中一个空间再上传".into(),
            });
            cx.notify();
            return None;
        };
        let prefix = self.current_prefix.clone().unwrap_or_default();
        Some((account_id, bucket, prefix))
    }

    /// 上传本地文件：gpui `prompt_for_paths`（多选，只要文件）→ 入队。
    /// 云端 key = 当前前缀 + 文件名。
    pub(super) fn start_files_upload(&mut self, cx: &mut Context<Self>) {
        if self.uploading {
            return;
        }
        let Some((account_id, bucket, prefix)) = self.upload_target(cx) else {
            return;
        };
        self.uploading = true;
        self.download_message = None;
        cx.notify();

        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("上传文件".into()),
        });

        cx.spawn(async move |this, cx| {
            enum PanelOutcome {
                Picked(Vec<PathBuf>),
                Cancelled,
                Failed(String),
            }
            let outcome = match receiver.await {
                Ok(Ok(Some(paths))) if !paths.is_empty() => PanelOutcome::Picked(paths),
                Ok(Ok(Some(_))) | Ok(Ok(None)) => PanelOutcome::Cancelled,
                Ok(Err(e)) => PanelOutcome::Failed(format!("无法打开文件面板：{e}")),
                Err(_) => PanelOutcome::Failed("文件面板结果通道已关闭".into()),
            };

            this.update(cx, |this, cx| {
                this.uploading = false;
                match outcome {
                    PanelOutcome::Cancelled => {}
                    PanelOutcome::Failed(text) => {
                        this.download_message = Some(DownloadMessage {
                            is_error: true,
                            text,
                        });
                    }
                    PanelOutcome::Picked(paths) => {
                        let cap_mb = this.settings.max_upload_size_mb;
                        let mut names = Vec::new();
                        let mut rejected = Vec::new();
                        for path in paths {
                            let name = path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("file")
                                .to_string();
                            // 上限拦截：超限文件不入队，逐个点名（与拖放同一判据）
                            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                            if upload_exceeds_cap(size, cap_mb) {
                                rejected.push((name, size));
                                continue;
                            }
                            let key = format!("{prefix}{name}");
                            this.engine.enqueue_upload(
                                account_id.as_str(),
                                bucket.as_str(),
                                key.as_str(),
                                path,
                                name.clone(),
                            );
                            names.push(name);
                        }
                        this.download_message =
                            Some(match (names.is_empty(), rejected.is_empty()) {
                                // 全部被拦下：只报拦截原因，不能报「已加入传输队列：」
                                (true, false) => DownloadMessage {
                                    is_error: true,
                                    text: upload_cap_rejection_message(&rejected, cap_mb),
                                },
                                (false, false) => DownloadMessage {
                                    is_error: false,
                                    text: format!(
                                        "已加入传输队列：{}；{}",
                                        names.join("、"),
                                        upload_cap_rejection_message(&rejected, cap_mb)
                                    ),
                                },
                                _ => DownloadMessage {
                                    is_error: false,
                                    text: format!("已加入传输队列：{}", names.join("、")),
                                },
                            });
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// 上传本地目录：`prompt_for_paths`（只要目录）→ 后台递归列举文件 → 入队。
    /// 云端 key = 当前前缀 + 目录名 + 相对路径（`/` 分隔，含顶层目录名）。
    pub(super) fn start_folder_upload(&mut self, cx: &mut Context<Self>) {
        if self.uploading {
            return;
        }
        let Some((account_id, bucket, prefix)) = self.upload_target(cx) else {
            return;
        };
        self.uploading = true;
        self.download_message = None;
        cx.notify();

        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("上传文件夹".into()),
        });

        cx.spawn(async move |this, cx| {
            enum PanelOutcome {
                Picked(Vec<PathBuf>),
                Cancelled,
                Failed(String),
            }
            let outcome = match receiver.await {
                Ok(Ok(Some(paths))) if !paths.is_empty() => PanelOutcome::Picked(paths),
                Ok(Ok(Some(_))) | Ok(Ok(None)) => PanelOutcome::Cancelled,
                Ok(Err(e)) => PanelOutcome::Failed(format!("无法打开目录面板：{e}")),
                Err(_) => PanelOutcome::Failed("目录面板结果通道已关闭".into()),
            };

            let roots = match outcome {
                PanelOutcome::Picked(paths) => paths,
                PanelOutcome::Cancelled => {
                    this.update(cx, |this, cx| {
                        this.uploading = false;
                        cx.notify();
                    })
                    .ok();
                    return;
                }
                PanelOutcome::Failed(text) => {
                    this.update(cx, |this, cx| {
                        this.uploading = false;
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

            let collected = cx
                .background_executor()
                .spawn(async move {
                    let mut files = Vec::new();
                    let mut errors = Vec::new();
                    for root in &roots {
                        match collect_folder_uploads(root) {
                            Ok(entries) => files.extend(entries),
                            Err(e) => errors.push(e),
                        }
                    }
                    (files, errors)
                })
                .await;

            this.update(cx, |this, cx| {
                this.uploading = false;
                let (files, errors) = collected;
                let cap_mb = this.settings.max_upload_size_mb;
                let mut rejected = Vec::new();
                let mut n = 0usize;
                for entry in files {
                    if upload_exceeds_cap(entry.size, cap_mb) {
                        rejected.push((entry.display_name, entry.size));
                        continue;
                    }
                    let key = format!("{prefix}{}", entry.relative_key);
                    this.engine.enqueue_upload(
                        account_id.as_str(),
                        bucket.as_str(),
                        key.as_str(),
                        entry.source,
                        entry.display_name,
                    );
                    n += 1;
                }
                if n == 0 {
                    let text = if !rejected.is_empty() {
                        upload_cap_rejection_message(&rejected, cap_mb)
                    } else if errors.is_empty() {
                        "目录为空，没有可上传的文件".into()
                    } else {
                        errors.join("；")
                    };
                    this.download_message = Some(DownloadMessage {
                        is_error: true,
                        text,
                    });
                } else {
                    let mut text = format!("已加入传输队列：{n} 个文件");
                    if !errors.is_empty() {
                        text.push_str("；部分目录失败：");
                        text.push_str(&errors.join("；"));
                    }
                    if !rejected.is_empty() {
                        text.push('；');
                        text.push_str(&upload_cap_rejection_message(&rejected, cap_mb));
                    }
                    this.download_message = Some(DownloadMessage {
                        is_error: !errors.is_empty(),
                        text,
                    });
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Finder / 其它 App 拖入文件：对象浏览区是投放目标（规范 §15）。
    pub(super) fn with_file_drop(
        &self,
        el: gpui::Div,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        // gpui 只在「已有 hover 样式 / 正在拖」时注册 mousemove→notify。
        // 系统文件拖入前一帧 active_drag 为空，不注册监听 → drag_over 永不刷新。
        // 空 hover() 让监听常驻；透明 2px 边框避免拖入时布局跳动。
        //
        // 拖入时的样子照参照实现：**灰色虚线框、整个内容区不加底色**（它只有一圈虚线，
        // 底色不变）。所以这里不设 `.bg(theme.drop_target)`——彩色底叠在虚线上会显得脏，
        // 也不是参照的观感。
        el.id("object-browser-drop")
            .border_2()
            .border_color(gpui::transparent_black())
            .hover(|style| style)
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.handle_dropped_paths(paths.paths(), cx);
            }))
            .drag_over::<ExternalPaths>(|style, _, _, cx| {
                style.border_dashed().border_color(cx.theme().drag_border)
            })
    }

    /// 入队一批本地文件（`path` + 已知字节数）。按上传大小上限**逐文件分流**：
    /// 能传的入队，超限的返回给调用方点名。返回 `(入队数, 超限项)`。
    pub(super) fn enqueue_file_uploads(
        &mut self,
        account_id: &str,
        bucket: &str,
        prefix: &str,
        files: Vec<(PathBuf, u64)>,
    ) -> (usize, Vec<(String, u64)>) {
        let cap_mb = self.settings.max_upload_size_mb;
        let mut enqueued = 0usize;
        let mut rejected = Vec::new();
        for (path, size) in files {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file")
                .to_string();
            if upload_exceeds_cap(size, cap_mb) {
                rejected.push((name, size));
                continue;
            }
            let key = format!("{prefix}{name}");
            self.engine
                .enqueue_upload(account_id, bucket, key.as_str(), path, name);
            enqueued += 1;
        }
        (enqueued, rejected)
    }

    pub(super) fn handle_dropped_paths(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        if paths.is_empty() {
            return;
        }
        let Some((account_id, bucket, prefix)) = self.upload_target(cx) else {
            return;
        };
        let mut files = Vec::new();
        let mut dirs = Vec::new();
        for path in paths {
            let meta = match std::fs::symlink_metadata(path) {
                Ok(m) => m,
                Err(e) => {
                    self.download_message = Some(DownloadMessage {
                        is_error: true,
                        text: format!("无法读取 {}：{e}", path.display()),
                    });
                    cx.notify();
                    return;
                }
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                dirs.push(path.clone());
            } else if meta.is_file() {
                // meta 已经取到了，大小顺手带上，不必再 stat
                files.push((path.clone(), meta.len()));
            }
        }
        if dirs.is_empty() {
            if files.is_empty() {
                self.download_message = Some(DownloadMessage {
                    is_error: true,
                    text: "没有可上传的文件（已跳过符号链接）".into(),
                });
            } else {
                let (n, rejected) = self.enqueue_file_uploads(&account_id, &bucket, &prefix, files);
                let cap_mb = self.settings.max_upload_size_mb;
                let text = match (n == 0, rejected.is_empty()) {
                    (true, false) => upload_cap_rejection_message(&rejected, cap_mb),
                    (false, false) => format!(
                        "已加入传输队列：{n} 个文件；{}",
                        upload_cap_rejection_message(&rejected, cap_mb)
                    ),
                    _ => format!("已加入传输队列：{n} 个文件"),
                };
                self.download_message = Some(DownloadMessage {
                    is_error: n == 0,
                    text,
                });
            }
            cx.notify();
            return;
        }

        cx.spawn(async move |this, cx| {
            let walked = cx
                .background_executor()
                .spawn(async move {
                    let mut entries = Vec::new();
                    let mut errors = Vec::new();
                    for dir in &dirs {
                        match collect_folder_uploads(dir) {
                            Ok(found) => entries.extend(found),
                            Err(e) => errors.push(e),
                        }
                    }
                    (entries, errors)
                })
                .await;
            this.update(cx, |this, cx| {
                let (n_files, mut rejected) =
                    this.enqueue_file_uploads(&account_id, &bucket, &prefix, files);
                let (entries, errors) = walked;
                let cap_mb = this.settings.max_upload_size_mb;
                let mut n_folder = 0usize;
                for entry in entries {
                    if upload_exceeds_cap(entry.size, cap_mb) {
                        rejected.push((entry.display_name, entry.size));
                        continue;
                    }
                    let key = format!("{prefix}{}", entry.relative_key);
                    this.engine.enqueue_upload(
                        account_id.as_str(),
                        bucket.as_str(),
                        key.as_str(),
                        entry.source,
                        entry.display_name,
                    );
                    n_folder += 1;
                }
                let n = n_files + n_folder;
                // 全部落空时才只报失败原因；只要有一项成功，成功与拦截信息并列
                let nothing_enqueued = n == 0;
                let mut text = if nothing_enqueued {
                    if errors.is_empty() && !rejected.is_empty() {
                        upload_cap_rejection_message(&rejected, cap_mb)
                    } else if errors.is_empty() {
                        "没有可上传的文件".into()
                    } else {
                        errors.join("；")
                    }
                } else {
                    format!("已加入传输队列：{n} 个文件")
                };
                if !nothing_enqueued {
                    if !errors.is_empty() {
                        text.push_str("；部分目录失败：");
                        text.push_str(&errors.join("；"));
                    }
                    if !rejected.is_empty() {
                        text.push('；');
                        text.push_str(&upload_cap_rejection_message(&rejected, cap_mb));
                    }
                }
                this.download_message = Some(DownloadMessage {
                    is_error: nothing_enqueued,
                    text,
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

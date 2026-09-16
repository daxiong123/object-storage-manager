//! 删除（⌘⌫）：确认后逐项删除，失败逐项可见不中断。

use super::*;

/// 删除确认里的明细摘要：单对象为空串（标题已含名字）；多对象列出
/// 前几个名字，超出截断（确认框不放长列表）。
pub(super) fn delete_summary(keys: &[String]) -> String {
    const MAX_NAMES: usize = 3;
    if keys.len() <= 1 {
        return String::new();
    }
    let names: Vec<String> = keys
        .iter()
        .take(MAX_NAMES)
        .map(|k| display_name(k).to_string())
        .collect();
    if keys.len() > MAX_NAMES {
        format!("（{} 等 {} 个）", names.join("、"), keys.len())
    } else {
        format!("（{}）", names.join("、"))
    }
}

impl WorkspaceView {
    pub(super) fn handle_delete_object(
        &mut self,
        _: &DeleteObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_and_delete_object(window, cx);
    }

    /// 远端删除必须确认（规范 §43，无废纸篓）。⌘⌫ / 菜单共用。
    /// 支持多选：选中多项时逐项删除，失败逐项可见（不静默）。
    pub(super) fn confirm_and_delete_object(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.is_some() || self.add_modal.is_some() {
            return;
        }
        if self.deleting || self.delete_prompt_open || self.quit_prompt_open {
            return;
        }
        // 多选集合（主选兼容：单选时两者一致）
        let keys: Vec<String> = if self.selected_object_keys.is_empty() {
            self.selected_cloud_object()
                .map(|o| vec![o.key.clone()])
                .unwrap_or_default()
        } else {
            self.selected_object_keys.iter().cloned().collect()
        };
        if keys.is_empty() {
            self.download_message = Some(DownloadMessage {
                is_error: true,
                text: "请先选中一个对象再删除".into(),
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
        let count = keys.len();
        let summary = delete_summary(&keys);
        self.delete_prompt_open = true;
        let message = if count == 1 {
            format!("删除“{}”？", display_name(&keys[0]))
        } else {
            format!("删除 {count} 个对象？")
        };
        let detail = format!("将从空间 {bucket} 永久删除{summary}，无法撤销。");
        let rx = window.prompt(
            PromptLevel::Warning,
            &message,
            Some(&detail),
            &[PromptButton::ok("删除"), PromptButton::cancel("取消")],
            cx,
        );
        let services = Arc::clone(&self.services);
        cx.spawn(async move |this, cx| {
            let answer = match rx.await {
                Ok(i) => i,
                Err(_) => {
                    this.update(cx, |this, _| this.delete_prompt_open = false)
                        .ok();
                    return;
                }
            };
            if answer != 0 {
                this.update(cx, |this, _| this.delete_prompt_open = false)
                    .ok();
                return;
            }
            this.update(cx, |this, cx| {
                this.delete_prompt_open = false;
                this.deleting = true;
                this.download_message = None;
                cx.notify();
            })
            .ok();
            // 逐项删除（多选批量 = 循环单删）：任一失败不中断剩余项，
            // 结束后汇总成功/失败明细，失败逐项可见（不静默）。
            let results = cx
                .background_executor()
                .spawn(async move {
                    let mut ok = Vec::new();
                    let mut failed = Vec::new();
                    for key in keys {
                        match services.delete_object(&account_id, &bucket, &key) {
                            Ok(()) => ok.push(key),
                            Err(e) => failed.push((key, e.to_string())),
                        }
                    }
                    (ok, failed)
                })
                .await;
            this.update(cx, |this, cx| {
                this.deleting = false;
                let (ok, failed) = results;
                if !ok.is_empty() {
                    this.clear_object_selection();
                    this.reload_objects(cx);
                }
                if failed.is_empty() {
                    let text = if ok.len() == 1 {
                        format!("已删除 {}", display_name(&ok[0]))
                    } else {
                        format!("已删除 {} 个对象", ok.len())
                    };
                    this.download_message = Some(DownloadMessage {
                        is_error: false,
                        text,
                    });
                } else {
                    let failed_text = failed
                        .iter()
                        .map(|(key, err)| format!("{}：{err}", display_name(key)))
                        .collect::<Vec<_>>()
                        .join("；");
                    let text = if ok.is_empty() {
                        format!("删除失败：{failed_text}")
                    } else {
                        format!(
                            "已删除 {} 个，失败 {} 个：{}",
                            ok.len(),
                            failed.len(),
                            failed_text
                        )
                    };
                    this.download_message = Some(DownloadMessage {
                        is_error: true,
                        text,
                    });
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

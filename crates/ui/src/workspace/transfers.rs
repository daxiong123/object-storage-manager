//! 传输面板与队列：持久化恢复、watch 订阅、任务行渲染。

use super::*;

/// 活动任务 → 持久化行。Running/Waiting/Queued 落成 queued（下次自动继续），
/// 用户暂停保持 paused。终态任务不落盘。
/// `region_of` 从 AppServices 会话缓存取腾讯云 bucket 的地域随行落盘——
/// 否则重启后恢复的任务在列表发生前就要解析地域，手填 Bucket 且无
/// `cos:GetService` 权限的账号会立刻失败。
pub(super) fn persistable_from_snapshot(
    tasks: &[TransferTask],
    region_of: impl Fn(&str, &str) -> Option<String>,
) -> Vec<PersistedTransfer> {
    tasks
        .iter()
        .filter(|t| t.state.is_active())
        .map(|t| {
            let (kind, account_id, bucket, key, local) = match &t.kind {
                TransferKind::Download {
                    account_id,
                    bucket,
                    key,
                    dest,
                } => (
                    "download",
                    account_id.clone(),
                    bucket.clone(),
                    key.clone(),
                    dest.clone(),
                ),
                TransferKind::Upload {
                    account_id,
                    bucket,
                    key,
                    source,
                } => (
                    "upload",
                    account_id.clone(),
                    bucket.clone(),
                    key.clone(),
                    source.clone(),
                ),
            };
            let region = region_of(&account_id, &bucket);
            let state = if t.state == TransferState::Paused {
                "paused"
            } else {
                "queued"
            };
            PersistedTransfer {
                kind: kind.into(),
                account_id,
                bucket,
                object_key: key,
                dest: local.to_string_lossy().into_owned(),
                display_name: t.display_name.clone(),
                state: state.into(),
                region,
                enqueued_at_millis: t.enqueued_at_millis as i64,
            }
        })
        .collect()
}

impl WorkspaceView {
    /// 启动时取出上次 ⌘Q「暂停并退出」留下的队列，入引擎后按原状态恢复。
    /// SQLite 走后台执行器；入队发生在首帧前后的短窗口，用户来不及抢点下载。
    pub(super) fn restore_persisted_transfers(&mut self, cx: &mut Context<Self>) {
        let services = Arc::clone(&self.services);
        cx.spawn(async move |this, cx| {
            let taken = {
                let services = Arc::clone(&services);
                cx.background_executor()
                    .spawn(async move { services.take_transfers() })
                    .await
            };
            this.update(cx, |this, cx| {
                match taken {
                    Ok(items) if !items.is_empty() => {
                        // 不 suspend/resume：尊重网络监视器当前挂起标志。
                        // 引擎已挂起时入队停在 Queued；未挂起则按并发上限启动。
                        for item in items {
                            // 落盘的地域先回填进会话缓存：恢复的任务构建
                            // provider 时直接带上，不再触碰 service 端点
                            if let Some(region) = &item.region {
                                services.remember_bucket_region(
                                    &item.account_id,
                                    &item.bucket,
                                    region,
                                );
                            }
                            let local = PathBuf::from(item.dest);
                            let id = if item.kind == "upload" {
                                this.engine.enqueue_upload(
                                    item.account_id,
                                    item.bucket,
                                    item.object_key,
                                    local,
                                    item.display_name,
                                )
                            } else {
                                this.engine.enqueue_download(
                                    item.account_id,
                                    item.bucket,
                                    item.object_key,
                                    local,
                                    item.display_name,
                                )
                            };
                            if item.state == "paused" {
                                this.engine.pause(id);
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        this.download_message = Some(DownloadMessage {
                            is_error: true,
                            text: format!("读取已保存的传输队列失败：{e}"),
                        });
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// 订阅引擎状态变更令牌：任何任务状态突变 → 取快照回填 → notify。
    /// 常驻 async 任务，纯事件驱动（规范禁轮询）。引擎/视图任一消亡即退出。
    pub(super) fn subscribe_transfers(engine: Arc<TransferEngine>, cx: &mut Context<Self>) {
        let mut changes = engine.subscribe();
        cx.spawn(async move |this, cx| {
            loop {
                if changes.changed().await.is_err() {
                    break; // 引擎已销毁（应用退出路径）
                }
                let snapshot = engine.snapshot();
                let alive = this
                    .update(cx, |this, cx| {
                        this.transfers = snapshot;
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    break; // 视图已释放
                }
            }
        })
        .detach();
    }

    /// 传输面板：底部状态条上方，展示进行中的上传/下载。收起态一行汇总
    /// （转圈 + 数量 + 总进度条 + 展开箭头）；展开态列出每任务明细
    /// （名字 + 进度条 + 字节 + 状态 + 取消/继续/重试按钮）。
    pub(super) fn render_transfer_panel(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let transfers = &self.transfers;
        if transfers.is_empty() {
            return div().into_any_element();
        }

        let active: Vec<&TransferTask> = transfers.iter().filter(|t| t.state.is_active()).collect();
        let finished: Vec<&TransferTask> =
            transfers.iter().filter(|t| t.state.is_finished()).collect();
        let total_count = transfers.len();
        let active_count = active.len();

        // 总进度：所有任务的 bytes_done / bytes_total 汇总
        let total_done: u64 = transfers.iter().map(|t| t.bytes_done).sum();
        let total_size: u64 = transfers.iter().filter_map(|t| t.bytes_total).sum();
        let overall_pct = if total_size > 0 {
            (total_done as f32 / total_size as f32 * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };
        let has_running = active.iter().any(|t| t.state == TransferState::Running);

        let summary = h_flex()
            .id("transfer-summary")
            .w_full()
            .h(tokens::text(32.))
            // 右侧是尾部展开图标：图标侧内边距比文字侧小 2px，
            // 否则对称内边距下箭头看起来被推得偏右（视觉对齐而非几何居中）
            .pl_3()
            .pr_2p5()
            .gap_2()
            .items_center()
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.list_head)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, _, cx| {
                this.transfers_expanded = !this.transfers_expanded;
                cx.notify();
            }))
            .when(has_running, |row| {
                row.child(Spinner::new().with_size(Size::Small))
            })
            .child(
                div()
                    .text_size(tokens::label())
                    .text_color(theme.foreground)
                    .child(if active_count > 0 {
                        format!("{active_count} 个传输中")
                    } else {
                        format!("{total_count} 个已完成")
                    }),
            )
            .child(div().h(px(4.)))
            .child(
                Progress::new("transfer-overall-progress")
                    .accessibility_label("总体传输进度")
                    .h(px(4.))
                    .value(overall_pct)
                    .flex_1(),
            )
            .child(
                div()
                    .text_size(tokens::caption())
                    .text_color(theme.muted_foreground)
                    .child(if total_size > 0 {
                        format!("{} / {}", format_size(total_done), format_size(total_size))
                    } else {
                        format_size(total_done)
                    }),
            )
            .child(Icon::new(if self.transfers_expanded {
                IconName::ChevronDown
            } else {
                IconName::ChevronUp
            }));

        let mut panel = v_flex().w_full().flex_shrink_0().child(summary);

        if self.transfers_expanded {
            for task in transfers {
                panel = panel.child(self.transfer_task_row(task.clone(), theme, cx));
            }
            if !finished.is_empty() {
                panel = panel.child(
                    h_flex().w_full().justify_end().px_3().py_1().child(
                        Button::new("transfers-clear-finished")
                            .label("清除已完成")
                            .ghost()
                            .with_size(Size::Small)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.engine.clear_finished();
                                this.transfers = this.engine.snapshot();
                                cx.notify();
                            })),
                    ),
                );
            }
        }

        panel.into_any_element()
    }

    pub(super) fn transfer_task_row(
        &self,
        task: TransferTask,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let pct = match task.bytes_total {
            Some(total) if total > 0 => {
                (task.bytes_done as f32 / total as f32 * 100.0).clamp(0.0, 100.0)
            }
            _ => 0.0,
        };
        let bytes_label = match task.bytes_total {
            Some(total) => format!("{} / {}", format_size(task.bytes_done), format_size(total)),
            None => format_size(task.bytes_done),
        };
        let state_color = match task.state {
            TransferState::Failed => theme.danger,
            TransferState::Completed => theme.success,
            _ => theme.muted_foreground,
        };

        // 分隔线与底色在外层容器上：失败原因要独占一行（规范：完整换行
        // 展示、不 truncate），所以它不能是主行的兄弟节点——放主行里会和
        // 按钮抢横向空间，被压成窄条逐字换行。
        let mut block = v_flex()
            .id(("transfer-row", task.id.0))
            .w_full()
            .border_t_1()
            .border_color(theme.table_row_border)
            .bg(theme.list_head);

        let mut row = h_flex()
            .w_full()
            .px_3()
            .py(tokens::row_pad_y_transfer())
            .gap_2()
            .items_center()
            .text_size(tokens::label())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(theme.foreground)
                    .child(task.display_name.clone()),
            )
            .child(
                Progress::new(("transfer-task-progress", task.id.0))
                    .accessibility_label(format!("{} 的传输进度", task.display_name))
                    .h(px(4.))
                    .value(pct)
                    // 宽度随字号缩放（tokens::text 而非裸 px）：这条与左右文字同排
                    .w(tokens::text(80.)),
            )
            .child(
                div()
                    .w(tokens::text(96.))
                    .flex_shrink_0()
                    .text_color(theme.muted_foreground)
                    .text_size(tokens::caption())
                    .child(bytes_label),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_color(state_color)
                    .child(task.state.label()),
            );

        // 操作按钮：活动态可取消，暂停/失败可继续（重试）
        match task.state {
            TransferState::Queued
            | TransferState::Running
            | TransferState::Waiting
            | TransferState::Paused => {
                row = row.child(
                    Button::new(("transfer-cancel", task.id.0))
                        .label("取消")
                        .ghost()
                        .with_size(Size::Small)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.engine.cancel(task.id);
                            this.transfers = this.engine.snapshot();
                            cx.notify();
                        })),
                );
            }
            TransferState::Failed => {
                row = row.child(
                    Button::new(("transfer-resume", task.id.0))
                        .label("重试")
                        .ghost()
                        .with_size(Size::Small)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.engine.resume(task.id);
                            this.transfers = this.engine.snapshot();
                            cx.notify();
                        })),
                );
            }
            TransferState::Completed | TransferState::Cancelled => {}
        }

        // 主行在上，失败原因在下方独占一行（全宽换行，不 truncate）
        block = block.child(row);
        if let Some(error) = &task.error {
            block = block.child(
                div()
                    .w_full()
                    .px_3()
                    .pb(tokens::row_pad_y_transfer())
                    .text_color(theme.danger)
                    .text_size(tokens::caption())
                    .line_height(gpui::DefiniteLength::Fraction(1.4))
                    .child(error.clone()),
            );
        }

        block
    }
}

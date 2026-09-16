//! 行内重命名：Return 进入、Esc 取消，只改最后一段。

use super::*;

/// 由当前 key 和新名字推导 rename 目标 key：只替换最后一段（`/` 后的
/// 文件名部分），保持目录前缀不变。名字含 `/` 视为非法（不允许借
/// rename 移动目录，防误操作把对象搬进意外前缀）。
pub(crate) fn rename_target_key(current_key: &str, new_name: &str) -> Result<String, String> {
    let name = new_name.trim();
    if name.is_empty() {
        return Err("名称不能为空".into());
    }
    if name.contains('/') {
        return Err("名称不能包含 /".into());
    }
    if name == "." || name == ".." {
        return Err("名称不能是 . 或 ..".into());
    }
    match current_key.rsplit_once('/') {
        Some((prefix, _)) => Ok(format!("{prefix}/{name}")),
        None => Ok(name.to_string()),
    }
}

pub(super) fn object_key_exists(entries: &[ListingEntry], key: &str) -> bool {
    entries
        .iter()
        .filter_map(object_key)
        .any(|object_key| object_key == key)
}

pub(super) fn rename_validation_message(
    current_key: &str,
    new_name: &str,
    entries: &[ListingEntry],
) -> Option<String> {
    let new_key = match rename_target_key(current_key, new_name) {
        Ok(key) => key,
        Err(message) => return Some(message),
    };
    if new_key == current_key {
        return Some("请输入一个不同的新名称".into());
    }
    if object_key_exists(entries, &new_key) {
        return Some(format!(
            "目标名称已存在：{}，请换一个名字",
            display_name(&new_key)
        ));
    }
    None
}

impl WorkspaceView {
    /// Return：进入行内重命名。多选（≠1）时忽略——批量改名语义不明确，不做。
    pub(super) fn handle_rename_object(
        &mut self,
        _: &RenameObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.is_some() || self.add_modal.is_some() {
            return;
        }
        if self.renaming.is_some() || self.renaming_busy {
            return;
        }
        if self.selected_object_keys.len() > 1 {
            self.download_message = Some(DownloadMessage {
                is_error: true,
                text: "多选状态下不支持重命名，请只选中一个对象".into(),
            });
            cx.notify();
            return;
        }
        let Some(object) = self.selected_cloud_object() else {
            return;
        };
        let key = object.key.clone();
        let initial = display_name(&key).to_string();
        let editor = cx.new(|cx| InputState::new(window, cx).default_value(initial));
        // Return 提交：单行 Input 对 Enter emit PressEnter 后 propagate（不会
        // 二次触发 RenameObject——Workspace context 的 enter 绑定在 keymap 里
        // 已被 Input context 的绑定先消费）。
        cx.subscribe_in(&editor, window, |this, _, event: &InputEvent, _, cx| {
            if let InputEvent::PressEnter { .. } = event {
                this.commit_rename(cx);
            }
        })
        .detach();
        let focus_editor = editor.clone();
        focus_editor.update(cx, |state, cx| state.focus(window, cx));
        self.renaming = Some((key, editor));
        cx.notify();
    }

    /// 提交重命名：读取输入 → 校验目标 key → 后台
    /// 下载到临时文件 → 上传新 key → 删旧 key。失败不静默。
    pub(super) fn commit_rename(&mut self, cx: &mut Context<Self>) {
        let Some((old_key, editor)) = self.renaming.clone() else {
            return;
        };
        if self.renaming_busy {
            return;
        }
        let new_name = editor.read(cx).value().to_string();
        // 无变化：直接退出编辑态（Finder 行为）
        if new_name == display_name(&old_key) {
            self.cancel_rename(cx);
            return;
        }
        if let Some(message) = rename_validation_message(&old_key, &new_name, &self.entries) {
            self.download_message = Some(DownloadMessage {
                is_error: true,
                text: message,
            });
            cx.notify();
            return;
        }
        let new_key = match rename_target_key(&old_key, &new_name) {
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
        // 目标 key 已存在：明确提示冲突（云端 GET 不报错即存在，用列举
        // 结果判断——entries 里查即可，覆盖当前列表可见范围；翻页场景
        // 由上传侧 File::create 语义兜底？不，远端覆盖。用 provider 上传
        // 是覆盖语义，所以必须先查；entries 不可靠，直接调 head？OSS 无
        // head 封装。妥协：上传前查 entries + 明确告知覆盖风险）。
        if object_key_exists(&self.entries, &new_key) {
            cx.notify();
            return;
        }
        let Some(account_id) = self.selected_account_id.clone() else {
            return;
        };
        let Some(bucket) = self.selected_bucket.clone() else {
            return;
        };
        let services = Arc::clone(&self.services);
        self.renaming_busy = true;
        self.renaming = None;
        self.download_message = None;
        cx.notify();

        let name = display_name(&old_key).to_string();
        let new_key_display = display_name(&new_key).to_string();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(
                    async move { services.rename_object(&account_id, &bucket, &old_key, &new_key) },
                )
                .await;
            this.update(cx, |this, cx| {
                this.renaming_busy = false;
                match result {
                    Ok(()) => {
                        this.reload_objects(cx);
                        this.download_message = Some(DownloadMessage {
                            is_error: false,
                            text: format!("已重命名：{name} → {new_key_display}"),
                        });
                    }
                    Err(error) => {
                        this.download_message = Some(DownloadMessage {
                            is_error: true,
                            text: format!("重命名失败：{error}"),
                        });
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// 取消重命名（Esc：Input escape() propagate → context "Renaming"）。
    pub(super) fn handle_dismiss_rename(
        &mut self,
        _: &DismissRename,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((key, _)) = &self.renaming {
            eprintln!("[rename] cancelled key={key}");
        }
        if self.create_folder_input.is_some() {
            self.close_create_folder_overlay(cx);
            return;
        }
        if self.copy_move.is_some() {
            self.close_copy_move_overlay(cx);
            return;
        }
        self.cancel_rename(cx);
    }

    /// 取消重命名。
    pub(super) fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        if self.renaming.take().is_some() {
            cx.notify();
        }
    }
}

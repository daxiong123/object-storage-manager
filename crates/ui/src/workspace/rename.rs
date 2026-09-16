//! 重命名弹层：原路径 + 重命名输入 + 目录说明 + 确认/取消（照参照实现）。
//!
//! 曾是**行内编辑**（Return 后该行变成 Input）。改成弹层的原因：行高被虚拟列表定死
//! （`uniform_list` 拿第 0 行的高度给所有行），行内塞不下第二行文字，校验提示只能挪到
//! 列表上方当横幅；而「原路径 / 新名称 / 影响范围说明 / 确认」四件事，弹层一次讲得清。

use super::*;

/// 名称没变时的校验文案。它**不算错误**——用户刚打开弹层就是这个状态——
/// 所以确认按钮据此置灰，但不显示成红色提示。
pub(crate) const RENAME_UNCHANGED: &str = "请输入一个不同的新名称";

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
        return Some(RENAME_UNCHANGED.into());
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
    /// Return：打开重命名弹层。多选（≠1）时不弹——批量改名语义不明确，不做。
    pub(super) fn handle_rename_object(
        &mut self,
        _: &RenameObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 已有别的浮层时不开：弹层叠弹层会让 Esc 与遮罩点击的归属无法推理。
        if self.palette.is_some()
            || self.add_modal.is_some()
            || self.settings_modal.is_some()
            || self.preview_overlay_open
            || self.details_overlay_open
        {
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
        // 不设 `clean_on_escape()`：Esc 要能从输入框冒泡到 context "Renaming"
        // 触发 DismissRename，否则焦点在输入框里时 Esc 关不掉弹层。
        let editor = cx.new(|cx| InputState::new(window, cx).default_value(initial));
        cx.subscribe_in(
            &editor,
            window,
            |this, _, event: &InputEvent, _, cx| match event {
                // 回车 = 确认修改（与「确认修改」按钮同一条路径）
                InputEvent::PressEnter { .. } => this.commit_rename(cx),
                // 输入变化要重渲染：确认按钮的可用态与校验提示都是即时算的
                InputEvent::Change => cx.notify(),
                _ => {}
            },
        )
        .detach();
        editor.update(cx, |state, cx| state.focus(window, cx));
        self.renaming = Some((key, editor));
        self.rename_error = None;
        cx.notify();
    }

    /// 重命名弹层（遮罩 / Esc 上下文 / 遮罩点击都归这里，卡片外观在
    /// `rename_dialog`）。
    pub(super) fn render_rename_overlay(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some((old_key, editor)) = self.renaming.as_ref() else {
            return div().into_any_element();
        };
        let original_name = display_name(old_key).to_string();
        let current_name = editor.read(cx).value().to_string();
        let validation = rename_validation_message(old_key, &current_name, &self.entries);
        let can_confirm = !self.renaming_busy && validation.is_none();
        // 「名称没变」不当错误显示（用户可能只是打开看一眼），只让按钮置灰。
        let message = self
            .rename_error
            .clone()
            .or_else(|| validation.filter(|message| message != RENAME_UNCHANGED));

        overlay::fade_in(
            "rename-overlay",
            overlay::mask(theme)
                .occlude()
                .key_context("Renaming")
                .on_action(cx.listener(Self::handle_dismiss_rename))
                // 遮罩点击 = 取消（进行中由 `cancel_rename` 自己拒绝）。
                // 先 stop_propagation：遮罩下面就是对象列表，否则这次点击会顺带
                // 清了列表的选择（列表容器的空白点击语义）。
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                        cx.stop_propagation();
                        this.cancel_rename(cx);
                    }),
                )
                .child(rename_dialog(
                    theme,
                    &original_name,
                    editor,
                    message,
                    self.renaming_busy,
                    can_confirm,
                    std::rc::Rc::new(cx.listener(|this, _, _, cx| this.cancel_rename(cx))),
                    std::rc::Rc::new(cx.listener(|this, _, _, cx| this.commit_rename(cx))),
                )),
        )
        .into_any_element()
    }
}

/// 弹层按钮的回调。用 `Rc` 而不是 `impl Fn`：取消回调要同时挂在标题栏的 ✕ 与
/// 底部的「取消」两处。
pub type RenameDialogCallback = std::rc::Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// 重命名弹层的**卡片外观**（标题 / 原路径 / 新名称输入 / 说明 / 按钮）。
///
/// 与业务接线解耦（校验、busy、回调都由调用方给）是为了能被离屏预览渲染出来做
/// 视觉验收——`crates/desktop/examples/view_preview.rs` 的 `rename` 模式画的就是
/// 这个函数，否则这个弹层的排版只能靠人肉去应用里点开看。
#[allow(clippy::too_many_arguments)]
pub fn rename_dialog(
    theme: &Theme,
    original_name: &str,
    editor: &Entity<InputState>,
    message: Option<String>,
    busy: bool,
    can_confirm: bool,
    on_cancel: RenameDialogCallback,
    on_confirm: RenameDialogCallback,
) -> impl IntoElement {
    overlay::surface(theme)
        .w_full()
        .max_w(px(440.))
        .p_4()
        .gap_3()
        .child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .text_size(tokens::title())
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child("重命名"),
                )
                .child(
                    ui::icon_button("close-rename-overlay", Icon::new(IconName::Close), "关闭")
                        .disabled(busy)
                        .on_click(forward(on_cancel.clone())),
                ),
        )
        .child(rename_field(
            theme,
            "原路径：",
            div()
                .min_w_0()
                .truncate()
                .text_size(tokens::body())
                .child(original_name.to_string()),
        ))
        .child(
            v_flex()
                .gap_1()
                .child(rename_field(
                    theme,
                    "重命名：",
                    Input::new(editor).disabled(busy),
                ))
                .children(message.map(|message| {
                    h_flex()
                        .gap_2()
                        .text_size(tokens::label())
                        .text_color(theme.danger)
                        .child(Icon::new(IconName::TriangleAlert))
                        .child(div().min_w_0().child(message))
                })),
        )
        .child(
            div()
                .text_size(tokens::label())
                .text_color(theme.muted_foreground)
                .child("请注意，若您针对目录重命名则该目录下所有路径名称将一并修改"),
        )
        .child(
            h_flex()
                .w_full()
                .justify_end()
                .gap_2()
                .border_t_1()
                .border_color(theme.border)
                .pt_3()
                .child(
                    Button::new("rename-cancel")
                        .label("取消")
                        .ghost()
                        .with_size(Size::Small)
                        .disabled(busy)
                        .on_click(forward(on_cancel.clone())),
                )
                .child(
                    Button::new("rename-confirm")
                        .label(if busy {
                            "重命名中…"
                        } else {
                            "确认修改"
                        })
                        .primary()
                        .loading(busy)
                        .with_size(Size::Small)
                        .disabled(!can_confirm)
                        .on_click(forward(on_confirm.clone())),
                ),
        )
}

/// 把 `Rc<dyn Fn>` 适配成 `on_click` 要的所有权闭包（同一个回调可能挂在多处）。
fn forward(handler: RenameDialogCallback) -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    move |event, window, cx| handler(event, window, cx)
}

/// 弹层里的「标签 + 值」一行：标签列宽走 `tokens::text`（随字号缩放），
/// 值列占满剩余宽度。
fn rename_field(theme: &Theme, label: &'static str, value: impl IntoElement) -> impl IntoElement {
    h_flex()
        .items_start()
        .gap_3()
        .child(
            div()
                .w(tokens::text(56.))
                .flex_shrink_0()
                .text_size(tokens::body())
                .text_color(theme.foreground)
                .child(label),
        )
        .child(div().flex_1().min_w_0().child(value))
}

impl WorkspaceView {
    /// 提交重命名：校验新名称 → 后台「下载到临时文件 → 上传新 key → 删旧 key」。
    ///
    /// 失败不静默：校验类问题就地显示并保持弹层打开；远端失败也留着弹层，
    /// 用户可改名重试。
    pub(super) fn commit_rename(&mut self, cx: &mut Context<Self>) {
        if self.renaming_busy {
            return;
        }
        let Some((old_key, editor)) = self.renaming.clone() else {
            return;
        };
        let new_name = editor.read(cx).value().to_string();
        // 目标 key 是否已存在也在这一步判（`rename_validation_message` 查 entries）：
        // 上传是覆盖语义，先查再传，别把同名对象直接盖掉。
        if let Some(message) = rename_validation_message(&old_key, &new_name, &self.entries) {
            self.rename_error = (message != RENAME_UNCHANGED).then_some(message);
            cx.notify();
            return;
        }
        let new_key = match rename_target_key(&old_key, &new_name) {
            Ok(key) => key,
            // 上面刚校验过，走到这里只可能是意外；不静默
            Err(message) => {
                self.rename_error = Some(message);
                cx.notify();
                return;
            }
        };
        let Some(account_id) = self.selected_account_id.clone() else {
            return;
        };
        let Some(bucket) = self.selected_bucket.clone() else {
            return;
        };
        let services = Arc::clone(&self.services);
        self.renaming_busy = true;
        self.rename_error = None;
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
                        this.renaming = None;
                        this.rename_error = None;
                        this.reload_objects(cx);
                        this.download_message = Some(DownloadMessage {
                            is_error: false,
                            text: format!("已重命名：{name} → {new_key_display}"),
                        });
                    }
                    Err(error) => {
                        // 弹层留着，错误就地显示，用户可直接改名重试
                        this.rename_error = Some(format!("重命名失败：{error}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Esc（context "Renaming" 的 dismiss）。文件夹 / 复制移动浮层也用这个 context，
    /// 所以按「谁开着关谁」的顺序判。
    pub(super) fn handle_dismiss_rename(
        &mut self,
        _: &DismissRename,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    /// 取消重命名（取消按钮 / 遮罩点击 / Esc 共用）。
    ///
    /// 进行中一律拒绝：后台任务完成时要回写这个弹层，关掉就丢了回写目标——
    /// 与「添加账号」保存中不许关闭同一条规矩。
    pub(super) fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        if self.renaming_busy {
            return;
        }
        let closed = self.renaming.take().is_some();
        let cleared_error = self.rename_error.take().is_some();
        if closed || cleared_error {
            cx.notify();
        }
    }
}

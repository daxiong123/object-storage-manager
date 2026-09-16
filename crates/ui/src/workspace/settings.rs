//! 设置（⌘,）模态的接线与渲染。

use super::*;

impl WorkspaceView {
    /// 设置模态遮罩（结构与添加账号一致）。
    pub(super) fn render_settings_modal_overlay(
        &self,
        modal: &Entity<SettingsModal>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mask = overlay::mask(theme)
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                    let Some(modal) = &this.settings_modal else {
                        return;
                    };
                    if !modal.read(cx).saving() {
                        modal.update(cx, SettingsModal::close);
                    }
                }),
            )
            .child(modal.clone());
        overlay::fade_in("settings-overlay", mask)
    }

    /// ⌘,：打开设置模态。
    pub(super) fn handle_open_settings(
        &mut self,
        _: &OpenSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings_modal.is_some() || self.palette.is_some() || self.add_modal.is_some() {
            return;
        }
        let path = self.settings_path.clone();
        let settings = self.settings.clone();
        let modal = cx.new(|cx| SettingsModal::new(settings, path, window, cx));
        cx.observe_in(&modal, window, Self::handle_settings_modal_changed)
            .detach();
        modal.update(cx, |modal, cx| modal.focus_first(window, cx));
        self.settings_modal = Some(modal);
        cx.notify();
    }

    /// 设置模态观察：每次保存（弹窗保持打开，验收反馈）取出并应用新值；
    /// 仅 closed（取消/Esc/点遮罩）时丢弃实体。
    pub(super) fn handle_settings_modal_changed(
        &mut self,
        modal: Entity<SettingsModal>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let closed = modal.read(cx).closed();
        // 保存成功（弹窗不关）：就地应用新设置 + 底部提示
        let saved = modal.update(cx, |modal, _| modal.take_saved());
        if let Some((settings, changed)) = saved {
            self.settings = settings;
            crate::theme::apply_settings(&self.settings, Some(window), cx);
            self.engine
                .set_max_parallel(self.settings.transfer_concurrency as usize);
            if changed {
                self.download_message = Some(DownloadMessage {
                    is_error: false,
                    text: "设置已保存，已对后续操作生效".into(),
                });
            }
            cx.notify();
        }
        if !closed {
            return;
        }
        self.settings_modal = None;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }
}

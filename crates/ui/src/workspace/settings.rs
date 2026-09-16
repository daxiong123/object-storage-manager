//! 设置（⌘,）模态的接线与渲染。

use super::*;
use object_storage_persistence::ThemeStyle;

impl WorkspaceView {
    /// 原型期：⌘⌥T 在 Linear / waku 两套调色板之间切换，便于在同一屏上对照。
    ///
    /// 走的是与设置模态**同一条落盘 + 应用路径**——它改的是真设置，不是一次性的
    /// 预览覆盖。这样评估完想删掉它，只需删掉本方法、`CycleThemeStyle` 这一个
    /// Action 与它的键位，不留任何状态残留。
    ///
    /// 落盘失败不静默：只改内存的话，重启后风格会自己变回去，用户无从理解。
    pub(super) fn handle_cycle_theme_style(
        &mut self,
        _: &CycleThemeStyle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let next = match self.settings.theme_style {
            ThemeStyle::Linear => ThemeStyle::Waku,
            ThemeStyle::Waku => ThemeStyle::Linear,
        };
        let mut settings = self.settings.clone();
        settings.theme_style = next;
        // 用注入的 settings_path 而不是 Settings::save() 的默认路径：与设置模态
        // 走同一个落点，路径只有一处定义。
        if let Err(error) = settings.save_at(self.settings_path.clone()) {
            self.download_message = Some(DownloadMessage {
                is_error: true,
                text: format!("视觉风格未能保存：{error}"),
            });
            cx.notify();
            return;
        }
        self.settings = settings;
        crate::theme::apply_settings(&self.settings, Some(window), cx);
        cx.notify();
    }

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

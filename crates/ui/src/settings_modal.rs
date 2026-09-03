//! 设置模态（⌘,）。自建 overlay，与「添加账号」同一机制。
//!
//! 字段：签名链接有效期（秒）/ 剪贴板自动清除（秒，0=关闭）。
//! 保存 = 校验 → 写 `settings.json`（`Settings::save`，Fail Fast）→
//! 回调 WorkspaceView 应用到运行时字段。保存中禁止关闭（同 AddAccountModal）。

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, Render, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Disableable as _, Icon, IconName, Sizable as _, Size, button::Button,
    button::ButtonVariants as _, h_flex, input::Input, input::InputState, v_flex,
};

use object_storage_persistence::Settings;

use crate::actions::DismissModal;

/// 设置保存成功后由 WorkspaceView 回收的载荷。
pub struct SettingsModal {
    initial: Settings,
    ttl: Entity<InputState>,
    clipboard: Entity<InputState>,
    /// 保存请求已发出、后台任务未返回
    saving: bool,
    error: Option<String>,
    /// 保存成功后的就地提示（弹窗不关，验收反馈）
    saved_note: Option<String>,
    /// 待 WorkspaceView 应用的新设置（观察者 take_saved 取走）
    pending_saved: Option<(Settings, bool)>,
    /// 已取消（WorkspaceView 据此丢弃本实体并归还焦点）
    closed: bool,
    /// 保存时的目标路径（后台任务要用；从 load 时的路径推导）
    settings_path: PathBuf,
    /// 直接保存（进程内设置，不做线程切换也能工作；保留 Arc 以对齐模态模式）
    _services: Arc<()>,
}

impl SettingsModal {
    pub fn new(
        settings: Settings,
        settings_path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let ttl = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("3600")
                .clean_on_escape()
                .default_value(settings.signed_url_ttl_secs.to_string())
        });
        let clipboard = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("60（0 = 不自动清除）")
                .clean_on_escape()
                .default_value(settings.clipboard_clear_secs.to_string())
        });
        Self {
            initial: settings,
            ttl,
            clipboard,
            saving: false,
            error: None,
            saved_note: None,
            pending_saved: None,
            closed: false,
            settings_path,
            _services: Arc::new(()),
        }
    }

    pub fn focus_first(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.ttl.update(cx, |state, cx| state.focus(window, cx));
    }

    /// 取出「已保存但尚未被 WorkspaceView 应用」的设置（观察者每次保存
    /// 后调用一次；弹窗保持打开，就地提示见 saved_note）。
    pub fn take_saved(&mut self) -> Option<(Settings, bool)> {
        self.pending_saved.take()
    }

    pub fn closed(&self) -> bool {
        self.closed
    }

    /// 在 Finder 中显示 settings.json（「打开配置文件」入口）。
    /// 文件可能尚不存在（从未保存过）：先落一份当前值再显示。
    fn reveal_settings_file(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let path = self.settings_path.clone();
        if !path.exists() {
            if let Err(error) = self.initial.save_at(path.clone()) {
                self.error = Some(format!("创建配置文件失败：{error}"));
                cx.notify();
                return;
            }
        }
        cx.reveal_path(&path);
    }

    pub fn saving(&self) -> bool {
        self.saving
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        self.closed = true;
        cx.notify();
    }

    fn handle_dismiss(&mut self, _: &DismissModal, _window: &mut Window, cx: &mut Context<Self>) {
        self.close(cx);
    }

    fn handle_cancel(
        &mut self,
        _: &gpui::ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close(cx);
    }

    fn handle_save(&mut self, _: &gpui::ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let ttl_text = self.ttl.read(cx).value().trim().to_string();
        let clip_text = self.clipboard.read(cx).value().trim().to_string();
        let ttl = match ttl_text.parse::<u64>() {
            Ok(v) => v,
            Err(_) => {
                self.error = Some("签名链接有效期必须是正整数（秒）".into());
                cx.notify();
                return;
            }
        };
        let clipboard = match clip_text.parse::<u64>() {
            Ok(v) => v,
            Err(_) => {
                self.error = Some("剪贴板自动清除必须是非负整数（秒，0 = 关闭）".into());
                cx.notify();
                return;
            }
        };
        let settings = Settings {
            signed_url_ttl_secs: ttl,
            clipboard_clear_secs: clipboard,
        };
        if let Err(message) = settings.validate() {
            self.error = Some(message);
            cx.notify();
            return;
        }
        self.saving = true;
        self.error = None;
        cx.notify();

        let path = self.settings_path.clone();
        // settings.json 写入是微秒级本地 IO，直接在 UI 线程做即可
        //（对比 AddAccountModal 的网络/钥匙串路径才需要后台线程）。
        let save_result = settings.save_at(path);
        self.saving = false;
        match save_result {
            Ok(()) => {
                // 保存成功：**不关闭弹层**（验收反馈）——就地显示成功状态，
                // 新设置经 pending_saved 交观察者应用；initial 同步为本值
                //（再点保存 = 无变化）。
                let changed = settings != self.initial;
                self.initial = settings.clone();
                self.pending_saved = Some((settings, changed));
                self.saved_note = Some("已保存 ✓ 关闭后生效提示见窗口底部".into());
                self.error = None;
            }
            Err(error) => {
                self.error = Some(format!("保存设置失败：{error}"));
            }
        }
        cx.notify();
    }
}

impl Render for SettingsModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let error = self.error.clone();
        let saved_note = self.saved_note.clone();

        div()
            .key_context("SettingsModal")
            .w(px(420.))
            .bg(theme.background)
            .border_1()
            .border_color(theme.border)
            .rounded(px(10.))
            .shadow_lg()
            .on_action(cx.listener(Self::handle_dismiss))
            .on_mouse_down(
                MouseButton::Left,
                |_event: &gpui::MouseDownEvent, _window, cx| {
                    // 卡片内点击阻断冒泡：否则事件到达遮罩的空白关闭
                    // handler，弹窗被误关（与 AddAccountModal 同机制）
                    cx.stop_propagation();
                },
            )
            .child(
                v_flex()
                    .p_4()
                    .gap_3()
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("设置"),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme.muted_foreground)
                                    .child("签名链接有效期（秒）"),
                            )
                            .child(Input::new(&self.ttl).small()),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme.muted_foreground)
                                    .child("复制链接后自动清空剪贴板（秒，0 = 不清除）"),
                            )
                            .child(Input::new(&self.clipboard).small()),
                    )
                    .children(error.map(|text| {
                        div()
                            .text_size(px(12.))
                            .text_color(theme.danger)
                            .child(text)
                    }))
                    .children(saved_note.map(|text| {
                        div()
                            .text_size(px(12.))
                            .text_color(theme.success)
                            .child(text)
                    }))
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("settings-cancel")
                                    .label("关闭")
                                    .ghost()
                                    .with_size(Size::Small)
                                    .on_click(cx.listener(Self::handle_cancel)),
                            )
                            .child(
                                Button::new("settings-save")
                                    .label("保存")
                                    .primary()
                                    .with_size(Size::Small)
                                    .disabled(self.saving)
                                    .on_click(cx.listener(Self::handle_save)),
                            ),
                    )
                    // 配置文件入口（验收反馈：不显示路径，直接提供打开）
                    .child(
                        h_flex()
                            .gap_1()
                            .text_size(px(11.))
                            .child(
                                Icon::new(IconName::FolderOpen).text_color(theme.muted_foreground),
                            )
                            .child(
                                Button::new("settings-open-file")
                                    .label("打开配置文件")
                                    .link()
                                    .with_size(Size::Small)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.reveal_settings_file(window, cx);
                                    })),
                            ),
                    ),
            )
    }
}

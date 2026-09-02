//! 添加账号模态（自建 overlay 模式，与命令面板同一机制，见其模块注释）。
//!
//! 字段：名称 / AccessKey / SecretKey。SecretKey 用 `InputState::masked(true)`
//! 密码框展示；提交走 `AppServices::add_qiniu_account`（gpui 后台线程），
//! Secret 只经 Keychain 落盘，数据库永不明文保存（spec §19）。
//!
//! 生命周期：WorkspaceView 创建实体并 observe；本视图置 `done`（保存成功）或
//! `closed`（取消）后由 WorkspaceView 丢弃实体。保存进行中（`saving`）禁止关闭
//! ——后台任务完成前实体不能被丢弃，否则任务回写 `done` 时 WeakEntity 已失效，
//! 新账号会静默丢失刷新。

use std::sync::Arc;

use gpui::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, Render, Styled, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme, Disableable as _, Icon, IconName, Sizable as _, Size, Theme, button::Button,
    button::ButtonVariants as _, h_flex, input::Input, input::InputState, v_flex,
};

use object_storage_app::AppServices;
use object_storage_domain::ProviderKind;

use crate::actions::DismissModal;

pub struct AddAccountModal {
    services: Arc<AppServices>,
    name: Entity<InputState>,
    access_key: Entity<InputState>,
    secret_key: Entity<InputState>,
    provider: ProviderKind,
    /// 保存请求已发出、后台任务未返回
    saving: bool,
    /// 后台任务返回的错误（中文，直接展示）
    error: Option<String>,
    /// 保存成功（WorkspaceView 据此刷新账号列表并丢弃本实体）
    done: bool,
    /// 已取消（WorkspaceView 据此丢弃本实体并归还焦点）
    closed: bool,
}

impl AddAccountModal {
    pub fn new(services: Arc<AppServices>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("如：个人七牛")
                .clean_on_escape()
        });
        let access_key = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("AccessKey ID")
                .clean_on_escape()
        });
        let secret_key = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("SecretKey（输入时不回显）")
                .masked(true)
        });
        Self {
            services,
            name,
            access_key,
            secret_key,
            provider: ProviderKind::Qiniu,
            saving: false,
            error: None,
            done: false,
            closed: false,
        }
    }

    /// 把焦点放入第一个输入框（创建后由 WorkspaceView 调用）。
    pub fn focus_first(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.name.update(cx, |state, cx| state.focus(window, cx));
    }

    pub fn done(&self) -> bool {
        self.done
    }

    pub fn closed(&self) -> bool {
        self.closed
    }

    pub fn saving(&self) -> bool {
        self.saving
    }

    /// 请求关闭（取消按钮 / 点遮罩 / Esc 共用）。保存中一律拒绝——见模块注释。
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
        let name = self.name.read(cx).value().to_string();
        let access_key = self.access_key.read(cx).value().to_string();
        let secret_key = self.secret_key.read(cx).value().to_string();

        self.saving = true;
        self.error = None;
        cx.notify();

        let provider = self.provider;
        let services = Arc::clone(&self.services);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match provider {
                        ProviderKind::Qiniu => {
                            services.add_qiniu_account(&name, &access_key, &secret_key)
                        }
                        ProviderKind::Aliyun => {
                            services.add_aliyun_account(&name, &access_key, &secret_key)
                        }
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(_) => this.done = true,
                    Err(e) => {
                        this.saving = false;
                        this.error = Some(e.to_string());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn render_field(
        &self,
        theme: &Theme,
        label: &'static str,
        input: &Entity<InputState>,
        hint: Option<&'static str>,
    ) -> impl IntoElement {
        v_flex()
            .gap_1()
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.muted_foreground)
                    .child(label),
            )
            .child(Input::new(input))
            .children(hint.map(|h| {
                div()
                    .text_size(px(11.))
                    .text_color(theme.muted_foreground)
                    .child(h)
            }))
    }

    fn render_error(&self, theme: &Theme) -> impl IntoElement {
        let Some(error) = &self.error else {
            return div().into_any_element();
        };
        h_flex()
            .gap_2()
            .px_2()
            .py_1()
            .rounded(px(6.))
            .text_color(theme.danger)
            .text_size(px(12.))
            .child(Icon::new(IconName::TriangleAlert))
            .child(div().truncate().child(error.clone()))
            .into_any_element()
    }

    fn render_footer(&self, theme: &Theme, cx: &Context<Self>) -> impl IntoElement {
        h_flex()
            .w_full()
            .justify_end()
            .gap_2()
            .border_t_1()
            .border_color(theme.border)
            .pt_3()
            .child(
                Button::new("modal-cancel")
                    .label("取消")
                    .ghost()
                    .disabled(self.saving)
                    .with_size(Size::Small)
                    .on_click(cx.listener(Self::handle_cancel)),
            )
            .child(
                Button::new("modal-save")
                    .label(if self.saving {
                        "保存中…"
                    } else {
                        "保存"
                    })
                    .primary()
                    .loading(self.saving)
                    .disabled(self.saving)
                    .with_size(Size::Small)
                    .on_click(cx.listener(Self::handle_save)),
            )
    }
}

impl Render for AddAccountModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();

        v_flex()
            .key_context("AccountModal")
            .on_action(cx.listener(Self::handle_dismiss))
            // 点击卡片内部不冒泡到遮罩（否则会误关模态）。
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(px(440.))
            .bg(theme.background)
            .border_1()
            .border_color(theme.border)
            .rounded(px(10.))
            .shadow_lg()
            .p_4()
            .gap_3()
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("添加账号"),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("provider-qiniu")
                            .label("七牛 Kodo")
                            .when(self.provider == ProviderKind::Qiniu, |b| b.primary())
                            .when(self.provider != ProviderKind::Qiniu, |b| b.ghost())
                            .with_size(Size::Small)
                            .disabled(self.saving)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.provider = ProviderKind::Qiniu;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("provider-aliyun")
                            .label("阿里云 OSS")
                            .when(self.provider == ProviderKind::Aliyun, |b| b.primary())
                            .when(self.provider != ProviderKind::Aliyun, |b| b.ghost())
                            .with_size(Size::Small)
                            .disabled(self.saving)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.provider = ProviderKind::Aliyun;
                                cx.notify();
                            })),
                    ),
            )
            .child(self.render_field(&theme, "名称", &self.name, Some("显示名，可随时修改")))
            .child(self.render_field(
                &theme,
                "AccessKey",
                &self.access_key,
                Some("明文标识，保存在本机数据库"),
            ))
            .child(self.render_field(
                &theme,
                "SecretKey",
                &self.secret_key,
                Some("仅存入 macOS 钥匙串，数据库永不明文保存"),
            ))
            .child(self.render_error(&theme))
            .child(self.render_footer(&theme, cx))
    }
}

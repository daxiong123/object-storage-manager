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
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, Styled, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme, Disableable as _, Icon, IconName, Selectable as _, Sizable as _, Size, Theme,
    button::Button, button::ButtonCustomVariant, button::ButtonGroup, button::ButtonVariants as _,
    h_flex, input::Input, input::InputState, v_flex,
};

use object_storage_app::AppServices;
use object_storage_domain::ProviderKind;

use crate::actions::DismissModal;
use crate::tokens;
use crate::ui;
use crate::ui::overlay;

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
                    .text_size(tokens::label())
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.muted_foreground)
                    .child(label),
            )
            .child(Input::new(input))
            .children(hint.map(|h| {
                div()
                    .text_size(tokens::caption())
                    .text_color(theme.muted_foreground)
                    .child(h)
            }))
    }

    /// 服务商选择：一条分段控件（两个互斥选项）。
    ///
    /// 选中态必须**一眼看得出来**，这里是三处刻意决定：
    ///
    /// - 用库自带的 `ButtonGroup` 而不是两个各自独立的 `Button`：它把两段并成一条
    ///   分段控件，并给每个子按钮打 `.toggled(bool)` 无障碍标记（"这段是按下的"）。
    /// - 选中底色走 `ButtonVariant::Custom` 的 **`active`** 色 = `sidebar_accent`
    ///   （与侧栏、设置左导航选中行同一个 token）。`ButtonGroup` 的选中样式取
    ///   variant 的 `active`，所以 accent 必须放在 `active` 上，不能放 `color`。
    ///   **不要退回 `Secondary` + `ghost` 的组合**：`Secondary` 的底是
    ///   `tokens.button_secondary`（≈ 弹层底色），与 ghost 的差别只有百分之一量级，
    ///   于是「选了 Kodo 还是 OSS」肉眼分不出——这正是它原先不显眼的原因。
    /// - 选中项**另加对勾图标 + accent 文字色**：颜色不是唯一信号（不得只靠颜色表意）。
    ///
    /// 宽度给固定档位（`tokens::text`，随字号缩放）而不是让内容撑开：两段等宽才像
    /// 一条分段控件，而且切换时不会因为多了个对勾图标把另一段挤动。
    fn render_provider_picker(&self, theme: &Theme, cx: &Context<Self>) -> impl IntoElement {
        let variant = ButtonCustomVariant::new(cx)
            // 非选中段：不填色也不描边（Custom 变体的填充与边框同色，给透明即两者皆无），
            // 露出弹层底色——选中段的 accent 实底因此是整条控件里唯一的色块。
            .color(gpui::transparent_black())
            // 选中段（Custom 的 selected 样式取这一格）
            .active(theme.sidebar_accent)
            .foreground(theme.foreground)
            .hover(theme.list_hover);

        v_flex()
            .gap_1()
            .child(
                div()
                    .text_size(tokens::label())
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.muted_foreground)
                    .child("服务商"),
            )
            .child(
                ButtonGroup::new("provider-group")
                    .custom(variant)
                    .with_size(Size::Small)
                    // 必须在 child() 之前：child() 用这里的值设置各子按钮的 disabled
                    .disabled(self.saving)
                    .child(self.provider_option(
                        theme,
                        cx,
                        "provider-qiniu",
                        "七牛 Kodo",
                        ProviderKind::Qiniu,
                    ))
                    .child(self.provider_option(
                        theme,
                        cx,
                        "provider-aliyun",
                        "阿里云 OSS",
                        ProviderKind::Aliyun,
                    )),
            )
    }

    fn provider_option(
        &self,
        theme: &Theme,
        cx: &Context<Self>,
        id: &'static str,
        label: &'static str,
        provider: ProviderKind,
    ) -> Button {
        let selected = self.provider == provider;
        Button::new(id)
            .label(label)
            .with_size(Size::Small)
            .w(tokens::text(120.))
            .selected(selected)
            .when(selected, |button| {
                button
                    .icon(Icon::new(IconName::Check))
                    .text_color(theme.sidebar_accent_foreground)
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.provider != provider {
                    this.provider = provider;
                    cx.notify();
                }
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
            .rounded(tokens::radius())
            .text_color(theme.danger)
            .text_size(tokens::label())
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

        overlay::surface(&theme)
            .key_context("AccountModal")
            .on_action(cx.listener(Self::handle_dismiss))
            .w_full()
            .max_w(px(440.))
            .p_4()
            .gap_3()
            // 标题栏：标题左、关闭按钮右（弹层规范，与其余 6 个弹层一致；
            // 标题字号同用 tokens::title()，别在这里退回 heading()）
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(tokens::title())
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("添加账号"),
                    )
                    .child(
                        ui::icon_button("close-account-modal", Icon::new(IconName::Close), "关闭")
                            .disabled(self.saving)
                            .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
                    ),
            )
            .child(self.render_provider_picker(&theme, cx))
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

//! 设置模态（⌘,）。自建 overlay，与「添加账号」同一机制。
//!
//! 字段：签名链接有效期（秒）/ 剪贴板自动清除（秒，0=关闭）。
//! 保存 = 校验 → 写 `settings.json`（`Settings::save`，Fail Fast）→
//! 回调 WorkspaceView 应用到运行时字段。保存中禁止关闭（同 AddAccountModal）。

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, PathPromptOptions, Render, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Disableable as _, Icon, IconName, Sizable as _, Size, button::Button,
    button::ButtonVariants as _, h_flex, input::Input, input::InputState, v_flex,
};

use object_storage_persistence::{
    AppearanceMode, CODE_FONT_SIZE_DEFAULT, Settings, TRANSFER_CONCURRENCY_DEFAULT,
};

use crate::actions::DismissModal;
use crate::tokens;

fn optional_text(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn parse_u64_field(value: &str, message: &str) -> Result<u64, String> {
    value.trim().parse::<u64>().map_err(|_| message.to_string())
}

fn parse_u32_field(value: &str, message: &str) -> Result<u32, String> {
    value.trim().parse::<u32>().map_err(|_| message.to_string())
}

fn parse_f32_field(value: &str, message: &str) -> Result<f32, String> {
    value.trim().parse::<f32>().map_err(|_| message.to_string())
}

/// 设置保存成功后由 WorkspaceView 回收的载荷。
pub struct SettingsModal {
    initial: Settings,
    ttl: Entity<InputState>,
    clipboard: Entity<InputState>,
    appearance_mode: AppearanceMode,
    ui_font_family: Entity<InputState>,
    ui_font_scale: Entity<InputState>,
    code_font_family: Entity<InputState>,
    code_font_size: Entity<InputState>,
    transfer_concurrency: Entity<InputState>,
    default_download_dir: Option<PathBuf>,
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
        let ui_font_family = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("系统默认")
                .clean_on_escape()
                .default_value(settings.ui_font_family.clone().unwrap_or_default())
        });
        let ui_font_scale = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("1.0")
                .clean_on_escape()
                .default_value(settings.ui_font_scale.to_string())
        });
        let code_font_family = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Menlo")
                .clean_on_escape()
                .default_value(settings.code_font_family.clone().unwrap_or_default())
        });
        let code_font_size = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(CODE_FONT_SIZE_DEFAULT.to_string())
                .clean_on_escape()
                .default_value(settings.code_font_size.to_string())
        });
        let transfer_concurrency = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(TRANSFER_CONCURRENCY_DEFAULT.to_string())
                .clean_on_escape()
                .default_value(settings.transfer_concurrency.to_string())
        });
        let default_download_dir = settings.default_download_dir.clone();
        Self {
            initial: settings.clone(),
            ttl,
            clipboard,
            appearance_mode: settings.appearance_mode,
            ui_font_family,
            ui_font_scale,
            code_font_family,
            code_font_size,
            transfer_concurrency,
            default_download_dir,
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
        if !path.exists()
            && let Err(error) = self.initial.save_at(path.clone())
        {
            self.error = Some(format!("创建配置文件失败：{error}"));
            cx.notify();
            return;
        }
        cx.reveal_path(&path);
    }

    fn choose_default_download_dir(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择默认下载目录".into()),
        });
        cx.spawn(async move |this, cx| {
            let outcome = match receiver.await {
                Ok(Ok(Some(mut paths))) => paths.pop(),
                Ok(Ok(None)) => None,
                Ok(Err(error)) => {
                    this.update(cx, |this, cx| {
                        this.error = Some(format!("无法打开目录选择器：{error}"));
                        cx.notify();
                    })
                    .ok();
                    return;
                }
                Err(_) => {
                    this.update(cx, |this, cx| {
                        this.error = Some("目录选择器结果通道已关闭".into());
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };
            if let Some(path) = outcome {
                this.update(cx, |this, cx| {
                    this.default_download_dir = Some(path);
                    this.error = None;
                    this.saved_note = None;
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    fn clear_default_download_dir(&mut self, cx: &mut Context<Self>) {
        self.default_download_dir = None;
        self.saved_note = None;
        cx.notify();
    }

    fn set_appearance_mode(&mut self, mode: AppearanceMode, cx: &mut Context<Self>) {
        self.appearance_mode = mode;
        self.saved_note = None;
        cx.notify();
    }

    fn set_input_value(
        input: &Entity<InputState>,
        value: impl Into<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = value.into();
        input.update(cx, |input, cx| input.set_value(value, window, cx));
    }

    fn build_settings(&self, cx: &mut Context<Self>) -> Result<Settings, String> {
        let ttl = parse_u64_field(
            self.ttl.read(cx).value().trim(),
            "签名链接有效期必须是正整数（秒）",
        )?;
        let clipboard = parse_u64_field(
            self.clipboard.read(cx).value().trim(),
            "剪贴板自动清除必须是非负整数（秒，0 = 关闭）",
        )?;
        let ui_font_scale = parse_f32_field(
            self.ui_font_scale.read(cx).value().trim(),
            "界面字号缩放必须是数字，例如 1.0",
        )?;
        let code_font_size = parse_u32_field(
            self.code_font_size.read(cx).value().trim(),
            "代码字体字号必须是整数",
        )?;
        let transfer_concurrency = parse_u32_field(
            self.transfer_concurrency.read(cx).value().trim(),
            "传输并发数必须是整数",
        )?;
        let settings = Settings {
            signed_url_ttl_secs: ttl,
            clipboard_clear_secs: clipboard,
            appearance_mode: self.appearance_mode,
            ui_font_family: optional_text(self.ui_font_family.read(cx).value().to_string()),
            ui_font_scale,
            code_font_family: optional_text(self.code_font_family.read(cx).value().to_string()),
            code_font_size,
            transfer_concurrency,
            default_download_dir: self.default_download_dir.clone(),
        };
        settings.validate()?;
        Ok(settings)
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
        let settings = match self.build_settings(cx) {
            Ok(settings) => settings,
            Err(message) => {
                self.error = Some(message);
                self.saved_note = None;
                cx.notify();
                return;
            }
        };
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
                self.saved_note = Some("已保存，设置已即时生效".into());
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
        let download_dir = self
            .default_download_dir
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "未设置（使用用户主目录）".into());

        div()
            .key_context("SettingsModal")
            .w(px(560.))
            .max_h(px(680.))
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
                    .gap_4()
                    .child(
                        div()
                            .text_size(tokens::text(16.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("设置"),
                    )
                    .child(
                        v_flex()
                            .gap_2()
                            .child(section_title("链接与剪贴板", &theme))
                            .child(field(
                                "签名链接有效期（秒）",
                                Input::new(&self.ttl).small(),
                                &theme,
                            ))
                            .child(field(
                                "复制链接后自动清空剪贴板（秒，0 = 不清除）",
                                Input::new(&self.clipboard).small(),
                                &theme,
                            )),
                    )
                    .child(
                        v_flex()
                            .gap_2()
                            .child(section_title("外观", &theme))
                            .child(
                                h_flex().gap_2().children(
                                    [
                                        (AppearanceMode::System, "跟随系统"),
                                        (AppearanceMode::Light, "浅色"),
                                        (AppearanceMode::Dark, "深色"),
                                    ]
                                    .into_iter()
                                    .map(|(mode, label)| {
                                        div()
                                            .px_3()
                                            .py_1()
                                            .rounded(px(6.))
                                            .text_size(tokens::text(12.))
                                            .bg(if self.appearance_mode == mode {
                                                theme.list_active
                                            } else {
                                                theme.sidebar
                                            })
                                            .hover(|el| el.bg(theme.accent))
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(move |this, _, _, cx| {
                                                    this.set_appearance_mode(mode, cx)
                                                }),
                                            )
                                            .child(label)
                                    }),
                                ),
                            )
                            .child(
                                v_flex()
                                    .gap_2()
                                    .child(label("界面字体", &theme))
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .child(
                                                Button::new("ui-font-system")
                                                    .label("系统默认")
                                                    .ghost()
                                                    .with_size(Size::Small)
                                                    .on_click(cx.listener(
                                                        |this, _, window, cx| {
                                                            Self::set_input_value(
                                                                &this.ui_font_family,
                                                                "",
                                                                window,
                                                                cx,
                                                            );
                                                        },
                                                    )),
                                            )
                                            .child(
                                                Button::new("ui-font-sf")
                                                    .label("SF Pro")
                                                    .ghost()
                                                    .with_size(Size::Small)
                                                    .on_click(cx.listener(
                                                        |this, _, window, cx| {
                                                            Self::set_input_value(
                                                                &this.ui_font_family,
                                                                "SF Pro",
                                                                window,
                                                                cx,
                                                            );
                                                        },
                                                    )),
                                            )
                                            .child(
                                                Button::new("ui-font-pingfang")
                                                    .label("苹方")
                                                    .ghost()
                                                    .with_size(Size::Small)
                                                    .on_click(cx.listener(
                                                        |this, _, window, cx| {
                                                            Self::set_input_value(
                                                                &this.ui_font_family,
                                                                "PingFang SC",
                                                                window,
                                                                cx,
                                                            );
                                                        },
                                                    )),
                                            ),
                                    )
                                    .child(Input::new(&self.ui_font_family).small()),
                            )
                            .child(field(
                                "界面字号缩放（0.85 - 1.40）",
                                Input::new(&self.ui_font_scale).small(),
                                &theme,
                            )),
                    )
                    .child(
                        v_flex()
                            .gap_2()
                            .child(section_title("代码字体", &theme))
                            .child(
                                v_flex()
                                    .gap_2()
                                    .child(label("字体族", &theme))
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .child(
                                                Button::new("code-font-menlo")
                                                    .label("Menlo")
                                                    .ghost()
                                                    .with_size(Size::Small)
                                                    .on_click(cx.listener(
                                                        |this, _, window, cx| {
                                                            Self::set_input_value(
                                                                &this.code_font_family,
                                                                "",
                                                                window,
                                                                cx,
                                                            );
                                                        },
                                                    )),
                                            )
                                            .child(
                                                Button::new("code-font-sfmono")
                                                    .label("SF Mono")
                                                    .ghost()
                                                    .with_size(Size::Small)
                                                    .on_click(cx.listener(
                                                        |this, _, window, cx| {
                                                            Self::set_input_value(
                                                                &this.code_font_family,
                                                                "SF Mono",
                                                                window,
                                                                cx,
                                                            );
                                                        },
                                                    )),
                                            )
                                            .child(
                                                Button::new("code-font-jetbrains")
                                                    .label("JetBrains Mono")
                                                    .ghost()
                                                    .with_size(Size::Small)
                                                    .on_click(cx.listener(
                                                        |this, _, window, cx| {
                                                            Self::set_input_value(
                                                                &this.code_font_family,
                                                                "JetBrains Mono",
                                                                window,
                                                                cx,
                                                            );
                                                        },
                                                    )),
                                            ),
                                    )
                                    .child(Input::new(&self.code_font_family).small()),
                            )
                            .child(field(
                                "字号（10 - 24）",
                                Input::new(&self.code_font_size).small(),
                                &theme,
                            )),
                    )
                    .child(
                        v_flex()
                            .gap_2()
                            .child(section_title("传输与下载", &theme))
                            .child(field(
                                "传输并发数（1 - 8）",
                                Input::new(&self.transfer_concurrency).small(),
                                &theme,
                            ))
                            .child(label("默认下载目录", &theme))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .px_3()
                                            .py_2()
                                            .rounded(px(6.))
                                            .bg(theme.sidebar)
                                            .text_size(tokens::text(12.))
                                            .truncate()
                                            .child(download_dir),
                                    )
                                    .child(
                                        Button::new("settings-pick-download-dir")
                                            .label("选择…")
                                            .with_size(Size::Small)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.choose_default_download_dir(cx);
                                            })),
                                    )
                                    .child(
                                        Button::new("settings-clear-download-dir")
                                            .label("清除")
                                            .ghost()
                                            .with_size(Size::Small)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.clear_default_download_dir(cx);
                                            })),
                                    ),
                            ),
                    )
                    .children(error.map(|text| {
                        div()
                            .text_size(tokens::text(12.))
                            .text_color(theme.danger)
                            .child(text)
                    }))
                    .children(saved_note.map(|text| {
                        div()
                            .text_size(tokens::text(12.))
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
                            .text_size(tokens::text(11.))
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

fn section_title(title: &'static str, theme: &gpui_component::Theme) -> impl IntoElement {
    div()
        .pt_2()
        .text_size(tokens::text(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(theme.foreground)
        .child(title)
}

fn label(text: &'static str, theme: &gpui_component::Theme) -> impl IntoElement {
    div()
        .text_size(tokens::text(12.))
        .text_color(theme.muted_foreground)
        .child(text)
}

fn field(
    label_text: &'static str,
    input: impl IntoElement,
    theme: &gpui_component::Theme,
) -> impl IntoElement {
    v_flex()
        .gap_1()
        .child(label(label_text, theme))
        .child(input)
}

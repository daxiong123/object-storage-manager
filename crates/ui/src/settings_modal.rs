//! 设置窗口（⌘,）。自建 overlay，OpenChamber 式两栏布局：
//! 左侧分组导航（200px）+ 右侧分页内容（Section / FieldRow 语义）。
//!
//! 保存 = 校验 → 写 `settings.json`（`Settings::save`，Fail Fast）→
//! 回调 WorkspaceView 应用到运行时字段。保存中禁止关闭（同 AddAccountModal）。
//! 弹层规范（agents.md）：Esc / 遮罩 / 标题栏 ✕ 三路关闭，busy 中由 close 拒绝；
//! 卡片阻断冒泡；footer「关闭在左、保存在右」。

use std::path::PathBuf;
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, PathPromptOptions, Render, StatefulInteractiveElement as _, Styled, Window,
    div, px,
};
use gpui_component::{
    ActiveTheme, Disableable as _, Icon, IconName, IndexPath, Sizable as _, Size, button::Button,
    button::ButtonVariants as _, h_flex, input::Input, input::InputState, input::NumberInput,
    input::NumberInputEvent, input::StepAction, radio::Radio, select::Select, select::SelectEvent,
    select::SelectState, v_flex,
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

/// 设置导航分组（左侧栏），按展示顺序。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsSection {
    General,
    Appearance,
    CodeFont,
    Transfer,
}

impl SettingsSection {
    fn title(self) -> &'static str {
        match self {
            Self::General => "通用",
            Self::Appearance => "外观",
            Self::CodeFont => "代码字体",
            Self::Transfer => "传输与下载",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::General => "签名链接与剪贴板行为。",
            Self::Appearance => "主题模式、界面字体与字号缩放。",
            Self::CodeFont => "代码与技术字段使用的等宽字体。",
            Self::Transfer => "传输队列并发与默认下载目录。",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::General => IconName::Settings,
            Self::Appearance => IconName::Palette,
            Self::CodeFont => IconName::ALargeSmall,
            Self::Transfer => IconName::ArrowDown,
        }
    }

    const ALL: [Self; 4] = [
        Self::General,
        Self::Appearance,
        Self::CodeFont,
        Self::Transfer,
    ];
}

/// 设置保存成功后由 WorkspaceView 回收的载荷。
pub struct SettingsModal {
    initial: Settings,
    ttl: Entity<InputState>,
    clipboard: Entity<InputState>,
    appearance_mode: AppearanceMode,
    /// 界面字体下拉（含自定义字体名；None 选项 = 系统默认）
    ui_font_select: Entity<SelectState<Vec<FontOption>>>,
    ui_font_scale: Entity<InputState>,
    /// 代码字体下拉
    code_font_select: Entity<SelectState<Vec<FontOption>>>,
    code_font_size: Entity<InputState>,
    transfer_concurrency: Entity<InputState>,
    default_download_dir: Option<PathBuf>,
    /// 左侧导航当前分组
    active_section: SettingsSection,
    /// 保存请求已发出、后台任务未返回
    saving: bool,
    error: Option<String>,
    /// 保存成功后的就地提示（footer 左侧；弹窗不关，验收反馈）
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
                .placeholder("60")
                .clean_on_escape()
                .default_value(settings.clipboard_clear_secs.to_string())
        });
        // 字体下拉选项：空串 = 系统默认 + 预设 + 当前自定义值（保证预选中命中）。
        let ui_font_items: Vec<FontOption> =
            font_select_items(UI_FONT_PRESETS, settings.ui_font_family.as_deref())
                .into_iter()
                .map(FontOption)
                .collect();
        let ui_font_select = cx.new(|cx| {
            SelectState::new(
                ui_font_items,
                selected_font_index(
                    &font_select_items(UI_FONT_PRESETS, settings.ui_font_family.as_deref()),
                    settings.ui_font_family.as_deref(),
                ),
                window,
                cx,
            )
        });
        let ui_font_scale = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("1.0")
                .clean_on_escape()
                .default_value(settings.ui_font_scale.to_string())
        });
        let code_font_items: Vec<FontOption> =
            font_select_items(CODE_FONT_PRESETS, settings.code_font_family.as_deref())
                .into_iter()
                .map(FontOption)
                .collect();
        let code_font_select = cx.new(|cx| {
            SelectState::new(
                code_font_items,
                selected_font_index(
                    &font_select_items(CODE_FONT_PRESETS, settings.code_font_family.as_deref()),
                    settings.code_font_family.as_deref(),
                ),
                window,
                cx,
            )
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
        // 字体选择确认后清掉过期的「已保存」提示（值已变化）。
        cx.subscribe_in(
            &ui_font_select,
            window,
            |this, _, _: &SelectEvent<_>, _, cx| {
                this.saved_note = None;
                cx.notify();
            },
        )
        .detach();
        cx.subscribe_in(
            &code_font_select,
            window,
            |this, _, _: &SelectEvent<_>, _, cx| {
                this.saved_note = None;
                cx.notify();
            },
        )
        .detach();
        // NumberInput 的 +/− 按钮只 emit Step 事件，数值写回由订阅方完成——
        // 不订阅则加减无效（验收问题）。步长 1；缩放字段按 f32 走独立订阅。
        let subscribe_step_u32 =
            |state: &Entity<InputState>, window: &mut Window, cx: &mut Context<Self>| {
                cx.subscribe_in(
                    state,
                    window,
                    |_, input: &Entity<InputState>, event: &NumberInputEvent, window, cx| {
                        input.update(cx, |input, cx| {
                            let value: u32 = input.value().trim().parse().unwrap_or(0);
                            let next = match event {
                                NumberInputEvent::Step(StepAction::Increment) => {
                                    value.saturating_add(1)
                                }
                                NumberInputEvent::Step(StepAction::Decrement) => {
                                    value.saturating_sub(1)
                                }
                            };
                            input.set_value(next.to_string(), window, cx);
                        });
                    },
                )
                .detach();
            };
        subscribe_step_u32(&code_font_size, window, cx);
        subscribe_step_u32(&transfer_concurrency, window, cx);
        {
            // 界面字号缩放：步长 0.05，范围 clamp 到 0.85–1.40（与 tokens 一致）。
            cx.subscribe_in(
                &ui_font_scale,
                window,
                |_, input: &Entity<InputState>, event: &NumberInputEvent, window, cx| {
                    input.update(cx, |input, cx| {
                        let value: f32 = input.value().trim().parse().unwrap_or(1.0);
                        let next = match event {
                            NumberInputEvent::Step(StepAction::Increment) => value + 0.05,
                            NumberInputEvent::Step(StepAction::Decrement) => value - 0.05,
                        };
                        let next = next.clamp(0.85, 1.40);
                        input.set_value(format!("{next:.2}"), window, cx);
                    });
                },
            )
            .detach();
        }
        Self {
            initial: settings.clone(),
            ttl,
            clipboard,
            appearance_mode: settings.appearance_mode,
            ui_font_select,
            ui_font_scale,
            code_font_select,
            code_font_size,
            transfer_concurrency,
            default_download_dir,
            active_section: SettingsSection::General,
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

    /// 在 Finder 中显示 settings.json（nav 底部「打开配置文件」入口）。
    /// 文件可能尚不存在（从未保存过）：先落一份当前值再显示。
    fn reveal_settings_file(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let path = self.settings_path.clone();
        if !path.exists()
            && let Err(error) = self.initial.save_at(path.clone())
        {
            self.error = Some(format!("创建配置文件失败：{error}"));
            self.saved_note = None;
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
        let font_value = |state: &Entity<SelectState<Vec<FontOption>>>| {
            state.read(cx).selected_value().cloned().unwrap_or_default()
        };
        let ui_font_family = font_value(&self.ui_font_select);
        let ui_font_family = optional_text(ui_font_family);
        let code_font_family = font_value(&self.code_font_select);
        let code_font_family = optional_text(code_font_family);
        let settings = Settings {
            signed_url_ttl_secs: ttl,
            clipboard_clear_secs: clipboard,
            appearance_mode: self.appearance_mode,
            ui_font_family,
            ui_font_scale,
            code_font_family,
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
                // 保存成功：**不关闭窗口**——footer 显示成功状态，
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let active = self.active_section;

        div()
            .key_context("SettingsModal")
            .w(px(800.))
            .h(px(560.))
            .bg(theme.background)
            .border_1()
            .border_color(theme.border)
            .rounded(px(10.))
            .shadow_lg()
            .overflow_hidden()
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
                    .size_full()
                    // 标题栏
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .px_4()
                            .py_3()
                            .child(
                                div()
                                    .text_size(tokens::text(16.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child("设置"),
                            )
                            .child(
                                Button::new("settings-close")
                                    .icon(Icon::new(IconName::Close).size_4())
                                    .ghost()
                                    .with_size(Size::Small)
                                    .disabled(self.saving)
                                    .on_click(cx.listener(Self::handle_cancel)),
                            ),
                    )
                    // 中部：左导航 + 右内容。
                    // 注意：h_flex 默认 items_center，这里必须显式 items_stretch
                    //（gpui 无 items_stretch 方法，items_center 之外的默认即
                    // stretch）——只写 h_flex() 会让页面容器被垂直居中，
                    // 顶部留出大片空白（验收问题）。页面容器自身
                    // items_start 保证内容自顶部排布。
                    .child(
                        h_flex()
                            .flex_1()
                            .min_h_0()
                            .flex_row()
                            .border_t_1()
                            .border_color(theme.border)
                            .child(self.render_nav(&theme, cx))
                            .child(self.render_page(active, &theme, window, cx)),
                    )
                    // footer：状态提示（左）+ 操作按钮（右）
                    .child(self.render_footer(&theme, cx)),
            )
    }
}

impl SettingsModal {
    fn render_nav(
        &self,
        theme: &gpui_component::Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active = self.active_section;
        v_flex()
            .w(px(200.))
            .h_full()
            .flex_shrink_0()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.border)
            .justify_between()
            .child(
                v_flex()
                    .p_2()
                    .gap_0p5()
                    .children(SettingsSection::ALL.map(|section| {
                        let selected = active == section;
                        let (text_color, bg) = if selected {
                            (theme.sidebar_accent_foreground, theme.list_active)
                        } else {
                            (theme.foreground, gpui::transparent_black())
                        };
                        div()
                            .id(match section {
                                SettingsSection::General => "settings-nav-general",
                                SettingsSection::Appearance => "settings-nav-appearance",
                                SettingsSection::CodeFont => "settings-nav-code-font",
                                SettingsSection::Transfer => "settings-nav-transfer",
                            })
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .h(px(32.))
                            .rounded(px(6.))
                            .bg(bg)
                            .text_color(text_color)
                            .text_size(tokens::text(13.))
                            .when(!selected, |el| {
                                el.hover(|el| {
                                    el.bg(theme.accent).text_color(theme.accent_foreground)
                                })
                            })
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.active_section = section;
                                    this.saved_note = None;
                                    cx.notify();
                                }),
                            )
                            .child(Icon::new(section.icon()).size_4().text_color(if selected {
                                theme.sidebar_accent_foreground
                            } else {
                                theme.muted_foreground
                            }))
                            .child(section.title())
                    })),
            )
            .child(
                // nav footer：打开配置文件（对齐 OpenChamber nav footer）
                v_flex()
                    .border_t_1()
                    .border_color(theme.border)
                    .p_2()
                    .child(
                        Button::new("settings-open-file")
                            .label("打开配置文件")
                            .icon(Icon::new(IconName::FolderOpen).size_3())
                            .ghost()
                            .with_size(Size::Small)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.reveal_settings_file(window, cx);
                            })),
                    ),
            )
    }

    fn render_page(
        &self,
        section: SettingsSection,
        theme: &gpui_component::Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .id("settings-page")
            .flex_1()
            .min_h_0()
            .h_full()
            .items_start()
            .overflow_y_scroll()
            .child(
                // 页头：标题 + 描述
                v_flex()
                    .px_5()
                    .pt_4()
                    .pb_3()
                    .gap_1()
                    .child(
                        div()
                            .text_size(tokens::text(15.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(section.title()),
                    )
                    .child(
                        div()
                            .text_size(tokens::text(12.))
                            .text_color(theme.muted_foreground)
                            .child(section.description()),
                    ),
            )
            .children(match section {
                SettingsSection::General => self.render_general_section(theme, cx),
                SettingsSection::Appearance => self.render_appearance_section(theme, window, cx),
                SettingsSection::CodeFont => self.render_code_font_section(theme, window, cx),
                SettingsSection::Transfer => self.render_transfer_section(theme, cx),
            })
    }

    // ---- 各页 Section（分隔线 + FieldRow 左标签右控件） ----

    fn render_general_section(
        &self,
        theme: &gpui_component::Theme,
        _cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        vec![
            section_divider(theme)
                .child(field_row(
                    "签名链接有效期",
                    Some("对象签名 GET 链接的有效秒数。"),
                    field_input(Input::new(&self.ttl)),
                    theme,
                ))
                .into_any_element(),
            section_divider(theme)
                .child(field_row(
                    "清空剪贴板",
                    Some("复制签名链接后自动清除的秒数，0 = 不清除。"),
                    field_input(Input::new(&self.clipboard)),
                    theme,
                ))
                .into_any_element(),
        ]
    }

    fn render_appearance_section(
        &self,
        theme: &gpui_component::Theme,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let mode = self.appearance_mode;
        let radio = |section_id: &'static str,
                     mode_value: AppearanceMode,
                     label: &'static str,
                     theme: &gpui_component::Theme| {
            h_flex()
                .items_center()
                .gap_2()
                .child(
                    Radio::new(section_id)
                        .checked(mode == mode_value)
                        .on_click({
                            let theme = theme.clone();
                            cx.listener(move |this, checked: &bool, _, cx| {
                                if *checked {
                                    this.set_appearance_mode(mode_value, cx);
                                }
                                let _ = &theme;
                            })
                        }),
                )
                .child(
                    div()
                        .text_size(tokens::text(13.))
                        .text_color(theme.foreground)
                        .child(label),
                )
        };
        vec![
            section_divider(theme)
                .child(field_row(
                    "外观模式",
                    Some("跟随系统时自动响应系统浅色/深色切换。"),
                    v_flex()
                        .gap_2()
                        .child(radio(
                            "appearance-system",
                            AppearanceMode::System,
                            "跟随系统",
                            theme,
                        ))
                        .child(radio(
                            "appearance-light",
                            AppearanceMode::Light,
                            "浅色",
                            theme,
                        ))
                        .child(radio(
                            "appearance-dark",
                            AppearanceMode::Dark,
                            "深色",
                            theme,
                        )),
                    theme,
                ))
                .into_any_element(),
            section_divider(theme)
                .child(field_row(
                    "界面字体",
                    Some("选择系统默认或预设字体。"),
                    font_select("ui-font", &self.ui_font_select, cx),
                    theme,
                ))
                .into_any_element(),
            section_divider(theme)
                .child(field_row(
                    "界面字号缩放",
                    Some("0.85 – 1.40，影响全部界面文字。"),
                    number_field_input(&self.ui_font_scale),
                    theme,
                ))
                .into_any_element(),
        ]
    }

    fn render_code_font_section(
        &self,
        theme: &gpui_component::Theme,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        vec![
            section_divider(theme)
                .child(field_row(
                    "字体族",
                    Some("默认使用 macOS 等宽字体，可选预设。"),
                    font_select("code-font", &self.code_font_select, cx),
                    theme,
                ))
                .into_any_element(),
            section_divider(theme)
                .child(field_row(
                    "字号",
                    Some("10 – 24，用于代码预览与技术字段。"),
                    number_field_input(&self.code_font_size),
                    theme,
                ))
                .into_any_element(),
        ]
    }

    fn render_transfer_section(
        &self,
        theme: &gpui_component::Theme,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let download_dir = self
            .default_download_dir
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "未设置（使用用户主目录）".into());
        vec![
            section_divider(theme)
                .child(field_row(
                    "传输并发数",
                    Some("同时进行的上传/下载任务数，1 – 8。"),
                    number_field_input(&self.transfer_concurrency),
                    theme,
                ))
                .into_any_element(),
            section_divider(theme)
                .child(field_row(
                    "默认下载目录",
                    Some("下载时先确认使用该目录，仍可另存。"),
                    v_flex()
                        .gap_2()
                        .child(
                            div()
                                .w_full()
                                .px_2p5()
                                .py_1p5()
                                .rounded(px(6.))
                                .bg(theme.sidebar)
                                .border_1()
                                .border_color(theme.border)
                                .text_size(tokens::text(12.))
                                .text_color(theme.muted_foreground)
                                .truncate()
                                .child(download_dir),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Button::new("settings-pick-download-dir")
                                        .label("选择…")
                                        .with_size(Size::Small)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.choose_default_download_dir(cx);
                                        })),
                                )
                                .when(self.default_download_dir.is_some(), |el| {
                                    el.child(
                                        Button::new("settings-clear-download-dir")
                                            .label("清除")
                                            .ghost()
                                            .with_size(Size::Small)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.clear_default_download_dir(cx);
                                            })),
                                    )
                                }),
                        ),
                    theme,
                ))
                .into_any_element(),
        ]
    }

    fn render_footer(
        &self,
        theme: &gpui_component::Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        h_flex()
            .items_center()
            .justify_between()
            .px_4()
            .py_3()
            .border_t_1()
            .border_color(theme.border)
            .child(
                // 状态区：错误优先于成功提示（占位保持高度稳定）
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(tokens::text(12.))
                    .text_color(if self.error.is_some() {
                        theme.danger
                    } else {
                        theme.success
                    })
                    .child(
                        self.error
                            .clone()
                            .or_else(|| self.saved_note.clone())
                            .unwrap_or_default(),
                    ),
            )
            .child(
                h_flex()
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
    }
}

// ---- 布局原语（Section / FieldRow，对齐 OpenChamber 语义） ----

/// Section：顶部分隔线 + 纵向 padding；第一段分隔线由页头承担视觉起点。
fn section_divider(theme: &gpui_component::Theme) -> gpui::Div {
    div()
        .w_full()
        .border_t_1()
        .border_color(theme.border)
        .px_5()
        .pt_4()
        .pb_4()
}

/// FieldRow：左标签列（label + helper）+ 右控件簇，左右两栏顶对齐。
fn field_row(
    label_text: &'static str,
    helper: Option<&'static str>,
    control: impl IntoElement,
    theme: &gpui_component::Theme,
) -> gpui::Div {
    h_flex()
        .items_start()
        .gap_4()
        .child(
            v_flex()
                .w(px(180.))
                .flex_shrink_0()
                .gap_0p5()
                .child(
                    div()
                        .text_size(tokens::text(13.))
                        .text_color(theme.foreground)
                        .child(label_text),
                )
                .children(helper.map(|text| {
                    div()
                        .text_size(tokens::text(11.))
                        .text_color(theme.muted_foreground)
                        .child(text)
                })),
        )
        .child(control.into_any_element())
}

/// 控件簇统一宽度上限（对齐 OpenChamber `max-w-[24rem]`）。
fn field_input(input: Input) -> gpui::AnyElement {
    div().w(px(240.)).child(input.small()).into_any_element()
}

/// 数值输入（带 +/− 步进按钮；步进事件由 validate 兜底，范围校验在保存时）。
fn number_field_input(state: &Entity<InputState>) -> gpui::AnyElement {
    div()
        .w(px(140.))
        .child(NumberInput::new(state).with_size(Size::Small))
        .into_any_element()
}

/// 字体下拉：固定宽度，选择即写回（SelectEvent::Confirm 在 new 时订阅）。
fn font_select(
    _id: &'static str,
    state: &Entity<SelectState<Vec<FontOption>>>,
    _cx: &mut Context<SettingsModal>,
) -> gpui::AnyElement {
    div()
        .w(px(240.))
        .child(Select::new(state).with_size(Size::Small))
        .into_any_element()
}

/// 字体下拉选项：`""` = 系统默认占位显示 + 预设；若当前值不在其中则追加
/// （保证任意已有自定义字体都能预选中且可还原）。
fn font_select_items(presets: &[&'static str], current: Option<&str>) -> Vec<String> {
    let mut items: Vec<String> = std::iter::once(String::new())
        .chain(presets.iter().map(|p| p.to_string()))
        .collect();
    if let Some(value) = current
        && !value.is_empty()
        && !items.contains(&value.to_string())
    {
        items.push(value.to_string());
    }
    items
}

/// 选项索引：当前值（None 或 "" 视为系统默认）在 items 中的位置。
fn selected_font_index(items: &[String], current: Option<&str>) -> Option<IndexPath> {
    let target = current.unwrap_or("");
    items
        .iter()
        .position(|item| item == target)
        .map(IndexPath::new)
}

/// Select 显示标题：空串显示「系统默认」。
#[derive(Clone)]
struct FontOption(String);

impl gpui_component::select::SelectItem for FontOption {
    type Value = String;

    fn title(&self) -> gpui::SharedString {
        if self.0.is_empty() {
            "系统默认".into()
        } else {
            self.0.clone().into()
        }
    }

    fn value(&self) -> &Self::Value {
        &self.0
    }
}

const UI_FONT_PRESETS: &[&str] = &["SF Pro", "PingFang SC"];
const CODE_FONT_PRESETS: &[&str] = &["Menlo", "SF Mono", "JetBrains Mono"];

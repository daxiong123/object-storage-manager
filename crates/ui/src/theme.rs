//! 应用主题（Linear 风格设计基调，agents.md §7「UI 设计基调」）。
//!
//! 语义 token 体系沿用 [OpenChamber theme-system](https://github.com/openchamber/openchamber)
//! 的四族划分，映射到 gpui-component 的 `ThemeColor`：
//!
//! - **surface**：background / foreground / muted / elevated / overlay / border
//! - **interactive**：hover / active / selection（选中态）/ focusRing
//! - **status**：error / warning / success / info——只用于真实反馈，不做装饰
//! - **primary**：主 CTA——「执行动作」；与 selection「当前选中」严格区分
//!
//! 铁律（对照 OpenChamber Theme System 核心规则）：
//! - UI 代码只允许 `cx.theme()` 语义字段，禁止硬编码 hex / hsla 调色板色；
//! - hover 只给可交互元素；
//! - 不用 primary/accent 色标记普通选中行（选中用 selection / sidebar_accent）。
//!
//! 视觉身份（Linear 风格，规范 §38「低饱和」）：主色为低饱和靛蓝
//! （hue 233，锚定 Linear `#5E6AD2`），亮/暗两套各自微调亮度；
//! 中性面色族统一冷灰（hue 225），极低对比面 + 发丝级 1px 边框；
//! 选中态 = 低饱和 indigo 淡染色（≠ primary）；小圆角（radius 5 / lg 8）。
//! 状态色（error/warning/success/info）保持库默认，只用于真实反馈。
//!
//! 实现说明：gpui-component 的 `ThemeConfig.colors` 颜色为 hex 字符串
//! （内部经 `try_parse_color` 解析），且 `ThemeRegistry` 的默认主题没有
//! 公开写入口。因此这里直接构造 `ThemeConfig`，经 `Theme::apply_config`
//! 写入全局 `Theme` 的 `light_theme` / `dark_theme`（pub 字段），再以
//! `Theme::change` 按系统外观应用。

use std::sync::{OnceLock, RwLock};

use gpui::{App, SharedString};
use gpui_component::{Theme, ThemeConfig, ThemeMode, highlighter::HighlightThemeStyle};
use object_storage_persistence::{AppearanceMode, CODE_FONT_SIZE_DEFAULT, Settings};

use crate::tokens;

/// 主色 hue：低饱和靛蓝（Linear indigo `#5E6AD2` 基底）。
const HUE_DEG: f32 = 233.0;
/// 中性面色族 hue：统一冷灰（Linear 的低对比中性面）。
const NEUTRAL_HUE: f32 = 225.0;

#[derive(Debug, Clone)]
struct ThemePreferences {
    appearance_mode: AppearanceMode,
    ui_font_family: Option<String>,
    ui_font_scale: f32,
    code_font_family: Option<String>,
    code_font_size: u32,
}

impl Default for ThemePreferences {
    fn default() -> Self {
        Self {
            appearance_mode: AppearanceMode::System,
            ui_font_family: None,
            ui_font_scale: 1.0,
            code_font_family: None,
            code_font_size: CODE_FONT_SIZE_DEFAULT,
        }
    }
}

impl ThemePreferences {
    fn from_settings(settings: &Settings) -> Self {
        Self {
            appearance_mode: settings.appearance_mode,
            ui_font_family: settings.ui_font_family.clone(),
            ui_font_scale: settings.ui_font_scale,
            code_font_family: settings.code_font_family.clone(),
            code_font_size: settings.code_font_size,
        }
    }
}

fn preferences() -> &'static RwLock<ThemePreferences> {
    static PREFERENCES: OnceLock<RwLock<ThemePreferences>> = OnceLock::new();
    PREFERENCES.get_or_init(|| RwLock::new(ThemePreferences::default()))
}

/// 一套完整的语义色板（四族映射，见模块注释）。
///
/// 与「逐字段 Option 覆盖」不同，这里一次给全 surface / primary /
/// interactive 的定制值——Linear 风格的核心是中性面整体降温（hue 225），
/// 只改个别字段会被库默认的纯灰（无色相）衬出不协调。
#[derive(Debug, Clone, Copy)]
struct Palette {
    // ---- surface 族 ----
    background: [f32; 4],
    foreground: [f32; 4],
    muted: [f32; 4],
    muted_foreground: [f32; 4],
    border: [f32; 4],
    sidebar: [f32; 4],
    sidebar_foreground: [f32; 4],
    sidebar_border: [f32; 4],
    popover: [f32; 4],
    secondary: [f32; 4],
    secondary_hover: [f32; 4],
    secondary_active: [f32; 4],
    input: [f32; 4],
    list: [f32; 4],
    list_even: [f32; 4],
    list_head: [f32; 4],
    table: [f32; 4],
    table_even: [f32; 4],
    table_head: [f32; 4],
    table_row_border: [f32; 4],
    title_bar: [f32; 4],
    overlay: [f32; 4],
    window_border: [f32; 4],
    group_box: [f32; 4],
    group_box_foreground: [f32; 4],
    description_list_label: [f32; 4],
    description_list_label_foreground: [f32; 4],
    // ---- primary 族：主 CTA（「执行动作」，非选中态） ----
    primary: [f32; 4],
    primary_hover: [f32; 4],
    primary_active: [f32; 4],
    primary_foreground: [f32; 4],
    progress_bar: [f32; 4],
    // ---- interactive / selection 族 ----
    accent: [f32; 4],
    accent_foreground: [f32; 4],
    ring: [f32; 4],
    sidebar_accent: [f32; 4],
    sidebar_accent_foreground: [f32; 4],
    list_active: [f32; 4],
    list_active_border: [f32; 4],
    table_active: [f32; 4],
    table_active_border: [f32; 4],
    selection: [f32; 4],
    link: [f32; 4],
    link_hover: [f32; 4],
    link_active: [f32; 4],
    drag_border: [f32; 4],
    drop_target: [f32; 4],
    /// 列表/菜单行 hover：中性冷灰（≠ 选中态的 indigo 淡染色——
    /// Linear 语义：hover 是位置反馈，选中才染色）。
    row_hover: [f32; 4],
}

/// 亮色套：白底冷灰面 + 低饱和靛蓝。
fn light_palette() -> Palette {
    let n = |s: f32, l: f32| hsl(NEUTRAL_HUE, s, l);
    let na = |s: f32, l: f32, a: f32| hsla(NEUTRAL_HUE, s, l, a);
    let p = |l: f32| hsl(HUE_DEG, 0.45, l);
    Palette {
        background: n(0.20, 1.00),
        foreground: n(0.10, 0.13),
        muted: n(0.12, 0.965),
        muted_foreground: n(0.06, 0.44),
        border: n(0.12, 0.915),
        sidebar: n(0.14, 0.978),
        sidebar_foreground: n(0.10, 0.20),
        sidebar_border: n(0.12, 0.915),
        popover: hsl(0., 0., 1.0),
        secondary: n(0.12, 0.955),
        secondary_hover: n(0.10, 0.935),
        secondary_active: n(0.12, 0.915),
        input: hsl(0., 0., 1.0),
        list: hsl(0., 0., 1.0),
        list_even: n(0.14, 0.985),
        list_head: n(0.14, 0.978),
        table: hsl(0., 0., 1.0),
        table_even: n(0.14, 0.985),
        table_head: n(0.14, 0.978),
        table_row_border: na(0.12, 0.915, 0.70),
        title_bar: n(0.14, 0.975),
        overlay: hsla(NEUTRAL_HUE, 0.10, 0.12, 0.32),
        window_border: n(0.12, 0.88),
        group_box: n(0.12, 0.965),
        group_box_foreground: n(0.10, 0.13),
        description_list_label: n(0.12, 0.965),
        description_list_label_foreground: n(0.10, 0.13),
        primary: p(0.55),
        primary_hover: p(0.48),
        primary_active: p(0.42),
        primary_foreground: hsl(0., 0., 1.0),
        progress_bar: p(0.55),
        accent: hsl(HUE_DEG, 0.35, 0.945),
        accent_foreground: hsl(HUE_DEG, 0.40, 0.30),
        ring: hsl(HUE_DEG, 0.45, 0.60),
        sidebar_accent: hsl(HUE_DEG, 0.32, 0.935),
        sidebar_accent_foreground: hsl(HUE_DEG, 0.40, 0.28),
        list_active: hsl(HUE_DEG, 0.38, 0.925),
        list_active_border: hsl(HUE_DEG, 0.45, 0.62),
        table_active: hsl(HUE_DEG, 0.38, 0.925),
        table_active_border: hsl(HUE_DEG, 0.45, 0.62),
        selection: hsl(HUE_DEG, 0.45, 0.80),
        link: hsl(HUE_DEG, 0.48, 0.50),
        link_hover: hsl(HUE_DEG, 0.48, 0.42),
        link_active: hsl(HUE_DEG, 0.48, 0.38),
        drag_border: hsl(HUE_DEG, 0.45, 0.60),
        drop_target: hsla(HUE_DEG, 0.45, 0.60, 0.22),
        row_hover: n(0.10, 0.94),
    }
}

/// 暗色套：深冷灰底 + 同系靛蓝提亮、压饱和。
fn dark_palette() -> Palette {
    let n = |s: f32, l: f32| hsl(NEUTRAL_HUE, s, l);
    let na = |s: f32, l: f32, a: f32| hsla(NEUTRAL_HUE, s, l, a);
    let p = |l: f32| hsl(HUE_DEG, 0.40, l);
    Palette {
        background: n(0.09, 0.075),
        foreground: n(0.10, 0.90),
        muted: n(0.08, 0.14),
        muted_foreground: n(0.06, 0.58),
        border: n(0.08, 0.165),
        sidebar: n(0.09, 0.055),
        sidebar_foreground: n(0.08, 0.88),
        sidebar_border: n(0.08, 0.14),
        popover: n(0.09, 0.10),
        secondary: n(0.08, 0.16),
        secondary_hover: n(0.08, 0.20),
        secondary_active: n(0.08, 0.24),
        input: n(0.09, 0.10),
        list: n(0.09, 0.075),
        list_even: n(0.08, 0.09),
        list_head: n(0.09, 0.065),
        table: n(0.09, 0.075),
        table_even: n(0.08, 0.09),
        table_head: n(0.09, 0.065),
        table_row_border: na(0.08, 0.165, 0.70),
        title_bar: n(0.09, 0.06),
        overlay: hsla(NEUTRAL_HUE, 0.10, 0.01, 0.55),
        window_border: n(0.08, 0.20),
        group_box: n(0.08, 0.11),
        group_box_foreground: n(0.10, 0.90),
        description_list_label: n(0.08, 0.11),
        description_list_label_foreground: n(0.10, 0.90),
        primary: p(0.58),
        primary_hover: p(0.64),
        primary_active: p(0.70),
        primary_foreground: hsl(0., 0., 1.0),
        progress_bar: p(0.58),
        accent: hsl(HUE_DEG, 0.30, 0.22),
        accent_foreground: hsl(HUE_DEG, 0.30, 0.88),
        ring: hsl(HUE_DEG, 0.40, 0.60),
        sidebar_accent: hsl(HUE_DEG, 0.28, 0.22),
        sidebar_accent_foreground: hsl(HUE_DEG, 0.30, 0.88),
        list_active: hsl(HUE_DEG, 0.28, 0.24),
        list_active_border: hsl(HUE_DEG, 0.40, 0.55),
        table_active: hsl(HUE_DEG, 0.28, 0.24),
        table_active_border: hsl(HUE_DEG, 0.40, 0.55),
        selection: hsl(HUE_DEG, 0.40, 0.35),
        link: hsl(HUE_DEG, 0.40, 0.72),
        link_hover: hsl(HUE_DEG, 0.40, 0.80),
        link_active: hsl(HUE_DEG, 0.40, 0.78),
        drag_border: hsl(HUE_DEG, 0.40, 0.60),
        drop_target: hsla(HUE_DEG, 0.40, 0.60, 0.25),
        row_hover: n(0.08, 0.13),
    }
}

/// HSL（h: 0..360, s/l: 0..1）→ HSLA 分量数组（alpha = 1）。
fn hsl(h_deg: f32, s: f32, l: f32) -> [f32; 4] {
    hsla(h_deg, s, l, 1.0)
}

/// HSLA（h: 0..360, s/l/a: 0..1）→ HSLA 分量数组。
fn hsla(h_deg: f32, s: f32, l: f32, a: f32) -> [f32; 4] {
    [h_deg / 360., s, l, a]
}

/// HSLA 分量数组 → ThemeConfig 需要的 8 位 hex（#RRGGBBAA）。
///
/// 手写 HSL→RGB 转换（CSS Color Module Level 4 公式），不依赖
/// `Colorize::to_hex` 的实现细节，测试可独立验证。
fn hsla_to_hex([h, s, l, a]: [f32; 4]) -> String {
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let channel = |hue_offset: f32| {
        let t = (hue_offset % 1.0 + 1.0) % 1.0;
        let c = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (c * 255.0).round().clamp(0.0, 255.0) as u8
    };
    format!(
        "#{:02X}{:02X}{:02X}{:02X}",
        channel(h + 1.0 / 3.0),
        channel(h),
        channel(h - 1.0 / 3.0),
        (a * 255.0).round().clamp(0.0, 255.0) as u8
    )
}

/// 亮/暗两套主题配置。
pub fn theme_configs() -> Vec<ThemeConfig> {
    theme_configs_for_preferences(&ThemePreferences::default())
}

fn theme_configs_for_preferences(prefs: &ThemePreferences) -> Vec<ThemeConfig> {
    vec![
        config(
            ThemeMode::Light,
            "CloudStorage Light",
            light_palette(),
            prefs,
        ),
        config(ThemeMode::Dark, "CloudStorage Dark", dark_palette(), prefs),
    ]
}

fn config(
    mode: ThemeMode,
    name: &'static str,
    p: Palette,
    prefs: &ThemePreferences,
) -> ThemeConfig {
    // ThemeConfigColors 的 base.* 字段（red/blue/...）为私有且无 pub 构造器，
    // 只能 Default 后逐字段赋值；clippy field_reassign_with_default 在此误报
    // （我们只覆盖定制字段，其余保持默认），就地压掉。
    #[allow(clippy::field_reassign_with_default)]
    fn build(
        mode: ThemeMode,
        name: &'static str,
        p: Palette,
        prefs: &ThemePreferences,
    ) -> ThemeConfig {
        let mut cfg = ThemeConfig::default();
        cfg.is_default = true;
        cfg.name = SharedString::from(name);
        cfg.mode = mode;
        cfg.font_size = Some(16.0 * prefs.ui_font_scale);
        cfg.font_family = prefs.ui_font_family.clone().map(SharedString::from);
        cfg.mono_font_size = Some(prefs.code_font_size as f32);
        cfg.mono_font_family = prefs.code_font_family.clone().map(SharedString::from);
        // Linear 风格小圆角：库组件（Button/Input/List/Table/Dialog…）经
        // Theme.radius 统一继承，视图层自绘卡片用 radius_lg 对齐。
        cfg.radius = Some(5);
        cfg.radius_lg = Some(8);
        // 代码编辑器（code_editor 文本预览/编辑）高亮配色：跟随亮/暗模式，
        // 中性底色与 surface 族同冷灰系。
        cfg.highlight = Some(highlight_theme_style(mode));
        // Linear 纯平：关闭库组件（Button/Input/Select/Checkbox/Radio/Slider）的
        // shadow_xs。自建浮层卡片的 shadow_lg() 是显式调用，不受此开关影响。
        cfg.shadow = Some(false);
        let hex = |v: [f32; 4]| SharedString::from(hsla_to_hex(v));
        let c = &mut cfg.colors;
        // surface
        c.background = Some(hex(p.background));
        c.foreground = Some(hex(p.foreground));
        c.muted = Some(hex(p.muted));
        c.muted_foreground = Some(hex(p.muted_foreground));
        c.border = Some(hex(p.border));
        c.sidebar = Some(hex(p.sidebar));
        c.sidebar_foreground = Some(hex(p.sidebar_foreground));
        c.sidebar_border = Some(hex(p.sidebar_border));
        c.popover = Some(hex(p.popover));
        c.popover_foreground = Some(hex(p.foreground));
        c.secondary = Some(hex(p.secondary));
        c.secondary_hover = Some(hex(p.secondary_hover));
        c.secondary_active = Some(hex(p.secondary_active));
        c.secondary_foreground = Some(hex(p.foreground));
        c.input = Some(hex(p.input));
        c.list = Some(hex(p.list));
        c.list_even = Some(hex(p.list_even));
        c.list_head = Some(hex(p.list_head));
        // hover 是中性位置反馈，选中才是 indigo 染色（selection ≠ hover）
        c.list_hover = Some(hex(p.row_hover));
        c.table = Some(hex(p.table));
        c.table_even = Some(hex(p.table_even));
        c.table_head = Some(hex(p.table_head));
        c.table_head_foreground = Some(hex(p.muted_foreground));
        c.table_hover = Some(hex(p.row_hover));
        c.table_row_border = Some(hex(p.table_row_border));
        c.title_bar = Some(hex(p.title_bar));
        c.title_bar_border = Some(hex(p.border));
        c.overlay = Some(hex(p.overlay));
        c.window_border = Some(hex(p.window_border));
        c.group_box = Some(hex(p.group_box));
        c.group_box_foreground = Some(hex(p.group_box_foreground));
        c.description_list_label = Some(hex(p.description_list_label));
        c.description_list_label_foreground = Some(hex(p.description_list_label_foreground));
        // primary
        c.primary = Some(hex(p.primary));
        c.primary_hover = Some(hex(p.primary_hover));
        c.primary_active = Some(hex(p.primary_active));
        c.primary_foreground = Some(hex(p.primary_foreground));
        c.progress_bar = Some(hex(p.progress_bar));
        // interactive / selection
        c.accent = Some(hex(p.accent));
        c.accent_foreground = Some(hex(p.accent_foreground));
        c.ring = Some(hex(p.ring));
        c.sidebar_accent = Some(hex(p.sidebar_accent));
        c.sidebar_accent_foreground = Some(hex(p.sidebar_accent_foreground));
        c.list_active = Some(hex(p.list_active));
        c.list_active_border = Some(hex(p.list_active_border));
        c.table_active = Some(hex(p.table_active));
        c.table_active_border = Some(hex(p.table_active_border));
        c.selection = Some(hex(p.selection));
        c.link = Some(hex(p.link));
        c.link_hover = Some(hex(p.link_hover));
        c.link_active = Some(hex(p.link_active));
        c.drag_border = Some(hex(p.drag_border));
        c.drop_target = Some(hex(p.drop_target));
        cfg
    }
    build(mode, name, p, prefs)
}

/// 代码高亮主题（编辑器底/前景/活动行/行号 + 常用语法 token）。
/// 亮暗两套同结构：暗色提高各 token 亮度保持辨识度；中性色与 surface
/// 族同冷灰系（hue 225），编辑器底色随亮暗模式微调。
///
/// `ThemeStyle` 字段私有且无 pub 构造器，只能走 serde 反序列化——
/// 与库加载内置主题（default-theme.json）同路径，hex 字符串即 gpui
/// `Hsla` 的 serde 格式。
fn highlight_theme_style(mode: ThemeMode) -> HighlightThemeStyle {
    let dark = mode.is_dark();
    let keyword = hsl_hex(233.0, 0.60, if dark { 0.72 } else { 0.46 });
    let string = hsl_hex(105.0, 0.45, if dark { 0.62 } else { 0.32 });
    let comment = hsl_hex(NEUTRAL_HUE, 0.08, if dark { 0.52 } else { 0.50 });
    let warm = hsl_hex(5.0, 0.65, if dark { 0.68 } else { 0.46 });
    let function = hsl_hex(230.0, 0.60, if dark { 0.76 } else { 0.40 });
    let type_ = hsl_hex(262.0, 0.50, if dark { 0.76 } else { 0.48 });
    let plain = if dark {
        hsl_hex(NEUTRAL_HUE, 0.10, 0.86)
    } else {
        hsl_hex(NEUTRAL_HUE, 0.10, 0.24)
    };
    let operator = hsl_hex(NEUTRAL_HUE, 0.12, if dark { 0.78 } else { 0.32 });
    let punctuation = hsl_hex(NEUTRAL_HUE, 0.10, if dark { 0.70 } else { 0.36 });
    let number = keyword.clone();
    let comment_doc = hsl_hex(NEUTRAL_HUE, 0.08, if dark { 0.56 } else { 0.46 });

    let json = serde_json::json!({
        "editor.background": if dark { hsl_hex(NEUTRAL_HUE, 0.14, 0.10) } else { hsl_hex(NEUTRAL_HUE, 0.20, 0.985) },
        "editor.foreground": plain.clone(),
        "editor.active_line.background": if dark { hsl_hex(NEUTRAL_HUE, 0.14, 0.145) } else { hsl_hex(NEUTRAL_HUE, 0.24, 0.945) },
        "editor.line_number": hsl_hex(NEUTRAL_HUE, 0.08, if dark { 0.42 } else { 0.62 }),
        "editor.active_line_number": hsl_hex(HUE_DEG, if dark { 0.40 } else { 0.50 }, if dark { 0.72 } else { 0.40 }),
        "syntax": {
            "keyword": { "color": keyword },
            "string": { "color": string },
            "comment": { "color": comment },
            "comment.doc": { "color": comment_doc },
            "number": { "color": number },
            "boolean": { "color": warm },
            "constant": { "color": warm },
            "function": { "color": function },
            "type": { "color": type_ },
            "variable": { "color": plain },
            "property": { "color": plain },
            "operator": { "color": operator },
            "punctuation": { "color": punctuation },
        },
    });
    serde_json::from_value(json)
        .unwrap_or_else(|error| panic!("内置高亮主题 JSON 必须合法: {error}"))
}

/// HSL → hex 字符串（`#RRGGBB`，gpui `Hsla` serde 支持的格式之一）。
fn hsl_hex(h_deg: f32, s: f32, l: f32) -> String {
    let hex = hsla_to_hex(hsla(h_deg, s, l, 1.0));
    hex[..7].to_string()
}

/// 把应用主题写入全局 Theme：亮/暗两套 `ThemeConfig` 挂到 pub 字段
/// `light_theme` / `dark_theme` 上，再按当前系统外观应用。
/// 在 `gpui_component::init` 之后、窗口创建之前调用一次。
pub fn init(cx: &mut App) {
    use std::rc::Rc;

    let prefs = preferences()
        .read()
        .unwrap_or_else(|poisoned| panic!("主题偏好锁已毒化: {poisoned}"))
        .clone();
    let configs = theme_configs_for_preferences(&prefs);
    let appearance = cx.window_appearance();
    let mode = effective_theme_mode(prefs.appearance_mode, ThemeMode::from(appearance));
    {
        let theme = Theme::global_mut(cx);
        for config in configs {
            if config.mode.is_dark() {
                theme.dark_theme = Rc::new(config);
            } else {
                theme.light_theme = Rc::new(config);
            }
        }
    }
    // Theme::change(mode) 会从 Theme 全局的 light/dark 字段 apply_config。
    // 传 window=None：初始化时还没有窗口；窗口创建后由系统外观事件驱动。
    let _ = mode;
    Theme::change(mode, None, cx);
}

/// 窗口外观变化时同步主题（WorkspaceView 创建窗口后调用一次）。
/// 外观默认跟随 System；设置为 Light/Dark 时忽略系统外观事件。
pub fn observe_appearance(window: &mut gpui::Window, cx: &mut App) {
    if preferences()
        .read()
        .unwrap_or_else(|poisoned| panic!("主题偏好锁已毒化: {poisoned}"))
        .appearance_mode
        != AppearanceMode::System
    {
        return;
    }
    let appearance = window.appearance();
    let mode = ThemeMode::from(appearance);
    if Theme::global(cx).mode != mode {
        Theme::change(mode, Some(window), cx);
    }
}

pub fn apply_settings(settings: &Settings, window: Option<&mut gpui::Window>, cx: &mut App) {
    let prefs = ThemePreferences::from_settings(settings);
    tokens::set_ui_font_scale(prefs.ui_font_scale);
    {
        let mut guard = preferences()
            .write()
            .unwrap_or_else(|poisoned| panic!("主题偏好锁已毒化: {poisoned}"));
        *guard = prefs.clone();
    }

    let system_mode = window
        .as_ref()
        .map(|window| ThemeMode::from(window.appearance()))
        .unwrap_or_else(|| ThemeMode::from(cx.window_appearance()));
    let mode = effective_theme_mode(prefs.appearance_mode, system_mode);
    let configs = theme_configs_for_preferences(&prefs);
    {
        use std::rc::Rc;

        let theme = Theme::global_mut(cx);
        for config in configs {
            if config.mode.is_dark() {
                theme.dark_theme = Rc::new(config);
            } else {
                theme.light_theme = Rc::new(config);
            }
        }
    }
    Theme::change(mode, window, cx);
}

fn effective_theme_mode(preference: AppearanceMode, system_mode: ThemeMode) -> ThemeMode {
    match preference {
        AppearanceMode::System => system_mode,
        AppearanceMode::Light => ThemeMode::Light,
        AppearanceMode::Dark => ThemeMode::Dark,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::WindowAppearance;

    #[test]
    fn hex_round_trip_anchors() {
        // 纯白 / 纯黑 / 纯红 / 青色锚点（CSS HSL 公式的已知值）
        assert_eq!(hsla_to_hex([0., 0., 1., 1.]), "#FFFFFFFF");
        assert_eq!(hsla_to_hex([0., 0., 0., 1.]), "#000000FF");
        assert_eq!(hsla_to_hex([0., 1., 0.5, 1.]), "#FF0000FF");
        assert_eq!(hsla_to_hex([0.5, 1., 0.5, 1.]), "#00FFFFFF");
    }

    #[test]
    fn hex_parses_back_via_try_parse_color() {
        // gpui-component 内部用 Rgba::try_from 解析 hex；8 位大写格式必须可用。
        // 这里用等价逻辑验证格式（# + 8 位 hex）。
        let hex = hsla_to_hex(hsl(HUE_DEG, 0.45, 0.55));
        assert_eq!(hex.len(), 9, "8 位 hex + #：{hex}");
        assert!(hex.starts_with('#'));
        assert!(hex[1..].chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn light_primary_is_low_saturation_indigo() {
        let [h, s, l, _] = light_palette().primary;
        assert!(s <= 0.5, "primary 饱和度 {s} 应为低饱和");
        assert!((l - 0.55).abs() < 1e-6);
        assert!((h - HUE_DEG / 360.).abs() < 1e-6);
    }

    #[test]
    fn dark_primary_is_low_saturation_indigo() {
        let [h, s, _, _] = dark_palette().primary;
        assert!(s <= 0.5, "primary 饱和度 {s} 应为低饱和");
        assert!((h - HUE_DEG / 360.).abs() < 1e-6);
    }

    #[test]
    fn selection_differs_from_primary() {
        // selection ≠ primary：选中态不能复用主 CTA 色
        for (name, p) in [("light", light_palette()), ("dark", dark_palette())] {
            assert_ne!(
                p.list_active, p.primary,
                "{name}: list_active 不应等于 primary"
            );
            assert_ne!(
                p.sidebar_accent, p.primary,
                "{name}: sidebar_accent 不应等于 primary"
            );
        }
    }

    #[test]
    fn row_hover_is_neutral_not_indigo() {
        // hover 是中性位置反馈（hue 225），选中才是 indigo 染色（hue 233）——
        // 二者肉眼可辨（Linear 语义：hover ≠ selection）
        for (name, p) in [("light", light_palette()), ("dark", dark_palette())] {
            let [hover_h, _, _, _] = p.row_hover;
            assert!(
                (hover_h - NEUTRAL_HUE / 360.).abs() < 1e-6,
                "{name}: row_hover 应为中性冷灰"
            );
            assert_ne!(p.row_hover, p.list_active, "{name}: hover ≠ 选中色");
        }
    }

    #[test]
    fn neutral_surfaces_share_cool_gray_hue() {
        // Linear 风格核心：中性面整体降温，全部 surface 字段同冷灰系（hue 225）
        for (name, p) in [("light", light_palette()), ("dark", dark_palette())] {
            let neutrals = [
                ("background", p.background),
                ("foreground", p.foreground),
                ("muted", p.muted),
                ("muted_foreground", p.muted_foreground),
                ("border", p.border),
                ("sidebar", p.sidebar),
                ("sidebar_border", p.sidebar_border),
                ("title_bar", p.title_bar),
                ("list_head", p.list_head),
            ];
            for (field, [h, _, _, _]) in neutrals {
                assert!(
                    (h - NEUTRAL_HUE / 360.).abs() < 1e-6,
                    "{name}.{field} hue 应为 {NEUTRAL_HUE}°"
                );
            }
        }
    }

    #[test]
    fn theme_configs_cover_both_modes_with_linear_geometry() {
        let configs = theme_configs();
        assert_eq!(configs.len(), 2);
        assert!(configs.iter().any(|t| t.mode == ThemeMode::Light));
        assert!(configs.iter().any(|t| t.mode == ThemeMode::Dark));
        for theme in &configs {
            assert!(theme.is_default);
            let primary = theme
                .colors
                .primary
                .as_ref()
                .map(|s| s.as_ref())
                .expect("primary 必须设置");
            assert_eq!(primary.len(), 9, "8 位 hex + #：{primary}");
            assert!(theme.colors.background.is_some(), "background 必须设置");
            assert_eq!(theme.radius, Some(5), "Linear 风格小圆角");
            assert_eq!(theme.radius_lg, Some(8));
            assert_eq!(theme.shadow, Some(false), "Linear 纯平：组件无阴影");
        }
    }

    #[test]
    fn appearance_maps_to_mode() {
        // gpui-component 已提供 WindowAppearance → ThemeMode 的 From 实现，
        // 这里验证映射语义（暗色两变体 → Dark）。
        assert_eq!(ThemeMode::from(WindowAppearance::Light), ThemeMode::Light);
        assert_eq!(
            ThemeMode::from(WindowAppearance::VibrantDark),
            ThemeMode::Dark
        );
        assert_eq!(ThemeMode::from(WindowAppearance::Dark), ThemeMode::Dark);
    }

    #[test]
    fn appearance_preference_overrides_system_mode() {
        assert_eq!(
            effective_theme_mode(AppearanceMode::System, ThemeMode::Dark),
            ThemeMode::Dark
        );
        assert_eq!(
            effective_theme_mode(AppearanceMode::Light, ThemeMode::Dark),
            ThemeMode::Light
        );
        assert_eq!(
            effective_theme_mode(AppearanceMode::Dark, ThemeMode::Light),
            ThemeMode::Dark
        );
    }

    #[test]
    fn theme_config_applies_font_preferences() {
        let prefs = ThemePreferences {
            appearance_mode: AppearanceMode::System,
            ui_font_family: Some("PingFang SC".into()),
            ui_font_scale: 1.15,
            code_font_family: Some("SF Mono".into()),
            code_font_size: 15,
        };
        let configs = theme_configs_for_preferences(&prefs);
        for config in configs {
            assert_eq!(
                config.font_family.as_ref().map(|s| s.as_ref()),
                Some("PingFang SC")
            );
            assert_eq!(config.font_size, Some(18.4));
            assert_eq!(
                config.mono_font_family.as_ref().map(|s| s.as_ref()),
                Some("SF Mono")
            );
            assert_eq!(config.mono_font_size, Some(15.0));
        }
    }
}

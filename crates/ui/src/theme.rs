//! 应用主题（OpenChamber 设计基调，agents.md §5「UI 设计基调」）。
//!
//! 参考 [OpenChamber theme-system](https://github.com/openchamber/openchamber)
//! 的语义 token 体系，映射到 gpui-component 的 `ThemeColor`：
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
//! 视觉身份：低饱和 Accent（规范 §7）。主色用低饱和青蓝（hue 210、低饱和度），
//! 亮/暗两套各自微调亮度，其余色阶保持 gpui-component 默认中性灰阶，
//! 保证组件库（Button/Input/List/Table 等）默认观感协调。
//!
//! 实现说明：gpui-component 的 `ThemeConfig.colors` 颜色为 hex 字符串
//! （内部经 `try_parse_color` 解析），且 `ThemeRegistry` 的默认主题没有
//! 公开写入口。因此这里直接构造 `ThemeConfig`（未定制字段留 `None`，
//! apply 时以对应模式默认色兜底），经 `Theme::apply_config` 写入全局
//! `Theme` 的 `light_theme` / `dark_theme`（pub 字段），再以
//! `Theme::change` 按系统外观应用。

use std::sync::{OnceLock, RwLock};

use gpui::{App, SharedString};
use gpui_component::{Theme, ThemeConfig, ThemeMode, highlighter::HighlightThemeStyle};
use object_storage_persistence::{AppearanceMode, CODE_FONT_SIZE_DEFAULT, Settings};

use crate::tokens;

/// 主色 hue：青蓝（低饱和 Accent 的基底）。
const HUE_DEG: f32 = 210.0;

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

/// 亮色套定制项。
fn light_overrides() -> ThemeColorOverrides {
    ThemeColorOverrides {
        // primary 族：主 CTA（「执行动作」，非选中态）
        primary: Some(hsl(HUE_DEG, 0.45, 0.42)),
        primary_hover: Some(hsl(HUE_DEG, 0.45, 0.36)),
        primary_active: Some(hsl(HUE_DEG, 0.45, 0.32)),
        primary_foreground: Some(hsl(0., 0., 1.)),
        // interactive 族：组件库 hover 高亮与焦点环
        accent: Some(hsl(HUE_DEG, 0.20, 0.93)),
        accent_foreground: Some(hsl(HUE_DEG, 0.35, 0.25)),
        ring: Some(hsl(HUE_DEG, 0.40, 0.55)),
        // selection 族：选中态（≠ primary）
        sidebar_accent: Some(hsl(HUE_DEG, 0.22, 0.92)),
        sidebar_accent_foreground: Some(hsl(HUE_DEG, 0.35, 0.25)),
        list_active: Some(hsl(HUE_DEG, 0.22, 0.92)),
        table_active: Some(hsl(HUE_DEG, 0.22, 0.92)),
    }
}

/// 暗色套：同色系，提高亮度、压低饱和以适配深底。
fn dark_overrides() -> ThemeColorOverrides {
    ThemeColorOverrides {
        primary: Some(hsl(HUE_DEG, 0.40, 0.62)),
        primary_hover: Some(hsl(HUE_DEG, 0.40, 0.68)),
        primary_active: Some(hsl(HUE_DEG, 0.40, 0.72)),
        primary_foreground: Some(hsl(HUE_DEG, 0.30, 0.10)),
        accent: Some(hsl(HUE_DEG, 0.25, 0.24)),
        accent_foreground: Some(hsl(HUE_DEG, 0.30, 0.85)),
        ring: Some(hsl(HUE_DEG, 0.40, 0.60)),
        sidebar_accent: Some(hsl(HUE_DEG, 0.25, 0.24)),
        sidebar_accent_foreground: Some(hsl(HUE_DEG, 0.30, 0.85)),
        list_active: Some(hsl(HUE_DEG, 0.25, 0.24)),
        table_active: Some(hsl(HUE_DEG, 0.25, 0.24)),
    }
}

/// 我们定制的语义 token 子集（四族映射，见模块注释）。
#[derive(Debug, Clone, Copy, PartialEq)]
struct ThemeColorOverrides {
    primary: Option<[f32; 4]>,
    primary_hover: Option<[f32; 4]>,
    primary_active: Option<[f32; 4]>,
    primary_foreground: Option<[f32; 4]>,
    accent: Option<[f32; 4]>,
    accent_foreground: Option<[f32; 4]>,
    ring: Option<[f32; 4]>,
    sidebar_accent: Option<[f32; 4]>,
    sidebar_accent_foreground: Option<[f32; 4]>,
    list_active: Option<[f32; 4]>,
    table_active: Option<[f32; 4]>,
}

/// HSL（h: 0..360, s/l: 0..1）→ HSLA 分量数组。
fn hsl(h_deg: f32, s: f32, l: f32) -> [f32; 4] {
    [h_deg / 360., s, l, 1.0]
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

/// 亮/暗两套主题配置（未列出的字段保持 None，apply 时以库默认色兜底）。
pub fn theme_configs() -> Vec<ThemeConfig> {
    theme_configs_for_preferences(&ThemePreferences::default())
}

fn theme_configs_for_preferences(prefs: &ThemePreferences) -> Vec<ThemeConfig> {
    vec![
        config(
            ThemeMode::Light,
            "CloudStorage Light",
            light_overrides(),
            prefs,
        ),
        config(
            ThemeMode::Dark,
            "CloudStorage Dark",
            dark_overrides(),
            prefs,
        ),
    ]
}

fn config(
    mode: ThemeMode,
    name: &'static str,
    o: ThemeColorOverrides,
    prefs: &ThemePreferences,
) -> ThemeConfig {
    // ThemeConfigColors 的 base.* 字段（red/blue/...）为私有且无 pub 构造器，
    // 只能 Default 后逐字段赋值；clippy field_reassign_with_default 在此误报
    // （我们只覆盖定制字段，其余保持默认），就地压掉。
    #[allow(clippy::field_reassign_with_default)]
    fn build(
        mode: ThemeMode,
        name: &'static str,
        o: ThemeColorOverrides,
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
        // 代码编辑器（code_editor 文本预览/编辑）高亮配色：跟随亮/暗模式，
        // 否则库默认主题与我们的中性灰底不协调（验收问题）。
        cfg.highlight = Some(highlight_theme_style(mode));
        let hex = |v: [f32; 4]| SharedString::from(hsla_to_hex(v));
        let c = &mut cfg.colors;
        c.primary = o.primary.map(hex);
        c.primary_hover = o.primary_hover.map(hex);
        c.primary_active = o.primary_active.map(hex);
        c.primary_foreground = o.primary_foreground.map(hex);
        c.accent = o.accent.map(hex);
        c.accent_foreground = o.accent_foreground.map(hex);
        c.ring = o.ring.map(hex);
        c.sidebar_accent = o.sidebar_accent.map(hex);
        c.sidebar_accent_foreground = o.sidebar_accent_foreground.map(hex);
        c.list_active = o.list_active.map(hex);
        c.table_active = o.table_active.map(hex);
        cfg
    }
    build(mode, name, o, prefs)
}

/// 代码高亮主题（编辑器底/前景/活动行/行号 + 常用语法 token）。
/// 亮暗两套同结构：暗色提高各 token 亮度保持辨识度；具体取值锚定
/// gpui-component 默认主题的语义（keyword/number 同族蓝、string 绿、
/// comment 灰、constant/boolean 暖红、type 紫），中性色与 surface 族一致。
///
/// `ThemeStyle` 字段私有且无 pub 构造器，只能走 serde 反序列化——
/// 与库加载内置主题（default-theme.json）同路径，hex 字符串即 gpui
/// `Hsla` 的 serde 格式。
fn highlight_theme_style(mode: ThemeMode) -> HighlightThemeStyle {
    let dark = mode.is_dark();
    let keyword = hsl_hex(210.0, 0.85, if dark { 0.68 } else { 0.42 });
    let string = hsl_hex(105.0, 0.55, if dark { 0.62 } else { 0.30 });
    let comment = hsl_hex(220.0, 0.08, if dark { 0.52 } else { 0.48 });
    let warm = hsl_hex(5.0, 0.75, if dark { 0.68 } else { 0.44 });
    let function = hsl_hex(230.0, 0.75, if dark { 0.74 } else { 0.36 });
    let type_ = hsl_hex(262.0, 0.60, if dark { 0.74 } else { 0.44 });
    let plain = if dark {
        hsl_hex(220.0, 0.10, 0.86)
    } else {
        hsl_hex(220.0, 0.10, 0.24)
    };
    let operator = hsl_hex(220.0, 0.12, if dark { 0.78 } else { 0.30 });
    let punctuation = hsl_hex(220.0, 0.10, if dark { 0.70 } else { 0.34 });
    let number = keyword.clone();
    let comment_doc = hsl_hex(220.0, 0.08, if dark { 0.56 } else { 0.44 });

    let json = serde_json::json!({
        "editor.background": if dark { hsl_hex(220.0, 0.14, 0.10) } else { hsl_hex(210.0, 0.20, 0.985) },
        "editor.foreground": plain.clone(),
        "editor.active_line.background": if dark { hsl_hex(220.0, 0.14, 0.145) } else { hsl_hex(210.0, 0.24, 0.945) },
        "editor.line_number": hsl_hex(220.0, 0.08, if dark { 0.42 } else { 0.62 }),
        "editor.active_line_number": hsl_hex(210.0, if dark { 0.40 } else { 0.55 }, if dark { 0.72 } else { 0.38 }),
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
    let hex = hsla_to_hex([h_deg / 360.0, s, l, 1.0]);
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
        let hex = hsla_to_hex(hsl(HUE_DEG, 0.45, 0.42));
        assert_eq!(hex.len(), 9, "8 位 hex + #：{hex}");
        assert!(hex.starts_with('#'));
        assert!(hex[1..].chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn light_primary_is_low_saturation_blue() {
        let o = light_overrides();
        let [h, s, l, _] = o.primary.expect("light primary 必须设置");
        assert!(s <= 0.5, "primary 饱和度 {s} 应为低饱和");
        assert!((l - 0.42).abs() < 1e-6);
        assert!((h - HUE_DEG / 360.).abs() < 1e-6);
    }

    #[test]
    fn dark_primary_is_low_saturation_blue() {
        let o = dark_overrides();
        let [h, s, _, _] = o.primary.expect("dark primary 必须设置");
        assert!(s <= 0.5, "primary 饱和度 {s} 应为低饱和");
        assert!((h - HUE_DEG / 360.).abs() < 1e-6);
    }

    #[test]
    fn selection_differs_from_primary() {
        // selection ≠ primary：选中态不能复用主 CTA 色
        for (name, o) in [("light", light_overrides()), ("dark", dark_overrides())] {
            assert_ne!(
                o.list_active, o.primary,
                "{name}: list_active 不应等于 primary"
            );
            assert_ne!(
                o.sidebar_accent, o.primary,
                "{name}: sidebar_accent 不应等于 primary"
            );
        }
    }

    #[test]
    fn theme_configs_cover_both_modes() {
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

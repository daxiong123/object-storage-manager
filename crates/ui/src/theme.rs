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

use gpui::{App, Hsla, SharedString};
use gpui_component::{Theme, ThemeConfig, ThemeMode, highlighter::HighlightThemeStyle};
use object_storage_persistence::{AppearanceMode, CODE_FONT_SIZE_DEFAULT, Settings, ThemeStyle};

use crate::tokens;

/// 主色 hue：低饱和靛蓝（Linear indigo `#5E6AD2` 基底）。
const HUE_DEG: f32 = 233.0;
/// 中性面色族 hue：统一冷灰（Linear 的低对比中性面）。
const NEUTRAL_HUE: f32 = 225.0;

#[derive(Debug, Clone)]
struct ThemePreferences {
    appearance_mode: AppearanceMode,
    theme_style: ThemeStyle,
    ui_font_family: Option<String>,
    ui_font_scale: f32,
    code_font_family: Option<String>,
    code_font_size: u32,
}

impl Default for ThemePreferences {
    fn default() -> Self {
        Self {
            appearance_mode: AppearanceMode::System,
            theme_style: ThemeStyle::Linear,
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
            theme_style: settings.theme_style,
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

/// 一套完整的语义色板（六族映射，见模块注释）。
///
/// 与「逐字段 Option 覆盖」不同，这里一次给全 surface / primary /
/// interactive / status 的定制值——Linear 风格的核心是中性面整体降温（hue 225），
/// 只改个别字段会被库默认的纯灰（无色相）衬出不协调。
///
/// **族纪律（由 `tests` 里的断言把守，不是靠自觉）**：
/// 1. surface / border / text 三族只允许冷灰（hue 225）或纯灰——界面主体不带色相；
/// 2. brand 族（primary / selection / accent / link）只用低饱和靛蓝 hue 233；
/// 3. status 族（danger / warning / success / info）是**唯一**允许外来色相的地方，
///    因为它承载真实反馈：颜色在这里是信息，不是装饰；
/// 4. hover 是中性位置反馈，选中才染色（hover ≠ selection）。
#[derive(Debug, Clone, Copy)]
struct Palette {
    // ---- 族 1/6：surface（面）——背景、侧栏、卡片、列表/表格底色、遮罩 ----
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
    // ---- 族 2/6：primary（主 CTA：「执行动作」，非选中态） ----
    primary: [f32; 4],
    primary_hover: [f32; 4],
    primary_active: [f32; 4],
    primary_foreground: [f32; 4],
    progress_bar: [f32; 4],
    // ---- 族 3/6：interactive / selection（可交互与选中） ----
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
    // ---- 族 6/6：status（真实反馈）——唯一允许外来色相的一族 ----
    //
    // 原先不设这四项，全部吃 gpui-component 的默认值；问题是库默认的 status
    // 色与我们降温后的中性面（hue 225）不同源，同一个红色按钮在亮/暗两套里
    // 与整体的冷暖关系不一致。显式给值后，亮/暗两套的 status 也由同一组
    // 色相推导，并且「颜色只用于表意」这条纪律才有断言可写
    // （见 `status_is_the_only_foreign_hue_family`）。
    danger: [f32; 4],
    danger_hover: [f32; 4],
    danger_active: [f32; 4],
    danger_foreground: [f32; 4],
    warning: [f32; 4],
    warning_hover: [f32; 4],
    warning_active: [f32; 4],
    warning_foreground: [f32; 4],
    success: [f32; 4],
    success_hover: [f32; 4],
    success_active: [f32; 4],
    success_foreground: [f32; 4],
    info: [f32; 4],
    info_hover: [f32; 4],
    info_active: [f32; 4],
    info_foreground: [f32; 4],
}

/// status 族色相：与中性面（225）和品牌靛蓝（233）都拉开距离，
/// 且四者互相远离（最近的一对是 danger/warning，41°），靠颜色就能区分
/// 「出错 / 留意 / 成功 / 提示」。warning 取 45 而非更常见的 38，正是为了让
/// 它与 danger（4）拉开可辨距离——写成断言见
/// `status_hues_are_mutually_distinguishable`。
const HUE_DANGER: f32 = 4.;
const HUE_WARNING: f32 = 45.;
const HUE_SUCCESS: f32 = 148.;
const HUE_INFO: f32 = 205.;

/// sRGB 十六进制 → HSLA 分量数组。
///
/// waku 风格那两套直接引用上游 `src/theme.rs` 里的 `rgb(0x……)` 字面值，走这个
/// 换算而不是我手工折成 HSL——取色结果必须能对着上游源码逐字段核对，手工换算
/// 引入的误差会让「对照」失去意义。
fn rgb(hex: u32) -> [f32; 4] {
    let r = ((hex >> 16) & 0xFF) as f32 / 255.;
    let g = ((hex >> 8) & 0xFF) as f32 / 255.;
    let b = (hex & 0xFF) as f32 / 255.;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.;
    let delta = max - min;
    if delta <= f32::EPSILON {
        return [0., 0., l, 1.];
    }
    let s = if l > 0.5 {
        delta / (2. - max - min)
    } else {
        delta / (max + min)
    };
    let h = if max == r {
        ((g - b) / delta).rem_euclid(6.)
    } else if max == g {
        (b - r) / delta + 2.
    } else {
        (r - g) / delta + 4.
    };
    [(h / 6.).rem_euclid(1.), s, l, 1.]
}

/// waku 风格的选中色相（上游 `theme.rs` 的 `selection`：`hsla(211, 1.0, 0.50, α)`）。
/// 注意它与 brand 是**分开的**：珊瑚色只做品牌标记，选中态用蓝色。
const WAKU_HUE_SELECTION: f32 = 211.;
/// waku 风格的中性/边框色相（上游的 `border` 用 `hsla(220, 0.10, …)`）。
const WAKU_HUE_NEUTRAL: f32 = 220.;

/// waku 风格·亮色。
///
/// 取自上游 `src/theme.rs` 的 `light()`：中性面是**无彩灰**（#F6F5F6 / #ECECEC /
/// #E6E6E6，色相无从谈起），不是我们 Linear 那套冷灰（hue 225）；边框是低透明度
/// 叠色而非实色；brand 是珊瑚 #C85F44；选中态是与 brand 分离的蓝
/// `hsla(211, 1.0, 0.50, 0.35)`；主 CTA 用单色 `inverse`（#202227）而不是彩色。
///
/// 已知差异：上游在 macOS 上侧栏取 `transparent_black()` 让系统 vibrancy 透出来。
/// 我们这一版**不做真 vibrancy**（需要给 crates/macos 加 NSVisualEffectView 绑定），
/// 所以侧栏取与 canvas 同色——观感是「整块平面 + 一条发丝分隔线」，比上游更平，
/// 这是本原型的已知偏差，不是取色错误。
fn waku_light_palette() -> Palette {
    // 上游的 border 是 hsla(220, 0.10, 0.12, 0.08) / (…, 0.15)
    let border = hsla(WAKU_HUE_NEUTRAL, 0.10, 0.12, 0.08);
    let border_strong = hsla(WAKU_HUE_NEUTRAL, 0.10, 0.12, 0.15);
    let canvas = rgb(0xF6F5F6);
    let raised = rgb(0xECECEC);
    let inset = rgb(0xE6E6E6);
    Palette {
        background: canvas,
        foreground: rgb(0x242424),
        muted: raised,
        muted_foreground: rgb(0x666666),
        border,
        sidebar: canvas,
        sidebar_foreground: rgb(0x242424),
        sidebar_border: border,
        popover: canvas,
        secondary: raised,
        secondary_hover: inset,
        secondary_active: rgb(0xDEDEDE),
        // 控件边框：比表格分隔线明显一档（分隔线可以极浅，控件边框不行）
        input: hsla(WAKU_HUE_NEUTRAL, 0.10, 0.12, 0.25),
        list: canvas,
        list_even: raised,
        list_head: raised,
        table: canvas,
        table_even: raised,
        table_head: raised,
        table_row_border: border_strong,
        title_bar: canvas,
        overlay: hsla(WAKU_HUE_NEUTRAL, 0.10, 0.10, 0.30),
        window_border: border_strong,
        group_box: raised,
        group_box_foreground: rgb(0x242424),
        description_list_label: raised,
        description_list_label_foreground: rgb(0x242424),
        // 单色 CTA（上游的 inverse / on_inverse），不是彩色主按钮
        primary: rgb(0x202227),
        primary_hover: rgb(0x2B2D33),
        primary_active: rgb(0x35383F),
        primary_foreground: rgb(0xF8F8F9),
        progress_bar: rgb(0xC85F44),
        // brand：珊瑚，只用于品牌标记与链接
        accent: rgb(0xC85F44),
        accent_foreground: rgb(0xF8F8F9),
        ring: hsla(WAKU_HUE_SELECTION, 1.0, 0.50, 1.0),
        // 上游侧栏的选中/悬停复用同一个 sidebar_item_background（中性层），
        // 不染色——这是 waku「颜色只用于表意」最直观的一处体现。
        sidebar_accent: raised,
        sidebar_accent_foreground: rgb(0x242424),
        // list_active 受库强制 alpha ≤ 0.2，源色必须很淡才不至于「选中态消失」
        // （见 selection_tint_survives_library_alpha_clamp），故单独取淡蓝。
        list_active: hsl(WAKU_HUE_SELECTION, 0.60, 0.91),
        list_active_border: hsla(WAKU_HUE_SELECTION, 1.0, 0.50, 1.0),
        table_active: hsl(WAKU_HUE_SELECTION, 0.60, 0.91),
        table_active_border: hsla(WAKU_HUE_SELECTION, 1.0, 0.50, 1.0),
        // 上游原值：hsla(211, 1.0, 0.50, 0.35)
        selection: hsla(WAKU_HUE_SELECTION, 1.0, 0.50, 0.35),
        link: rgb(0xC85F44),
        link_hover: rgb(0xB4523A),
        link_active: rgb(0xA04731),
        drag_border: hsla(WAKU_HUE_SELECTION, 1.0, 0.50, 1.0),
        drop_target: hsla(WAKU_HUE_SELECTION, 1.0, 0.50, 0.22),
        // 行 hover：上游说的「6% 中性层」
        row_hover: hsla(WAKU_HUE_NEUTRAL, 0.03, 0.30, 0.06),
        // status：上游原值 warning #A66B20 / success #2F8F52 / danger #C64A42；
        // info 上游没有，取它的选中蓝同系以保持一致。
        danger: rgb(0xC64A42),
        danger_hover: rgb(0xB04039),
        danger_active: rgb(0x9A3730),
        danger_foreground: rgb(0xF8F8F9),
        warning: rgb(0xA66B20),
        warning_hover: rgb(0x935E1C),
        warning_active: rgb(0x805118),
        warning_foreground: rgb(0xF8F8F9),
        success: rgb(0x2F8F52),
        success_hover: rgb(0x297D48),
        success_active: rgb(0x236B3E),
        success_foreground: rgb(0xF8F8F9),
        info: rgb(0x2A7FBF),
        info_hover: rgb(0x256FA7),
        info_active: rgb(0x205F8F),
        info_foreground: rgb(0xF8F8F9),
    }
}

/// waku 风格·暗色。取自上游 `dark()`（canvas #1A1A1A / raised #232323 /
/// inset #151515 / text #E2E2E2 / accent #E2795B / selection
/// `hsla(211, 1.0, 0.50, 0.55)`）。
fn waku_dark_palette() -> Palette {
    let border = hsla(WAKU_HUE_NEUTRAL, 0.10, 0.90, 0.07);
    let border_strong = hsla(WAKU_HUE_NEUTRAL, 0.10, 0.90, 0.14);
    let canvas = rgb(0x1A1A1A);
    let raised = rgb(0x232323);
    let inset = rgb(0x151515);
    Palette {
        background: canvas,
        foreground: rgb(0xE2E2E2),
        muted: raised,
        muted_foreground: rgb(0xA3A3A3),
        border,
        sidebar: canvas,
        sidebar_foreground: rgb(0xE2E2E2),
        sidebar_border: border,
        popover: raised,
        secondary: raised,
        secondary_hover: rgb(0x2C2C2C),
        secondary_active: rgb(0x353535),
        input: hsla(WAKU_HUE_NEUTRAL, 0.10, 0.90, 0.30),
        list: canvas,
        list_even: raised,
        list_head: inset,
        table: canvas,
        table_even: raised,
        table_head: inset,
        table_row_border: border_strong,
        title_bar: canvas,
        overlay: hsla(WAKU_HUE_NEUTRAL, 0.10, 0.01, 0.55),
        window_border: border_strong,
        group_box: raised,
        group_box_foreground: rgb(0xE2E2E2),
        description_list_label: raised,
        description_list_label_foreground: rgb(0xE2E2E2),
        // 暗色下单色 CTA 反过来：浅色底 + 深色字（上游 inverse / on_inverse）
        primary: rgb(0xE7E9EC),
        primary_hover: rgb(0xD8DADE),
        primary_active: rgb(0xC9CBD0),
        primary_foreground: rgb(0x17181C),
        progress_bar: rgb(0xE2795B),
        accent: rgb(0xE2795B),
        accent_foreground: rgb(0x17181C),
        ring: hsla(WAKU_HUE_SELECTION, 1.0, 0.50, 1.0),
        sidebar_accent: raised,
        sidebar_accent_foreground: rgb(0xE2E2E2),
        list_active: hsl(WAKU_HUE_SELECTION, 0.30, 0.24),
        list_active_border: hsla(WAKU_HUE_SELECTION, 1.0, 0.50, 1.0),
        table_active: hsl(WAKU_HUE_SELECTION, 0.30, 0.24),
        table_active_border: hsla(WAKU_HUE_SELECTION, 1.0, 0.50, 1.0),
        // 上游原值：hsla(211, 1.0, 0.50, 0.55)
        selection: hsla(WAKU_HUE_SELECTION, 1.0, 0.50, 0.55),
        link: rgb(0xE2795B),
        link_hover: rgb(0xE88D72),
        link_active: rgb(0xEEA189),
        drag_border: hsla(WAKU_HUE_SELECTION, 1.0, 0.50, 1.0),
        drop_target: hsla(WAKU_HUE_SELECTION, 1.0, 0.50, 0.25),
        row_hover: hsla(WAKU_HUE_NEUTRAL, 0.03, 0.90, 0.06),
        // status：上游原值 warning #E0B36A / success #62C987 / danger #E2726A
        danger: rgb(0xE2726A),
        danger_hover: rgb(0xE8867F),
        danger_active: rgb(0xEE9A94),
        danger_foreground: rgb(0x17181C),
        warning: rgb(0xE0B36A),
        warning_hover: rgb(0xE6C183),
        warning_active: rgb(0xECCF9C),
        warning_foreground: rgb(0x17181C),
        success: rgb(0x62C987),
        success_hover: rgb(0x79D39A),
        success_active: rgb(0x90DDAD),
        success_foreground: rgb(0x17181C),
        info: rgb(0x5AA9E0),
        info_hover: rgb(0x73B7E6),
        info_active: rgb(0x8CC5EC),
        info_foreground: rgb(0x17181C),
    }
}

/// OSS Browser（阿里官方客户端）风格 · 亮色。
///
/// 取值**逐项来自该应用自己的 CSS**（`/Applications/oss-browser2.app` →
/// `.webpack/renderer/main.css`）：它的 `:root` 里有 `--oss-primary-color: #0064c8`
/// 等自定义属性，表头/行高/文字色则在 `.ant-table-*` 规则里。要点：
///
/// - 主色是**深蓝 `#0064c8`**（不是 Linear 的靛蓝、也不是 waku 的珊瑚）；
/// - 选中是**淡蓝**：行选中 `#eff3f8`、侧栏/菜单选中 `#e6f7ff`；
/// - hover 是**纯中性灰 `#f2f2f2`**（无色相）；边框 `#f0f0f0` 极浅；
/// - 表头底色 `#fafafc`、文字 `#1f2024`，次要文字 `#898989`；
/// - 状态色是经典饱和三色：成功 `rgb(80,187,53)`、错误 `rgb(250,73,74)`、
///   警告 `rgb(246,164,52)`；它没有 info，用它的链接蓝 `#1677ff` 顶。
///
/// `selection` 不是直接抄 `#eff3f8`：gpui-component 会把 `selection` 的 alpha
/// 强压到 ≤0.3，源色太淡会渲染到看不见（见 `selection_tint_survives_library_alpha_clamp`）。
/// 这里取 `#b8cee6` 作**源色**，0.3 压完后叠在白底上恰好约等于它的 `#eff3f8`。
fn oss_browser_light_palette() -> Palette {
    let border = rgb(0xF0F0F0);
    Palette {
        background: rgb(0xFFFFFF),
        foreground: rgb(0x1F2024),
        muted: rgb(0xFAFAFC),
        muted_foreground: rgb(0x898989),
        border,
        sidebar: rgb(0xFAFAFC),
        sidebar_foreground: rgb(0x333333),
        sidebar_border: border,
        popover: rgb(0xFFFFFF),
        secondary: rgb(0xFAFAFC),
        secondary_hover: rgb(0xF2F2F2),
        secondary_active: rgb(0xEFEFEF),
        // Ant 的控件边框是 `@border-color-base` = #d9d9d9——比表格分隔线 #f0f0f0
        // 明显更深；两者混用会让复选框「没有边框」。
        input: rgb(0xD9D9D9),
        list: rgb(0xFFFFFF),
        list_even: rgb(0xFAFAFC),
        list_head: rgb(0xFAFAFC),
        table: rgb(0xFFFFFF),
        table_even: rgb(0xFAFAFC),
        table_head: rgb(0xFAFAFC),
        // 它的表格分隔线就是实色 #f0f0f0（Ant 式），不另加透明度
        table_row_border: border,
        title_bar: rgb(0xFFFFFF),
        // Ant 的模态遮罩是 rgba(0,0,0,0.45)
        overlay: hsla(0., 0., 0., 0.45),
        window_border: border,
        group_box: rgb(0xFAFAFC),
        group_box_foreground: rgb(0x1F2024),
        description_list_label: rgb(0xFAFAFC),
        description_list_label_foreground: rgb(0x1F2024),
        primary: rgb(0x0064C8),
        primary_hover: rgb(0x0170CB),
        primary_active: rgb(0x0053A6),
        primary_foreground: rgb(0xFFFFFF),
        progress_bar: rgb(0x0064C8),
        // accent / accent_foreground 在本仓库当**图标色**用（非文本类型的文件图标、
        // 文件夹图标），所以取它的主色蓝——不能取淡蓝，否则图标看不见。
        accent: rgb(0x0064C8),
        accent_foreground: rgb(0x0064C8),
        ring: rgb(0x1677FF),
        sidebar_accent: rgb(0xE6F7FF),
        sidebar_accent_foreground: rgb(0x0064C8),
        list_active: rgb(0xE6F7FF),
        list_active_border: rgb(0x1677FF),
        table_active: rgb(0xE6F7FF),
        table_active_border: rgb(0x1677FF),
        selection: rgb(0xB8CEE6),
        link: rgb(0x1677FF),
        link_hover: rgb(0x69B1FF),
        link_active: rgb(0x0958D9),
        drag_border: rgb(0x1677FF),
        drop_target: hsla(215., 1.0, 0.53, 0.22),
        row_hover: rgb(0xF2F2F2),
        // 状态色：基色取自上游 CSS，**hover/active 在同色相上推亮度**得到，
        // 不直接抄它的色阶——上游那套色阶越亮色相越漂（`#50BB35` 108° 与
        // `#73D13D` 98° 差 10°，同一族会出现两种绿）。纪律测试就是抓这个的。
        danger: rgb(0xFA494A),
        danger_hover: hsl(0., 0.945, 0.70),
        danger_active: hsl(0., 0.945, 0.56),
        danger_foreground: rgb(0xFFFFFF),
        warning: rgb(0xF6A434),
        warning_hover: hsl(34.6, 0.915, 0.65),
        warning_active: hsl(34.6, 0.915, 0.52),
        warning_foreground: rgb(0xFFFFFF),
        success: rgb(0x50BB35),
        success_hover: hsl(108., 0.558, 0.53),
        success_active: hsl(108., 0.558, 0.41),
        success_foreground: rgb(0xFFFFFF),
        info: rgb(0x1677FF),
        info_hover: hsl(215., 1.0, 0.60),
        info_active: hsl(215., 1.0, 0.47),
        info_foreground: rgb(0xFFFFFF),
    }
}

/// OSS Browser 风格 · 深色。
///
/// 该应用的 CSS **只有亮色**（没有任何深色覆盖），所以这一套取 **Ant Design 官方
/// 深色主题**的取值（body `#141414`、容器 `#1f1f1f`、边框 `#303030`、主色
/// `#1668dc`、正文 `rgba(255,255,255,0.85)`），而不是我凭空配的——保持同一套
/// 设计语言，来源可查。
fn oss_browser_dark_palette() -> Palette {
    let border = rgb(0x303030);
    Palette {
        background: rgb(0x141414),
        foreground: rgb(0xD9D9D9),
        muted: rgb(0x1F1F1F),
        muted_foreground: rgb(0x737373),
        border,
        sidebar: rgb(0x1F1F1F),
        sidebar_foreground: rgb(0xD9D9D9),
        sidebar_border: border,
        popover: rgb(0x262626),
        secondary: rgb(0x1F1F1F),
        secondary_hover: rgb(0x262626),
        secondary_active: rgb(0x303030),
        input: rgb(0x424242),
        list: rgb(0x141414),
        list_even: rgb(0x1A1A1A),
        list_head: rgb(0x1F1F1F),
        table: rgb(0x141414),
        table_even: rgb(0x1A1A1A),
        table_head: rgb(0x1F1F1F),
        table_row_border: border,
        title_bar: rgb(0x1F1F1F),
        overlay: hsla(0., 0., 0., 0.55),
        window_border: border,
        group_box: rgb(0x1F1F1F),
        group_box_foreground: rgb(0xD9D9D9),
        description_list_label: rgb(0x1F1F1F),
        description_list_label_foreground: rgb(0xD9D9D9),
        primary: rgb(0x1668DC),
        primary_hover: rgb(0x2B7BE0),
        primary_active: rgb(0x0F5BC4),
        primary_foreground: rgb(0xFFFFFF),
        progress_bar: rgb(0x1668DC),
        accent: rgb(0x1668DC),
        accent_foreground: rgb(0x1668DC),
        ring: rgb(0x1677FF),
        sidebar_accent: rgb(0x111D2C),
        sidebar_accent_foreground: rgb(0x1668DC),
        list_active: rgb(0x111D2C),
        list_active_border: rgb(0x1677FF),
        table_active: rgb(0x111D2C),
        table_active_border: rgb(0x1677FF),
        selection: rgb(0x14467F),
        link: rgb(0x1677FF),
        link_hover: rgb(0x69B1FF),
        link_active: rgb(0x0958D9),
        drag_border: rgb(0x1677FF),
        drop_target: hsla(215., 1.0, 0.53, 0.25),
        row_hover: rgb(0x262626),
        // 同亮色套：hover/active 在同色相上推亮度派生
        danger: rgb(0xFF7875),
        danger_hover: hsl(1.3, 1.0, 0.79),
        danger_active: hsl(1.3, 1.0, 0.63),
        danger_foreground: rgb(0x141414),
        warning: rgb(0xFFC069),
        warning_hover: hsl(34.8, 1.0, 0.77),
        warning_active: hsl(34.8, 1.0, 0.63),
        warning_foreground: rgb(0x141414),
        success: rgb(0x73D13D),
        success_hover: hsl(98.1, 0.617, 0.60),
        success_active: hsl(98.1, 0.617, 0.46),
        success_foreground: rgb(0x141414),
        info: rgb(0x4096FF),
        info_hover: hsl(213., 1.0, 0.70),
        info_active: hsl(213., 1.0, 0.55),
        info_foreground: rgb(0x141414),
    }
}

/// 亮色套：白底冷灰面 + 低饱和靛蓝。
fn light_palette(style: ThemeStyle) -> Palette {
    match style {
        ThemeStyle::Waku => return waku_light_palette(),
        ThemeStyle::OssBrowser => return oss_browser_light_palette(),
        ThemeStyle::Linear => {}
    }
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
        // `input` 是**控件边框**色（复选框未选中态与输入框边框都用它，见下面的测试），
        // 取比 border 略深一档，保证在浅底上看得见。
        input: n(0.12, 0.86),
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
        // status 族：亮色实底 + 白字，明度对齐 primary（0.55）以便同一排按钮等高观感。
        danger: hsl(HUE_DANGER, 0.62, 0.50),
        danger_hover: hsl(HUE_DANGER, 0.62, 0.44),
        danger_active: hsl(HUE_DANGER, 0.62, 0.38),
        danger_foreground: hsl(0., 0., 1.0),
        warning: hsl(HUE_WARNING, 0.72, 0.42),
        warning_hover: hsl(HUE_WARNING, 0.72, 0.36),
        warning_active: hsl(HUE_WARNING, 0.72, 0.31),
        warning_foreground: hsl(0., 0., 1.0),
        success: hsl(HUE_SUCCESS, 0.52, 0.38),
        success_hover: hsl(HUE_SUCCESS, 0.52, 0.33),
        success_active: hsl(HUE_SUCCESS, 0.52, 0.28),
        success_foreground: hsl(0., 0., 1.0),
        info: hsl(HUE_INFO, 0.60, 0.44),
        info_hover: hsl(HUE_INFO, 0.60, 0.38),
        info_active: hsl(HUE_INFO, 0.60, 0.33),
        info_foreground: hsl(0., 0., 1.0),
    }
}

/// 暗色套：深冷灰底 + 同系靛蓝提亮、压饱和。
fn dark_palette(style: ThemeStyle) -> Palette {
    match style {
        ThemeStyle::Waku => return waku_dark_palette(),
        ThemeStyle::OssBrowser => return oss_browser_dark_palette(),
        ThemeStyle::Linear => {}
    }
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
        input: n(0.08, 0.30),
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
        // status 族：暗色同样用「实底 + 白字」，明度对齐暗色 primary（0.58），
        // 只把饱和度略压（暗底上高饱和会发荧光）。
        danger: hsl(HUE_DANGER, 0.55, 0.56),
        danger_hover: hsl(HUE_DANGER, 0.55, 0.62),
        danger_active: hsl(HUE_DANGER, 0.55, 0.68),
        danger_foreground: hsl(0., 0., 1.0),
        warning: hsl(HUE_WARNING, 0.62, 0.52),
        warning_hover: hsl(HUE_WARNING, 0.62, 0.58),
        warning_active: hsl(HUE_WARNING, 0.62, 0.64),
        warning_foreground: hsl(0., 0., 1.0),
        success: hsl(HUE_SUCCESS, 0.45, 0.48),
        success_hover: hsl(HUE_SUCCESS, 0.45, 0.54),
        success_active: hsl(HUE_SUCCESS, 0.45, 0.60),
        success_foreground: hsl(0., 0., 1.0),
        info: hsl(HUE_INFO, 0.52, 0.54),
        info_hover: hsl(HUE_INFO, 0.52, 0.60),
        info_active: hsl(HUE_INFO, 0.52, 0.66),
        info_foreground: hsl(0., 0., 1.0),
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
    let style = prefs.theme_style;
    vec![
        config(
            ThemeMode::Light,
            "CloudStorage Light",
            light_palette(style),
            prefs,
        ),
        config(
            ThemeMode::Dark,
            "CloudStorage Dark",
            dark_palette(style),
            prefs,
        ),
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
        // 小圆角：库组件（Button/Input/List/Table/Dialog…）经 Theme.radius 统一继承，
        // 视图层自绘卡片用 radius_lg 对齐。取值与 tokens.rs 的 radius()/radius_lg()
        // 必须一致（有测试钉住），参照实现是 Ant 系的小圆角（2 / 4）。
        cfg.radius = Some(2);
        cfg.radius_lg = Some(4);
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
        // ⚠️ gpui-component 的 apply_config 会强制压这三个字段的 alpha
        // （schema.rs 末尾）：list_active / table_active ≤ 0.2，selection ≤ 0.3。
        // 这里写的是不透明色，实际渲染是它们按上述 alpha 叠在底色上——
        // list_active 源色 l=0.925 太淡，压到 0.2 后叠白底只剩约 2% 差异，
        // 选中态肉眼不可见（实测 249,249,252 vs 白 255,255,255）。
        // 所以**列表选中态一律用 `selection`**（源色 l=0.80，压到 0.3 约
        // (229,232,245)，与侧栏不透明的 `sidebar_accent` 观感一致），
        // 不要用 list_active / table_active 做选中底色。
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
        // status：唯一允许外来色相的一族，用来承载真实反馈。
        c.danger = Some(hex(p.danger));
        c.danger_hover = Some(hex(p.danger_hover));
        c.danger_active = Some(hex(p.danger_active));
        c.danger_foreground = Some(hex(p.danger_foreground));
        c.warning = Some(hex(p.warning));
        c.warning_hover = Some(hex(p.warning_hover));
        c.warning_active = Some(hex(p.warning_active));
        c.warning_foreground = Some(hex(p.warning_foreground));
        c.success = Some(hex(p.success));
        c.success_hover = Some(hex(p.success_hover));
        c.success_active = Some(hex(p.success_active));
        c.success_foreground = Some(hex(p.success_foreground));
        c.info = Some(hex(p.info));
        c.info_hover = Some(hex(p.info_hover));
        c.info_active = Some(hex(p.info_active));
        c.info_foreground = Some(hex(p.info_foreground));
        cfg
    }
    build(mode, name, p, prefs)
}

/// 图片描边：`img()` 用的 1px 内描边取色。
///
/// 亮色纯黑 10%、暗色纯白 10%——**刻意不参与上面的冷灰中性色族（hue 225）**。
/// 带色相的描边会吸附图片下方的底色，在图片边缘看起来像脏边；它也不能取
/// accent / primary，描边是中性的边缘分隔线，不是主题元素。这两条都容易在
/// 「统一色族」时被顺手改掉，所以在此写明。
///
/// 之所以是模块级函数而不是 `Palette` 字段：`ThemeConfigColors` 是
/// gpui-component 的固定字段集，没有图片描边这一位，无法经 `ThemeConfig`
/// 下发；放在这里仍然是「颜色只在 theme.rs 定义」。
pub fn image_outline(mode: ThemeMode) -> Hsla {
    if mode.is_dark() {
        gpui::hsla(0., 0., 1., 0.10)
    } else {
        gpui::hsla(0., 0., 0., 0.10)
    }
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

    /// 四种组合：两套风格 × 亮/暗。族纪律必须对**全部**组合成立，
    /// 否则新加的风格可以偷偷破坏纪律而测试全绿。
    fn all_palettes() -> Vec<(String, Palette)> {
        let mut out = Vec::new();
        // 必须列出**全部**风格：漏一套 = 那套可以偷偷破坏族纪律而测试全绿
        for (style_name, style) in [
            ("linear", ThemeStyle::Linear),
            ("waku", ThemeStyle::Waku),
            ("ossbrowser", ThemeStyle::OssBrowser),
        ] {
            out.push((format!("{style_name}/light"), light_palette(style)));
            out.push((format!("{style_name}/dark"), dark_palette(style)));
        }
        out
    }

    impl Palette {
        /// 中性族 = surface + border + text + 纯白前景色（`*_foreground` 里
        /// 只有 status/primary 用纯白；accent 系前景是靛蓝，归 brand 族）。
        fn neutral_family(&self) -> Vec<(&'static str, [f32; 4])> {
            vec![
                ("background", self.background),
                ("foreground", self.foreground),
                ("muted", self.muted),
                ("muted_foreground", self.muted_foreground),
                ("border", self.border),
                ("sidebar", self.sidebar),
                ("sidebar_foreground", self.sidebar_foreground),
                ("sidebar_border", self.sidebar_border),
                ("popover", self.popover),
                ("secondary", self.secondary),
                ("secondary_hover", self.secondary_hover),
                ("secondary_active", self.secondary_active),
                ("input", self.input),
                ("list", self.list),
                ("list_even", self.list_even),
                ("list_head", self.list_head),
                ("table", self.table),
                ("table_even", self.table_even),
                ("table_head", self.table_head),
                ("table_row_border", self.table_row_border),
                ("title_bar", self.title_bar),
                ("overlay", self.overlay),
                ("window_border", self.window_border),
                ("group_box", self.group_box),
                ("group_box_foreground", self.group_box_foreground),
                ("description_list_label", self.description_list_label),
                (
                    "description_list_label_foreground",
                    self.description_list_label_foreground,
                ),
                ("row_hover", self.row_hover),
                ("primary_foreground", self.primary_foreground),
                ("danger_foreground", self.danger_foreground),
                ("warning_foreground", self.warning_foreground),
                ("success_foreground", self.success_foreground),
                ("info_foreground", self.info_foreground),
            ]
        }

        /// brand 族 = primary + selection/accent/link 及它们的靛蓝前景色。
        fn brand_family(&self) -> Vec<(&'static str, [f32; 4])> {
            vec![
                ("primary", self.primary),
                ("primary_hover", self.primary_hover),
                ("primary_active", self.primary_active),
                ("progress_bar", self.progress_bar),
                ("accent", self.accent),
                ("accent_foreground", self.accent_foreground),
                ("ring", self.ring),
                ("sidebar_accent", self.sidebar_accent),
                ("sidebar_accent_foreground", self.sidebar_accent_foreground),
                ("list_active", self.list_active),
                ("list_active_border", self.list_active_border),
                ("table_active", self.table_active),
                ("table_active_border", self.table_active_border),
                ("selection", self.selection),
                ("link", self.link),
                ("link_hover", self.link_hover),
                ("link_active", self.link_active),
                ("drag_border", self.drag_border),
                ("drop_target", self.drop_target),
            ]
        }

        /// status 族 = 实底（前景色是纯白，归中性族）。
        fn status_family(&self) -> Vec<(&'static str, [f32; 4])> {
            vec![
                ("danger", self.danger),
                ("danger_hover", self.danger_hover),
                ("danger_active", self.danger_active),
                ("warning", self.warning),
                ("warning_hover", self.warning_hover),
                ("warning_active", self.warning_active),
                ("success", self.success),
                ("success_hover", self.success_hover),
                ("success_active", self.success_active),
                ("info", self.info),
                ("info_hover", self.info_hover),
                ("info_active", self.info_active),
            ]
        }
    }

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
        let [h, s, l, _] = light_palette(ThemeStyle::Linear).primary;
        assert!(s <= 0.5, "primary 饱和度 {s} 应为低饱和");
        assert!((l - 0.55).abs() < 1e-6);
        assert!((h - HUE_DEG / 360.).abs() < 1e-6);
    }

    #[test]
    fn dark_primary_is_low_saturation_indigo() {
        let [h, s, _, _] = dark_palette(ThemeStyle::Linear).primary;
        assert!(s <= 0.5, "primary 饱和度 {s} 应为低饱和");
        assert!((h - HUE_DEG / 360.).abs() < 1e-6);
    }

    #[test]
    fn selection_differs_from_primary() {
        // selection ≠ primary：选中态不能复用主 CTA 色
        for (name, p) in all_palettes() {
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

    /// 选中态能否看见，取决于「源色明度 × 库强制的 alpha 上限」：
    /// gpui-component 的 `apply_config` 把 list_active / table_active 的 alpha
    /// 压到 ≤0.2、selection 压到 ≤0.3（schema.rs 末尾），叠加后与底色的明度差
    /// 小于约 5% 就肉眼不可见。
    ///
    /// 这正是踩过的坑：`list_active` 源色 l=0.925，在 0.2 档只差 1.5%
    /// （实测渲染 249,249,252 对白底 255,255,255），对象列表与命令面板的
    /// 选中态形同不存在。现在选中底色统一用 `selection`（0.3 档 → 约 6%，
    /// 实测 229,232,245），与侧栏不透明的 `sidebar_accent` 观感一致。
    #[test]
    fn selection_tint_survives_library_alpha_clamp() {
        const LIST_ACTIVE_CLAMP: f32 = 0.2;
        const SELECTION_CLAMP: f32 = 0.3;
        const MIN_VISIBLE_DELTA: f32 = 0.05;

        for (name, p) in all_palettes() {
            let base_l = p.background[2];
            let selection_delta = SELECTION_CLAMP * (p.selection[2] - base_l).abs();
            assert!(
                selection_delta >= MIN_VISIBLE_DELTA,
                "{name}: selection 叠加后明度差仅 {selection_delta:.3}，选中态会看不见"
            );

            // 反例即记录本身：list_active 在 0.2 档下确实不可见，不能拿来做
            // 选中底色。若本断言失败，说明库的 clamp 或源色变了——此时应
            // 重新评估选中色，而不是把它改回 list_active。
            let list_active_delta = LIST_ACTIVE_CLAMP * (p.list_active[2] - base_l).abs();
            assert!(
                list_active_delta < MIN_VISIBLE_DELTA,
                "{name}: list_active 变得可见了（{list_active_delta:.3}）——库 clamp 或源色已变，需重新判断选中色"
            );
        }
    }

    #[test]
    fn row_hover_is_neutral_not_indigo() {
        // hover 是中性位置反馈（hue 225），选中才是 indigo 染色（hue 233）——
        // 二者肉眼可辨（Linear 语义：hover ≠ selection）
        for (name, p) in all_palettes() {
            assert!(
                is_neutral(p.row_hover),
                "{name}: row_hover 必须是中性层，不能是染色（{:?}）",
                p.row_hover
            );
            assert_ne!(p.row_hover, p.list_active, "{name}: hover ≠ 选中色");
        }
    }

    #[test]
    fn control_border_is_visible_against_the_input_background() {
        // `theme.input` 是**控件边框**色，不是输入框底色：
        // gpui-component 用 `cx.theme().input` 画复选框的未选中边框
        // （checkbox.rs:231）与输入框边框（input.rs 的 `border_color`），
        // 而输入框**底色**另走 `input_background()`（亮色=background，
        // 暗色=input 混 30% 透明）。
        //
        // 我一度把它当底色填成纯白，结果复选框「没有边框」——这条测试把
        // 「边框必须与它所处的底明显不同」钉住，不靠肉眼发现。
        for (name, p) in all_palettes() {
            let [_, _, l, a] = p.input;
            let bg_l = p.background[2];
            let dark = bg_l < 0.5;
            let border_l = if dark {
                // 暗色的 input_background() 是 input 与透明 30% 混合，再叠在 background 上
                0.3 * l + 0.7 * bg_l
            } else {
                a * l + (1. - a) * bg_l
            };
            assert!(
                (border_l - bg_l).abs() >= 0.05,
                "{name}: 控件边框与输入框底色太接近（{border_l:.3} vs {bg_l:.3}）——\
                 复选框会看起来「没有边框」"
            );
        }
    }

    #[test]
    fn grays_and_colors_are_clearly_separated() {
        // 「颜色只用于表意」的可测量形式：中性族的最大彩度必须**明显**低于
        // status 族的最小彩度，界面才不会分不清「这是底」还是「这是反馈」。
        // 把它写成比值而不是审美形容词，回归时才知道自己破坏了什么。
        //
        // 说明：原先这里断言「所有中性字段 hue 都是 225」，但两套色板的中性彩度
        // 都低到（最大 0.029）色相在视觉上已不可辨——那条断言检查的是声明而非
        // 可见效果，加了 waku（220）之后更是只剩维护成本。真正要守的是灰与彩的
        // 边界，即下面这条。
        for (name, p) in all_palettes() {
            let max_neutral = p
                .neutral_family()
                .iter()
                .map(|(_, v)| chroma(*v))
                .fold(0., f32::max);
            let min_status = p
                .status_family()
                .iter()
                .map(|(_, v)| chroma(*v))
                .fold(f32::MAX, f32::min);
            assert!(
                max_neutral * 3. < min_status,
                "{name}: 中性族最大彩度 {max_neutral:.3} 与 status 族最小彩度 \
                 {min_status:.3} 太接近——灰底与反馈色会相互串味"
            );
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
            assert_eq!(
                theme.radius,
                Some(2),
                "小圆角（与 tokens.rs 的 radius() 同刻度）"
            );
            assert_eq!(theme.radius_lg, Some(4));
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
            theme_style: ThemeStyle::Linear,
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

    #[test]
    fn image_outline_is_neutral_black_or_white() {
        // 图片描边必须中性（饱和度为 0）且只有 10% 不透明度：带色相会吸附
        // 图片下方的底色，边缘看起来像脏边；太不透明则会压过图片本身。
        let light = image_outline(ThemeMode::Light);
        assert_eq!(light.s, 0., "亮色描边不得有饱和度");
        assert_eq!(
            light.l, 0.,
            "亮色描边必须是纯黑，不能用近黑灰（slate/zinc 一类）"
        );
        assert!((light.a - 0.10).abs() < 1e-6, "alpha 应为 0.10");

        let dark = image_outline(ThemeMode::Dark);
        assert_eq!(dark.s, 0., "暗色描边不得有饱和度");
        assert_eq!(dark.l, 1., "暗色描边必须是纯白");
        assert!((dark.a - 0.10).abs() < 1e-6, "alpha 应为 0.10");
    }

    // ---- 族纪律：把「颜色只用于表意」写成断言，而不是靠自觉 ----
    //
    // 这几条测试的作用不是验证某次改动，而是**挡住以后的顺手改动**：
    // 「统一色族」时最容易发生的事，就是把状态色摊到中性面上，或反过来
    // 给选中态塞一个更鲜艳的颜色。
    //
    // 断言写成**风格无关**的形式：中性判据是「色相落在冷灰带内且低饱和」，
    // 而不是硬编码某一个 hue——否则新加一套风格（比如 waku 的 220）就会
    // 被误判为违规。
    const NEUTRAL_BAND: (f32, f32) = (215., 235.);
    /// 绝对彩度（RGB 极差）上限：高于此值肉眼可辨为「有色」。
    const MAX_NEUTRAL_CHROMA: f32 = 0.06;
    /// 彩度低于此值即视为纯灰（覆盖 ±2/255 的取整噪声）。
    const PURE_GRAY_CHROMA: f32 = 0.008;
    /// 非中性字段的色相必须能对上某个「锚点」（具名彩色角色），误差窗口用来吸收
    /// 取整与「同一色相经亮度推导 hover/active」时的几度漂移。8° 仍能抓住混进来的
    /// 青绿/橙等其他色系。
    const ANCHOR_TOLERANCE_DEG: f32 = 8.;

    /// 绝对彩度 = ΔRGB。**不能拿 HSL 的 s 判「是不是灰」**：接近黑/白时 s 会
    /// 失真——`rgb(0xF6F5F6)` 只差 1/255 个通道，却能算出 s = 0.053 与色相 300°，
    /// 判据会被这种数学噪声误导（本文件的第一次实现就栽在这里）。
    fn chroma([_, s, l, _]: [f32; 4]) -> f32 {
        2. * s * l.min(1. - l)
    }

    fn is_pure_gray(v: [f32; 4]) -> bool {
        chroma(v) <= PURE_GRAY_CHROMA
    }

    fn is_neutral(value: [f32; 4]) -> bool {
        if is_pure_gray(value) {
            return true; // 纯灰：色相无意义
        }
        let hue = value[0] * 360.;
        chroma(value) <= MAX_NEUTRAL_CHROMA && (NEUTRAL_BAND.0..=NEUTRAL_BAND.1).contains(&hue)
    }

    /// 该色板的「宣示色相」：品牌 + 四个状态。任何带色相的非中性字段都必须
    /// 能对上其中之一——这样「多出来一个青绿」会被抓到，而不是靠人眼发现。
    fn anchor_hues(p: &Palette) -> Vec<(&'static str, f32)> {
        vec![
            ("accent", p.accent[0] * 360.),
            ("selection", p.selection[0] * 360.),
            // ring / link / sidebar_accent 也是具名彩色角色：OSS Browser 那套的蓝
            // 分好几个色相（主色 210、链接 215、淡蓝选中 199），只列 accent/selection
            // 会把自家角色误判成「没来由的颜色」
            ("ring", p.ring[0] * 360.),
            ("link", p.link[0] * 360.),
            ("sidebar_accent", p.sidebar_accent[0] * 360.),
            ("danger", p.danger[0] * 360.),
            ("warning", p.warning[0] * 360.),
            ("success", p.success[0] * 360.),
            ("info", p.info[0] * 360.),
        ]
    }

    fn all_fields(p: &Palette) -> Vec<(&'static str, [f32; 4])> {
        let mut v = p.neutral_family();
        v.extend(p.brand_family());
        v.extend(p.status_family());
        v
    }

    #[test]
    fn neutral_families_carry_no_hue() {
        // surface / border / text 是界面主体，只允许冷灰（色相落在 215–235 的
        // 低饱和带）或纯灰。任何外来色相都会让整片界面「花」起来，并且和
        // status 族抢「颜色 = 信息」的语义。
        for (mode, p) in all_palettes() {
            for (field, value) in p.neutral_family() {
                let [h, s, _, _] = value;
                assert!(
                    is_neutral(value),
                    "{mode}: 中性族字段 {field} 带了外来色相（h={:.0}, s={s}）——\
                     中性面只允许冷灰（{:.0}–{:.0}）或纯灰",
                    h * 360.,
                    NEUTRAL_BAND.0,
                    NEUTRAL_BAND.1
                );
            }
        }
    }

    #[test]
    fn every_colored_field_matches_a_declared_anchor() {
        // 全字段覆盖：每个带色相的字段都必须对上 accent / selection / 某个状态色。
        // 这条是「颜色只用于表意」最直接的机械化表达——放进一个没来由的颜色
        // 就会红，不需要任何人记得来审查色板。
        for (mode, p) in all_palettes() {
            let anchors = anchor_hues(&p);
            for (field, value) in all_fields(&p) {
                if is_neutral(value) {
                    continue;
                }
                let hue = value[0] * 360.;
                let matched = anchors.iter().any(|(_, a)| {
                    let raw = (hue - a).abs();
                    raw.min(360. - raw) <= ANCHOR_TOLERANCE_DEG
                });
                assert!(
                    matched,
                    "{mode}: 字段 {field} 的色相 {hue:.0}° 对不上任何宣示色相 —— \
                     带色相的颜色只能来自 accent/selection/状态色，不能随手新加"
                );
            }
        }
    }

    #[test]
    fn status_is_the_only_foreign_hue_family() {
        // status 必须真的带色（s ≥ 0.35）——灰掉的状态色等于把「出错」和普通
        // 文字混为一谈；且不得落在中性带里（那样就和界面主体分不开了）。
        for (mode, p) in all_palettes() {
            for (field, value) in p.status_family() {
                let [h, _, _, _] = value;
                assert!(
                    chroma(value) >= 0.25,
                    "{mode}: 状态色 {field} 彩度过低（chroma={:.3}），\
                     无法承载「这是反馈」的信息",
                    chroma(value)
                );
                assert!(
                    !is_neutral(value),
                    "{mode}: 状态色 {field} 看起来就是灰的（h={:.0}）——\
                     状态色必须一眼能和界面主体区分开",
                    h * 360.
                );
            }
        }
    }

    #[test]
    fn status_hues_are_mutually_distinguishable() {
        // 四个状态之间色相至少差 30°：靠颜色就能区分「出错 / 留意 / 成功 / 提示」，
        // 而不是要用户去读文字（无障碍要求：不能只靠颜色，但颜色也不能没用）。
        //
        // 阈值取 30 而不是 40：waku 的 danger(4) 与 warning(37) 只差 33°，
        // 这是上游取色本身的取舍（红与琥珀本就相邻），保留它以便对照真实观感。
        let hues = [
            ("danger", HUE_DANGER),
            ("warning", HUE_WARNING),
            ("success", HUE_SUCCESS),
            ("info", HUE_INFO),
        ];
        for (i, (a_name, a)) in hues.iter().enumerate() {
            for (b_name, b) in hues.iter().skip(i + 1) {
                let raw = (a - b).abs();
                let diff = raw.min(360. - raw);
                assert!(
                    diff >= 30.,
                    "{a_name} 与 {b_name} 色相仅差 {diff:.0}°，肉眼难以区分"
                );
            }
        }
    }
}

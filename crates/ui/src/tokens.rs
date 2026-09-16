//! 设计 token：界面字号、圆角与行距的唯一定义处。
//!
//! `text()` 把基准像素乘以用户设置的字号缩放（0.85–1.40），因此**所有**
//! 字号都必须经它取值——直接写 `text_size(px(13.))` 会在用户调大字号后
//! 与其它文字脱节。新增文字时从下面阶梯里选一档，不要引入第 9 个数字。
//!
//! 阶梯里也包含空态/预览的大图标尺寸（`icon_sm` / `icon_lg`）：图标同样要
//! 跟随字号缩放，走同一入口比另开一套缩放逻辑简单。

use std::sync::atomic::{AtomicU32, Ordering};

use gpui::{Pixels, px};

const DEFAULT_SCALE: u32 = 100;
static UI_FONT_SCALE_PERCENT: AtomicU32 = AtomicU32::new(DEFAULT_SCALE);

pub fn set_ui_font_scale(scale: f32) {
    let percent = (scale.clamp(0.85, 1.40) * 100.0).round() as u32;
    UI_FONT_SCALE_PERCENT.store(percent, Ordering::Relaxed);
}

pub fn ui_font_scale() -> f32 {
    UI_FONT_SCALE_PERCENT.load(Ordering::Relaxed) as f32 / 100.0
}

/// 按当前界面字号缩放取像素值。字号相关的一切（含列宽、内边距）都走这里，
/// 否则 1.0 以外的缩放档位下就会出现文字与容器错位。
pub fn text(base: f32) -> Pixels {
    px(base * ui_font_scale())
}

// ---- 字号阶梯（基准像素；实际渲染值 = 基准 × 字号缩放） ----

/// 分组标题、次要说明、键位提示。
pub const CAPTION: f32 = 11.;
/// 元数据：大小/时间列、状态条、面包屑。
pub const LABEL: f32 = 12.;
/// 正文、列表行文本、输入框。
pub const BODY: f32 = 13.;
/// 空态标题。
pub const HEADING: f32 = 15.;
/// 弹层标题。
pub const TITLE: f32 = 16.;
/// 关于弹层的应用名。
pub const DISPLAY: f32 = 18.;
/// 空态图标。
pub const ICON_SM: f32 = 28.;
/// 预览弹层大图标。
pub const ICON_LG: f32 = 42.;

/// 分组标题 / 次要说明 / 键位提示。
pub fn caption() -> Pixels {
    text(CAPTION)
}

/// 元数据（大小、时间、计数、面包屑）。
pub fn label() -> Pixels {
    text(LABEL)
}

/// 正文与列表行文本。
pub fn body() -> Pixels {
    text(BODY)
}

/// 空态标题。
pub fn heading() -> Pixels {
    text(HEADING)
}

/// 弹层标题。
pub fn title() -> Pixels {
    text(TITLE)
}

/// 关于弹层应用名。
pub fn display() -> Pixels {
    text(DISPLAY)
}

/// 空态图标。
pub fn icon_sm() -> Pixels {
    text(ICON_SM)
}

/// 预览弹层大图标。
pub fn icon_lg() -> Pixels {
    text(ICON_LG)
}

// ---- 几何 ----

/// 圆角：chip / 列表行 / 横幅等小元素。与 theme.rs 写入组件的
/// `Theme.radius` 同一刻度——不要再出现第三种值。
pub fn radius() -> Pixels {
    px(5.)
}

/// 圆角：弹层卡片（= theme.rs 的 `Theme.radius_lg`）。
pub fn radius_lg() -> Pixels {
    px(8.)
}

/// 圆角：应用图标容器（近似 macOS squircle，不参与上面两档）。
pub fn radius_icon() -> Pixels {
    px(14.)
}

/// 嵌套圆角：外圆角 − 内缩，即同心圆角规则（`outer = inner + padding`）。
///
/// 嵌套面**紧贴**时必须用它推导，不要直接复用 `radius()`：浮层卡片是
/// `radius_lg()`（8），里面的行 `mx_1`（4px）内缩，行圆角应为 4 而不是 5；
/// 大 1px 的内圆角会让行角看起来比卡片角「方」——这类「看着不对但说不出
/// 哪不对」多半就是同心圆角没算。内缩接近或超过外圆角时各层已属独立表面，
/// 各自取值即可，不必套这个算式。
///
/// 已知限制：`inset` 若取自 `mx_1` 一类 rem 刻度会随字号缩放，而
/// `radius_lg()` 是固定像素，140% 档位下会有约 1.6px 偏差。
pub fn radius_nested(inset: Pixels) -> Pixels {
    (radius_lg() - inset).max(px(0.))
}

/// 列表行纵向内边距。行高由内容撑开，不另设固定行高——字号缩放时
/// 固定行高会与文字错位。
pub fn row_pad_y() -> Pixels {
    px(6.)
}

/// 侧栏行纵向内边距：密度比内容区列表更紧（Finder 侧栏惯例）。
pub fn row_pad_y_sidebar() -> Pixels {
    px(5.)
}

/// 传输任务行纵向内边距（信息密度最高）。
pub fn row_pad_y_transfer() -> Pixels {
    px(4.)
}

/// 对象列表「大小」列宽（100% 字号下的设计值）。必须随字号缩放：
/// 固定像素在 140% 档位会被「最新修改时间」表头和时间串撑爆。
pub fn col_size_width() -> Pixels {
    text(96.)
}

/// 对象列表「最新修改时间」列宽。
pub fn col_time_width() -> Pixels {
    text(148.)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    /// 字号缩放是**进程级全局**状态（`UI_FONT_SCALE_PERCENT`），而 cargo 默认并行
    /// 跑测试。两个测试交错就会互相踩：曾实测到 `column_widths_follow_font_scale`
    /// 两次读取之间被 `ui_font_scale_clamps_bounds` 重置回 1.0，断言变成
    /// `148px == 207.2px`（偶发，满负载跑全量测试时更容易命中）。
    /// 全局状态必须串行访问，这里用互斥锁钉住。
    static SCALE_LOCK: Mutex<()> = Mutex::new(());

    /// 取锁并把缩放还原成默认值，避免测试之间互相污染。
    /// 锁中毒（持锁线程 panic）时取回内部值继续，不让一个失败级联成全红。
    fn scale_guard() -> MutexGuard<'static, ()> {
        let guard = SCALE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_ui_font_scale(1.0);
        guard
    }

    #[test]
    fn ui_font_scale_clamps_bounds() {
        let _guard = scale_guard();
        set_ui_font_scale(0.5);
        assert_eq!(ui_font_scale(), 0.85);
        set_ui_font_scale(2.0);
        assert_eq!(ui_font_scale(), 1.4);
        set_ui_font_scale(1.15);
        assert_eq!(ui_font_scale(), 1.15);
        set_ui_font_scale(1.0);
    }

    #[test]
    fn column_widths_follow_font_scale() {
        let _guard = scale_guard();
        // 列宽必须跟随字号：否则最大档位下列头/时间会被压爆。
        let base_size = col_size_width();
        let base_time = col_time_width();
        set_ui_font_scale(1.4);
        assert_eq!(col_size_width(), base_size * 1.4);
        assert_eq!(col_time_width(), base_time * 1.4);
        // 时间列必须容得下「最新修改时间」六个字 + 时间串
        assert!(col_time_width() > col_size_width());
        set_ui_font_scale(1.0);
    }

    #[test]
    fn radius_scale_matches_theme_scale() {
        // 与 theme.rs 的 Theme.radius / radius_lg 对齐（5 / 8），
        // 图标容器是唯一的例外档位。
        assert_eq!(radius(), px(5.));
        assert_eq!(radius_lg(), px(8.));
        assert!(radius_icon() > radius_lg());
    }

    #[test]
    fn nested_radius_is_concentric_with_card_radius() {
        // 同心圆角：外圆角 = 内圆角 + 内缩。浮层行是 mx_1（4px）内缩，
        // 所以行圆角必须是 radius_lg() - 4px，而不是直接复用 radius()。
        let row_inset = px(4.);
        assert_eq!(radius_nested(row_inset), px(4.));
        // 恒等式成立：内圆角 + 内缩 == 外圆角
        assert_eq!(radius_nested(row_inset) + row_inset, radius_lg());
        // 记录旧写法差多少：radius() 比同心值大 1px
        assert_eq!(radius() - radius_nested(row_inset), px(1.));
        // 内缩达到外圆角量级时不再是紧贴嵌套，取 0 而不是负数
        assert_eq!(radius_nested(px(16.)), px(0.));
    }

    #[test]
    fn font_ramp_is_strictly_ascending() {
        let ramp = [
            CAPTION, LABEL, BODY, HEADING, TITLE, DISPLAY, ICON_SM, ICON_LG,
        ];
        assert!(
            ramp.windows(2).all(|pair| pair[0] < pair[1]),
            "字号阶梯必须严格递增：{ramp:?}"
        );
    }
}

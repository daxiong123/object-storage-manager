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

// 尺度参照 OSS Browser（Ant Design 系）：它 CSS 里 12px 出现 14 次、是列表/表格/
// 标签的绝对主力，标题用 14px，应用名 20px。所以是把**正文档位压到 12**、标题压到 14，
// 而不是我们原先偏大的 13/15/16。
//
// 阶梯允许**相等**（12 同时用于元数据与正文、14 同时用于空态标题与弹层标题）——
// 参考设计本来就有并列档位，为此把「严格递增」的测试放宽成「非递减」。
/// 分组标题、键位提示。
pub const CAPTION: f32 = 11.;
/// 元数据：大小/时间列、状态条、面包屑，以及**列表/表格正文**。
pub const LABEL: f32 = 12.;
/// 正文、列表行文本、输入框（与 LABEL 同档，参照实现如此）。
pub const BODY: f32 = 12.;
/// 空态标题。
pub const HEADING: f32 = 14.;
/// 弹层标题（参照实现的 modal 标题就是 14px）。
pub const TITLE: f32 = 14.;
/// 关于弹层的应用名。
pub const DISPLAY: f32 = 20.;
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

/// 圆角：chip / 列表行 / 横幅等小元素。
///
/// 参照实现是 Ant Design v4 系（CSS 里用 `--ant-color-link` 这个 v4 命名），
/// 基础圆角 **2px**，整屏观感方正——截图里按钮与输入框的角接近直角。
/// 与 theme.rs 写入组件的 `Theme.radius` 同一刻度，不要再出现第三种值。
pub fn radius() -> Pixels {
    px(2.)
}

/// 圆角：弹层卡片（= theme.rs 的 `Theme.radius_lg`）。取 Ant 的 lg 档 4px。
pub fn radius_lg() -> Pixels {
    px(4.)
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

/// 对象列表行盒高度（不含上下内边距）。
///
/// 对象列表走虚拟列表（`gpui::uniform_list`），它**按第 0 行的高度给所有行
/// 排版**（`item_height * item_count`），所以行高必须是确定的，不能像侧栏
/// 那样由内容撑开——原先「行高由内容撑开，不另设固定行高」的约定只适用于
/// 非虚拟列表。
///
/// 取 24 是为了对上参照实现的**表格行高 36px**（其 CSS 把 `.ant-table-thead` 与行
/// 都固定成 36px）：24 + 上下内边距 12 = 36。也明显大于行内 `Input::small()` 的
/// 24px 高度，留量充足。
const ROW_BOX: f32 = 24.;

/// 对象列表行高 = 行盒 + 上下内边距。
///
/// 行必须显式用这个高度（`.h(row_height())`）**并且**同时设
/// `row_line_height()`，二者一起保证「行高 = 行盒 + 内边距 + 边框」恒等：
/// 只固定高度而不固定行盒，文字的行盒高度由字体度量决定，字号缩放后可能
/// 高于或低于行盒，表现为裁字或行间留缝。
pub fn row_height() -> Pixels {
    text(ROW_BOX) + row_pad_y() * 2
}

/// 行内文字的行盒高度（与 `row_height()` 同一基准，一起改）。
pub fn row_line_height() -> Pixels {
    text(ROW_BOX)
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

/// 对象列表行首**复选框**列宽（参照实现的表格第一列）。
pub fn col_check_width() -> Pixels {
    text(28.)
}

/// 对象列表行末**操作**列宽（参照实现的表格最后一列）。
///
/// 两个图标（下载 + 更多）+ 间距：参照实现这一列放 4 个图标，我们只做有实际动作的
/// 两个（收藏/解冻不做）。表头也用同一宽度占位，否则表头与数据行会整体错开。
pub fn col_action_width() -> Pixels {
    text(64.)
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
        assert_eq!(col_check_width(), text(28.));
        assert_eq!(col_action_width(), text(64.));
        set_ui_font_scale(1.0);
    }

    #[test]
    fn radius_scale_matches_theme_scale() {
        // 与 theme.rs 的 Theme.radius / radius_lg 对齐（2 / 4），
        // 图标容器是唯一的例外档位。
        assert_eq!(radius(), px(2.));
        assert_eq!(radius_lg(), px(4.));
        assert!(radius_icon() > radius_lg());
    }

    #[test]
    fn nested_radius_is_concentric_with_card_radius() {
        // 同心圆角：外圆角 = 内圆角 + 内缩。内缩到外圆角量级时不再是紧贴嵌套，
        // 取 0 而不是负数。
        assert_eq!(radius_nested(px(16.)), px(0.));
        assert_eq!(radius_nested(px(0.)), radius_lg());
        // 恒等式：内圆角 + 内缩 == 外圆角（在还能推导出正值的范围内）
        for inset in [1., 2., 3.] {
            let inset = px(inset);
            assert_eq!(radius_nested(inset) + inset, radius_lg());
        }
    }

    #[test]
    fn font_ramp_is_non_decreasing() {
        // 允许相等：参照设计里 12 同时用于元数据与正文、14 同时用于空态标题与弹层标题。
        // 仍然禁止「倒退」——档位必须随语义变重而单调不减。
        let ramp = [
            CAPTION, LABEL, BODY, HEADING, TITLE, DISPLAY, ICON_SM, ICON_LG,
        ];
        assert!(
            ramp.windows(2).all(|pair| pair[0] <= pair[1]),
            "字号阶梯不得倒退：{ramp:?}"
        );
    }

    #[test]
    fn row_height_fits_the_inline_rename_input() {
        let _guard = scale_guard();
        // 虚拟列表按第 0 行定高，所以行高必须能容下行内重命名的
        // `Input::small()`（gpui-component 按 h_6() 固定 24px，不随字号缩放）。
        // 这条不变量一旦破了，重命名时输入框会被定高裁掉——而且只在
        // 「点了重命名」时才看得见，最容易漏掉。
        const INPUT_SMALL_HEIGHT: f32 = 24.;
        for scale in [1.0_f32, 1.4] {
            set_ui_font_scale(scale);
            assert!(
                row_height() >= px(INPUT_SMALL_HEIGHT),
                "字号 {scale} 下行高 {:?} 容不下 24px 的 Input::small()",
                row_height()
            );
            // 行盒必须小于行高（否则内边距被吃掉），且行高必须跟随字号缩放
            assert!(row_line_height() < row_height());
        }
        set_ui_font_scale(1.0);
    }

    #[test]
    fn row_height_follows_font_scale() {
        let _guard = scale_guard();
        let base_line = row_line_height();
        let base = row_height();
        set_ui_font_scale(1.4);
        // 行盒随字号缩放，内边距是固定像素——所以总高度不是简单 ×1.4
        assert_eq!(row_line_height(), base_line * 1.4);
        assert_eq!(row_height(), row_line_height() + row_pad_y() * 2.);
        assert!(row_height() > base, "字号变大后行高必须变大");
        // 增量应恰为行盒的增量；f32 下用容差而不是精确相等
        assert!((row_height() - base - base_line * 0.4).abs() < px(0.01));
    }
}

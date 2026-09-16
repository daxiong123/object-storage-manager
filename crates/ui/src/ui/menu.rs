//! 锚定菜单（下拉 / 右键菜单）的共用卡片与条目形状。
//!
//! 对象右键菜单与 Titlebar「更多」菜单此前各自手写同一套卡片与行样式，
//! 真正的差异只有宽度之外的**禁用判定与图标有无**；相同的部分收在这里，
//! 免得改一处悬停底色要改两遍。

use gpui::{
    Div, ElementId, Hsla, InteractiveElement as _, Pixels, Stateful, Styled as _, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{Theme, h_flex};

use crate::tokens;
use crate::ui::overlay;

/// 菜单卡片宽度（两处菜单共用同一设计宽度）。
fn width() -> Pixels {
    tokens::text(154.)
}

/// 菜单卡片：浮层表面 + 固定宽度 + 纵向内边距，并 `occlude` 挡住下层鼠标事件
/// （否则点到菜单空白处会穿透到下面的列表）。
pub(crate) fn popup(theme: &Theme) -> Div {
    overlay::surface(theme).w(width()).py_2().occlude()
}

/// 菜单条目外壳：同心圆角 + 悬停底色 + 正文色（删除类条目由调用方给 `danger`）。
///
/// 调用方接 `.child(..)` / `.on_click(..)`；禁用条目传 `interactive = false`，
/// 这样既不接悬停也不该接点击（点击由调用方按同一条件跳过）。
pub(crate) fn item(
    theme: &Theme,
    id: impl Into<ElementId>,
    color: Hsla,
    interactive: bool,
) -> Stateful<Div> {
    h_flex()
        .id(id)
        .mx_1()
        .px_3()
        .py_2()
        .gap_2()
        // 同心圆角：卡片 radius_lg()=8 − mx_1 内缩 4 = 4
        .rounded(tokens::radius_nested(px(4.)))
        .text_size(tokens::body())
        .text_color(color)
        .when(interactive, |row| row.hover(|row| row.bg(theme.list_hover)))
}

/// 菜单分组之间的分隔线。
pub(crate) fn separator(theme: &Theme) -> Div {
    div().mx_2().my_1().h(px(1.)).bg(theme.border)
}

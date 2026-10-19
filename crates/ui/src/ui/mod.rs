//! 跨视图复用的 UI 基础件（对齐 waku 的 `src/ui/*`）。
//!
//! **只放基础件，不放业务视图**：账号 / 空间 / 对象 / 传输各自的面板在
//! `crate::workspace` 与各模态文件里，这里只收「两个以上视图重复、且能收成
//! 具名形状」的东西。
//!
//! 为什么是**具名形状**而不是通用抽象：一个泛化的 `row(..)` / `card(..)` 要塞进
//! 宽度、内边距、圆角、悬停、间距、有无图标等一堆参数，读起来并不比各视图显式写
//! 链式调用清楚。waku 的 `src/ui/mod.rs` 同样只提供 `icon_button` / `toggle_switch`
//! 这类具体形状，而不是通用行外壳。

pub(crate) mod menu;
pub(crate) mod motion;
pub(crate) mod overlay;

use gpui::{
    App, ClickEvent, ElementId, Entity, IntoElement, ParentElement as _, SharedString, Styled as _,
    Window, div,
};
use gpui_component::{
    Icon, IconName, Sizable as _, Size, Theme, button::Button, button::ButtonVariants as _, h_flex,
    input::Input, input::InputState,
};

/// 紧凑 ghost 图标按钮：图标 + 幽灵底 + Small 尺寸 + tooltip。
///
/// 工具栏、侧栏 44px 图标栏、各浮层标题栏的关闭按钮都走这一个形状——
/// 尺寸与底色散落各处时，同一排按钮会一个高一个矮。
pub(crate) fn icon_button(
    id: impl Into<ElementId>,
    icon: Icon,
    tooltip: impl Into<SharedString>,
) -> Button {
    Button::new(id)
        .icon(icon)
        .ghost()
        .with_size(Size::Small)
        .tooltip(tooltip)
}

/// 前缀搜索：一个输入框，旁边一个安静的提交按钮。
///
/// **宽度由调用方给，本控件铺满它**（`w_full`）。别指望它自己量出宽度：输入框是
/// `flex_1`（flex-basis 0），放进一条内容尺寸的 flex 行里会塌成一条缝——工具栏最初
/// 就是这么塌的（定宽容器不会自动撑满它的 flex 子项，得子项自己 `w_full`）。
///
/// 收进 `ui/` 是为了能被单独渲染做视觉验收
/// （`crates/desktop/examples/view_preview.rs` 离屏渲染的就是这个函数）。
///
/// 不要把放大镜焊在输入框右边做成接合控件：那是管理后台的搜索条。回车和按钮
/// 走同一条提交路径；**不要覆写 `Input` 的 border_color**，聚焦环由输入框自己画。
pub fn compact_search_field(
    input: &Entity<InputState>,
    theme: &Theme,
    on_submit: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .items_center()
        .gap_1()
        .child(div().flex_1().min_w_0().child(Input::new(input).small()))
        .child(
            Button::new("compact-search-submit")
                .icon(Icon::new(IconName::Search))
                .ghost()
                .with_size(Size::Small)
                .tooltip("按前缀搜索（回车）")
                .text_color(theme.muted_foreground)
                .on_click(on_submit),
        )
}

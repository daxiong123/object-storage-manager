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
    Window, div, px,
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

/// 紧凑搜索控件：输入框 + **接合**在右侧的放大镜按钮（参照实现的样子）。
///
/// 收进 `ui/` 的第二个理由不只是复用：它必须能被**单独渲染**出来做视觉验收
/// （`crates/desktop/examples/search_field_preview.rs` 离屏渲染的就是这个函数，
/// 而不是照抄一份）。gpui 不建 AX 树，控件级的视觉回归只能这样看。
///
/// 结构要点（从参照实现的截图逐像素量出来的）：
///
/// - 两段之间**没有间隙**，接缝那条竖线只由输入框自己的右边框提供；所以输入框
///   右角改方角，按钮左角方、右角圆，且按钮**不画左边框**；
/// - 未聚焦时两段的边框同色（`theme.input`），聚焦时输入框整圈变蓝（`Input` 自己
///   按 `theme.ring` 画）——**不要在这里覆写 `Input` 的 border_color**，那会把
///   聚焦色一起盖掉，焦点态就没了；
/// - 按钮分两层：外壳 div 画外观（上/右/下三条边框 + 右角圆角 + 底色），里层
///   ghost `Button` 管交互（tooltip / 焦点 / 点击）。库的 `Button::border_edges`
///   与 `ManagedTooltipExt` 都是 `pub(crate)`：前者拿不掉单条边，后者让纯自绘
///   div 挂不上 tooltip，所以两层各取所需。
pub fn compact_search_field(
    input: &Entity<InputState>,
    theme: &Theme,
    on_submit: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    // 24×24：与 `Input::small()` 的高度（`StyleSized::input_h(Small)` = h_6）对齐。
    // 这里不能走 `tokens::text()`——输入框高度是库里的固定 24px，按钮跟着字号缩放就错位了。
    //
    // 圆角内外两层必须一致（左角方、右角圆）：外壳决定边框轮廓，里层决定 hover 底色的轮廓。
    h_flex()
        .items_center()
        .child(
            div().flex_1().min_w_0().child(
                Input::new(input)
                    .small()
                    // 右角方角：右边那段让给紧邻的搜索按钮
                    .rounded_tr(px(0.))
                    .rounded_br(px(0.)),
            ),
        )
        .child(
            div()
                .size_6()
                .border_t_1()
                .border_r_1()
                .border_b_1()
                .border_color(theme.input)
                .rounded_tl(px(0.))
                .rounded_bl(px(0.))
                .rounded_tr(theme.radius)
                .rounded_br(theme.radius)
                .bg(theme.background)
                .child(
                    Button::new("compact-search-submit")
                        .icon(Icon::new(IconName::Search))
                        .ghost()
                        .with_size(Size::Small)
                        .size_full()
                        .rounded_tl(px(0.))
                        .rounded_bl(px(0.))
                        .rounded_tr(theme.radius)
                        .rounded_br(theme.radius)
                        .tooltip("按前缀搜索（回车）")
                        .text_color(theme.foreground)
                        .on_click(on_submit),
                ),
        )
}

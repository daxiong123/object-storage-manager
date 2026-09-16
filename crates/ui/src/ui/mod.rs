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

use gpui::{ElementId, SharedString};
use gpui_component::{Icon, Sizable as _, Size, button::Button, button::ButtonVariants as _};

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

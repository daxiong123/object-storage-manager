//! 自建浮层的统一外壳（agents.md §5「弹层规范」）。
//!
//! 规范本身是几条约定，散落在 9 处手写实现里就总有漏项——这里把**能漏的
//! 部分**变成构造器：漏不掉的才算规范。各弹层各自保留的只有真正不同的
//! 部分（宽度、快捷键上下文、动作与内容）。
//!
//! 已踩过的坑（都有对应验收问题）：
//! - 卡片不阻断 `mouse_down`：点卡片任意处都被遮罩当成「点击外部」而关闭；
//! - 卡片不阻断 `mouse_up`：卡片内含滚动列表时，滚动手势结束的 up 冒泡到
//!   遮罩，表现为「滚动一下就关闭」。
//!
//! 两相阻断都在 `surface()` 里做掉，调用方不必再记。

use gpui::{
    AnimationExt as _, Div, ElementId, InteractiveElement as _, IntoElement, MouseButton,
    Styled as _,
};
use gpui_component::{Theme, v_flex};

use crate::tokens;
use crate::ui::motion;

/// 浮层遮罩：铺满父容器（父容器需 `relative`）、居中承载卡片、统一遮罩色。
///
/// 带内边距是必要的：卡片的宽度是 `w_full() + max_w(设计宽度)`（见
/// `surface` 调用点），窗口比设计宽度窄时卡片会跟着缩——不留边距就会
/// 紧贴甚至压住窗口边缘。
///
/// 「点击遮罩关闭」「Esc 关闭」「occlude」等按弹层各自的语义由调用方接在
/// 返回值的链式调用上——它们确实各不相同（有的走 `UnifiedDismiss`，有的走
/// 专属 context 的 dismiss action；busy 时各 close handler 自行拒绝）。
pub(crate) fn mask(theme: &Theme) -> Div {
    v_flex()
        .absolute()
        .inset_0()
        .items_center()
        .justify_center()
        .p_6()
        .bg(theme.overlay)
}

/// 浮层表面：弹层卡片与锚定菜单共用（统一底色、发丝边框、`radius_lg`
/// 圆角、`shadow_lg`），并阻断两相鼠标冒泡。
///
/// 宽度 / 高度 / 内边距 / `overflow_hidden` / `occlude` 由调用方按内容决定：
/// 弹层要裁剪圆角外的内容、锚定菜单不需要宽度约束，这些差异是真实的，
/// 不适合下沉到这里。
pub(crate) fn surface(theme: &Theme) -> Div {
    v_flex()
        .bg(theme.background)
        .border_1()
        .border_color(theme.border)
        .rounded(tokens::radius_lg())
        .shadow_lg()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// 给**已装配完成**的浮层加进场淡入，遮罩与卡片一起淡入。
///
/// **必须在链式装配的最后一步调用**：`with_animation` 返回 `AnimationElement`，
/// 之后不能再接 `Div` 的方法（`key_context` / `on_action` / `child` 等都要在
/// 传进来之前接完）。所以这里不把动画塞进 `mask()`——mask 的方法链与 `child`
/// 都由各浮层自己接，塞进去就没有位置既加子元素又包动画。
///
/// 透明度是**组透明度**（gpui 的 `element_opacity` 相乘下传至子树），所以淡化
/// 遮罩即淡化整层，不需要分别处理遮罩与卡片。
///
/// 动画状态按「元素 id + 渲染树位置」缓存，且**卸载即被丢弃**（gpui 帧末只保留
/// 本帧访问过的 state），因此每次打开浮层都会重放进场——这正是想要的行为。
/// 不要把 id 改成随状态变化的值，否则每次重渲染都会重放。
///
/// 时长与曲线见 `motion::overlay_enter()`；`reduce_motion` 由 gpui 的
/// `with_animation` 自行尊重，这里不必再判。
///
/// 只做进场，不是遗漏：
/// - div 没有 `transform`/`scale`（`Transformation` 只存在于 SVG），所以既做不了
///   位移进场，也做不了 `scale(0.96)` 按下反馈；
/// - 没有元素级 blur（只有阴影的 `blur_radius`），所以没有 `blur(4px)`；
/// - `Animation` 没有完成回调，退场需要「先播动画、再由定时器真正卸载」的延迟
///   机制，会把 focus 归还与实体丢弃推迟上百毫秒，而 preview / details /
///   copy_move 三个渲染器在状态被清空后会自塌为空元素，不能简单「置空但保持
///   挂载」。故退场仍即时。
pub(crate) fn fade_in(id: impl Into<ElementId>, el: Div) -> impl IntoElement {
    el.with_animation(id, motion::overlay_enter(), |this, delta| {
        this.opacity(delta)
    })
}

//! 动效常量：时长与曲线集中在这里，不再让 `180` / `cubic-bezier(...)` 散落各处。
//!
//! **`reduce_motion` 不需要自己判**：走 `AnimationExt::with_animation` 的动效，
//! gpui 自身已尊重 `App::reduce_motion()`——开启时直接渲染终态且不排帧
//! （`gpui-pre/src/elements/animation.rs:407`，`AnimationExt` 的文档也写明了）。
//! 只有**装饰性**的 `window.request_animation_frame` 调用需要显式判。

use std::time::Duration;

use gpui::Animation;
use gpui_component::animation::cubic_bezier;

/// 浮层进场时长。退场应更短，但当前不做退场（理由见 `overlay::fade_in`）。
pub(crate) const OVERLAY_ENTER_MILLIS: u64 = 180;

/// 浮层进场动画：`cubic-bezier(0.2, 0, 0, 1)`。
///
/// 返回构造函数而非常量：`Animation` 的 easing 是 `Rc<dyn Fn(f32) -> f32>`，
/// 无法在 `const` 上下文里构造。曲线用 gpui-component 的实现——gpui 本体没有
/// cubic-bezier。
pub(crate) fn overlay_enter() -> Animation {
    Animation::new(Duration::from_millis(OVERLAY_ENTER_MILLIS))
        .with_easing(cubic_bezier(0.2, 0.0, 0.0, 1.0))
}

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

pub fn text(size: f32) -> Pixels {
    px(size * ui_font_scale())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_font_scale_clamps_bounds() {
        set_ui_font_scale(0.5);
        assert_eq!(ui_font_scale(), 0.85);
        set_ui_font_scale(2.0);
        assert_eq!(ui_font_scale(), 1.4);
        set_ui_font_scale(1.15);
        assert_eq!(ui_font_scale(), 1.15);
        set_ui_font_scale(1.0);
    }
}

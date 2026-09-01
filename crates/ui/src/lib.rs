mod workspace_view;

use gpui::{Bounds, Pixels, WindowOptions};
use gpui_component::TitleBar;

pub use workspace_view::WorkspaceView;

/// 应用初始化（在 gpui_component::init 之后调用）。
pub fn init(_cx: &mut gpui::App) {
    // TODO: 注册主题/Accent/Action 等全局 UI 状态。
}

/// 主窗口配置：Unified Titlebar（Traffic Lights 与内容一体，agents.md §7）。
pub fn window_options(bounds: Bounds<Pixels>) -> WindowOptions {
    WindowOptions {
        titlebar: Some(TitleBar::title_bar_options()),
        window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
        ..Default::default()
    }
}

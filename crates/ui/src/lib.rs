pub mod account_modal;
pub mod actions;
pub mod command_palette;
mod theme;
mod workspace_view;

use gpui::{Bounds, Pixels, WindowOptions};
use gpui_component::TitleBar;

pub use account_modal::AddAccountModal;
pub use command_palette::{CommandKind, PaletteCommand, PaletteHandler};
pub use theme::observe_appearance;
pub use workspace_view::WorkspaceView;

/// 应用初始化（在 gpui_component::init 之后调用）。
pub fn init(cx: &mut gpui::App) {
    // 全局快捷键（⌘Q / ⌘W / ⌘⌥S / ⌘⌥I）。
    actions::bind_keys(cx);
    // 应用主题（OpenChamber 设计基调）：必须在 gpui_component::init 之后、
    // 窗口创建之前写入，首帧即是我们的视觉身份。
    theme::init(cx);
}

/// 主窗口配置：Unified Titlebar（Traffic Lights 与内容一体，agents.md §7）。
pub fn window_options(bounds: Bounds<Pixels>) -> WindowOptions {
    WindowOptions {
        titlebar: Some(TitleBar::title_bar_options()),
        window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
        ..Default::default()
    }
}

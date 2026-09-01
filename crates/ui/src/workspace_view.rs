use gpui::*;
use gpui_component::{ActiveTheme, TitleBar, h_flex, v_flex};

/// 主窗口三栏 Workspace 骨架。
///
/// 目标形态（agents.md §7）：Titlebar + Sidebar + Object Table + Inspector。
/// 当前为骨架占位：仅 Unified Titlebar + 居中占位内容，三栏由 Resizable 逐步落地。
pub struct WorkspaceView;

impl WorkspaceView {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WorkspaceView {
    fn default() -> Self {
        Self::new()
    }
}

impl Render for WorkspaceView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .size_full()
            .bg(theme.background)
            .child(
                TitleBar::new().child(
                    h_flex()
                        .w_full()
                        .pr_2()
                        .justify_between()
                        .child("CloudStorage")
                        .child("七牛 Kodo / 阿里云 OSS"),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .text_color(theme.muted_foreground)
                    .child("三栏 Workspace 占位 — Sidebar / Object Table / Inspector"),
            )
    }
}

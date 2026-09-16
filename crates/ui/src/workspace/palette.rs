//! 命令面板（⌘K）：动态命令构建、生命周期与浮层渲染。

use super::*;

impl WorkspaceView {
    pub(super) fn handle_open_command_palette(
        &mut self,
        _: &OpenCommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.is_some() {
            return; // 已打开（⌘K 重复触发为无操作）
        }
        let extra = self.bucket_jump_commands(cx);
        let palette = cx.new(|cx| CommandPaletteView::new(window, cx, extra));
        // 面板关闭（open=false）后由观察者丢弃实体并归还焦点。
        cx.observe_in(&palette, window, Self::handle_palette_changed)
            .detach();
        palette.update(cx, |palette, cx| palette.focus_input(window, cx));
        self.palette = Some(palette);
        cx.notify();
    }

    /// 面板状态观察：面板自己调用 close() 置 open=false 时，这里收尾——
    /// 丢弃实体（遮罩与卡片随之消失）并把焦点归还 Workspace 根节点。
    /// 在 observe 回调里丢弃面板是安全的：回调参数持有的 Entity 让它
    /// 存活到本次调用结束。
    pub(super) fn handle_palette_changed(
        &mut self,
        palette: Entity<CommandPaletteView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if palette.read(cx).open() {
            return; // 过滤/选行等常规通知，不处理
        }
        self.palette = None;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// 命令面板遮罩：点击空白处关闭；卡片自身的 on_mouse_down 会阻止
    /// 冒泡，所以点卡片内部不会触发这里。
    pub(super) fn render_palette_overlay(
        &self,
        palette: &Entity<CommandPaletteView>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // 这里的遮罩**不能**用 `overlay::mask()`：命令面板卡片由 GPUI Kit `Command`
        // 自行绝对定位（按窗口宽度算 left），而绝对定位子元素的包含块是这个遮罩的
        // **padding box**——mask 自带 p_6，套上去会把卡片整体右移 24px 而偏心。
        // 所以命令面板用无内边距的遮罩（其余 9 个浮层仍一律走 mask()）。
        let scrim = div()
            .absolute()
            .inset_0()
            .occlude() // 挡住下层元素的鼠标交互
            .bg(theme.overlay)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    if let Some(palette) = &this.palette {
                        palette.update(cx, |palette, cx| palette.close(window, cx));
                    }
                }),
            )
            .child(palette.clone());
        overlay::fade_in("command-palette", scrim)
    }
}

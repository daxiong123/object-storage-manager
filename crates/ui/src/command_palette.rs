//! 命令面板（⌘K，规范 §22）。
//!
//! 自建实现（gpui-component 0.5.1 无 command_palette 模块）。键盘机制依赖
//! gpui 0.2.2 keymap 语义（源码验证，详见 docs/notes/gpui-api-notes.md）：
//! - 单行 `InputState` 只在 multi_line 下注册 MoveUp/MoveDown 的 on_action，
//!   因此 context "Palette" 的 "up"/"down" 绑定能接住方向键做行选择；
//! - 输入框 Esc 走 `escape()`：未设 clean_on_escape → `cx.propagate()`
//!   → context "Palette" 的 PaletteClose 接住；
//! - 回车通过订阅 `InputEvent::PressEnter` 执行选中命令。

use std::rc::Rc;

use gpui::{
    Action, AnyElement, App, AppContext as _, ClickEvent, Context, Entity, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement, Render, SharedString, StatefulInteractiveElement as _,
    Styled, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme, Theme, h_flex, input::Input, input::InputEvent, input::InputState, kbd::Kbd,
    v_flex,
};

use crate::actions::{
    AddAccount, CloseWindow, CopyObjectUrl, DeleteObject, DownloadObject, PaletteClose,
    PaletteSelectNext, PaletteSelectPrev, PreviewObject, Quit, Refresh, SaveTextObject,
    ToggleInspector, ToggleSidebar, UploadFiles, UploadFolder,
};

/// 自定义命令处理器（无键位提示）。
pub type PaletteHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// 命令种类：分发 gpui Action（键位提示自动从绑定表反查），或直接执行闭包。
pub enum CommandKind {
    Action(Box<dyn Action>),
    Handler(PaletteHandler),
}

/// 命令面板条目（规范 §22：与菜单/快捷键共享同一 Action）。
pub struct PaletteCommand {
    pub title: SharedString,
    /// 额外匹配关键词（英文/拼音别名）。
    pub keywords: Vec<SharedString>,
    pub kind: CommandKind,
}

impl PaletteCommand {
    pub fn action(title: impl Into<SharedString>, action: Box<dyn Action>) -> Self {
        Self {
            title: title.into(),
            keywords: Vec::new(),
            kind: CommandKind::Action(action),
        }
    }

    pub fn handler(title: impl Into<SharedString>, handler: PaletteHandler) -> Self {
        Self {
            title: title.into(),
            keywords: Vec::new(),
            kind: CommandKind::Handler(handler),
        }
    }

    /// 追加匹配关键词（字面量，'static）。
    pub fn keywords(mut self, keywords: &'static [&'static str]) -> Self {
        self.keywords = keywords.iter().map(|k| SharedString::from(*k)).collect();
        self
    }
}

/// 命令面板视图。WorkspaceView 收到 OpenCommandPalette 时创建，
/// 关闭（open=false）后由 WorkspaceView 的 observe 丢弃实体并归还焦点；
/// 因此每次打开都是全新状态（查询词与选中行自动重置）。
pub struct CommandPaletteView {
    input: Entity<InputState>,
    commands: Vec<PaletteCommand>,
    /// 命中过滤的下标（指向 commands）。
    filtered: Vec<usize>,
    /// 当前选中行在 filtered 中的位置。
    selected: usize,
    last_query: String,
    open: bool,
}

impl CommandPaletteView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("搜索命令…"));
        cx.subscribe_in(&input, window, Self::on_input_event)
            .detach();

        let commands = Self::default_commands();
        let filtered = (0..commands.len()).collect();
        Self {
            input,
            commands,
            filtered,
            selected: 0,
            last_query: String::new(),
            open: true,
        }
    }

    /// 初始命令集。命令来自 crate::actions 的共享 Action（规范 §22）。
    fn default_commands() -> Vec<PaletteCommand> {
        vec![
            PaletteCommand::action("切换边栏", Box::new(ToggleSidebar))
                .keywords(&["sidebar", "panel"]),
            PaletteCommand::action("切换检查器", Box::new(ToggleInspector))
                .keywords(&["inspector", "panel"]),
            PaletteCommand::action("关闭窗口", Box::new(CloseWindow))
                .keywords(&["close", "window"]),
            PaletteCommand::action("退出 CloudStorage", Box::new(Quit)).keywords(&["quit", "exit"]),
            PaletteCommand::action("添加账号", Box::new(AddAccount))
                .keywords(&["account", "add", "qiniu"]),
            PaletteCommand::action("下载对象…", Box::new(DownloadObject))
                .keywords(&["download", "object"]),
            PaletteCommand::action("上传文件…", Box::new(UploadFiles))
                .keywords(&["upload", "file"]),
            PaletteCommand::action("上传文件夹…", Box::new(UploadFolder)).keywords(&[
                "upload",
                "folder",
                "directory",
            ]),
            PaletteCommand::action("刷新", Box::new(Refresh)).keywords(&["refresh", "reload"]),
            PaletteCommand::action("删除对象…", Box::new(DeleteObject))
                .keywords(&["delete", "remove"]),
            PaletteCommand::action("预览对象", Box::new(PreviewObject))
                .keywords(&["preview", "quick look"]),
            PaletteCommand::action("复制对象链接", Box::new(CopyObjectUrl))
                .keywords(&["copy", "url", "link", "share"]),
            PaletteCommand::action("保存并上传", Box::new(SaveTextObject))
                .keywords(&["save", "upload", "edit"]),
        ]
    }

    /// 是否处于打开状态（WorkspaceView 的 observe 据此丢弃实体）。
    pub fn open(&self) -> bool {
        self.open
    }

    /// 把焦点移入搜索输入框（打开面板后必须调用，否则 ⌘K 面板无输入焦点）。
    pub fn focus_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |state, cx| state.focus(window, cx));
    }

    /// 关闭面板：置 open=false 并 notify，由 WorkspaceView 的 observe 收尾
    /// （丢弃实体 + 焦点归还 Workspace 根节点）。
    pub fn close(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        cx.notify();
    }

    // ---- 事件 ----

    fn handle_close(&mut self, _: &PaletteClose, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
    }

    fn handle_select_prev(
        &mut self,
        _: &PaletteSelectPrev,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.filtered.is_empty() {
            return;
        }
        self.selected = if self.selected == 0 {
            self.filtered.len() - 1
        } else {
            self.selected - 1
        };
        cx.notify();
    }

    fn handle_select_next(
        &mut self,
        _: &PaletteSelectNext,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.filtered.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.filtered.len();
        cx.notify();
    }

    fn on_input_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                let query = self.input.read(cx).value().to_string();
                if query == self.last_query {
                    return;
                }
                self.apply_filter(&query);
                self.last_query = query;
                cx.notify();
            }
            InputEvent::PressEnter { .. } => self.execute_selected(window, cx),
            _ => {}
        }
    }

    fn apply_filter(&mut self, query: &str) {
        let q = query.trim().to_lowercase();
        self.filtered = self
            .commands
            .iter()
            .enumerate()
            .filter(|(_, cmd)| {
                q.is_empty()
                    || cmd.title.to_lowercase().contains(&q)
                    || cmd.keywords.iter().any(|k| k.to_lowercase().contains(&q))
            })
            .map(|(ix, _)| ix)
            .collect();
        self.selected = 0;
    }

    /// 执行选中命令。先关闭面板再执行：WorkspaceView 在效果刷新时丢弃面板并
    /// 归还焦点，保证「关闭窗口」这类命令不被遗留的面板焦点拖累。
    fn execute_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.filtered.get(self.selected).copied() else {
            return;
        };
        self.close(window, cx);
        match &self.commands[ix].kind {
            CommandKind::Action(action) => {
                // 从当前焦点（面板输入框）沿渲染树冒泡；WorkspaceView 根节点
                // 持有同名 on_action，Quit 由 App 全局 capture 接住。
                window.dispatch_action(action.boxed_clone(), cx);
            }
            CommandKind::Handler(handler) => handler(window, cx),
        }
    }

    // ---- 渲染 ----

    fn render_list(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut list = v_flex()
            .id("palette-list")
            .w_full()
            .max_h(px(340.))
            .overflow_y_scroll()
            .py_1();

        if self.filtered.is_empty() {
            return list.child(
                div()
                    .px_3()
                    .py_4()
                    .text_size(px(13.))
                    .text_color(theme.muted_foreground)
                    .child("无匹配命令"),
            );
        }

        for (row_ix, &cmd_ix) in self.filtered.iter().enumerate() {
            let cmd = &self.commands[cmd_ix];
            let selected = row_ix == self.selected;
            // 键位提示从真实绑定表反查（与菜单同一数据源，规范 §26）。
            let hint: Option<AnyElement> = match &cmd.kind {
                CommandKind::Action(action) => {
                    Kbd::binding_for_action(action.as_ref(), None, window)
                        .map(|kbd| kbd.into_any_element())
                }
                CommandKind::Handler(_) => None,
            };

            list = list.child(
                h_flex()
                    .id(("palette-row", row_ix))
                    .mx_1()
                    .px_3()
                    .py(px(6.))
                    .rounded(px(6.))
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .text_size(px(13.))
                    .when(selected, |row| row.bg(theme.accent))
                    .hover(|row| row.bg(theme.accent))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.selected = row_ix;
                        this.execute_selected(window, cx);
                    }))
                    .child(div().flex_1().child(cmd.title.clone()))
                    .children(hint),
            );
        }
        list
    }
}

impl Render for CommandPaletteView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        // 水平居中；顶部偏移 96px，接近 Spotlight 的视觉比例。
        let viewport = window.viewport_size();
        let card_w = px(560.);
        let left = (viewport.width - card_w) / 2.;

        v_flex()
            .key_context("Palette")
            .on_action(cx.listener(Self::handle_select_prev))
            .on_action(cx.listener(Self::handle_select_next))
            .on_action(cx.listener(Self::handle_close))
            // 卡片内点击不冒泡到遮罩（否则点卡片会关闭面板）。
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .absolute()
            .left(left)
            .top(px(96.))
            .w(card_w)
            .bg(theme.background)
            .border_1()
            .border_color(theme.border)
            .rounded(px(10.))
            .shadow_lg()
            .overflow_hidden()
            .child(
                h_flex()
                    .w_full()
                    .px_2()
                    .py_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(Input::new(&self.input).flex_1()),
            )
            .child(self.render_list(&theme, window, cx))
            .child(
                h_flex()
                    .w_full()
                    .justify_end()
                    .px_3()
                    .py_1()
                    .border_t_1()
                    .border_color(theme.border)
                    .text_size(px(11.))
                    .text_color(theme.muted_foreground)
                    .child("↑↓ 选择 · ↵ 执行 · Esc 关闭"),
            )
    }
}

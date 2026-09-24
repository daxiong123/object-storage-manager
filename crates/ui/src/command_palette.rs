//! 命令面板（⌘K，规范 §22）。
//!
//! 过滤、虚拟列表、键盘导航与无障碍语义由 GPUI Kit `Command` 提供；
//! 本层只保留应用命令及动态 Bucket 跳转的编排。

use std::rc::Rc;

use gpui::{
    Action, App, AppContext as _, Context, Entity, Focusable as _, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement as _, Render, SharedString, Styled as _, Window, div,
    px,
};
use gpui_component::{
    ActiveTheme as _, command::Command, command::CommandItem, command::CommandState, h_flex,
    kbd::Kbd, v_flex,
};

use crate::actions::{
    AddAccount, CloseWindow, CopyObjectUrl, DeleteObject, DismissCommandPalette, DownloadObject,
    FocusPath, NavigateBack, NavigateForward, OpenAbout, OpenObject, OpenSettings, PreviewObject,
    Quit, Refresh, RenameObject, RevealInFinder, SaveTextObject, SelectObjectAll, ToggleSidebar,
    UploadFiles, UploadFolder,
};
use crate::tokens;

/// 自定义命令处理器（用于动态 Bucket 跳转，无键位提示）。
pub type PaletteHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// 命令种类：分发共享 gpui Action，或直接执行动态闭包。
pub enum CommandKind {
    Action(Box<dyn Action>),
    Handler(PaletteHandler),
}

pub struct PaletteCommand {
    pub title: SharedString,
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

    pub fn handler<F>(title: impl Into<SharedString>, handler: F) -> Self
    where
        F: Fn(&mut Window, &mut App) + 'static,
    {
        Self {
            title: title.into(),
            keywords: Vec::new(),
            kind: CommandKind::Handler(Rc::new(handler)),
        }
    }

    pub fn keywords(mut self, keywords: &'static [&'static str]) -> Self {
        self.keywords = keywords.iter().map(|keyword| (*keyword).into()).collect();
        self
    }
}

pub struct CommandPaletteView {
    state: Entity<CommandState>,
    commands: Vec<PaletteCommand>,
    open: bool,
}

impl CommandPaletteView {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        mut extra: Vec<PaletteCommand>,
    ) -> Self {
        extra.extend(Self::default_commands());
        Self {
            state: cx.new(|cx| CommandState::new(window, cx)),
            commands: extra,
            open: true,
        }
    }

    fn default_commands() -> Vec<PaletteCommand> {
        vec![
            PaletteCommand::action("切换边栏", Box::new(ToggleSidebar))
                .keywords(&["sidebar", "panel"]),
            PaletteCommand::action("关闭窗口", Box::new(CloseWindow))
                .keywords(&["close", "window"]),
            PaletteCommand::action("退出 CloudStorage", Box::new(Quit)).keywords(&["quit", "exit"]),
            PaletteCommand::action("添加账号", Box::new(AddAccount))
                .keywords(&["account", "add", "qiniu", "aliyun", "tencent", "cos"]),
            PaletteCommand::action("关于 CloudStorage", Box::new(OpenAbout))
                .keywords(&["about", "version", "license"]),
            PaletteCommand::action("设置…", Box::new(OpenSettings)).keywords(&[
                "settings",
                "preferences",
                "ttl",
                "clipboard",
            ]),
            PaletteCommand::action("下载对象…", Box::new(DownloadObject))
                .keywords(&["download", "object"]),
            PaletteCommand::action("用默认应用打开", Box::new(OpenObject))
                .keywords(&["open", "with", "launch"]),
            PaletteCommand::action("在 Finder 中显示", Box::new(RevealInFinder))
                .keywords(&["finder", "show", "reveal", "folder"]),
            PaletteCommand::action("上传文件…", Box::new(UploadFiles))
                .keywords(&["upload", "file"]),
            PaletteCommand::action("上传文件夹…", Box::new(UploadFolder)).keywords(&[
                "upload",
                "folder",
                "directory",
            ]),
            PaletteCommand::action("刷新", Box::new(Refresh)).keywords(&["refresh", "reload"]),
            PaletteCommand::action("全选对象", Box::new(SelectObjectAll))
                .keywords(&["select", "all", "objects"]),
            PaletteCommand::action("重命名…", Box::new(RenameObject))
                .keywords(&["rename", "modify", "name"]),
            PaletteCommand::action("删除对象…", Box::new(DeleteObject))
                .keywords(&["delete", "remove"]),
            PaletteCommand::action("预览对象", Box::new(PreviewObject))
                .keywords(&["preview", "quick look"]),
            PaletteCommand::action("复制对象链接", Box::new(CopyObjectUrl))
                .keywords(&["copy", "url", "link", "share"]),
            PaletteCommand::action("保存并上传", Box::new(SaveTextObject))
                .keywords(&["save", "upload", "edit"]),
            PaletteCommand::action("导航：后退", Box::new(NavigateBack))
                .keywords(&["back", "navigate", "history"]),
            PaletteCommand::action("导航：前进", Box::new(NavigateForward))
                .keywords(&["forward", "navigate", "history"]),
            PaletteCommand::action("跳转路径…", Box::new(FocusPath))
                .keywords(&["go", "path", "jump", "prefix"]),
        ]
    }

    pub fn open(&self) -> bool {
        self.open
    }

    pub fn focus_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.focus_handle(cx).focus(window, cx);
    }

    pub fn close(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        cx.notify();
    }

    fn execute(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(command) = self.commands.get(index) else {
            return;
        };
        let command = match &command.kind {
            CommandKind::Action(action) => CommandKind::Action(action.boxed_clone()),
            CommandKind::Handler(handler) => CommandKind::Handler(handler.clone()),
        };

        self.close(window, cx);
        match command {
            CommandKind::Action(action) => window.dispatch_action(action, cx),
            CommandKind::Handler(handler) => handler(window, cx),
        }
    }

    fn command_items(&self) -> Vec<CommandItem> {
        self.commands
            .iter()
            .map(|command| {
                let item = CommandItem::new()
                    .label(command.title.clone())
                    .keywords(command.keywords.clone());
                let CommandKind::Action(action) = &command.kind else {
                    return item;
                };
                let action = action.boxed_clone();
                let title = command.title.clone();
                item.child(move |window, _| {
                    h_flex()
                        .w_full()
                        .justify_between()
                        .gap_3()
                        .child(div().flex_1().child(title.clone()))
                        .children(Kbd::binding_for_action(action.as_ref(), None, window))
                })
            })
            .collect()
    }
}

impl Render for CommandPaletteView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let card_width = px(560.);
        let left = (window.viewport_size().width - card_width) / 2.;
        let confirm_owner = cx.weak_entity();
        let cancel_owner = cx.weak_entity();

        let command = Command::new(&self.state)
            .items(self.command_items())
            .placeholder("搜索命令…")
            .max_h(px(340.))
            .empty(|_, _, cx| {
                div()
                    .px_3()
                    .py_4()
                    .text_size(tokens::text(13.))
                    .text_color(cx.theme().muted_foreground)
                    .child("无匹配命令")
            })
            .footer(|_, _, cx| {
                h_flex()
                    .w_full()
                    .justify_end()
                    .px_3()
                    .py_1()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .text_size(tokens::text(11.))
                    .text_color(cx.theme().muted_foreground)
                    .child("↑↓ 选择 · ↵ 执行 · Esc 关闭")
            })
            .on_confirm(move |index, window, cx| {
                _ = confirm_owner.update(cx, |palette, cx| {
                    palette.execute(index.row, window, cx);
                });
            })
            .on_cancel(move |window, cx| {
                _ = cancel_owner.update(cx, |palette, cx| palette.close(window, cx));
            });

        v_flex()
            .absolute()
            .left(left)
            .top(px(96.))
            .w(card_width)
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_action(
                cx.listener(|palette, _: &DismissCommandPalette, window, cx| {
                    palette.close(window, cx);
                }),
            )
            .child(command)
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn native_command_filters_and_confirms_dynamic_items(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });

        let confirmations = Rc::new(Cell::new(0));
        let confirmations_for_handler = confirmations.clone();
        let (palette, cx) = cx.add_window_view(move |window, cx| {
            CommandPaletteView::new(
                window,
                cx,
                vec![
                    PaletteCommand::handler("唯一动态命令", move |_, _| {
                        confirmations_for_handler.set(confirmations_for_handler.get() + 1);
                    })
                    .keywords(&["needle-unique"]),
                ],
            )
        });

        cx.run_until_parked();
        let state = cx.update(|window, cx| {
            _ = window.draw(cx);
            palette.read(cx).state.clone()
        });
        cx.update(|window, cx| {
            state.update(cx, |state, cx| {
                state.set_query("needle-unique", window, cx);
                assert_eq!(state.matched_count(), 1);
                state.focus(window, cx);
            });
        });
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();

        assert_eq!(confirmations.get(), 1);
        assert!(!palette.read_with(cx, |palette, _| palette.open()));
    }

    #[gpui::test]
    fn escape_closes_even_with_a_nonempty_query(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let (palette, cx) =
            cx.add_window_view(|window, cx| CommandPaletteView::new(window, cx, Vec::new()));

        cx.run_until_parked();
        let state = cx.update(|window, cx| {
            _ = window.draw(cx);
            let state = palette.read(cx).state.clone();
            state.update(cx, |state, cx| {
                state.set_query("refresh", window, cx);
                state.focus(window, cx);
            });
            state
        });
        assert!(state.read_with(cx, |state, cx| !state.query(cx).is_empty()));
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();

        assert!(!palette.read_with(cx, |palette, _| palette.open()));
    }
}

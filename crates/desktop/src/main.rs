use gpui::*;
use gpui::{Menu, MenuItem, OsAction};
use gpui_component::Root;
use object_storage_ui::{self as ui, WorkspaceView};
use ui::actions::{
    CloseWindow, Copy, Cut, Paste, Quit, Redo, SelectAll, ToggleInspector, ToggleSidebar, Undo,
};

/// macOS 应用菜单（规范 §11/§22：与快捷键共享同一 Action；§26：⌘ 符号随键位自动显示）。
///
/// Edit 菜单走 `os_action` 触发系统原生的撤销/剪贴板行为；其 gpui Action 本身无需窗口处理。
fn app_menus() -> Vec<Menu> {
    vec![
        Menu {
            name: "CloudStorage".into(),
            items: vec![
                MenuItem::separator(),
                MenuItem::action("退出 CloudStorage", Quit),
            ],
        },
        Menu {
            name: "编辑".into(),
            items: vec![
                MenuItem::os_action("撤销", Undo, OsAction::Undo),
                MenuItem::os_action("重做", Redo, OsAction::Redo),
                MenuItem::separator(),
                MenuItem::os_action("剪切", Cut, OsAction::Cut),
                MenuItem::os_action("拷贝", Copy, OsAction::Copy),
                MenuItem::os_action("粘贴", Paste, OsAction::Paste),
                MenuItem::separator(),
                MenuItem::os_action("全选", SelectAll, OsAction::SelectAll),
            ],
        },
        Menu {
            name: "显示".into(),
            items: vec![
                MenuItem::action("切换边栏", ToggleSidebar),
                MenuItem::action("切换检查器", ToggleInspector),
            ],
        },
        Menu {
            name: "窗口".into(),
            items: vec![MenuItem::action("关闭窗口", CloseWindow)],
        },
    ]
}

fn main() {
    let app = Application::new().with_assets(gpui_component_assets::Assets);

    app.run(move |cx| {
        // 必须最先初始化 gpui-component（官方要求）。
        gpui_component::init(cx);
        ui::init(cx);
        cx.set_menus(app_menus());
        // Quit 用全局监听（capture 阶段，不依赖窗口焦点）：最后一个窗口关闭后仍可 ⌘Q 退出。
        // TODO(§57)：传输引擎落地后，改为弹退出确认（默认「暂停并持久化」）。
        cx.on_action(|_: &Quit, cx| cx.quit());

        cx.spawn(async move |cx| {
            let bounds = cx.update(|app| Bounds::centered(None, size(px(1280.), px(820.)), app))?;
            cx.open_window(ui::window_options(bounds), |window, cx| {
                let workspace = cx.new(WorkspaceView::new);
                // 菜单 Action 经焦点链派发：初始焦点置于 Workspace 根节点。
                window.focus(&workspace.focus_handle(cx));
                // 窗口第一层视图必须是 Root。
                cx.new(|cx| Root::new(workspace, window, cx))
            })?;

            Ok::<_, anyhow::Error>(())
        })
        .detach();
    });
}

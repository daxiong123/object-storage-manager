use gpui::*;
use gpui::{Menu, MenuItem, OsAction};
use gpui_component::Root;
use object_storage_app::AppServices;
use object_storage_ui::{self as ui, WorkspaceView};
use std::sync::Arc;
use ui::actions::{
    CloseWindow, Copy, Cut, DownloadObject, OpenCommandPalette, Paste, Quit, Redo, Refresh,
    SelectAll, ToggleInspector, ToggleSidebar, Undo, UploadFiles, UploadFolder,
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
                MenuItem::action("命令面板…", OpenCommandPalette),
                MenuItem::separator(),
                MenuItem::action("刷新", Refresh),
                MenuItem::separator(),
                MenuItem::action("切换边栏", ToggleSidebar),
                MenuItem::action("切换检查器", ToggleInspector),
            ],
        },
        Menu {
            name: "对象".into(),
            items: vec![
                MenuItem::action("下载…", DownloadObject),
                MenuItem::action("上传文件…", UploadFiles),
                MenuItem::action("上传文件夹…", UploadFolder),
            ],
        },
        Menu {
            name: "窗口".into(),
            items: vec![MenuItem::action("关闭窗口", CloseWindow)],
        },
    ]
}

fn main() {
    // AppServices（SQLite + Keychain + tokio 运行时）在进 UI 前组装。
    // 打不开数据库属于启动级错误：直接报错退出（Fail Fast，规范 §8），
    // 不进半可用的界面。
    let services = match AppServices::open() {
        Ok(services) => Arc::new(services),
        Err(e) => {
            eprintln!("CloudStorage 启动失败：{e}");
            std::process::exit(1);
        }
    };

    let app = Application::new().with_assets(gpui_component_assets::Assets);

    app.run(move |cx| {
        // 必须最先初始化 gpui-component（官方要求）。
        gpui_component::init(cx);
        ui::init(cx);
        cx.set_menus(app_menus());
        // Quit 全局监听在 bubble 末尾：有窗口时由 WorkspaceView 先处理（有传输则弹确认），
        // 仅当无人处理或显式 propagate 时落到这里——窗口全关后仍可 ⌘Q 退出。
        cx.on_action(|_: &Quit, cx| cx.quit());

        cx.spawn(async move |cx| {
            let bounds = cx.update(|app| Bounds::centered(None, size(px(1280.), px(820.)), app))?;
            cx.open_window(ui::window_options(bounds), |window, cx| {
                let workspace = cx.new(|cx| WorkspaceView::new(Arc::clone(&services), cx));
                // 菜单 Action 经焦点链派发：初始焦点置于 Workspace 根节点。
                window.focus(&workspace.focus_handle(cx));
                // 窗口第一层视图必须是 Root。
                cx.new(|cx| Root::new(workspace, window, cx))
            })?;

            // 终端直启（cargo run）不触发 LaunchServices 激活，主动把应用带到前台，
            // 否则窗口不可交互。.app 包启动时此调用无副作用（本已在前台）。
            cx.update(|app| app.activate(true))?;

            Ok::<_, anyhow::Error>(())
        })
        .detach();
    });
}

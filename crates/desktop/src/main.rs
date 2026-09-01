use gpui::*;
use gpui_component::Root;
use object_storage_ui::{self as ui, WorkspaceView};

fn main() {
    let app = Application::new().with_assets(gpui_component_assets::Assets);

    app.run(move |cx| {
        // 必须最先初始化 gpui-component（官方要求）。
        gpui_component::init(cx);
        ui::init(cx);

        cx.spawn(async move |cx| {
            let bounds = cx.update(|app| Bounds::centered(None, size(px(1280.), px(820.)), app))?;
            cx.open_window(ui::window_options(bounds), |window, cx| {
                let workspace = cx.new(|_| WorkspaceView::new());
                // 窗口第一层视图必须是 Root。
                cx.new(|cx| Root::new(workspace, window, cx))
            })?;

            Ok::<_, anyhow::Error>(())
        })
        .detach();
    });
}

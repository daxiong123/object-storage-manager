//! 临时预览程序：把「添加账号」弹层渲染到一个**屏幕外、不抢焦点**的窗口里，
//! 用来截图核对服务商分段控件的选中态是否看得出（agents.md「UI 冒烟边界」：
//! gpui 不建 AX 树，常规手段无法脚本化验收）。
//!
//! 关键点：`focus: false` + 窗口坐标 -10000，所以既不会抢走别的应用焦点，
//! 也不会出现在任何显示器上；渲染完成后由调用方用 `screencapture -l <窗口号>` 抓图。
//!
//! 这是排查用的一次性工具，验收完即删。

use std::sync::Arc;

use gpui::*;
use gpui_component::{Root, Theme, ThemeMode};
use object_storage_app::AppServices;
use object_storage_ui::account_modal::AddAccountModal;

fn main() {
    // 弹层在渲染期不碰 services（只有保存才用），指向临时库即可，避免动用户数据。
    let db = std::env::temp_dir().join("osm-provider-preview.sqlite");
    let services = match AppServices::open_at(&db) {
        Ok(services) => Arc::new(services),
        Err(e) => {
            eprintln!("预览程序启动失败：{e}");
            std::process::exit(1);
        }
    };

    let app = gpui_platform::application().with_assets(gpui_kit_assets::Assets);

    app.run(move |cx| {
        gpui_component::init(cx);
        object_storage_ui::init(cx);

        // 系统会在傍晚自动切暗色，亮/暗两套的选中底都得看：暗色的
        // `sidebar_accent` 是 #111D2C，贴在深色面上是否还分辨得出是另一个问题。
        if std::env::var("OSM_PREVIEW_DARK").is_ok() {
            Theme::change(ThemeMode::Dark, None, cx);
        }

        let bounds = Bounds {
            origin: point(px(-10000.), px(-10000.)),
            size: size(px(520.), px(660.)),
        };
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                focus: false,
                show: true,
                ..Default::default()
            },
            move |window, cx| {
                let modal = cx.new(|cx| AddAccountModal::new(Arc::clone(&services), window, cx));
                cx.new(|cx| Root::new(modal, window, cx))
            },
        )
        .expect("打开预览窗口");

        // 这里**不**调用 app.activate(true)：预览不能抢用户前台焦点。
        // 留出渲染时间后自行退出。
        println!("预览窗口已创建（屏幕外，未激活）");
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(45))
                .await;
            cx.update(|app| app.quit());
        })
        .detach();
    });
}

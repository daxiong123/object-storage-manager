//! 视图预览工具：把**生产代码里的**某个视图渲染到一个屏幕外、**不抢焦点**的窗口里，
//! 再用窗口号抓图，用于视觉验收。
//!
//! 为什么需要它：gpui 不建 AX 树，UI 交互没法脚本化；但单个视图的**视觉**可以——
//! 不抢用户前台焦点，也不用合成鼠标/键盘去「打开」界面（前台是谁不由你决定，
//! 实测合成点击会落到用户正在用的别的应用上）。详见
//! `docs/notes/gpui-api-notes.md`「离屏预览 + 截图验收」。
//!
//! 用法：
//! ```bash
//! OSM_PREVIEW_VIEW=search nohup ./target/debug/examples/view_preview > /tmp/preview.log 2>&1 &
//! PID=$(pgrep -f examples/view_preview | head -1)
//! # 按 pid 取 CGWindowNumber（窗口坐标会被系统夹回屏幕内，不能靠 -10000 判断）
//! screencapture -x -o -l <窗口号> /tmp/view.png
//! ```
//!
//! 可用的 `OSM_PREVIEW_VIEW`：
//! - `search`（默认）：对象搜索控件，`object_storage_ui::compact_search_field`；
//!   加 `OSM_PREVIEW_FOCUS=1` 让输入框带焦点（看聚焦边框）。
//! - `provider`：「添加账号」弹层里的服务商分段控件，直接渲染整个 `AddAccountModal`。
//!
//! 两个开关对所有视图都生效：`OSM_PREVIEW_DARK=1` 切暗色（暗色的选中底/边框是另一套值）。

use std::sync::Arc;

use gpui::*;
use gpui_component::{ActiveTheme as _, Root, Theme, ThemeMode, input::InputState, v_flex};
use object_storage_app::AppServices;

/// 搜索控件：只画这一条控件，四周留白以便量边框。
struct SearchHarness {
    input: Entity<InputState>,
}

impl Render for SearchHarness {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        v_flex()
            .size_full()
            .justify_center()
            .items_center()
            .bg(theme.background)
            .child(
                // 宽度与对象工具栏里一致（`render_object_search` 的 `w(tokens::text(220.))`）
                div().w(object_storage_ui::tokens::text(220.)).child(
                    object_storage_ui::compact_search_field(&self.input, &theme, |_, _, _| {}),
                ),
            )
    }
}

fn main() {
    let view = std::env::var("OSM_PREVIEW_VIEW").unwrap_or_else(|_| "search".to_string());
    let dark = std::env::var("OSM_PREVIEW_DARK").is_ok();
    let focus = std::env::var("OSM_PREVIEW_FOCUS").is_ok();

    let app = gpui_platform::application().with_assets(gpui_kit_assets::Assets);

    app.run(move |cx| {
        gpui_component::init(cx);
        object_storage_ui::init(cx);
        if dark {
            Theme::change(ThemeMode::Dark, None, cx);
        }

        let size = match view.as_str() {
            "provider" => size(px(520.), px(660.)),
            _ => size(px(260.), px(80.)),
        };
        let bounds = Bounds {
            origin: point(px(-10000.), px(-10000.)),
            size,
        };
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                // 不抢用户前台焦点。窗口坐标会被系统夹回屏幕内，所以抓图要按窗口号找。
                focus: false,
                show: true,
                ..Default::default()
            },
            {
                let view = view.clone();
                move |window, cx| match view.as_str() {
                    "provider" => {
                        // 弹层渲染期不碰 services（只有保存才用），指向临时库即可。
                        let db = std::env::temp_dir().join("osm-view-preview.sqlite");
                        let services = Arc::new(
                            AppServices::open_at(&db).expect("打开预览用的临时数据库失败"),
                        );
                        let modal = cx.new(|cx| {
                            object_storage_ui::AddAccountModal::new(services, window, cx)
                        });
                        cx.new(|cx| Root::new(modal, window, cx))
                    }
                    _ => {
                        let input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("文件前缀搜索"));
                        if focus {
                            input.update(cx, |state, cx| state.focus(window, cx));
                        }
                        let harness = cx.new(|_| SearchHarness { input });
                        cx.new(|cx| Root::new(harness, window, cx))
                    }
                }
            },
        )
        .expect("打开预览窗口");

        // 这里**不**调用 app.activate(true)：预览不能抢用户前台焦点。
        println!("预览窗口已创建（view={view}，屏幕外，未激活）");
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(45))
                .await;
            cx.update(|app| app.quit());
        })
        .detach();
    });
}

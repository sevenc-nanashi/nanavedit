mod editor;
#[cfg(target_os = "windows")]
mod preview;

use gpui::AppContext;

fn main() {
    gpui_platform::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::install(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let bounds =
                gpui::Bounds::centered(None, gpui::size(gpui::px(1280.0), gpui::px(800.0)), cx);
            cx.open_window(
                gpui::WindowOptions {
                    window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
                    window_min_size: Some(gpui::size(gpui::px(900.0), gpui::px(600.0))),
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some("nanavedit".into()),
                        appears_transparent: false,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| editor::Editor::new(window, cx)),
            )
            .expect("編集ウィンドウを作成できませんでした");
            cx.activate(true);
        });
}

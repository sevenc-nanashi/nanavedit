mod editor;
pub mod effects;
#[cfg(target_os = "windows")]
mod input;
mod media;
#[cfg(target_os = "windows")]
mod preview;
mod settings;
mod theme;
mod timeline;

use gpui::AppContext;

fn main() {
    gpui_platform::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            effects::definitions();
            gpui_kit::install(cx);
            #[cfg(target_os = "windows")]
            input::install(cx);
            settings::initialize(cx);
            cx.activate(true);
        });
}

fn open_editor(cx: &mut gpui::App) {
    let bounds = gpui::Bounds::centered(None, gpui::size(gpui::px(1280.0), gpui::px(800.0)), cx);
    let handle = cx
        .open_window(
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
    cx.global_mut::<settings::SettingsState>().editor = Some(handle);
}

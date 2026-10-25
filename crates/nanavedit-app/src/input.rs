use anyhow::{Context as _, Result};
use gpui::Global;
pub use nanavedit_media::input::{Client, Entry, Registry, frame};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
};

pub struct InputPlugins(pub Arc<Registry>);
impl Global for InputPlugins {}

pub fn directory() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .context("設定フォルダーを取得できません")?
        .join("nanavedit/Plugin"))
}
pub fn install(cx: &mut gpui::App) {
    cx.set_quit_mode(gpui::QuitMode::Explicit);
    let (registry, finished) = Registry::new();
    cx.set_global(InputPlugins(registry.clone()));
    let shutdown = registry.clone();
    cx.on_app_quit(move |_| {
        let registry = shutdown.clone();
        async move {
            registry.shutdown().await;
        }
    })
    .detach();
    let task = cx.background_executor().spawn(async move {
        let result = directory().and_then(|directory| {
            let base = directory.parent().expect("plugin parent");
            registry.scan(&directory, base, crate::editor::UI_FONT)
        });
        if let Err(error) = result {
            registry.entries.lock().expect("registry lock").push(Entry {
                path: PathBuf::new(),
                name: "プラグイン検索".into(),
                filters: String::new(),
                error: Some(format!("{error:#}")),
                client: None,
            });
        }
        registry.scanning.store(false, Ordering::Release);
        let _ = finished.try_send(());
    });
    cx.spawn(async move |cx| {
        task.await;
        cx.update(|cx| cx.refresh_windows());
    })
    .detach();
}

pub fn quit(cx: &mut gpui::App) {
    let registry = cx.global::<InputPlugins>().0.clone();
    if !registry.begin_shutdown() {
        return;
    }
    // Wait before App::quit, whose observer deadline is too short for native decoders.
    cx.spawn(async move |cx| {
        registry.shutdown().await;
        cx.update(|cx| cx.quit());
    })
    .detach();
}

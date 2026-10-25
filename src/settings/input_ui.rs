use super::*;
use crate::input::{Client, InputPlugins};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::sync::atomic::Ordering;

impl SettingsWindow {
    fn configure_input(&mut self, client: Client, window: &mut Window, cx: &mut Context<Self>) {
        let parent = match window.window_handle().map(|handle| handle.as_raw()) {
            Ok(RawWindowHandle::Win32(handle)) => handle.hwnd.get() as usize,
            _ => {
                self.error = Some("設定ウィンドウのHWNDを取得できません".into());
                cx.notify();
                return;
            }
        };
        self.loading = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = client.configure(parent).await;
            let _ = this.update(cx, |view, cx| {
                view.loading = false;
                view.error = result
                    .err()
                    .map(|error| format!("入力プラグインの設定に失敗しました: {error:#}"));
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn input_plugins(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let registry = &cx.global::<InputPlugins>().0;
        let scanning = registry.scanning.load(Ordering::Acquire);
        let entries = registry.entries.lock().expect("registry lock").clone();
        let theme = Theme::get(cx);
        let entity = cx.entity().downgrade();
        let mut content = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().mt_4().text_size(px(20.0)).child("入力プラグイン"))
            .child("Windows x64の.aui2に対応しています。追加・更新後は再起動してください。")
            .child(
                Button::new("open-input-folder")
                    .label("プラグインフォルダーを開く")
                    .secondary()
                    .control_size(ControlSize::Sm)
                    .on_click(move |_, cx| match crate::input::directory() {
                        Ok(path) => cx.open_with_system(&path),
                        Err(error) => {
                            let _ = entity.update(cx, |view, cx| {
                                view.error = Some(error.to_string());
                                cx.notify();
                            });
                        }
                    }),
            );
        if scanning {
            content = content.child("プラグインを検索中…");
        } else if entries.is_empty() {
            content = content.child("入力プラグインがありません。");
        }
        for (index, entry) in entries.into_iter().enumerate() {
            let mut row = div()
                .flex()
                .flex_col()
                .gap_1()
                .p_2()
                .border_1()
                .border_color(theme.colors.divider)
                .child(entry.name)
                .child(
                    div()
                        .text_color(theme.colors.text_muted)
                        .child(entry.path.display().to_string()),
                );
            if let Some(error) = entry.error {
                row = row.child(
                    div()
                        .text_color(theme.colors.danger)
                        .child(format!("読み込み失敗: {error}")),
                );
            } else if let Some(client) = entry.client {
                let opened = client.opened.load(Ordering::Relaxed);
                row = row.child(format!(
                    "読み込み済み · {} · 使用中の素材: {opened}",
                    entry.filters
                ));
                if client.can_config {
                    let entity = cx.entity().downgrade();
                    row = row.child(
                        Button::new(format!("configure-input-{index}"))
                            .label("プラグイン設定…")
                            .secondary()
                            .control_size(ControlSize::Sm)
                            .disabled(self.loading || opened > 0)
                            .on_click(move |window, cx| {
                                let _ = entity.update(cx, |view, cx| {
                                    view.configure_input(client.clone(), window, cx)
                                });
                            }),
                    );
                    if opened > 0 {
                        row = row
                            .child("Undo履歴を含めて使用中です。設定するには再起動してください。");
                    }
                }
            }
            content = content.child(row);
        }
        content
    }
}

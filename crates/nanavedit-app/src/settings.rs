use crate::{editor::Editor, theme::Base16Scheme, timeline::ObjectKind};
#[cfg(windows)]
mod input_ui;
use anyhow::{Context as _, Result, ensure};
use gpui::{
    App, AppContext, Context, FocusHandle, Global, IntoElement, Render, Window, WindowHandle, div,
    prelude::*, px,
};
use gpui_kit::{
    prelude::{Button, ControlSize, Disableable, Sizable},
    theme::Theme,
};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::PathBuf,
};

const MAX_SETTINGS_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ObjectColorStyle {
    Original,
    #[default]
    Rainbow,
}

impl ObjectColorStyle {
    pub fn color_index(self, kind: &ObjectKind) -> usize {
        // https://github.com/sevenc-nanashi/tinted-aviutl2/tree/main/templates
        match kind {
            ObjectKind::Input { .. } => 13,
            ObjectKind::Filter => 11,
            ObjectKind::Effect => 12,
            ObjectKind::Camera { .. } => match self {
                Self::Original => 12,
                Self::Rainbow => 14,
            },
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppSettings {
    pub theme: Base16Scheme,
    pub object_colors: ObjectColorStyle,
}

pub struct SettingsState {
    pub current: Option<AppSettings>,
    pub editor: Option<WindowHandle<Editor>>,
    window: Option<WindowHandle<SettingsWindow>>,
    load_error: Option<String>,
}

impl Global for SettingsState {}

pub fn current(cx: &App) -> &AppSettings {
    cx.global::<SettingsState>()
        .current
        .as_ref()
        .expect("editor opens only after settings are loaded")
}

fn config_path() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .context("ユーザー設定ディレクトリを取得できません")?
        .join("nanavedit/settings.json"))
}

fn read_file(path: &std::path::Path) -> Result<String> {
    let file =
        std::fs::File::open(path).with_context(|| format!("{} を開けません", path.display()))?;
    let mut content = String::new();
    file.take(MAX_SETTINGS_BYTES + 1)
        .read_to_string(&mut content)?;
    ensure!(
        content.len() as u64 <= MAX_SETTINGS_BYTES,
        "設定ファイルは1MiB以下にしてください"
    );
    Ok(content)
}

fn load() -> Result<AppSettings> {
    let path = config_path()?;
    match read_file(&path) {
        Ok(content) => serde_json::from_str(&content).context("保存済み設定が不正です"),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            Ok(AppSettings::default())
        }
        Err(error) => Err(error),
    }
}

fn save(settings: &AppSettings) -> Result<()> {
    let path = config_path()?;
    let parent = path.parent().expect("configuration path has a parent");
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut file, settings)?;
    file.write_all(b"\n")?;
    file.as_file().sync_all()?;
    file.persist(&path)
        .with_context(|| format!("{} に保存できません", path.display()))?;
    Ok(())
}

pub fn initialize(cx: &mut App) {
    let (current, load_error) = match load() {
        Ok(settings) => {
            settings.theme.install(cx);
            (Some(settings), None)
        }
        Err(error) => (
            None,
            Some(format!(
                "設定を読み込めませんでした: {error:#}\nYAMLを読み込むか、初期配色へ戻してください。保存済みファイルは変更していません。"
            )),
        ),
    };
    cx.set_global(SettingsState {
        current,
        editor: None,
        window: None,
        load_error,
    });
    cx.on_window_closed(|cx, id| {
        let state = cx.global_mut::<SettingsState>();
        if state.editor.is_some_and(|window| window.window_id() == id) {
            state.editor = None;
            quit(cx);
        } else {
            if state.window.is_some_and(|window| window.window_id() == id) {
                state.window = None;
            }
            if cx.windows().is_empty() {
                quit(cx);
            }
        }
    })
    .detach();
    if cx.global::<SettingsState>().current.is_some() {
        crate::open_editor(cx);
    } else {
        open(cx);
    }
}

fn quit(cx: &mut App) {
    #[cfg(windows)]
    crate::input::quit(cx);
    #[cfg(not(windows))]
    cx.quit();
}

pub fn open(cx: &mut App) {
    if let Some(handle) = cx.global::<SettingsState>().window {
        handle
            .update(cx, |_, window, _| window.activate_window())
            .expect("tracked settings window is open");
        return;
    }
    let bounds = gpui::Bounds::centered(None, gpui::size(px(600.0), px(650.0)), cx);
    let handle = cx
        .open_window(
            gpui::WindowOptions {
                window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
                window_min_size: Some(gpui::size(px(460.0), px(420.0))),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("アプリ設定 — nanavedit".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| SettingsWindow::new(window, cx)),
        )
        .expect("アプリ設定ウィンドウを作成できませんでした");
    cx.global_mut::<SettingsState>().window = Some(handle);
}

struct SettingsWindow {
    focus: FocusHandle,
    loading: bool,
    error: Option<String>,
}

impl SettingsWindow {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            focus,
            loading: false,
            error: cx.global::<SettingsState>().load_error.clone(),
        }
    }

    fn commit(&mut self, settings: AppSettings, cx: &mut Context<Self>) {
        match save(&settings) {
            Ok(()) => {
                settings.theme.install(cx);
                let state = cx.global_mut::<SettingsState>();
                state.current = Some(settings);
                state.load_error = None;
                self.error = None;
                if state.editor.is_none() {
                    crate::open_editor(cx);
                }
                cx.refresh_windows();
            }
            Err(error) => self.error = Some(format!("設定を保存できませんでした: {error:#}")),
        }
        cx.notify();
    }

    fn choose_yaml(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        let paths = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Base16 YAMLを読み込む".into()),
        });
        self.loading = true;
        cx.notify();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let result: Result<Option<Base16Scheme>> = async {
                let Some(paths) = paths.await?? else {
                    return Ok(None);
                };
                let path = paths
                    .into_iter()
                    .next()
                    .context("YAMLが選択されていません")?;
                executor
                    .spawn(async move { Base16Scheme::from_yaml(&read_file(&path)?).map(Some) })
                    .await
            }
            .await;
            let _ = this.update(cx, |view, cx| {
                view.loading = false;
                match result {
                    Ok(Some(theme)) => {
                        let object_colors = match &cx.global::<SettingsState>().current {
                            Some(settings) => settings.object_colors,
                            None => ObjectColorStyle::default(),
                        };
                        view.commit(
                            AppSettings {
                                theme,
                                object_colors,
                            },
                            cx,
                        );
                    }
                    Ok(None) => {}
                    Err(error) => {
                        view.error = Some(format!("テーマを読み込めませんでした: {error:#}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        gpui_kit::semantics::SemanticCoordinator::global(cx).begin_frame(window);
        let theme = Theme::get(cx).clone();
        let settings = cx.global::<SettingsState>().current.clone();
        let entity = cx.entity().downgrade();
        let reset_entity = entity.clone();
        let mut content = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(div().text_size(px(20.0)).child("外観"))
            .child("Base16テーマ")
            .when_some(settings.as_ref(), |content, settings| {
                let mut swatches = div().flex().flex_wrap().gap_2();
                for index in 0..16 {
                    swatches = swatches.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .w(px(56.0))
                            .child(
                                div()
                                    .h(px(24.0))
                                    .bg(settings.theme.palette.paint(index))
                                    .border_1()
                                    .border_color(theme.colors.divider),
                            )
                            .child(format!("base{index:02X}")),
                    );
                }
                content
                    .child(settings.theme.name.clone())
                    .child(
                        div()
                            .text_color(theme.colors.text_muted)
                            .child(settings.theme.author.clone()),
                    )
                    .child(swatches)
            })
            .child(
                Button::new("load-theme")
                    .label(if self.loading {
                        "読み込み中…"
                    } else {
                        "YAMLを読み込む…"
                    })
                    .secondary()
                    .control_size(ControlSize::Sm)
                    .disabled(self.loading)
                    .on_click(move |_, cx| {
                        let _ = entity.update(cx, |view, cx| view.choose_yaml(cx));
                    }),
            )
            .child("オブジェクトの色分け");
        let mut choices = div().flex().gap_2();
        for (id, label, style) in [
            ("original", "Original", ObjectColorStyle::Original),
            ("rainbow", "Rainbow", ObjectColorStyle::Rainbow),
        ] {
            let entity = cx.entity().downgrade();
            choices = choices.child(
                Button::new(id)
                    .label(label)
                    .secondary()
                    .control_size(ControlSize::Sm)
                    .checked_state(settings.as_ref().is_some_and(|s| s.object_colors == style))
                    .when(
                        settings.as_ref().is_some_and(|s| s.object_colors == style),
                        |button| button.primary(),
                    )
                    .disabled(self.loading || settings.is_none())
                    .on_click(move |_, cx| {
                        let _ = entity.update(cx, |view, cx| {
                            let mut settings = current(cx).clone();
                            settings.object_colors = style;
                            view.commit(settings, cx);
                        });
                    }),
            );
        }
        content = content
            .child(choices)
            .child(
                Button::new("reset-theme")
                    .label("初期配色へ戻す")
                    .secondary()
                    .control_size(ControlSize::Sm)
                    .disabled(self.loading)
                    .on_click(move |_, cx| {
                        let _ = reset_entity
                            .update(cx, |view, cx| view.commit(AppSettings::default(), cx));
                    }),
            )
            .child(
                div()
                    .text_color(theme.colors.text_muted)
                    .child("変更は保存後すぐに反映され、次回起動にも引き継がれます。"),
            )
            .when_some(self.error.as_ref(), |content, error| {
                content.child(div().text_color(theme.colors.danger).child(error.clone()))
            });
        #[cfg(windows)]
        {
            content = content.child(self.input_plugins(cx));
        }
        div()
            .id("app-settings")
            .track_focus(&self.focus)
            .tab_group()
            .size_full()
            .bg(theme.colors.canvas)
            .text_color(theme.colors.text)
            .font_family(crate::editor::UI_FONT)
            .text_size(px(13.0))
            .on_key_down(|event, window, cx| {
                if event.keystroke.key == "tab" {
                    if event.keystroke.modifiers.shift {
                        window.focus_prev(cx);
                    } else {
                        window.focus_next(cx);
                    }
                    cx.stop_propagation();
                }
            })
            .child(
                div()
                    .id("app-settings-scroll")
                    .size_full()
                    .overflow_y_scroll()
                    .p_4()
                    .child(content),
            )
    }
}

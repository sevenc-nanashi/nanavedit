use crate::effects::{EffectDefinition, EffectInstance, EffectRole};
use crate::timeline::Timeline;
use effects_ui::CreationTarget;
use gpui::{
    Context, CursorStyle, Div, FocusHandle, IntoElement, MouseButton, Point, Render, Window, div,
    prelude::*, px,
};
use gpui_kit::prelude::{Button, ControlSize, Disableable, Sizable};
use std::{cell::Cell, rc::Rc};
mod effects_ui;
mod timeline_ui;
#[cfg(target_os = "windows")]
mod video_input;
use gpui_kit::{
    semantics::{NodeSpec, Role, Semantic, SemanticCoordinator},
    theme::Theme,
};

const DIVIDER: f32 = 8.0;
const HEADER: f32 = 36.0;
const PADDING: f32 = 24.0;

#[cfg(target_os = "windows")]
pub(crate) const UI_FONT: &str = "Yu Gothic UI";
#[cfg(target_os = "macos")]
pub(crate) const UI_FONT: &str = "Hiragino Sans";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub(crate) const UI_FONT: &str = "Noto Sans CJK JP";

#[derive(Clone, Copy)]
enum Panel {
    Settings,
    Timeline,
}

pub struct Editor {
    settings_width: f32,
    timeline_height: f32,
    focus: FocusHandle,
    settings_focus: FocusHandle,
    timeline_focus: FocusHandle,
    drag: Option<(Panel, Point<gpui::Pixels>)>,
    timeline: Timeline,
    edit_focus: FocusHandle,
    timeline_buttons: [FocusHandle; 6],
    timeline_scroll: gpui::ScrollHandle,
    timeline_layers: usize,
    timeline_bounds: Rc<Cell<gpui::Bounds<gpui::Pixels>>>,
    clip_drag: Option<timeline_ui::Drag>,
    bad_drop: bool,
    object_menu: gpui::Entity<gpui_kit::prelude::ContextMenu>,
    addition_target: Option<(usize, u64)>,
    effect_menu: gpui::Entity<gpui_kit::prelude::ContextMenu>,
    effect_add_target: Option<u64>,
    effect_menu_bounds: Rc<Cell<gpui::Bounds<gpui::Pixels>>>,
    effect_edit: Option<crate::timeline::Document>,
    effect_cancelled: bool,
    pointer_down: bool,
    effect_drag: Option<(u64, &'static str)>,
    effects_focus: FocusHandle,
    number_inputs: std::collections::HashMap<
        (u64, &'static str),
        gpui::Entity<gpui_kit::prelude::NumberInput>,
    >,
    choice_inputs:
        std::collections::HashMap<(u64, &'static str), gpui::Entity<gpui_kit::prelude::Select>>,
    number_edit: Option<(u64, &'static str)>,
    #[cfg(target_os = "windows")]
    preview: Option<crate::preview::Preview>,
    loading: bool,
    preview_error: Option<String>,
    #[cfg(target_os = "windows")]
    gpu_failed: bool,
    #[cfg(target_os = "windows")]
    video_state: video_input::VideoState,
}

impl Editor {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let edit_focus = cx.focus_handle().tab_stop(true).tab_index(3);
        let effects_focus = cx.focus_handle();
        cx.on_focus_out(&effects_focus, window, |editor, _, _, cx| {
            editor.finish_effect_edit(false, cx);
        })
        .detach();
        let object_menu = cx.new(|cx| {
            gpui_kit::prelude::ContextMenu::new("object-menu", window, cx).name("オブジェクト追加")
        });
        cx.subscribe(&object_menu, |editor, _, event, cx| match event {
            gpui_kit::prelude::ContextMenuEvent::Invoked(id) => editor.add_from_menu(id, cx),
            gpui_kit::prelude::ContextMenuEvent::Unavailable(error) => {
                editor.preview_error = Some(format!("メニューを表示できません: {error}"));
                cx.notify();
            }
            _ => {}
        })
        .detach();
        cx.on_blur(&edit_focus, window, |editor, _, cx| {
            editor.finish_clip_drag(true, cx);
        })
        .detach();
        let effect_menu = cx.new(|cx| {
            gpui_kit::prelude::ContextMenu::new("effect-menu", window, cx).name("エフェクトを追加")
        });
        cx.subscribe(&effect_menu, |editor, _, event, cx| match event {
            gpui_kit::prelude::ContextMenuEvent::Invoked(id) => {
                let clip_id = editor
                    .effect_add_target
                    .take()
                    .expect("effect menu has a target");
                editor.start_effect_creation(
                    crate::effects::definition(id),
                    CreationTarget::Effect { clip_id },
                    cx,
                );
            }
            gpui_kit::prelude::ContextMenuEvent::Unavailable(error) => {
                editor.preview_error = Some(format!("メニューを表示できません: {error}"));
                cx.notify();
            }
            _ => {}
        })
        .detach();
        cx.observe_window_activation(window, |editor, window, cx| {
            if !window.is_window_active() {
                editor.finish_clip_drag(true, cx);
                editor.finish_effect_edit(false, cx);
                editor.pointer_down = false;
                editor.effect_drag = None;
                editor.effect_cancelled = false;
                editor.drag = None;
            }
        })
        .detach();
        #[cfg(target_os = "windows")]
        let (preview, preview_error) = {
            let (sender, receiver) = async_channel::bounded(1);
            cx.spawn(async move |this, cx| {
                while let Ok(error) = receiver.recv().await {
                    if this
                        .update(cx, |editor, cx| {
                            editor.preview_error = Some(error);
                            editor.gpu_failed = true;
                            editor.timeline.playing = None;
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
            match crate::preview::Preview::new(window, sender) {
                Ok(preview) => (Some(preview), None),
                Err(error) => (None, Some(format!("GPUの初期化に失敗しました: {error:#}"))),
            }
        };
        #[cfg(not(target_os = "windows"))]
        let preview_error = Some("画像プレビューは現在Windowsのみ対応しています".to_string());
        #[cfg(target_os = "windows")]
        let gpu_failed = preview_error.is_some();
        Self {
            settings_width: 300.0,
            timeline_height: 240.0,
            focus,
            settings_focus: cx.focus_handle().tab_stop(true).tab_index(1),
            timeline_focus: cx.focus_handle().tab_stop(true).tab_index(2),
            drag: None,
            timeline: Timeline::default(),
            edit_focus,
            timeline_buttons: std::array::from_fn(|index| {
                cx.focus_handle()
                    .tab_stop(true)
                    .tab_index(4 + index as isize)
            }),
            timeline_scroll: gpui::ScrollHandle::new(),
            timeline_layers: 32,
            timeline_bounds: Rc::new(Cell::new(gpui::Bounds::default())),
            clip_drag: None,
            bad_drop: false,
            object_menu,
            addition_target: None,
            effect_menu,
            effect_add_target: None,
            effect_menu_bounds: Rc::new(Cell::new(gpui::Bounds::default())),
            effect_edit: None,
            effect_cancelled: false,
            pointer_down: false,
            effect_drag: None,
            effects_focus,
            number_inputs: Default::default(),
            choice_inputs: Default::default(),
            number_edit: None,
            #[cfg(target_os = "windows")]
            preview,
            loading: false,
            preview_error,
            #[cfg(target_os = "windows")]
            gpu_failed,
            #[cfg(target_os = "windows")]
            video_state: Default::default(),
        }
    }

    #[cfg(target_os = "windows")]
    fn open_file_parameter(
        &mut self,
        clip_id: u64,
        effect_id: u64,
        definition: &'static crate::effects::ParamDefinition,
        cx: &mut Context<Self>,
    ) {
        if definition.file == Some(crate::effects::FileKind::Video) {
            self.open_video_parameter(clip_id, effect_id, definition, cx);
            return;
        }
        if self.loading || self.gpu_failed {
            return;
        }
        self.finish_clip_drag(false, cx);
        self.finish_effect_edit(false, cx);
        self.timeline.playing = None;
        let kind = definition.file.expect("file parameter has a kind");
        let limit = self
            .preview
            .as_ref()
            .expect("available preview owns a renderer")
            .max_dimension();
        let paths = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(match kind {
                crate::effects::FileKind::Image => "PNG / JPEG画像を開く".into(),
                crate::effects::FileKind::Video => unreachable!("handled video input"),
            }),
        });
        self.loading = true;
        cx.notify();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let result: anyhow::Result<Option<(std::path::PathBuf, image::RgbaImage)>> = async {
                let Some(paths) = paths.await?? else {
                    return Ok(None);
                };
                let path = paths
                    .into_iter()
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("画像が選択されていません"))?;
                executor
                    .spawn(async move {
                        match kind {
                            crate::effects::FileKind::Image => {
                                crate::preview::load_image(&path, limit)
                                    .map(|image| Some((path, image)))
                            }
                            crate::effects::FileKind::Video => unreachable!("handled video input"),
                        }
                    })
                    .await
            }
            .await;
            let _ = this.update(cx, |editor, cx| {
                editor.loading = false;
                // A delayed decode must not overwrite a device-loss notification.
                if !editor.gpu_failed {
                    match result {
                        Ok(Some((path, image))) => {
                            let id = editor.timeline.allocate_id();
                            let asset = crate::timeline::ImageAsset {
                                id,
                                name: path
                                    .file_name()
                                    .expect("selected file has a name")
                                    .to_string_lossy()
                                    .into_owned(),
                                width: image.width(),
                                height: image.height(),
                            };
                            match editor
                                .preview
                                .as_ref()
                                .expect("available preview owns a renderer")
                                .register_image(id, image)
                            {
                                Ok(()) => {
                                    let before = editor.timeline.document.clone();
                                    let effect = editor
                                        .timeline
                                        .document
                                        .clips
                                        .iter_mut()
                                        .find(|clip| clip.id == clip_id)
                                        .expect("file target exists while editing is disabled")
                                        .instance_mut(effect_id);
                                    effect
                                        .params
                                        .iter_mut()
                                        .find(|param| param.id == definition.id)
                                        .expect("file parameter is declared")
                                        .value = crate::effects::ParamValue::File(Some(
                                        std::sync::Arc::new(crate::effects::FileAsset {
                                            path,
                                            resource: crate::effects::Resource::Image(
                                                std::sync::Arc::new(asset),
                                            ),
                                        }),
                                    ));
                                    editor.timeline.commit(before);
                                    editor.sync_timeline();
                                    editor.preview_error = None;
                                }
                                Err(error) => {
                                    editor.preview_error =
                                        Some(format!("画像の表示に失敗しました: {error:#}"))
                                }
                            }
                        }
                        Ok(None) => {}
                        Err(error) => {
                            editor.preview_error =
                                Some(format!("画像を読み込めませんでした: {error:#}"))
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn preview_header(&self) -> impl IntoElement {
        div()
            .flex()
            .w_full()
            .justify_between()
            .child("プレビュー")
            .when(self.loading, |header| header.child("読み込み中…"))
            .when(
                {
                    #[cfg(target_os = "windows")]
                    {
                        self.video_state.busy
                    }
                    #[cfg(not(target_os = "windows"))]
                    {
                        false
                    }
                },
                |header| header.child("映像を取得中…"),
            )
            .child(
                Button::new("app-settings")
                    .label("アプリ設定")
                    .secondary()
                    .control_size(ControlSize::Sm)
                    .on_click(|_, cx| crate::settings::open(cx)),
            )
    }

    fn preview_content(&self, frame_width: f32, theme: &Theme) -> impl IntoElement {
        let frame = div()
            .w(px(frame_width))
            .h(px(frame_width * 9.0 / 16.0))
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::black())
            .text_center()
            .text_color(theme.colors.text_muted);
        #[cfg(target_os = "windows")]
        let frame = if !self.gpu_failed {
            frame.child(
                gpui::platform_view(
                    self.preview
                        .as_ref()
                        .expect("available preview owns a renderer")
                        .handle
                        .clone(),
                )
                .size_full(),
            )
        } else {
            frame.child("プレビューを利用できません")
        };
        #[cfg(not(target_os = "windows"))]
        let frame = frame.child("このOSではプレビュー未対応です");
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(theme.colors.sunken)
            .when_some(self.preview_error.clone(), |element, error| {
                element.child(
                    div()
                        .id("preview-error")
                        .h(px(40.0))
                        .flex_none()
                        .px_2()
                        .overflow_y_scroll()
                        .child(error),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(frame),
            )
    }

    fn constrain(&mut self, window: &Window) {
        let viewport = window.viewport_size();
        self.settings_width = self.settings_width.clamp(
            220.0,
            (f32::from(viewport.width) - DIVIDER - 400.0).max(220.0),
        );
        self.timeline_height = self.timeline_height.clamp(
            160.0,
            (f32::from(viewport.height) - DIVIDER - 160.0).max(160.0),
        );
    }

    fn resize(&mut self, panel: Panel, delta: f32, window: &Window, cx: &mut Context<Self>) {
        match panel {
            Panel::Settings => self.settings_width += delta,
            Panel::Timeline => self.timeline_height += delta,
        }
        self.constrain(window);
        cx.notify();
    }

    fn divider(&self, panel: Panel, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let (id, label, focus, cursor, orientation) = match panel {
            Panel::Settings => (
                "settings-divider",
                "設定パネルの幅を変更",
                &self.settings_focus,
                CursorStyle::ResizeLeftRight,
                gpui::accesskit::Orientation::Vertical,
            ),
            Panel::Timeline => (
                "timeline-divider",
                "タイムラインの高さを変更",
                &self.timeline_focus,
                CursorStyle::ResizeUpDown,
                gpui::accesskit::Orientation::Horizontal,
            ),
        };

        // SplitPaneは比率を保持するため、ここでは固定ピクセル寸法の境界を使う。
        div()
            .id(id)
            .track_focus(focus)
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .cursor(cursor)
            .bg(theme.colors.canvas)
            .hover(|style| style.bg(theme.colors.hover))
            .focus(|style| style.bg(theme.colors.focus))
            .when(matches!(panel, Panel::Settings), |element| {
                element.w(px(DIVIDER)).h_full()
            })
            .when(matches!(panel, Panel::Timeline), |element| {
                element.h(px(DIVIDER)).w_full()
            })
            .child(
                div()
                    .bg(theme.colors.divider)
                    .when(matches!(panel, Panel::Settings), |element| {
                        element.w(px(2.0)).h(px(24.0))
                    })
                    .when(matches!(panel, Panel::Timeline), |element| {
                        element.h(px(2.0)).w(px(24.0))
                    }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |editor, event: &gpui::MouseDownEvent, window, cx| {
                    match panel {
                        Panel::Settings => editor.settings_focus.focus(window, cx),
                        Panel::Timeline => editor.timeline_focus.focus(window, cx),
                    }
                    editor.drag = Some((panel, event.position));
                    cx.notify();
                }),
            )
            .on_key_down(
                cx.listener(move |editor, event: &gpui::KeyDownEvent, window, cx| {
                    let delta = match (panel, event.keystroke.key.as_str()) {
                        (Panel::Settings, "left") | (Panel::Timeline, "up") => 10.0,
                        (Panel::Settings, "right") | (Panel::Timeline, "down") => -10.0,
                        _ => return,
                    };
                    editor.resize(panel, delta, window, cx);
                    cx.stop_propagation();
                }),
            )
            .semantic_in(
                cx,
                NodeSpec::new(id, Role::Splitter)
                    .text(label)
                    .orientation(orientation),
            )
    }
}

fn panel(title: impl IntoElement, theme: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .size_full()
        .min_w_0()
        .min_h_0()
        .overflow_hidden()
        .bg(theme.colors.panel)
        .child(
            div()
                .h(px(HEADER))
                .flex_none()
                .flex()
                .items_center()
                .px(px(16.0))
                .border_b_1()
                .border_color(theme.colors.hairline)
                .child(title),
        )
}

fn empty_message(message: &'static str, theme: &Theme) -> Div {
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .items_center()
        .justify_center()
        .p(px(PADDING))
        .text_center()
        .text_color(theme.colors.text_muted)
        .child(message)
}

impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        SemanticCoordinator::global(cx).begin_frame(window);
        self.scroll_clip_drag(window, cx);
        self.timeline.tick();
        #[cfg(target_os = "windows")]
        self.update_preview(cx);
        if self.timeline.playing.is_some() {
            window.request_animation_frame();
        }
        self.constrain(window);
        let theme = Theme::get(cx).clone();
        let viewport = window.viewport_size();
        let preview_width = f32::from(viewport.width) - self.settings_width - DIVIDER;
        let top_height = f32::from(viewport.height) - self.timeline_height - DIVIDER;
        let error_height = if self.preview_error.is_some() {
            40.0
        } else {
            0.0
        };
        let frame_width = (preview_width - PADDING * 2.0)
            .min((top_height - HEADER - error_height - PADDING * 2.0) * 16.0 / 9.0)
            .max(0.0);

        div()
            .id("editor")
            .track_focus(&self.focus)
            .tab_group()
            .tab_stop(false)
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(theme.colors.canvas)
            .text_color(theme.colors.text)
            .font_family(UI_FONT)
            .text_size(px(13.0))
            .capture_key_down(|event, _, cx| {
                if event.keystroke.modifiers.control && event.keystroke.key == "," {
                    crate::settings::open(cx);
                    cx.stop_propagation();
                }
            })
            .capture_any_mouse_down(cx.listener(|editor, event: &gpui::MouseDownEvent, _, cx| {
                editor.finish_effect_edit(false, cx);
                editor.pointer_down = event.button == MouseButton::Left;
                editor.effect_drag = None;
                editor.effect_cancelled = false;
            }))
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
            .on_mouse_move(
                cx.listener(|editor, event: &gpui::MouseMoveEvent, window, cx| {
                    editor.move_clip_drag(event, cx);
                    if event.pressed_button != Some(MouseButton::Left) {
                        if editor.number_edit.is_none() {
                            editor.finish_effect_edit(false, cx);
                        }
                        editor.pointer_down = false;
                        editor.effect_drag = None;
                        editor.effect_cancelled = false;
                    }
                    if let Some((panel, previous)) = editor.drag {
                        if event.pressed_button != Some(MouseButton::Left) {
                            editor.drag = None;
                            return;
                        }
                        let delta = match panel {
                            Panel::Settings => previous.x - event.position.x,
                            Panel::Timeline => previous.y - event.position.y,
                        };
                        editor.drag = Some((panel, event.position));
                        editor.resize(panel, delta.into(), window, cx);
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|editor, _, _, cx| {
                    editor.drag = None;
                    editor.finish_clip_drag(false, cx);
                    if editor.number_edit.is_none() {
                        editor.finish_effect_edit(false, cx);
                    }
                    editor.effect_cancelled = false;
                    editor.pointer_down = false;
                    editor.effect_drag = None;
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|editor, _, _, cx| {
                    editor.drag = None;
                    editor.finish_clip_drag(false, cx);
                    if editor.number_edit.is_none() {
                        editor.finish_effect_edit(false, cx);
                    }
                    editor.effect_cancelled = false;
                    editor.pointer_down = false;
                    editor.effect_drag = None;
                }),
            )
            .child(
                div()
                    .flex()
                    .w_full()
                    .h(px(top_height))
                    .flex_none()
                    .child(
                        div().w(px(preview_width)).h_full().flex_none().child(
                            panel(self.preview_header(), &theme)
                                .child(self.preview_content(frame_width, &theme)),
                        ),
                    )
                    .child(self.divider(Panel::Settings, cx))
                    .child(div().w(px(self.settings_width)).h_full().flex_none().child(
                        panel("設定", &theme).child(self.selection_details(&theme, window, cx)),
                    )),
            )
            .child(self.object_menu.clone())
            .child(self.effect_menu.clone())
            .child(self.divider(Panel::Timeline, cx))
            .child(
                div()
                    .w_full()
                    .h(px(self.timeline_height))
                    .flex_none()
                    .child(
                        panel(self.timeline_toolbar(cx), &theme).child(self.timeline_content(cx)),
                    ),
            )
    }
}

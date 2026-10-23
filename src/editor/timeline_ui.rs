use super::*;
use crate::timeline::{Clip, Document, FPS, ObjectKind};
use gpui::{Bounds, Pixels, canvas, point};
use gpui_kit::prelude::ScrollArea;
use std::time::Instant;

const LABEL: f32 = 100.0;
const RULER: f32 = 28.0;
const ROW: f32 = 40.0;

fn timeline_line(color: gpui::Hsla) -> impl IntoElement + gpui::Styled {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            // ScrollArea fades primitives at their centers; keep long lines' centers in view.
            let visible = bounds.intersect(&window.content_mask().bounds);
            if !visible.is_empty() {
                window.paint_quad(gpui::fill(visible, color));
            }
        },
    )
    .absolute()
}

#[derive(Clone, Copy)]
pub(super) enum Edit {
    Move,
    Start,
    End,
}

pub(super) enum Drag {
    Seek,
    Clip {
        before: Document,
        original: Box<Clip>,
        x: f32,
        edit: Edit,
        scroll_at: Instant,
    },
}

#[derive(Clone, Copy)]
enum Command {
    Play,
    Undo,
    Redo,
    Delete,
    ZoomIn,
    ZoomOut,
}

impl Editor {
    pub(super) fn sync_preview(&self) {
        #[cfg(target_os = "windows")]
        if !self.gpu_failed
            && let Some(preview) = &self.preview
        {
            preview.set_objects(self.timeline.active_objects(), self.timeline.frame);
        }
    }

    pub(super) fn sync_timeline(&self) {
        self.sync_preview();
        #[cfg(target_os = "windows")]
        if let Some(preview) = &self.preview {
            preview.retain_images(&self.timeline.retained_images());
        }
    }

    fn command(&mut self, command: Command, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.finish_clip_drag(false, cx);
        self.finish_effect_edit(false, cx);
        if !matches!(command, Command::Play) {
            self.timeline.playing = None;
        }
        match command {
            Command::Play => self.timeline.toggle_playback(),
            Command::Undo => self.timeline.undo(false),
            Command::Redo => self.timeline.undo(true),
            Command::Delete => self.timeline.delete_selected(),
            Command::ZoomIn | Command::ZoomOut => {
                let old = self.timeline.zoom;
                let new = (old
                    * if matches!(command, Command::ZoomIn) {
                        2.0
                    } else {
                        0.5
                    })
                .clamp(1.0, 16.0);
                let mut offset = self.timeline_scroll.offset();
                offset.x -= px(self.timeline.frame as f32 * (new - old));
                offset.x = offset.x.min(px(0.0));
                self.timeline.zoom = new;
                self.timeline_scroll.set_offset(offset);
            }
        }
        self.sync_timeline();
        cx.notify();
    }

    pub(super) fn finish_clip_drag(&mut self, cancel: bool, cx: &mut Context<Self>) {
        if let Some(drag) = self.clip_drag.take() {
            if let Drag::Clip { before, .. } = drag {
                if cancel {
                    self.timeline.document = before;
                    if let Some(clip) = self.timeline.selected_clip() {
                        self.timeline.layer = clip.layer;
                    }
                } else {
                    self.timeline.commit(before);
                }
            }
            self.bad_drop = false;
            self.sync_timeline();
            cx.notify();
        }
    }

    fn local_position(&self, position: Point<Pixels>) -> Point<f32> {
        let origin = self.timeline_bounds.get().origin;
        point(
            f32::from(position.x - origin.x),
            f32::from(position.y - origin.y),
        )
    }

    fn open_object_menu(
        &mut self,
        position: Point<Pixels>,
        target: (usize, u64),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.loading {
            return;
        }
        self.finish_clip_drag(false, cx);
        self.finish_effect_edit(false, cx);
        self.timeline.playing = None;
        self.addition_target = Some(target);
        let mut items: Vec<_> = crate::effects::definitions()
            .iter()
            .filter(|definition| definition.info.role == EffectRole::Input)
            .map(|definition| {
                gpui_kit::prelude::MenuItem::command(
                    format!("input:{}", definition.info.id),
                    format!("{}を追加", definition.info.name),
                )
                .disabled(self.loading)
            })
            .collect();
        items.extend([
            gpui_kit::prelude::MenuItem::command("filter", "フィルタオブジェクトを追加"),
            gpui_kit::prelude::MenuItem::command("effect", "エフェクトオブジェクトを追加"),
            gpui_kit::prelude::MenuItem::command("camera", "カメラ制御を追加"),
        ]);
        let result = self.object_menu.update(cx, |menu, cx| {
            menu.set_items(items, window, cx)?;
            menu.open_at(position, window, cx)
        });
        if let Err(error) = result {
            self.preview_error = Some(format!("メニューを表示できません: {error}"));
        }
        cx.notify();
    }

    pub(super) fn add_from_menu(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        let (layer, frame) = self
            .addition_target
            .take()
            .expect("menu has an insertion target");
        if let Some(id) = id.strip_prefix("input:") {
            self.start_effect_creation(
                crate::effects::definition(id),
                CreationTarget::Input { layer, frame },
                cx,
            );
        } else {
            match id {
                "filter" => self.timeline.add_object(ObjectKind::Filter, layer, frame),
                "effect" => self.timeline.add_object(ObjectKind::Effect, layer, frame),
                "camera" => {
                    let settings = EffectInstance::new(
                        self.timeline.allocate_id(),
                        crate::effects::definition("builtin.camera"),
                    );
                    self.timeline
                        .add_object(ObjectKind::Camera { settings }, layer, frame);
                }
                _ => unreachable!("known object menu command"),
            }
        }
        self.sync_timeline();
        cx.notify();
    }

    fn seek(&mut self, x: f32) {
        self.timeline.playing = None;
        self.timeline.frame = ((x - LABEL) / self.timeline.zoom).round().max(0.0) as u64;
    }

    fn begin_clip_drag(
        &mut self,
        event: &gpui::MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.loading {
            return;
        }
        self.edit_focus.focus(window, cx);
        self.finish_effect_edit(false, cx);
        self.timeline.playing = None;
        let position = self.local_position(event.position);
        if position.y < RULER {
            if position.x >= LABEL {
                self.seek(position.x);
                self.clip_drag = Some(Drag::Seek);
            }
        } else {
            let layer = ((position.y - RULER) / ROW).floor() as usize;
            self.timeline.layer = layer;
            let x = position.x - LABEL;
            let zoom = self.timeline.zoom;
            let clip = self
                .timeline
                .document
                .clips
                .iter()
                .find(|c| {
                    c.layer == layer && x >= c.start as f32 * zoom && x < c.end() as f32 * zoom
                })
                .cloned();
            self.timeline.selected = clip.as_ref().map(|c| c.id);
            if let Some(original) = clip {
                let edge = (original.length as f32 * zoom / 3.0).min(6.0);
                let edit = if x - original.start as f32 * zoom < edge {
                    Edit::Start
                } else if original.end() as f32 * zoom - x < edge {
                    Edit::End
                } else {
                    Edit::Move
                };
                self.clip_drag = Some(Drag::Clip {
                    before: self.timeline.document.clone(),
                    original: Box::new(original),
                    x: position.x,
                    edit,
                    scroll_at: Instant::now(),
                });
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn move_clip_drag(&mut self, event: &gpui::MouseMoveEvent, cx: &mut Context<Self>) {
        if self.clip_drag.is_none() {
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            self.finish_clip_drag(false, cx);
            return;
        }
        let position = self.local_position(event.position);
        self.update_clip_drag(position, cx);
    }

    pub(super) fn scroll_clip_drag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Drag::Clip {
            edit: Edit::Move,
            scroll_at,
            ..
        }) = self.clip_drag.as_mut()
        else {
            return;
        };
        let now = Instant::now();
        let elapsed = now.duration_since(*scroll_at).as_secs_f32().min(0.05);
        *scroll_at = now;
        let mouse = window.mouse_position();
        let bounds = self.timeline_scroll.bounds();
        if mouse.x < bounds.left()
            || mouse.x >= bounds.right()
            || mouse.y < bounds.top()
            || mouse.y >= bounds.bottom()
        {
            return;
        }
        let direction = if mouse.y >= bounds.bottom() - px(ROW) {
            1.0
        } else if mouse.y < bounds.top() + px(ROW) {
            -1.0
        } else {
            return;
        };
        let mut offset = self.timeline_scroll.offset();
        let height = RULER + self.timeline_layers as f32 * ROW;
        let limit = (height - f32::from(bounds.size.height)).max(0.0);
        let next = (f32::from(offset.y) - direction * ROW * 8.0 * elapsed).clamp(-limit, 0.0);
        let delta = f32::from(offset.y) - next;
        if delta != 0.0 {
            let mut position = self.local_position(mouse);
            position.y += delta;
            offset.y = px(next);
            self.timeline_scroll.set_offset(offset);
            self.update_clip_drag(position, cx);
            window.request_animation_frame();
        }
    }

    fn update_clip_drag(&mut self, position: Point<f32>, cx: &mut Context<Self>) {
        match self.clip_drag.as_ref().expect("active drag") {
            Drag::Seek => self.seek(position.x),
            Drag::Clip {
                original, x, edit, ..
            } => {
                let delta = ((position.x - x) / self.timeline.zoom).round() as i64;
                let mut clip = original.as_ref().clone();
                match edit {
                    Edit::Move => {
                        clip.start = original.start.saturating_add_signed(delta);
                        if position.y < RULER {
                            self.bad_drop = true;
                            cx.notify();
                            return;
                        }
                        clip.layer = ((position.y - RULER) / ROW).floor() as usize;
                    }
                    Edit::Start => {
                        clip.start = original
                            .start
                            .saturating_add_signed(delta)
                            .min(original.end() - 1);
                        clip.length = original.end() - clip.start;
                    }
                    Edit::End => {
                        clip.length = original
                            .end()
                            .saturating_add_signed(delta)
                            .max(original.start + 1)
                            - original.start;
                    }
                }
                self.bad_drop = !self.timeline.place(clip);
            }
        }
        cx.notify();
    }

    pub(super) fn timeline_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut toolbar = div()
            .flex()
            .w_full()
            .items_center()
            .gap_1()
            .overflow_hidden()
            .capture_key_down(|event, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space")
                    && (event.is_held || event.keystroke.modifiers.modified())
                {
                    window.prevent_default();
                    cx.stop_propagation();
                }
            })
            .capture_key_up(|event, window, cx| {
                // shortcut: Kit 0.2.0 activates on keydown and GPUI again on keyup; remove after upstream fixes it.
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    window.prevent_default();
                    cx.stop_propagation();
                }
            });
        let buttons = [
            (
                "timeline-play",
                if self.timeline.playing.is_some() {
                    "停止"
                } else {
                    "再生"
                },
                Command::Play,
                self.timeline.end() == 0,
            ),
            (
                "timeline-undo",
                "戻す",
                Command::Undo,
                !self.timeline.can_undo(),
            ),
            (
                "timeline-redo",
                "やり直す",
                Command::Redo,
                !self.timeline.can_redo(),
            ),
            (
                "timeline-delete",
                "削除",
                Command::Delete,
                self.timeline.selected.is_none(),
            ),
            (
                "timeline-zoom-out",
                "−",
                Command::ZoomOut,
                self.timeline.zoom <= 1.0,
            ),
            (
                "timeline-zoom-in",
                "＋",
                Command::ZoomIn,
                self.timeline.zoom >= 16.0,
            ),
        ];
        for (index, (id, label, command, disabled)) in buttons.into_iter().enumerate() {
            let entity = cx.entity().downgrade();
            toolbar = toolbar.child(
                Button::new(id)
                    .label(label)
                    .secondary()
                    .control_size(ControlSize::Sm)
                    .track_focus(&self.timeline_buttons[index])
                    .disabled(self.loading || disabled)
                    .on_click(move |window, cx| {
                        let _ = entity.update(cx, |editor, cx| {
                            editor.timeline_buttons[index].focus(window, cx);
                            editor.command(command, cx);
                        });
                    }),
            );
        }
        toolbar.child(div().ml_2().whitespace_nowrap().child(format!(
            "{:02}:{:02}:{:02}  /  {}f · 30fps",
            self.timeline.frame / FPS / 60,
            self.timeline.frame / FPS % 60,
            self.timeline.frame % FPS,
            self.timeline.frame,
        )))
    }

    pub(super) fn timeline_content(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let appearance = crate::settings::current(cx);
        let palette = &appearance.theme.palette;
        let viewport_height = self.timeline_height - HEADER - theme.measures.scrollbar_track;
        let top = -f32::from(self.timeline_scroll.offset().y);
        let first_layer = (((top - RULER).max(0.0) / ROW).floor() as usize).saturating_sub(1);
        let last_layer = ((top + viewport_height - RULER).max(0.0) / ROW).ceil() as usize + 1;
        // Keep empty rows ahead of the viewport without adding them to document history.
        self.timeline_layers = self.timeline_layers.max(last_layer + 32);
        for clip in &self.timeline.document.clips {
            self.timeline_layers = self.timeline_layers.max(clip.layer + 1);
        }
        let zoom = self.timeline.zoom;
        let frames = self
            .timeline
            .end()
            .max(self.timeline.frame + FPS * 5)
            .max(FPS * 30);
        let width = LABEL + frames as f32 * zoom;
        let height = RULER + self.timeline_layers as f32 * ROW;
        let bounds = self.timeline_bounds.clone();
        let mut content = div()
            .relative()
            .w(px(width))
            .h(px(height))
            .flex_none()
            .child(
                canvas(
                    move |measured: Bounds<Pixels>, window, _| {
                        // Scrollbar geometry is read before this frame's content layout.
                        if bounds.get().size != measured.size {
                            window.request_animation_frame();
                        }
                        bounds.set(measured);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            );
        let offset = -f32::from(self.timeline_scroll.offset().x);
        let viewport = f32::from(self.timeline_scroll.bounds().size.width).max(1280.0);
        let first_second = ((offset - LABEL).max(0.0) / zoom / FPS as f32).floor() as u64;
        let last_second =
            (((offset + viewport) / zoom / FPS as f32).ceil() as u64).min(frames / FPS);
        for second in first_second..=last_second {
            content = content.child(
                div()
                    .absolute()
                    .left(px(LABEL + second as f32 * FPS as f32 * zoom))
                    .top_0()
                    .h(px(height))
                    .w(px(1.0))
                    .child(timeline_line(theme.colors.hairline).size_full())
                    .child(
                        div()
                            .pl_1()
                            .w(px(80.0))
                            .h(px(RULER))
                            .text_color(theme.colors.text_muted)
                            .child(format!("{second}s")),
                    ),
            );
        }
        for layer in first_layer..last_layer {
            content = content.child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(RULER + layer as f32 * ROW))
                    .w(px(width))
                    .h(px(ROW))
                    .child(
                        timeline_line(theme.colors.hairline)
                            .bottom_0()
                            .w_full()
                            .h(px(1.0)),
                    )
                    .child(
                        div()
                            .w(px(LABEL))
                            .h_full()
                            .px_2()
                            .flex()
                            .items_center()
                            .bg(if layer == self.timeline.layer {
                                theme.colors.hover
                            } else {
                                theme.colors.panel
                            })
                            .child(format!("Layer {}", layer + 1)),
                    ),
            );
        }
        for clip in &self.timeline.document.clips {
            if !(first_layer..last_layer).contains(&clip.layer) {
                continue;
            }
            let clip_width = clip.length as f32 * zoom;
            let selected = self.timeline.selected == Some(clip.id);
            let edge = (clip_width / 3.0).min(6.0);
            content = content.child(
                div()
                    .id(gpui::SharedString::from(format!("clip-{}", clip.id)))
                    .absolute()
                    .left(px(LABEL + clip.start as f32 * zoom))
                    .top(px(RULER + clip.layer as f32 * ROW + 3.0))
                    .w(px(clip_width))
                    .h(px(ROW - 6.0))
                    .overflow_hidden()
                    .rounded_sm()
                    .bg(palette.object(appearance.object_colors.color_index(&clip.kind), selected))
                    .border_1()
                    .border_color(if selected {
                        palette.paint(6)
                    } else {
                        theme.colors.divider
                    })
                    .cursor(if self.bad_drop {
                        CursorStyle::OperationNotAllowed
                    } else {
                        CursorStyle::OpenHand
                    })
                    .child(
                        div()
                            .px_2()
                            .whitespace_nowrap()
                            .child(clip.name().to_owned()),
                    )
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top_0()
                            .h_full()
                            .w(px(edge))
                            .cursor(if self.bad_drop {
                                CursorStyle::OperationNotAllowed
                            } else {
                                CursorStyle::ResizeLeftRight
                            }),
                    )
                    .child(
                        div()
                            .absolute()
                            .right_0()
                            .top_0()
                            .h_full()
                            .w(px(edge))
                            .cursor(if self.bad_drop {
                                CursorStyle::OperationNotAllowed
                            } else {
                                CursorStyle::ResizeLeftRight
                            }),
                    )
                    .semantic_in(
                        cx,
                        NodeSpec::new(format!("clip-{}", clip.id), Role::Row)
                            .text(format!(
                                "{}、レイヤー{}、開始{}、長さ{}フレーム",
                                clip.name(),
                                clip.layer + 1,
                                clip.start,
                                clip.length
                            ))
                            .selected(selected),
                    ),
            );
        }
        content = content.child(
            div()
                .absolute()
                .left(px(LABEL + self.timeline.frame as f32 * zoom))
                .top_0()
                .w(px(2.0))
                .h(px(height))
                .child(timeline_line(palette.paint(8)).size_full()),
        );
        let selected = self
            .timeline
            .selected_clip()
            .map(|c| c.name())
            .unwrap_or("なし");
        let viewport = div()
            .id("timeline-editor")
            .w_full()
            .h(px(viewport_height))
            .overflow_scroll()
            .track_scroll(&self.timeline_scroll)
            .track_focus(&self.edit_focus)
            .cursor(if self.bad_drop {
                CursorStyle::OperationNotAllowed
            } else {
                CursorStyle::Arrow
            })
            .on_mouse_down(MouseButton::Left, cx.listener(Self::begin_clip_drag))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|editor, event: &gpui::MouseDownEvent, window, cx| {
                    let local = editor.local_position(event.position);
                    if local.x < LABEL || local.y < RULER {
                        return;
                    }
                    let layer = ((local.y - RULER) / ROW).floor() as usize;
                    let frame = ((local.x - LABEL) / editor.timeline.zoom).round() as u64;
                    editor.open_object_menu(event.position, (layer, frame), window, cx);
                    cx.stop_propagation();
                }),
            )
            .on_key_down(
                cx.listener(|editor, event: &gpui::KeyDownEvent, window, cx| {
                    if editor.loading {
                        return;
                    }
                    let key = event.keystroke.key.as_str();
                    if key == "f10" && event.keystroke.modifiers.shift {
                        let bounds = editor.timeline_scroll.bounds();
                        let position = bounds.origin + point(px(LABEL), px(RULER));
                        editor.open_object_menu(
                            position,
                            (editor.timeline.layer, editor.timeline.frame),
                            window,
                            cx,
                        );
                        window.prevent_default();
                    } else if key == "escape" {
                        editor.finish_clip_drag(true, cx);
                    } else if event.keystroke.modifiers.control && key == "z" {
                        editor.command(
                            if event.keystroke.modifiers.shift {
                                Command::Redo
                            } else {
                                Command::Undo
                            },
                            cx,
                        );
                    } else if event.keystroke.modifiers.control && key == "y" {
                        editor.command(Command::Redo, cx);
                    } else if key == "space" {
                        editor.command(Command::Play, cx);
                    } else if key == "delete" || key == "backspace" {
                        editor.command(Command::Delete, cx);
                    } else {
                        let frame = match key {
                            "left" => editor.timeline.frame.saturating_sub(1),
                            "right" => editor.timeline.frame.saturating_add(1),
                            "home" => 0,
                            "end" => editor.timeline.end().saturating_sub(1),
                            _ => return,
                        };
                        editor.finish_clip_drag(false, cx);
                        editor.timeline.playing = None;
                        editor.timeline.frame = frame;
                        cx.notify();
                    }
                    cx.stop_propagation();
                }),
            )
            .child(content)
            .semantic_in(
                cx,
                NodeSpec::new("timeline-editor", Role::Group)
                    .text("タイムライン")
                    .value(format!(
                        "{}フレーム、レイヤー{}、選択: {selected}",
                        self.timeline.frame,
                        self.timeline.layer + 1
                    )),
            );
        ScrollArea::new("timeline-scroll")
            .both()
            .bound_to(self.timeline_scroll.clone())
            .child(viewport)
    }
}

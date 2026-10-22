use super::*;
use crate::timeline::{Effect, EffectKind, FPS, ObjectKind};
use gpui_kit::prelude::Slider;

#[derive(Clone)]
enum EffectCommand {
    Add(EffectKind),
    Toggle(u64),
    Remove(u64),
    Move(u64, isize),
}

impl Editor {
    pub(super) fn finish_effect_edit(&mut self, cancel: bool, cx: &mut Context<Self>) {
        if let Some(before) = self.effect_edit.take() {
            if cancel {
                self.timeline.document = before;
                self.effect_cancelled = self.pointer_down;
            } else {
                self.timeline.commit(before);
            }
            self.sync_timeline();
            cx.notify();
        }
    }

    fn effect_command(&mut self, command: EffectCommand, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.finish_effect_edit(false, cx);
        self.timeline.playing = None;
        let before = self.timeline.document.clone();
        let new_id = if matches!(command, EffectCommand::Add(_)) {
            Some(self.timeline.allocate_id())
        } else {
            None
        };
        let Some(clip) = self
            .timeline
            .document
            .clips
            .iter_mut()
            .find(|c| Some(c.id) == self.timeline.selected)
        else {
            return;
        };
        match command {
            EffectCommand::Add(kind) => clip.effects.push(Effect {
                id: new_id.expect("allocated effect ID"),
                kind,
                enabled: true,
            }),
            EffectCommand::Toggle(id) => {
                let effect = clip
                    .effects
                    .iter_mut()
                    .find(|e| e.id == id)
                    .expect("selected effect exists");
                effect.enabled = !effect.enabled;
            }
            EffectCommand::Remove(id) => clip.effects.retain(|e| e.id != id),
            EffectCommand::Move(id, delta) => {
                let index = clip
                    .effects
                    .iter()
                    .position(|e| e.id == id)
                    .expect("selected effect exists");
                let next = index
                    .checked_add_signed(delta)
                    .expect("enabled move has a destination");
                clip.effects.swap(index, next);
            }
        }
        self.timeline.commit(before);
        self.sync_timeline();
        cx.notify();
    }

    fn effect_button(
        &self,
        id: String,
        label: &str,
        command: EffectCommand,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> Button {
        let entity = cx.entity().downgrade();
        Button::new(gpui::SharedString::from(id))
            .label(label.to_owned())
            .secondary()
            .control_size(ControlSize::Sm)
            .disabled(self.loading || disabled)
            .on_click(move |_, cx| {
                let _ = entity.update(cx, |editor, cx| editor.effect_command(command.clone(), cx));
            })
    }

    pub(super) fn selection_details(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(clip) = self.timeline.selected_clip() else {
            return empty_message("オブジェクトが選択されていません", theme).into_any_element();
        };
        let mut details = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .child(clip.name().to_owned())
            .when_some(clip.image(), |element, image| {
                element.child(format!("{} × {} px", image.width, image.height))
            })
            .child(format!("レイヤー {}", clip.layer + 1))
            .child(format!("開始: {} フレーム", clip.start))
            .child(format!(
                "長さ: {} フレーム（{:.2}秒）",
                clip.length,
                clip.length as f64 / FPS as f64
            ));
        if matches!(clip.kind, ObjectKind::Effect) {
            let target = self
                .timeline
                .effect_target(clip.id)
                .and_then(|id| self.timeline.document.clips.iter().find(|c| c.id == id));
            details = details.child(match target {
                Some(target) => format!("適用対象: {}", target.name()),
                None => "適用対象なし".to_owned(),
            });
        }
        details = details.child(
            div()
                .flex()
                .gap_1()
                .child(self.effect_button(
                    "add-opacity".into(),
                    "不透明度を追加",
                    EffectCommand::Add(EffectKind::Opacity(100)),
                    false,
                    cx,
                ))
                .child(self.effect_button(
                    "add-invert".into(),
                    "色反転を追加",
                    EffectCommand::Add(EffectKind::Invert),
                    false,
                    cx,
                )),
        );
        for (index, effect) in clip.effects.iter().enumerate() {
            let id = effect.id;
            let mut card = div()
                .flex()
                .flex_col()
                .gap_2()
                .p_2()
                .border_1()
                .border_color(theme.colors.divider)
                .child(format!("{}. {}", index + 1, effect.kind.name()))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .child(self.effect_button(
                            format!("effect-{id}-toggle"),
                            if effect.enabled {
                                "無効にする"
                            } else {
                                "有効にする"
                            },
                            EffectCommand::Toggle(id),
                            false,
                            cx,
                        ))
                        .child(self.effect_button(
                            format!("effect-{id}-up"),
                            "上へ",
                            EffectCommand::Move(id, -1),
                            index == 0,
                            cx,
                        ))
                        .child(self.effect_button(
                            format!("effect-{id}-down"),
                            "下へ",
                            EffectCommand::Move(id, 1),
                            index + 1 == clip.effects.len(),
                            cx,
                        ))
                        .child(self.effect_button(
                            format!("effect-{id}-remove"),
                            "削除",
                            EffectCommand::Remove(id),
                            false,
                            cx,
                        )),
                );
            if let EffectKind::Opacity(value) = effect.kind {
                let entity = cx.entity().downgrade();
                let clip_id = clip.id;
                card = card.child(
                    div()
                        .capture_any_mouse_down(cx.listener(
                            move |editor, event: &gpui::MouseDownEvent, _, _| {
                                if event.button == MouseButton::Left {
                                    editor.effect_drag = Some(id);
                                }
                            },
                        ))
                        .child(
                            Slider::new(gpui::SharedString::from(format!("effect-{id}-opacity")))
                                .label("不透明度")
                                .range(0.0, 100.0)
                                .step(1.0)
                                .value(value as f32)
                                .display(format!("{value}%"))
                                .disabled(self.loading || !effect.enabled)
                                .on_change(move |value, _, cx| {
                                    let _ = entity.update(cx, |editor, cx| {
                                        if editor.loading
                                            || editor.effect_cancelled
                                            || (editor.pointer_down
                                                && editor.effect_drag != Some(id))
                                        {
                                            return;
                                        }
                                        if editor.effect_edit.is_none() {
                                            editor.effect_edit =
                                                Some(editor.timeline.document.clone());
                                        }
                                        editor.timeline.playing = None;
                                        let clip = editor
                                            .timeline
                                            .document
                                            .clips
                                            .iter_mut()
                                            .find(|c| c.id == clip_id)
                                            .expect("edited object exists");
                                        let effect = clip
                                            .effects
                                            .iter_mut()
                                            .find(|e| e.id == id)
                                            .expect("edited effect exists");
                                        effect.kind = EffectKind::Opacity(value.round() as u8);
                                        cx.notify();
                                    });
                                }),
                        ),
                );
            }
            details = details.child(card);
        }
        div()
            .id("effect-settings")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_focus(&self.effects_focus)
            .tab_stop(false)
            .capture_key_down(
                cx.listener(|editor, event: &gpui::KeyDownEvent, window, cx| {
                    if editor.loading {
                        return;
                    }
                    let key = event.keystroke.key.as_str();
                    if key == "escape" {
                        editor.finish_effect_edit(true, cx);
                        window.prevent_default();
                        cx.stop_propagation();
                    } else if event.keystroke.modifiers.control && matches!(key, "z" | "y") {
                        editor.finish_effect_edit(false, cx);
                        editor
                            .timeline
                            .undo(key == "y" || event.keystroke.modifiers.shift);
                        editor.sync_timeline();
                        cx.notify();
                        window.prevent_default();
                        cx.stop_propagation();
                    } else if matches!(key, "enter" | "space")
                        && (event.is_held || event.keystroke.modifiers.modified())
                    {
                        window.prevent_default();
                        cx.stop_propagation();
                    }
                }),
            )
            .capture_key_up(cx.listener(|editor, event: &gpui::KeyUpEvent, window, cx| {
                let key = event.keystroke.key.as_str();
                if matches!(
                    key,
                    "left" | "right" | "up" | "down" | "home" | "end" | "tab"
                ) {
                    editor.finish_effect_edit(false, cx);
                    editor.effect_cancelled = false;
                }
                // shortcut: Kit 0.2.0 also synthesizes button clicks on keyup; remove with its upstream fix.
                if matches!(key, "enter" | "space") {
                    window.prevent_default();
                    cx.stop_propagation();
                }
            }))
            .child(details)
            .into_any_element()
    }
}

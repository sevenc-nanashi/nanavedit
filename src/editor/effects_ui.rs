use super::*;
use crate::effects::{ParamDefinition, ParamValue};
use crate::timeline::{FPS, ObjectKind, OutputDestination};
use gpui_kit::prelude::{Checkbox, Slider};

#[derive(Clone, Copy)]
pub(super) enum CreationTarget {
    Input { layer: usize, frame: u64 },
    Effect { clip_id: u64 },
}

#[derive(Clone)]
enum EffectCommand {
    Toggle(u64),
    Remove(u64),
    Move(u64, isize),
}

impl Editor {
    pub(super) fn can_initialize(&self, definition: &EffectDefinition) -> bool {
        if self.loading {
            return false;
        }
        match definition.info.initialization {
            Initialization::Defaults => true,
            Initialization::ImageFile => {
                #[cfg(target_os = "windows")]
                {
                    !self.gpu_failed
                }
                #[cfg(not(target_os = "windows"))]
                {
                    false
                }
            }
        }
    }

    pub(super) fn start_effect_creation(
        &mut self,
        definition: &'static EffectDefinition,
        target: CreationTarget,
        cx: &mut Context<Self>,
    ) {
        if !self.can_initialize(definition) {
            return;
        }
        self.finish_clip_drag(false, cx);
        self.finish_effect_edit(false, cx);
        self.timeline.playing = None;
        match definition.info.initialization {
            Initialization::Defaults => {
                self.insert_effect(definition, target, Vec::new());
                self.sync_timeline();
            }
            Initialization::ImageFile => {
                #[cfg(target_os = "windows")]
                self.open_image(definition, target, cx);
            }
        }
        cx.notify();
    }

    pub(super) fn insert_effect(
        &mut self,
        definition: &'static EffectDefinition,
        target: CreationTarget,
        resources: Vec<Resource>,
    ) {
        let instance = EffectInstance::new(self.timeline.allocate_id(), definition, resources);
        match target {
            CreationTarget::Input { layer, frame } => self.timeline.add_object(
                ObjectKind::Input {
                    input: instance,
                    output: OutputDestination::Canvas,
                },
                layer,
                frame,
            ),
            CreationTarget::Effect { clip_id } => {
                let before = self.timeline.document.clone();
                self.timeline
                    .document
                    .clips
                    .iter_mut()
                    .find(|c| c.id == clip_id)
                    .expect("effect target exists")
                    .effects
                    .push(instance);
                self.timeline.commit(before);
            }
        }
    }

    fn open_effect_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.finish_effect_edit(false, cx);
        self.timeline.playing = None;
        self.effect_add_target = self.timeline.selected;
        let items = crate::effects::definitions()
            .iter()
            .filter(|d| d.info.role == EffectRole::Effect)
            .map(|d| {
                gpui_kit::prelude::MenuItem::command(d.info.id, d.info.name)
                    .disabled(!self.can_initialize(d))
            })
            .collect();
        let position = self.effect_menu_bounds.get().bottom_left();
        let result = self.effect_menu.update(cx, |menu, cx| {
            menu.set_items(items, window, cx)?;
            menu.open_at(position, window, cx)
        });
        if let Err(error) = result {
            self.preview_error = Some(format!("メニューを表示できません: {error}"));
        }
        cx.notify();
    }

    fn change_param(
        &mut self,
        clip_id: u64,
        effect_id: u64,
        definition: &'static ParamDefinition,
        value: ParamValue,
        cx: &mut Context<Self>,
    ) {
        if self.loading
            || self.effect_cancelled
            || (self.pointer_down && self.effect_drag != Some((effect_id, definition.id)))
        {
            return;
        }
        if self.effect_edit.is_none() {
            self.effect_edit = Some(self.timeline.document.clone());
        }
        self.timeline.playing = None;
        let effect = self
            .timeline
            .document
            .clips
            .iter_mut()
            .find(|c| c.id == clip_id)
            .expect("edited object exists")
            .instance_mut(effect_id);
        effect
            .params
            .iter_mut()
            .find(|p| p.id == definition.id)
            .expect("edited parameter exists")
            .value = value;
        if matches!(value, ParamValue::Bool(_)) {
            self.finish_effect_edit(false, cx);
        }
        cx.notify();
    }

    fn param_control(
        &self,
        clip_id: u64,
        effect: &EffectInstance,
        definition: &'static ParamDefinition,
        value: ParamValue,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let effect_id = effect.id;
        let param_id = definition.id;
        let id = gpui::SharedString::from(format!("effect-{effect_id}-param-{param_id}"));
        let entity = cx.entity().downgrade();
        let disabled = self.loading || !effect.enabled;
        let control = match value {
            ParamValue::Bool(value) => Checkbox::new(id)
                .label(definition.name)
                .checked(value)
                .disabled(disabled)
                .on_change(move |value, _, cx| {
                    let _ = entity.update(cx, |editor, cx| {
                        editor.change_param(
                            clip_id,
                            effect_id,
                            definition,
                            ParamValue::Bool(value),
                            cx,
                        )
                    });
                })
                .into_any_element(),
            ParamValue::Integer(_) | ParamValue::Float(_) => {
                let number = definition.number.expect("numeric parameter has a range");
                Slider::new(id)
                    .label(definition.name)
                    .range(number.min, number.max)
                    .step(number.step)
                    .value(value.number())
                    .display(format!(
                        "{:.*}{}",
                        number.precision,
                        value.number(),
                        number.unit
                    ))
                    .disabled(disabled)
                    .on_change(move |value, _, cx| {
                        let _ = entity.update(cx, |editor, cx| {
                            editor.change_param(
                                clip_id,
                                effect_id,
                                definition,
                                definition.slider_value(value),
                                cx,
                            )
                        });
                    })
                    .into_any_element()
            }
        };
        div()
            .capture_any_mouse_down(cx.listener(
                move |editor, event: &gpui::MouseDownEvent, _, _| {
                    if event.button == MouseButton::Left {
                        editor.effect_drag = Some((effect_id, param_id));
                    }
                },
            ))
            .child(control)
            .into_any_element()
    }

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
        let entity = cx.entity().downgrade();
        let bounds = self.effect_menu_bounds.clone();
        details = details.child(
            div()
                .on_children_prepainted(move |children, _, _| {
                    if let Some(first) = children.first() {
                        bounds.set(*first);
                    }
                })
                .child(
                    Button::new("add-effect")
                        .label("エフェクトを追加")
                        .secondary()
                        .control_size(ControlSize::Sm)
                        .disabled(self.loading)
                        .on_click(move |window, cx| {
                            let _ =
                                entity.update(cx, |editor, cx| editor.open_effect_menu(window, cx));
                        }),
                ),
        );
        let input_count = usize::from(matches!(clip.kind, ObjectKind::Input { .. }));
        for (position, effect) in clip.instances().enumerate() {
            let id = effect.id;
            let info = &effect.definition().info;
            let mut card = div()
                .flex()
                .flex_col()
                .gap_2()
                .p_2()
                .border_1()
                .border_color(theme.colors.divider);
            if position < input_count {
                card = card.child(format!("入力: {}", info.name));
            } else {
                let index = position - input_count;
                card = card.child(format!("{}. {}", index + 1, info.name)).child(
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
            }
            for definition in info.param_definitions {
                let value = crate::effects::param(&effect.params, definition.id);
                card = card.child(self.param_control(clip.id, effect, definition, value, cx));
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

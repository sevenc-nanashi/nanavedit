use super::*;
use crate::effects::{ParamDefinition, ParamValue};
use crate::timeline::{FPS, ObjectKind};
use gpui::Focusable;
use gpui_kit::prelude::{
    Checkbox, NumberInput, NumberInputEvent, Select, SelectEvent, SelectOption, Slider,
};

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
    pub(super) fn start_effect_creation(
        &mut self,
        definition: &'static EffectDefinition,
        target: CreationTarget,
        cx: &mut Context<Self>,
    ) {
        if self.loading {
            return;
        }
        self.finish_clip_drag(false, cx);
        self.finish_effect_edit(false, cx);
        self.timeline.playing = None;
        self.insert_effect(definition, target);
        self.sync_timeline();
        cx.notify();
    }

    pub(super) fn insert_effect(
        &mut self,
        definition: &'static EffectDefinition,
        target: CreationTarget,
    ) {
        let instance = EffectInstance::new(self.timeline.allocate_id(), definition);
        match target {
            CreationTarget::Input { layer, frame } => {
                let output = EffectInstance::new(
                    self.timeline.allocate_id(),
                    crate::effects::definition("builtin.standard_draw"),
                );
                self.timeline.add_object(
                    ObjectKind::Input {
                        input: instance,
                        output,
                    },
                    layer,
                    frame,
                );
            }
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
                gpui_kit::prelude::MenuItem::command(d.info.id, d.info.name).disabled(self.loading)
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
        let immediate = matches!(
            value,
            ParamValue::Bool(_) | ParamValue::Choice(_) | ParamValue::File(_)
        );
        effect
            .params
            .iter_mut()
            .find(|p| p.id == definition.id)
            .expect("edited parameter exists")
            .value = value;
        if immediate {
            self.finish_effect_edit(false, cx);
        }
        cx.notify();
    }

    fn param_control(
        &mut self,
        clip_id: u64,
        effect: &EffectInstance,
        definition: &'static ParamDefinition,
        value: ParamValue,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let effect_id = effect.id;
        let param_id = definition.id;
        let id = gpui::SharedString::from(format!("effect-{effect_id}-param-{param_id}"));
        let entity = cx.entity().downgrade();
        let disabled = self.loading || !effect.enabled;
        let control = match value {
            ParamValue::File(file) => {
                #[cfg(target_os = "windows")]
                let unavailable = self.gpu_failed;
                #[cfg(not(target_os = "windows"))]
                let unavailable = true;
                let control = div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(definition.name)
                    .child(div().overflow_hidden().child(match &file {
                        Some(file) => file.path.display().to_string(),
                        None => "未選択".to_owned(),
                    }))
                    .child(
                        Button::new(id)
                            .label(if file.is_some() {
                                "変更…"
                            } else {
                                "選択…"
                            })
                            .secondary()
                            .control_size(ControlSize::Sm)
                            .disabled(disabled || unavailable)
                            .on_click(move |_, cx| {
                                #[cfg(target_os = "windows")]
                                let _ = entity.update(cx, |editor, cx| {
                                    editor.open_file_parameter(clip_id, effect_id, definition, cx);
                                });
                                #[cfg(not(target_os = "windows"))]
                                let _ = cx;
                            }),
                    );
                #[cfg(not(target_os = "windows"))]
                let control = control.child("ファイル読み込みは現在Windowsのみ対応しています");
                control.into_any_element()
            }
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
                let key = (effect_id, param_id);
                let input = if let Some(input) = self.number_inputs.get(&key) {
                    input.clone()
                } else {
                    let input = cx.new(|cx| {
                        NumberInput::new(format!("{id}-number"), window, cx)
                            .name(definition.name)
                            .value(value.number() as f64)
                            .range(number.min as f64, number.max as f64)
                            .step(number.step as f64)
                            .precision(number.precision)
                            .unit(number.unit)
                            .required(true)
                            .control_size(ControlSize::Sm)
                    });
                    cx.subscribe(&input, move |editor, input, event, cx| match event {
                        NumberInputEvent::Changed(value) => {
                            editor.number_edit = Some(key);
                            let valid = value.is_finite()
                                && (number.min as f64..=number.max as f64).contains(value)
                                && (!matches!(definition.default, ParamValue::Integer(_))
                                    || value.fract() == 0.0);
                            input.update(cx, |input, cx| input.set_invalid(!valid, cx));
                            if valid {
                                editor.change_param(
                                    clip_id,
                                    effect_id,
                                    definition,
                                    definition.slider_value(*value as f32),
                                    cx,
                                );
                            }
                        }
                        NumberInputEvent::Unparsable(_) => {
                            editor.number_edit = Some(key);
                        }
                        NumberInputEvent::Submit => editor.finish_effect_edit(false, cx),
                    })
                    .detach();
                    let focus = input.read(cx).field().read(cx).focus_handle(cx);
                    cx.on_blur(&focus, window, |editor, _, cx| {
                        editor.finish_effect_edit(false, cx)
                    })
                    .detach();
                    self.number_inputs.insert(key, input.clone());
                    input
                };
                input.update(cx, |input, cx| {
                    if input.is_disabled() != disabled {
                        input.set_disabled(disabled, cx);
                    }
                    if self.number_edit != Some(key)
                        && input.current() != Some(value.number() as f64)
                    {
                        input.set_value(value.number() as f64, cx);
                        input.set_invalid(false, cx);
                    }
                });
                let slider = Slider::new(id)
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
                    });
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(slider)
                    .child(input)
                    .into_any_element()
            }
            ParamValue::Choice(value) => {
                let key = (effect_id, param_id);
                let input = if let Some(input) = self.choice_inputs.get(&key) {
                    input.clone()
                } else {
                    let input = cx.new(|cx| {
                        Select::new(id, window, cx)
                            .name(definition.name)
                            .options(
                                definition
                                    .choices
                                    .iter()
                                    .map(|(id, label)| SelectOption::new(*id, *label)),
                            )
                            .selected(value)
                            .control_size(ControlSize::Sm)
                    });
                    cx.subscribe(&input, move |editor, _, event, cx| {
                        if let SelectEvent::Selected(id) = event {
                            let value = definition
                                .choices
                                .iter()
                                .find(|(value, _)| *value == id.as_ref())
                                .expect("declared choice")
                                .0;
                            editor.change_param(
                                clip_id,
                                effect_id,
                                definition,
                                ParamValue::Choice(value),
                                cx,
                            );
                        }
                    })
                    .detach();
                    self.choice_inputs.insert(key, input.clone());
                    input
                };
                input.update(cx, |input, cx| {
                    if input.selected_id().map(|v| v.as_ref()) != Some(value) {
                        input.set_selected(Some(value.into()), cx);
                    }
                    if input.is_disabled() != disabled {
                        input.set_disabled(disabled, cx);
                    }
                });
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(definition.name)
                    .child(input)
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
        if let Some(key) = self.number_edit.take() {
            let value = self
                .timeline
                .document
                .clips
                .iter()
                .flat_map(|clip| clip.instances())
                .find(|effect| effect.id == key.0)
                .map(|effect| crate::effects::param(&effect.params, key.1));
            if let (Some(input), Some(value)) = (self.number_inputs.get(&key), value) {
                input.update(cx, |input, cx| {
                    input.set_value(value.number() as f64, cx);
                    input.set_invalid(false, cx);
                });
            }
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
        &mut self,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let clip = self.timeline.selected_clip().cloned();
        self.number_inputs.retain(|(id, _), _| {
            clip.as_ref()
                .is_some_and(|c| c.instances().any(|e| e.id == *id))
        });
        self.choice_inputs.retain(|(id, _), _| {
            clip.as_ref()
                .is_some_and(|c| c.instances().any(|e| e.id == *id))
        });
        let Some(clip) = clip else {
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
        if !matches!(clip.kind, ObjectKind::Camera { .. }) {
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
                                let _ = entity
                                    .update(cx, |editor, cx| editor.open_effect_menu(window, cx));
                            }),
                    ),
            );
        }
        let input_count = match clip.kind {
            ObjectKind::Input { .. } => 2,
            ObjectKind::Camera { .. } => 1,
            _ => 0,
        };
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
                card = card.child(info.name);
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
            let mut group = None;
            for definition in info.param_definitions {
                if definition.group != group {
                    group = definition.group;
                    if let Some(group) = group {
                        card = card.child(div().mt_2().child(group));
                    }
                }
                let value = crate::effects::param(&effect.params, definition.id);
                card =
                    card.child(self.param_control(clip.id, effect, definition, value, window, cx));
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
                if editor.number_edit.is_none()
                    && matches!(
                        key,
                        "left" | "right" | "up" | "down" | "home" | "end" | "tab"
                    )
                {
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

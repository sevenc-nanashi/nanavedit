use super::*;
use crate::{project::Project, timeline::ProjectSettings};
use anyhow::{Context as _, Result, ensure};
use gpui_kit::prelude::{NumberInput, NumberInputEvent};
use std::path::PathBuf;

impl Editor {
    pub(super) fn project_header_height(&self) -> f32 {
        crate::menus::HEIGHT
            + if self.project_settings_visible {
                PROJECT_CONTROLS_HEIGHT
            } else {
                0.0
            }
    }

    pub(super) fn update_menubar(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.menubar.update(cx, |menubar, cx| {
            menubar.set_state(self.loading, self.project_settings_visible, window, cx);
        });
    }

    pub(super) fn project_controls(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let settings = self.timeline.document.settings;
        let mut row = div()
            .flex()
            .flex_none()
            .h(px(PROJECT_CONTROLS_HEIGHT))
            .px(px(16.0))
            .gap(px(8.0))
            .items_center();
        for (index, (name, value, max)) in [
            ("幅", settings.width as f64, 8192.0),
            ("高さ", settings.height as f64, 8192.0),
            ("fps", settings.fps as f64, 240.0),
        ]
        .into_iter()
        .enumerate()
        {
            if self.project_inputs.len() <= index {
                let input = cx.new(|cx| {
                    NumberInput::new(format!("project-{index}"), window, cx)
                        .name(name)
                        .value(value)
                        .range(1.0, max)
                        .step(1.0)
                        .precision(0)
                        .required(true)
                        .control_size(ControlSize::Sm)
                });
                cx.subscribe(&input, move |_, input, event, cx| {
                    if let NumberInputEvent::Changed(value) = event {
                        input.update(cx, |input, cx| {
                            input.set_invalid(
                                !value.is_finite()
                                    || value.fract() != 0.0
                                    || !(1.0..=max).contains(value),
                                cx,
                            )
                        });
                    }
                    cx.notify();
                })
                .detach();
                self.project_inputs.push(input);
            }
            self.project_inputs[index].update(cx, |input, cx| input.set_disabled(self.loading, cx));
            row = row.child(
                div()
                    .w(px(110.0))
                    .child(name)
                    .child(self.project_inputs[index].clone()),
            );
        }
        let entity = cx.entity().downgrade();
        row.child(
            Button::new("apply-project")
                .label("設定を適用")
                .disabled(self.loading)
                .on_click(move |_, cx| {
                    let _ = entity.update(cx, |editor, cx| {
                        let result = editor.apply_project_settings(cx);
                        editor.preview_error =
                            result.err().map(|e| format!("設定を変更できません: {e:#}"));
                        cx.notify();
                    });
                }),
        )
    }

    fn apply_project_settings(&mut self, cx: &mut Context<Self>) -> Result<()> {
        if self.loading {
            return Ok(());
        }

        let values = self
            .project_inputs
            .iter()
            .map(|input| input.read(cx).shown(cx).context("数値を入力してください"))
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            values
                .iter()
                .all(|v| v.is_finite() && v.fract() == 0.0 && *v >= 1.0),
            "正の整数を入力してください"
        );
        ensure!(
            values[0] <= 8192.0 && values[1] <= 8192.0 && values[2] <= 240.0,
            "設定値が範囲外です"
        );
        let settings = ProjectSettings {
            width: values[0] as u32,
            height: values[1] as u32,
            fps: values[2] as u64,
        };
        settings.validate()?;
        #[cfg(target_os = "windows")]
        if let Some(preview) = &self.preview {
            ensure!(
                settings.width <= preview.max_dimension()
                    && settings.height <= preview.max_dimension(),
                "GPUの最大解像度を超えています"
            );
        }
        for clip in &self.timeline.document.clips {
            if let Some(video) = clip.video() {
                ensure!(
                    clip.source_start + clip.length <= video.duration(settings.fps),
                    "fps変更後に動画の範囲を超えるクリップがあります。先にクリップを短くしてください"
                );
            }
        }
        self.finish_clip_drag(false, cx);
        self.finish_effect_edit(false, cx);
        self.timeline.playing = None;
        let before = self.timeline.document.clone();
        self.timeline.document.settings = settings;
        self.timeline.commit(before);
        self.sync_timeline();
        Ok(())
    }

    pub(super) fn save_project(&mut self, save_as: bool, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.finish_clip_drag(false, cx);
        self.finish_effect_edit(false, cx);
        self.timeline.playing = None;
        let document = self.timeline.document.clone();
        let path = if save_as {
            None
        } else {
            self.project_path.clone()
        };
        let prompt = if path.is_none() {
            match std::env::current_dir() {
                Ok(directory) => {
                    Some(cx.prompt_for_new_path(&directory, Some("project.nanavedit")))
                }
                Err(error) => {
                    self.preview_error = Some(error.to_string());
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };
        self.loading = true;
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let result: Result<Option<PathBuf>> = async {
                let path = match path {
                    Some(path) => path,
                    None => match prompt.expect("save dialog").await?? {
                        Some(path) => path,
                        None => return Ok(None),
                    },
                };
                executor
                    .spawn(async move {
                        Project::capture(&document)?.save(&path)?;
                        Ok(Some(path))
                    })
                    .await
            }
            .await;
            let _ = this.update(cx, |editor, cx| {
                editor.loading = false;
                match result {
                    Ok(Some(path)) => {
                        editor.project_path = Some(path);
                        editor.saved_document = editor.timeline.document.clone();
                        editor.preview_error = None;
                    }
                    Ok(None) => {}
                    Err(error) => editor.preview_error = Some(format!("保存できません: {error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn open_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.finish_clip_drag(false, cx);
        self.finish_effect_edit(false, cx);
        self.timeline.playing = None;
        let confirmation = (self.timeline.document != self.saved_document).then(|| {
            window.prompt(
                gpui::PromptLevel::Warning,
                "未保存の変更を破棄して開きますか？",
                None,
                &["キャンセル", "破棄して開く"],
                cx,
            )
        });
        self.loading = true;
        #[cfg(target_os = "windows")]
        let resources = self
            .preview
            .as_ref()
            .filter(|_| !self.gpu_failed)
            .map(|preview| {
                (
                    (preview.max_dimension(), preview.max_video_buffer_size()),
                    cx.global::<crate::input::InputPlugins>().0.clone(),
                )
            });
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let result: Result<_> = async {
                if let Some(confirmation) = confirmation
                    && confirmation.await? != 1
                {
                    return Ok(None);
                }
                let paths = cx.update(|cx| {
                    cx.prompt_for_paths(gpui::PathPromptOptions {
                        files: true,
                        directories: false,
                        multiple: false,
                        prompt: Some("プロジェクトを開く".into()),
                    })
                });
                let Some(paths) = paths.await?? else {
                    return Ok(None);
                };
                let path = paths
                    .into_iter()
                    .next()
                    .context("ファイルが選択されていません")?;
                let read_path = path.clone();
                let restored = executor
                    .spawn(async move { Project::read(&read_path) })
                    .await?;
                #[cfg(target_os = "windows")]
                let mut restored = restored;
                #[cfg(target_os = "windows")]
                let mut images = Vec::new();
                for (effect_id, param_id, kind, file_path) in restored.files {
                    #[cfg(not(target_os = "windows"))]
                    {
                        let _ = (effect_id, param_id, kind, file_path);
                        anyhow::bail!("素材の読み込みはWindowsのみ対応しています");
                    }
                    #[cfg(target_os = "windows")]
                    {
                        use crate::effects::{FileAsset, FileKind, ParamValue, Resource};
                        let (limit, plugins) = resources
                            .as_ref()
                            .context("素材の読み込みにはGPUプレビューが必要です")?;
                        let resource = match kind {
                            FileKind::Image => {
                                let decode_path = file_path.clone();
                                let dimension = limit.0;
                                let image = executor
                                    .spawn(async move {
                                        crate::preview::load_image(&decode_path, dimension)
                                    })
                                    .await?;
                                let id =
                                    this.update(cx, |editor, _| editor.timeline.allocate_id())?;
                                let asset = crate::timeline::ImageAsset {
                                    id,
                                    name: file_path
                                        .file_name()
                                        .context("素材名がありません")?
                                        .to_string_lossy()
                                        .into_owned(),
                                    width: image.width(),
                                    height: image.height(),
                                };
                                images.push((id, image));
                                Resource::Image(std::sync::Arc::new(asset))
                            }
                            FileKind::Video => Resource::Video(
                                plugins
                                    .open(file_path.clone(), *limit, restored.document.settings.fps)
                                    .await?,
                            ),
                        };
                        let effect = restored
                            .document
                            .clips
                            .iter_mut()
                            .find(|clip| clip.instances().any(|effect| effect.id == effect_id))
                            .expect("validated effect")
                            .instance_mut(effect_id);
                        effect
                            .params
                            .iter_mut()
                            .find(|param| param.id == param_id)
                            .expect("validated parameter")
                            .value = ParamValue::File(Some(std::sync::Arc::new(FileAsset {
                            path: file_path,
                            resource,
                        })));
                    }
                }
                for clip in &restored.document.clips {
                    if let Some(video) = clip.video() {
                        ensure!(
                            clip.source_start + clip.length
                                <= video.duration(restored.document.settings.fps),
                            "動画素材よりクリップが長くなっています"
                        );
                    }
                }
                #[cfg(target_os = "windows")]
                this.update(cx, |editor, _| -> Result<()> {
                    ensure!(
                        !editor.gpu_failed || images.is_empty(),
                        "GPU描画を続行できません"
                    );
                    if let Some(preview) = &editor.preview {
                        ensure!(
                            restored.document.settings.width <= preview.max_dimension()
                                && restored.document.settings.height <= preview.max_dimension(),
                            "GPUの最大解像度を超えています"
                        );
                        for (id, image) in images {
                            preview.register_image(id, image)?;
                        }
                    }
                    Ok(())
                })??;
                Ok(Some((path, restored.document)))
            }
            .await;
            let _ = this.update(cx, |editor, cx| {
                editor.loading = false;
                match result {
                    Ok(Some((path, document))) => {
                        editor.saved_document = document.clone();
                        editor.timeline.replace_document(document);
                        editor.project_path = Some(path);
                        editor.project_inputs.clear();
                        editor.number_inputs.clear();
                        editor.choice_inputs.clear();
                        editor
                            .timeline_scroll
                            .set_offset(gpui::point(px(0.0), px(0.0)));
                        editor.preview_error = None;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        editor.preview_error = Some(format!("プロジェクトを開けません: {error:#}"))
                    }
                }
                editor.sync_timeline();
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

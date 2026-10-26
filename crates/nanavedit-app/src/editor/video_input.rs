use super::*;
use crate::{
    effects::{FileAsset, ParamDefinition, ParamValue, Resource},
    timeline::Clip,
};
use anyhow::{Context as _, Result};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[derive(Default)]
pub(super) struct VideoState {
    requested: Option<(Vec<Clip>, u64, crate::timeline::ProjectSettings)>,
    generation: Arc<AtomicU64>,
    presented: u64,
    pub busy: bool,
}

impl Editor {
    pub(super) fn open_video_parameter(
        &mut self,
        clip_id: u64,
        effect_id: u64,
        definition: &'static ParamDefinition,
        cx: &mut Context<Self>,
    ) {
        if self.loading || self.gpu_failed {
            return;
        }
        self.finish_clip_drag(false, cx);
        self.finish_effect_edit(false, cx);
        self.timeline.playing = None;
        let preview = self.preview.as_ref().expect("live preview");
        let limit = (preview.max_dimension(), preview.max_video_buffer_size());
        let plugins = cx.global::<crate::input::InputPlugins>().0.clone();
        let paths = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("動画ファイルを開く（入力プラグイン）".into()),
        });
        let fps = self.timeline.document.settings.fps;
        self.loading = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result: Result<_> = async {
                let Some(paths) = paths.await?? else {
                    return Ok(None);
                };
                let path = paths
                    .into_iter()
                    .next()
                    .context("ファイルが選択されていません")?;
                let video = plugins.open(path.clone(), limit, fps).await?;
                Ok(Some((path, video)))
            }
            .await;
            let _ = this.update(cx, |editor, cx| {
                editor.loading = false;
                if !editor.gpu_failed {
                    match result {
                        Ok(Some((path, video))) => {
                            let before = editor.timeline.document.clone();
                            let original = editor
                                .timeline
                                .document
                                .clips
                                .iter()
                                .find(|clip| clip.id == clip_id)
                                .expect("file target");
                            let mut available = video.duration(fps).min(u64::MAX - original.start);
                            for other in &editor.timeline.document.clips {
                                if other.id != clip_id
                                    && other.layer == original.layer
                                    && other.start >= original.end()
                                {
                                    available = available.min(other.start - original.start);
                                }
                            }
                            let first = original.video().is_none();
                            let clip = editor
                                .timeline
                                .document
                                .clips
                                .iter_mut()
                                .find(|clip| clip.id == clip_id)
                                .expect("file target");
                            clip.length = if first {
                                available
                            } else {
                                clip.length.min(available)
                            };
                            clip.source_start = 0;
                            clip.instance_mut(effect_id)
                                .params
                                .iter_mut()
                                .find(|param| param.id == definition.id)
                                .expect("file parameter")
                                .value = ParamValue::File(Some(Arc::new(FileAsset {
                                path,
                                resource: Resource::Video(video),
                            })));
                            editor.timeline.commit(before);
                            editor.sync_timeline();
                            editor.preview_error = None;
                        }
                        Ok(None) => {}
                        Err(error) => {
                            editor.preview_error =
                                Some(format!("動画を読み込めませんでした: {error:#}"))
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn update_preview(&mut self, cx: &mut Context<Self>) {
        if self.gpu_failed {
            return;
        }
        // Finish one complete scene during playback; clock ticks must not starve a slower decoder.
        if self.video_state.busy && self.timeline.playing.is_some() {
            return;
        }
        let desired = (
            self.timeline.active_objects(),
            self.timeline.frame,
            self.timeline.document.settings,
        );
        if self.video_state.requested.as_ref() != Some(&desired) {
            self.video_state.generation.fetch_add(1, Ordering::AcqRel);
            self.video_state.requested = Some(desired.clone());
        }
        let generation = self.video_state.generation.load(Ordering::Acquire);
        if self.video_state.busy || self.video_state.presented == generation {
            return;
        }
        let (objects, frame, settings) = desired;
        if !nanavedit_engine::render::video::has_video(&objects) {
            self.preview
                .as_ref()
                .expect("live preview")
                .set_objects(objects, frame, settings);
            self.video_state.presented = generation;
            return;
        }
        self.video_state.busy = true;
        let current = self.video_state.generation.clone();
        cx.spawn(async move |this, cx| {
            let result = nanavedit_engine::render::video::decode_frames(
                &objects,
                frame,
                settings.fps,
                current,
                generation,
            )
            .await;
            let _ = this.update(cx, |editor, cx| {
                editor.video_state.busy = false;
                if !editor.gpu_failed
                    && editor.video_state.generation.load(Ordering::Acquire) == generation
                {
                    editor.video_state.presented = generation;
                    let preview = editor.preview.as_ref().expect("live preview");
                    let result = result.and_then(|prepared| {
                        preview.prepare_video(&prepared.frames, prepared.objects)
                    });
                    match result {
                        Ok(()) => {
                            preview.set_objects(objects, frame, settings);
                            editor.preview_error = None;
                        }
                        Err(error) => {
                            editor.timeline.playing = None;
                            editor.preview_error =
                                Some(format!("動画フレームを取得できませんでした: {error:#}"));
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

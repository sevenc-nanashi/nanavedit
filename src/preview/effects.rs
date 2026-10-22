use super::*;
use crate::timeline::{Effect, EffectKind, InputSource, ObjectKind, OutputDestination};
use wgpu::util::DeviceExt;

#[derive(Clone)]
pub(super) struct Source {
    view: wgpu::TextureView,
    binding: wgpu::BindGroup,
    width: u32,
    height: u32,
    premultiplied: bool,
}

struct EvaluationContext<'a> {
    object: &'a Clip,
    scene_frame: u64,
    screen_size: (u32, u32),
}

struct Params {
    alpha: f32,
}

impl Default for Params {
    fn default() -> Self {
        Self { alpha: 1.0 }
    }
}

enum Operation<'a> {
    Input(&'a InputSource),
    Effect(&'a Effect),
}

struct Pass {
    alpha: f32,
    invert: bool,
    load: wgpu::LoadOp<wgpu::Color>,
    present: bool,
}

impl Default for Pass {
    fn default() -> Self {
        Self {
            alpha: 1.0,
            invert: false,
            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            present: false,
        }
    }
}

impl Gpu {
    pub(super) fn source(
        &self,
        view: wgpu::TextureView,
        width: u32,
        height: u32,
        premultiplied: bool,
    ) -> Source {
        let binding = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("effect source"),
            layout: &self.image_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        Source {
            view,
            binding,
            width,
            height,
            premultiplied,
        }
    }

    fn acquire(&mut self, width: u32, height: u32) -> Source {
        if let Some(index) = self
            .scratch
            .iter()
            .position(|s| s.width == width && s.height == height)
        {
            return self.scratch.swap_remove(index);
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("effect scratch"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        self.source(
            texture.create_view(&Default::default()),
            width,
            height,
            true,
        )
    }

    fn release(&mut self, source: Source) {
        // Only scratch images are premultiplied; imported assets remain immutable.
        if source.premultiplied {
            self.scratch.push(source);
        }
    }

    fn render_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: &Source,
        target: (&wgpu::TextureView, u32, u32),
        settings: Pass,
    ) {
        let values = [
            settings.alpha,
            if source.premultiplied { 1.0 } else { 0.0 },
            if settings.invert { 1.0 } else { 0.0 },
            0.0,
        ];
        let bytes: Vec<u8> = values.into_iter().flat_map(f32::to_le_bytes).collect();
        let buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("effect params"),
                contents: &bytes,
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let binding = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("effect params"),
            layout: &self.params_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });
        let (view, width, height) = target;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("effect / composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: settings.load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        let scale = (width as f32 / source.width as f32).min(height as f32 / source.height as f32);
        let w = source.width as f32 * scale;
        let h = source.height as f32 * scale;
        pass.set_viewport(
            (width as f32 - w) / 2.0,
            (height as f32 - h) / 2.0,
            w,
            h,
            0.0,
            1.0,
        );
        pass.set_pipeline(if settings.present {
            &self.pipeline
        } else {
            &self.effect_pipeline
        });
        pass.set_bind_group(0, &source.binding, &[]);
        pass.set_bind_group(1, &binding, &[]);
        pass.draw(0..3, 0..1);
    }

    fn evaluate(
        &mut self,
        source: Option<Source>,
        params: &mut Params,
        operation: Operation<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<Source> {
        match operation {
            Operation::Input(InputSource::Image(image)) => self
                .images
                .get(&image.id)
                .cloned()
                .context("入力画像が登録されていません"),
            Operation::Effect(effect) => {
                let source = source.context("エフェクトの入力画像がありません")?;
                if !effect.enabled {
                    return Ok(source);
                }
                match effect.kind {
                    EffectKind::Opacity(percent) => {
                        params.alpha *= percent as f32 / 100.0;
                        Ok(source)
                    }
                    EffectKind::Invert => {
                        let target = self.acquire(source.width, source.height);
                        self.render_pass(
                            encoder,
                            &source,
                            (&target.view, target.width, target.height),
                            Pass {
                                invert: true,
                                ..Default::default()
                            },
                        );
                        self.release(source);
                        Ok(target)
                    }
                }
            }
        }
    }

    fn evaluate_effects(
        &mut self,
        mut source: Source,
        params: &mut Params,
        context: &EvaluationContext<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<Source> {
        let object = context.object;
        for effect in &object.effects {
            source = self
                .evaluate(Some(source), params, Operation::Effect(effect), encoder)
                .with_context(|| {
                    format!(
                        "オブジェクト {}（{}）、エフェクト {}（{}）、ローカルフレーム {}、画面 {}×{}",
                        object.id,
                        object.name(),
                        effect.id,
                        effect.kind.name(),
                        context.scene_frame - object.start,
                        context.screen_size.0, context.screen_size.1
                    )
                })?;
        }
        Ok(source)
    }

    fn flush_input(
        &mut self,
        pending: &mut Option<(Source, Params)>,
        scene: &Source,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        if let Some((source, params)) = pending.take() {
            self.render_pass(
                encoder,
                &source,
                (&scene.view, scene.width, scene.height),
                Pass {
                    alpha: params.alpha,
                    load: wgpu::LoadOp::Load,
                    ..Default::default()
                },
            );
            self.release(source);
        }
    }

    pub(super) fn render_objects(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> Result<()> {
        // shortcut: retain only currently needed sizes; add a global GPU budget for large projects.
        let sizes: HashSet<_> = std::iter::once((width, height))
            .chain(
                self.visible
                    .iter()
                    .filter_map(|c| c.image().map(|i| (i.width, i.height))),
            )
            .collect();
        self.scratch
            .retain(|s| sizes.contains(&(s.width, s.height)));
        let mut scene = self.acquire(width, height);
        {
            let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &scene.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
        let mut pending = None;
        for object in self.visible.clone() {
            let context = EvaluationContext {
                object: &object,
                scene_frame: self.frame,
                screen_size: (width, height),
            };
            match &object.kind {
                ObjectKind::Input {
                    input,
                    output: OutputDestination::Canvas,
                } => {
                    self.flush_input(&mut pending, &scene, encoder);
                    let mut params = Params::default();
                    let source = self
                        .evaluate(None, &mut params, Operation::Input(input), encoder)
                        .with_context(|| {
                            format!("入力オブジェクト {}（{}）", object.id, object.name())
                        })?;
                    let source = self.evaluate_effects(source, &mut params, &context, encoder)?;
                    pending = Some((source, params));
                }
                ObjectKind::Effect => {
                    if let Some((source, mut params)) = pending.take() {
                        let source =
                            self.evaluate_effects(source, &mut params, &context, encoder)?;
                        pending = Some((source, params));
                    }
                }
                ObjectKind::Filter => {
                    self.flush_input(&mut pending, &scene, encoder);
                    let mut params = Params::default();
                    scene = self.evaluate_effects(scene, &mut params, &context, encoder)?;
                    if params.alpha != 1.0 {
                        let target = self.acquire(width, height);
                        self.render_pass(
                            encoder,
                            &scene,
                            (&target.view, width, height),
                            Pass {
                                alpha: params.alpha,
                                ..Default::default()
                            },
                        );
                        self.release(scene);
                        scene = target;
                    }
                }
            }
        }
        self.flush_input(&mut pending, &scene, encoder);
        self.render_pass(
            encoder,
            &scene,
            (view, width, height),
            Pass {
                present: true,
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                ..Default::default()
            },
        );
        self.release(scene);
        Ok(())
    }
}

use super::*;
use crate::effects::{EffectInstance, Param, Resource};
use crate::timeline::{ObjectKind, OutputDestination};
use wgpu::util::DeviceExt;

pub(super) struct GpuResources {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub pipeline: wgpu::RenderPipeline,
    pub effect_pipeline: wgpu::RenderPipeline,
    pub image_layout: wgpu::BindGroupLayout,
    pub params_layout: wgpu::BindGroupLayout,
    pub sampler: wgpu::Sampler,
    pub images: HashMap<u64, Source>,
    pub scratch: Vec<Source>,
    pub used_sizes: HashSet<(u32, u32)>,
}

pub(super) fn depends_on_time(objects: &[Clip]) -> bool {
    let mut has_input = false;
    for object in objects {
        match object.kind {
            ObjectKind::Input { .. } => has_input = true,
            ObjectKind::Filter => has_input = false,
            ObjectKind::Effect if !has_input => continue,
            ObjectKind::Effect => {}
        }
        if object
            .instances()
            .any(|effect| effect.enabled && effect.definition().info.time_dependent)
        {
            return true;
        }
    }
    false
}

pub trait EffectProcessor {
    fn process(
        &mut self,
        source: Option<Source>,
        params: &[Param],
        render_params: &mut RenderParams,
        context: &mut ProcessContext<'_>,
    ) -> Result<Source>;
}

pub struct ProcessContext<'a> {
    resources: &'a mut GpuResources,
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub bindings: &'a [Resource],
    pub object_id: u64,
    pub effect_id: u64,
    pub layer: usize,
    pub scene_frame: u64,
    pub frame: u64,
    pub total_frames: u64,
    pub fps: u64,
    pub screen_size: (u32, u32),
}

impl ProcessContext<'_> {
    pub fn device(&self) -> &wgpu::Device {
        &self.resources.device
    }
    pub fn queue(&self) -> &wgpu::Queue {
        &self.resources.queue
    }
    pub fn time(&self) -> f64 {
        self.frame as f64 / self.fps as f64
    }

    pub fn image(&self, index: usize) -> Result<Source> {
        let asset = self
            .bindings
            .get(index)
            .context("画像リソースが指定されていません")?
            .image();
        let source = self
            .resources
            .images
            .get(&asset.id)
            .context("入力画像が登録されていません")?;
        Ok(Source {
            view: source.view.clone(),
            binding: source.binding.clone(),
            width: source.width,
            height: source.height,
            premultiplied: source.premultiplied,
            temporary: false,
        })
    }

    /// Temporary outputs use premultiplied linear RGBA16Float.
    pub fn acquire(&mut self, width: u32, height: u32) -> Result<Source> {
        self.resources.acquire(width, height)
    }
    pub fn release(&mut self, source: Source) {
        self.resources.release(source);
    }

    pub fn create_pipeline(&self, label: &str, fragment: &str) -> Result<wgpu::RenderPipeline> {
        create_pipeline(
            &self.resources.device,
            [&self.resources.image_layout, &self.resources.params_layout],
            label,
            fragment,
            wgpu::TextureFormat::Rgba16Float,
            None,
        )
    }

    pub fn render(
        &mut self,
        source: &Source,
        target: &Source,
        pipeline: &wgpu::RenderPipeline,
        uniform: &[u8],
    ) -> Result<()> {
        ensure!(target.temporary, "元画像への上書きはできません");
        ensure!(
            uniform.len() >= 16 && uniform.len().is_multiple_of(16),
            "uniformは16バイト単位で指定してください"
        );
        self.resources.draw(
            self.encoder,
            source,
            DrawTarget {
                view: &target.view,
                width: target.width,
                height: target.height,
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                fit: false,
            },
            pipeline,
            uniform,
        );
        Ok(())
    }
}

pub(super) fn create_pipeline(
    device: &wgpu::Device,
    layouts: [&wgpu::BindGroupLayout; 2],
    label: &str,
    fragment: &str,
    format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
) -> Result<wgpu::RenderPipeline> {
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        // DXC treats this label as a filename and rejects non-ASCII names.
        label: Some("effect shader"),
        source: wgpu::ShaderSource::Wgsl(
            format!("{}\n{fragment}", include_str!("vertex.wgsl")).into(),
        ),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(layouts[0]), Some(layouts[1])],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });
    if let Some(error) = gpui::block_on(scope.pop()) {
        bail!("{label}: {error}");
    }
    Ok(pipeline)
}

struct DrawTarget<'a> {
    view: &'a wgpu::TextureView,
    width: u32,
    height: u32,
    load: wgpu::LoadOp<wgpu::Color>,
    fit: bool,
}

pub struct Source {
    view: wgpu::TextureView,
    binding: wgpu::BindGroup,
    width: u32,
    height: u32,
    premultiplied: bool,
    temporary: bool,
}

impl Source {
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn premultiplied(&self) -> bool {
        self.premultiplied
    }
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }
}

struct EvaluationContext<'a> {
    object: &'a Clip,
    scene_frame: u64,
    screen_size: (u32, u32),
}

pub struct RenderParams {
    pub alpha: f32,
}

impl Default for RenderParams {
    fn default() -> Self {
        Self { alpha: 1.0 }
    }
}

struct Pass {
    alpha: f32,
    load: wgpu::LoadOp<wgpu::Color>,
    present: bool,
}

impl Default for Pass {
    fn default() -> Self {
        Self {
            alpha: 1.0,
            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            present: false,
        }
    }
}

impl GpuResources {
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
            temporary: false,
        }
    }

    fn acquire(&mut self, width: u32, height: u32) -> Result<Source> {
        self.used_sizes.insert((width, height));
        ensure!(
            width > 0
                && height > 0
                && width <= self.device.limits().max_texture_dimension_2d
                && height <= self.device.limits().max_texture_dimension_2d,
            "出力画像のサイズがGPUの上限を超えています"
        );
        if let Some(index) = self
            .scratch
            .iter()
            .position(|s| s.width == width && s.height == height)
        {
            return Ok(self.scratch.swap_remove(index));
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
        let mut source = self.source(
            texture.create_view(&Default::default()),
            width,
            height,
            true,
        );
        source.temporary = true;
        Ok(source)
    }

    fn release(&mut self, source: Source) {
        if source.temporary {
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
            0.0,
            0.0,
        ];
        let bytes: Vec<u8> = values.into_iter().flat_map(f32::to_le_bytes).collect();
        let (view, width, height) = target;
        self.draw(
            encoder,
            source,
            DrawTarget {
                view,
                width,
                height,
                load: settings.load,
                fit: true,
            },
            if settings.present {
                &self.pipeline
            } else {
                &self.effect_pipeline
            },
            &bytes,
        );
    }

    fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: &Source,
        target: DrawTarget<'_>,
        pipeline: &wgpu::RenderPipeline,
        uniform: &[u8],
    ) {
        let buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("effect params"),
                contents: uniform,
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
        let DrawTarget {
            view,
            width,
            height,
            load,
            fit,
        } = target;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("effect / composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        if fit {
            let scale =
                (width as f32 / source.width as f32).min(height as f32 / source.height as f32);
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
        }
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &source.binding, &[]);
        pass.set_bind_group(1, &binding, &[]);
        pass.draw(0..3, 0..1);
    }
}

impl Gpu {
    fn evaluate(
        &mut self,
        source: Option<Source>,
        params: &mut RenderParams,
        effect: &EffectInstance,
        evaluation: &EvaluationContext<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<Source> {
        if !effect.enabled {
            return source.context("無効な入力には画像がありません");
        }
        let definition = effect.definition();
        let mut context = ProcessContext {
            resources: &mut self.resources,
            encoder,
            bindings: &effect.resources,
            object_id: evaluation.object.id,
            effect_id: effect.id,
            layer: evaluation.object.layer,
            scene_frame: evaluation.scene_frame,
            frame: evaluation.scene_frame - evaluation.object.start,
            total_frames: evaluation.object.length,
            fps: crate::timeline::FPS,
            screen_size: evaluation.screen_size,
        };
        let result = (|| {
            let processor = match self.processors.entry(effect.type_id) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert((definition.create_processor)(&context)?)
                }
            };
            processor.process(source, &effect.params, params, &mut context)
        })();
        result.with_context(|| {
            format!(
                "{}、オブジェクト {}、エフェクト {}、ローカルフレーム {}",
                definition.info.name, context.object_id, context.effect_id, context.frame
            )
        })
    }

    fn evaluate_effects(
        &mut self,
        mut source: Source,
        params: &mut RenderParams,
        context: &EvaluationContext<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<Source> {
        for effect in &context.object.effects {
            source = self.evaluate(Some(source), params, effect, context, encoder)?;
        }
        Ok(source)
    }

    fn flush_input(
        &mut self,
        pending: &mut Option<(Source, RenderParams)>,
        scene: &Source,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        if let Some((source, params)) = pending.take() {
            self.resources.render_pass(
                encoder,
                &source,
                (&scene.view, scene.width, scene.height),
                Pass {
                    alpha: params.alpha,
                    load: wgpu::LoadOp::Load,
                    ..Default::default()
                },
            );
            self.resources.release(source);
        }
    }

    pub(super) fn render_objects(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> Result<()> {
        // shortcut: pool sizes follow this evaluation; add a global budget for large projects.
        self.resources.used_sizes.clear();
        let mut scene = self.resources.acquire(width, height)?;
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
                    let mut params = RenderParams::default();
                    let source = self
                        .evaluate(None, &mut params, input, &context, encoder)
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
                    let mut params = RenderParams::default();
                    scene = self.evaluate_effects(scene, &mut params, &context, encoder)?;
                    if params.alpha != 1.0 {
                        let target = self.resources.acquire(width, height)?;
                        self.resources.render_pass(
                            encoder,
                            &scene,
                            (&target.view, width, height),
                            Pass {
                                alpha: params.alpha,
                                ..Default::default()
                            },
                        );
                        self.resources.release(scene);
                        scene = target;
                    }
                }
            }
        }
        self.flush_input(&mut pending, &scene, encoder);
        self.resources.render_pass(
            encoder,
            &scene,
            (view, width, height),
            Pass {
                present: true,
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                ..Default::default()
            },
        );
        self.resources.release(scene);
        self.resources.scratch.retain(|source| {
            self.resources
                .used_sizes
                .contains(&(source.width, source.height))
        });

        Ok(())
    }
}

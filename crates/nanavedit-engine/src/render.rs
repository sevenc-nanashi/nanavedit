pub mod effects;
pub mod video;
use crate::timeline::Clip;
use anyhow::{Result, ensure};
pub use effects::depends_on_time;
use effects::{EffectProcessor, GpuResources};
use image::RgbaImage;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

pub struct Renderer {
    pub settings: crate::timeline::ProjectSettings,
    videos: video::VideoTextures,
    device: wgpu::Device,
    queue: wgpu::Queue,
    resources: GpuResources,
    processors: HashMap<&'static str, Box<dyn EffectProcessor>>,
}

impl Renderer {
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Result<Self> {
        let image_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("effect image"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let params_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("effect params"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline = effects::create_pipeline(
            &device,
            [&image_layout, &params_layout],
            "composite / present",
            (
                include_str!("render/vertex.wgsl"),
                include_str!("composite.wgsl"),
            ),
            format,
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        )?;
        let effect_pipeline = effects::create_pipeline(
            &device,
            [&image_layout, &params_layout],
            "composite",
            (
                include_str!("render/vertex.wgsl"),
                include_str!("composite.wgsl"),
            ),
            wgpu::TextureFormat::Rgba16Float,
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        )?;
        let object_pipeline = effects::create_pipeline(
            &device,
            [&image_layout, &params_layout],
            "standard draw",
            (include_str!("render/object.wgsl"), ""),
            wgpu::TextureFormat::Rgba16Float,
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        )?;
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Ok(Self {
            settings: Default::default(),
            videos: video::VideoTextures::new(&device),
            device: device.clone(),
            queue: queue.clone(),
            resources: GpuResources {
                device,
                queue,
                pipeline,
                effect_pipeline,
                object_pipeline,
                image_layout,
                params_layout,
                sampler,
                scratch: Vec::new(),
                used_sizes: HashSet::new(),
                images: HashMap::new(),
            },
            processors: HashMap::new(),
        })
    }

    pub fn register_image(&mut self, id: u64, image: RgbaImage) -> Result<()> {
        let (width, height) = image.dimensions();
        let limit = self.device.limits().max_texture_dimension_2d;
        ensure!(
            width > 0 && height > 0 && width <= limit && height <= limit,
            "画像サイズがGPUの上限（{limit}px）を超えています"
        );
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("preview image"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            texture.as_image_copy(),
            &image,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            size,
        );
        let view = texture.create_view(&Default::default());
        let source = self.resources.source(view, width, height, false);
        self.resources.images.insert(id, source);
        Ok(())
    }

    pub fn prepare_video(
        &mut self,
        frames: &[(video::Key, Arc<crate::input::frame::Frame>)],
        objects: HashMap<u64, video::Key>,
    ) -> Result<()> {
        self.videos.prepare(&self.resources, frames, objects)
    }

    pub fn retain_images(&mut self, retained: &HashSet<u64>) {
        self.resources.images.retain(|id, _| retained.contains(id));
    }

    /// Render prepared, active objects in layer order into a texture matching the configured format.
    /// The caller owns the target and may present it or copy it after this submission.
    pub fn render(
        &mut self,
        objects: &[Clip],
        frame: u64,
        target: &wgpu::TextureView,
        size: (u32, u32),
    ) -> Result<()> {
        self.settings.validate()?;
        ensure!(
            self.settings.width <= self.device.limits().max_texture_dimension_2d
                && self.settings.height <= self.device.limits().max_texture_dimension_2d,
            "シーンサイズがGPUの上限を超えています"
        );
        let (width, height) = size;
        let limit = self.device.limits().max_texture_dimension_2d;
        ensure!(
            width > 0 && height > 0 && width <= limit && height <= limit,
            "描画サイズがGPUの範囲外です"
        );
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.render_objects(&mut encoder, objects, frame, target, size)?;
        self.queue.submit([encoder.finish()]);
        Ok(())
    }
}

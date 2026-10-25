use super::effects::{GpuResources, Source};
use crate::input::frame::{BUDGET, Frame};
use anyhow::{Result, ensure};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};
use wgpu::util::DeviceExt;

pub type Key = (u64, i32);
struct Cached {
    key: Key,
    texture: wgpu::Texture,
    source: Source,
    bytes: usize,
}
pub struct VideoTextures {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    entries: VecDeque<Cached>,
    bytes: usize,
    upload: Option<wgpu::Buffer>,
    pub objects: HashMap<u64, Key>,
}
impl VideoTextures {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("video pixel conversion"),
            source: wgpu::ShaderSource::Wgsl(include_str!("video.wgsl").into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("video pixel conversion"),
            layout: None,
            module: &shader,
            entry_point: Some("convert"),
            compilation_options: Default::default(),
            cache: None,
        });
        let layout = pipeline.get_bind_group_layout(0);
        Self {
            pipeline,
            layout,
            entries: VecDeque::new(),
            bytes: 0,
            upload: None,
            objects: HashMap::new(),
        }
    }
    pub fn get(&self, object: u64) -> Result<Source> {
        let key = self
            .objects
            .get(&object)
            .ok_or_else(|| anyhow::anyhow!("動画フレームが準備されていません"))?;
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.key == *key)
            .ok_or_else(|| anyhow::anyhow!("動画フレームが失効しました"))?;
        let source = &entry.source;
        Ok(Source {
            view: source.view.clone(),
            binding: source.binding.clone(),
            width: source.width,
            height: source.height,
            premultiplied: true,
            temporary: false,
        })
    }
    pub(super) fn prepare(
        &mut self,
        resources: &GpuResources,
        frames: &[(Key, Arc<Frame>)],
        objects: HashMap<u64, Key>,
    ) -> Result<()> {
        let needed: HashSet<_> = objects.values().copied().collect();
        let bytes: usize = frames
            .iter()
            .map(|(_, frame)| frame.format.width as usize * frame.format.height as usize * 8)
            .sum();
        ensure!(
            bytes <= BUDGET,
            "同時表示する動画フレームがGPUキャッシュ上限256MiBを超えています"
        );
        let device = &resources.device;
        for (_, frame) in frames {
            ensure!(
                frame.format.bytes.div_ceil(4) * 4
                    <= device.limits().max_storage_buffer_binding_size as usize,
                "フレームがGPU転送バッファの上限を超えています"
            );
        }
        for (key, frame) in frames {
            if let Some(index) = self.entries.iter().position(|entry| entry.key == *key) {
                let entry = self.entries.remove(index).expect("cached frame");
                self.entries.push_back(entry);
                continue;
            }
            let format = frame.format;
            let bytes = format.width as usize * format.height as usize * 8;
            let mut reusable = None;
            while self.bytes + bytes > BUDGET {
                let index = self
                    .entries
                    .iter()
                    .position(|entry| !needed.contains(&entry.key))
                    .expect("scene fits cache");
                let old = self.entries.remove(index).expect("evicted frame");
                self.bytes -= old.bytes;
                if old.source.width == format.width && old.source.height == format.height {
                    reusable = Some(old.texture);
                }
            }
            let texture = reusable.unwrap_or_else(|| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("video frame"),
                    size: wgpu::Extent3d {
                        width: format.width,
                        height: format.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba16Float,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::STORAGE_BINDING,
                    view_formats: &[],
                })
            });
            let view = texture.create_view(&Default::default());
            let size = frame.bytes().len() as u64;
            if self
                .upload
                .as_ref()
                .is_none_or(|buffer| buffer.size() < size)
            {
                self.upload = Some(device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("video pixels"),
                    size,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
            }
            let raw = self.upload.as_ref().expect("upload buffer");
            resources.queue.write_buffer(raw, 0, frame.bytes());
            let params: Vec<u8> = [
                format.width,
                format.height,
                format.stride,
                format.kind,
                u32::from(format.bottom_up),
                0,
                0,
                0,
            ]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
            let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("video format"),
                contents: &params,
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("video conversion"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: raw,
                            offset: 0,
                            size: std::num::NonZeroU64::new(size),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: params.as_entire_binding(),
                    },
                ],
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &binding, &[]);
                pass.dispatch_workgroups(format.width.div_ceil(8), format.height.div_ceil(8), 1);
            }
            resources.queue.submit([encoder.finish()]);
            let source = resources.source(view, format.width, format.height, true);
            self.entries.push_back(Cached {
                key: *key,
                texture,
                source,
                bytes,
            });
            self.bytes += bytes;
        }
        self.objects = objects;
        Ok(())
    }
}

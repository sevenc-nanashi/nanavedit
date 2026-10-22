pub(crate) mod effects;
use crate::timeline::Clip;
use effects::{EffectProcessor, GpuResources};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    num::NonZeroIsize,
    path::Path,
    rc::Rc,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Context as _, Result, bail, ensure};
use gpui::{PlatformViewHandle, Window};
use image::{ImageDecoder as _, ImageFormat, ImageReader, RgbaImage};
use raw_window_handle::{
    HasWindowHandle, RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle,
};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::*,
        },
    },
    core::w,
};

pub fn load_image(path: &Path, max_dimension: u32) -> Result<RgbaImage> {
    let mut reader = ImageReader::open(path)?.with_guessed_format()?;
    ensure!(
        matches!(reader.format(), Some(ImageFormat::Png | ImageFormat::Jpeg)),
        "PNGまたはJPEG画像を選択してください"
    );
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(max_dimension);
    limits.max_image_height = Some(max_dimension);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits.clone());
    let mut decoder = reader.into_decoder()?;
    // Reserve decoded, orientation-copy and RGBA buffers before any pixel allocation.
    limits.reserve(decoder.total_bytes() * 2)?;
    let (width, height) = decoder.dimensions();
    limits.reserve(u64::from(width) * u64::from(height) * 4)?;
    decoder.set_limits(limits)?;
    let orientation = decoder.orientation()?;
    let mut image = image::DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    Ok(image.into_rgba8())
}

pub struct Preview {
    pub handle: PlatformViewHandle,
    native: Rc<NativePreview>,
}

impl Preview {
    pub fn new(window: &Window, errors: async_channel::Sender<String>) -> Result<Self> {
        let RawWindowHandle::Win32(parent) = HasWindowHandle::window_handle(window)?.as_raw()
        else {
            bail!("Windowsのウィンドウハンドルを取得できませんでした");
        };
        static CLASS: OnceLock<std::result::Result<(), String>> = OnceLock::new();
        CLASS
            .get_or_init(|| unsafe {
                let class = WNDCLASSW {
                    lpfnWndProc: Some(window_proc),
                    hInstance: GetModuleHandleW(None).map_err(|e| e.to_string())?.into(),
                    lpszClassName: w!("NanaveditPreview"),
                    ..Default::default()
                };
                if RegisterClassW(&class) == 0 {
                    return Err(windows::core::Error::from_win32().to_string());
                }
                Ok(())
            })
            .as_ref()
            .map_err(|message| anyhow::anyhow!("描画ウィンドウの登録に失敗しました: {message}"))?;

        let native = Rc::new(NativePreview {
            hwnd: Cell::new(HWND::default()),
            host: Cell::new(HWND::default()),
            gpu: RefCell::new(None),
            failed: Arc::new(AtomicBool::new(false)),
            errors,
        });
        // The UI thread owns the HWND and its state; WM_NCDESTROY clears the pointer.
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("NanaveditPreview"),
                w!("プレビュー"),
                WS_CHILD | WS_CLIPSIBLINGS,
                0,
                0,
                1,
                1,
                Some(HWND(parent.hwnd.get() as *mut _)),
                None,
                Some(GetModuleHandleW(None)?.into()),
                None,
            )?
        };
        native.hwnd.set(hwnd);
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Rc::as_ptr(&native) as isize);
        }
        *native.gpu.borrow_mut() = Some(Gpu::new(hwnd, &native)?);
        // Frame-owned handle clones must outlive the surface and HWND owner.
        let handle = unsafe { PlatformViewHandle::from_hwnd(hwnd) }.keep_alive(native.clone());
        let weak = Rc::downgrade(&native);
        let handle = handle.with_win32_resize_handler(move |_| {
            if let Some(native) = weak.upgrade() {
                if let Err(error) = native.attach_host() {
                    native.failed.store(true, Ordering::Relaxed);
                    let _ = native
                        .errors
                        .try_send(format!("プレビューの配置に失敗しました: {error:#}"));
                }
                native.invalidate();
            }
        });
        Ok(Self { handle, native })
    }

    pub fn max_dimension(&self) -> u32 {
        self.native
            .gpu
            .borrow()
            .as_ref()
            .expect("live preview owns a GPU")
            .device
            .limits()
            .max_texture_dimension_2d
    }

    pub fn register_image(&self, id: u64, image: RgbaImage) -> Result<()> {
        ensure!(
            !self.native.failed.load(Ordering::Relaxed),
            "GPU描画を続行できません。アプリを再起動してください"
        );
        self.native
            .gpu
            .borrow_mut()
            .as_mut()
            .context("描画ウィンドウが閉じられています")?
            .register_image(id, image)?;
        Ok(())
    }

    pub fn set_objects(&self, objects: Vec<Clip>, frame: u64) {
        let mut gpu = self.native.gpu.borrow_mut();
        if let Some(gpu) = gpu.as_mut() {
            let time_changed = gpu.frame != frame && effects::depends_on_time(&objects);
            gpu.frame = frame;
            if gpu.visible != objects || time_changed {
                gpu.visible = objects;
                self.native.invalidate();
            }
        }
    }

    pub fn retain_images(&self, retained: &HashSet<u64>) {
        // shortcut: history keeps images resident; add an eviction budget for large projects.
        if let Some(gpu) = self.native.gpu.borrow_mut().as_mut() {
            gpu.resources.images.retain(|id, _| retained.contains(id));
        }
    }
}

struct NativePreview {
    hwnd: Cell<HWND>,
    host: Cell<HWND>,
    gpu: RefCell<Option<Gpu>>,
    failed: Arc<AtomicBool>,
    errors: async_channel::Sender<String>,
}

impl NativePreview {
    fn attach_host(&self) -> Result<()> {
        let host = unsafe { GetParent(self.hwnd.get())? };
        if self.host.get() != host {
            unsafe {
                SetWindowSubclass(host, Some(host_window_proc), 0, 0).ok()?;
            }
            self.host.set(host);
        }
        Ok(())
    }

    fn invalidate(&self) {
        if !self.hwnd.get().is_invalid() {
            unsafe {
                let _ = InvalidateRect(Some(self.hwnd.get()), None, false);
            }
        }
    }

    fn paint(&self) {
        if self.failed.load(Ordering::Relaxed) {
            return;
        }
        let mut rect = RECT::default();
        let result = (|| -> Result<()> {
            unsafe {
                GetClientRect(self.hwnd.get(), &mut rect)?;
            }
            if let Some(gpu) = self.gpu.borrow_mut().as_mut() {
                gpu.draw(rect.right.max(0) as u32, rect.bottom.max(0) as u32)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.failed.store(true, Ordering::Relaxed);
            let _ = self
                .errors
                .try_send(format!("プレビューの描画に失敗しました: {error:#}"));
        }
    }
}

unsafe extern "system" fn host_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    _: usize,
) -> LRESULT {
    // shortcut: gpui-box 0.2.0 inserts its host behind its owner; remove when upstream fixes host stacking.
    if message == WM_WINDOWPOSCHANGING {
        let position = unsafe { &mut *(lparam.0 as *mut WINDOWPOS) };
        if !position.flags.contains(SWP_NOZORDER)
            && let Ok(owner) = unsafe { GetWindow(hwnd, GW_OWNER) }
            && position.hwndInsertAfter == owner
        {
            match unsafe { GetWindow(owner, GW_HWNDPREV) } {
                Ok(previous) if previous == hwnd => position.flags |= SWP_NOZORDER,
                Ok(previous) => position.hwndInsertAfter = previous,
                // No previous window means the owner is already at the top.
                Err(_) => position.hwndInsertAfter = HWND_TOP,
            }
        }
    } else if message == WM_NCDESTROY {
        unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(host_window_proc), id);
        }
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

impl Drop for NativePreview {
    fn drop(&mut self) {
        self.gpu.get_mut().take();
        let hwnd = self.hwnd.replace(HWND::default());
        if !hwnd.is_invalid() {
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                let _ = DestroyWindow(hwnd);
            }
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // The pointer is installed only while the Rc-owned state is alive on this thread.
    let state =
        unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const NativePreview).as_ref() };
    match message {
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            unsafe {
                let _ = BeginPaint(hwnd, &mut paint);
            }
            if let Some(state) = state {
                state.paint();
            }
            unsafe {
                let _ = EndPaint(hwnd, &paint);
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_SIZE | WM_SHOWWINDOW => {
            if let Some(state) = state {
                state.invalidate();
            }
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_NCDESTROY => {
            if let Some(state) = state {
                state.gpu.borrow_mut().take();
                state.hwnd.set(HWND::default());
            }
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    resources: GpuResources,
    processors: HashMap<&'static str, Box<dyn EffectProcessor>>,
    visible: Vec<Clip>,
    frame: u64,
}

impl Gpu {
    fn new(hwnd: HWND, native: &NativePreview) -> Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::DX12,
            flags: Default::default(),
            backend_options: Default::default(),
            memory_budget_thresholds: Default::default(),
            display: None,
        });
        // NativePreview drops this surface before destroying the HWND, including on parent close.
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(RawDisplayHandle::Windows(WindowsDisplayHandle::new())),
                raw_window_handle: RawWindowHandle::Win32(Win32WindowHandle::new(
                    NonZeroIsize::new(hwnd.0 as isize).expect("created HWND is non-null"),
                )),
            })?
        };
        let adapter = gpui::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))?;
        ensure!(
            adapter.get_info().device_type != wgpu::DeviceType::Cpu,
            "DX12対応のGPUが必要です"
        );
        let (device, queue) = gpui::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("preview"),
            ..Default::default()
        }))?;
        let errors = native.errors.clone();
        let failed = native.failed.clone();
        device.set_device_lost_callback(move |reason, message| {
            if reason != wgpu::DeviceLostReason::Destroyed {
                failed.store(true, Ordering::Relaxed);
                let _ = errors.try_send(format!(
                    "GPUとの接続が失われました。再起動してください: {message}"
                ));
            }
        });
        let errors = native.errors.clone();
        let failed = native.failed.clone();
        device.on_uncaptured_error(Arc::new(move |error| {
            failed.store(true, Ordering::Relaxed);
            let _ = errors.try_send(format!("GPUエラー: {error}"));
        }));
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .context("sRGB描画に対応していません")?;
        let mut config = surface
            .get_default_config(&adapter, 1, 1)
            .context("描画Surfaceに対応していません")?;
        config.format = format;
        config.present_mode = wgpu::PresentMode::Fifo;
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
                visibility: wgpu::ShaderStages::FRAGMENT,
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
            include_str!("preview.wgsl"),
            format,
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        )?;
        let effect_pipeline = effects::create_pipeline(
            &device,
            [&image_layout, &params_layout],
            "composite",
            include_str!("preview.wgsl"),
            wgpu::TextureFormat::Rgba16Float,
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        )?;
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        // Zero marks the surface as unconfigured until GPUI allocates its physical size.
        config.width = 0;
        config.height = 0;
        Ok(Self {
            surface,
            device: device.clone(),
            queue: queue.clone(),
            config,
            resources: GpuResources {
                device,
                queue,
                pipeline,
                effect_pipeline,
                image_layout,
                params_layout,
                sampler,
                scratch: Vec::new(),
                used_sizes: HashSet::new(),
                images: HashMap::new(),
            },
            processors: HashMap::new(),
            visible: Vec::new(),
            frame: 0,
        })
    }

    fn register_image(&mut self, id: u64, image: RgbaImage) -> Result<()> {
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

    fn draw(&mut self, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 {
            self.config.width = 0;
            return Ok(());
        }
        if self.config.width != width || self.config.height != height {
            ensure!(
                width <= self.device.limits().max_texture_dimension_2d
                    && height <= self.device.limits().max_texture_dimension_2d,
                "プレビュー領域がGPUの最大サイズを超えています"
            );
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
        }
        let mut acquired = self.surface.get_current_texture();
        if matches!(
            acquired,
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated
        ) {
            self.surface.configure(&self.device, &self.config);
            acquired = self.surface.get_current_texture();
        }
        let (frame, reconfigure) = match acquired {
            wgpu::CurrentSurfaceTexture::Success(frame) => (frame, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => (frame, true),
            wgpu::CurrentSurfaceTexture::Occluded => return Ok(()),
            wgpu::CurrentSurfaceTexture::Timeout => bail!("GPUの描画待機がタイムアウトしました"),
            _ => bail!("描画Surfaceを取得できませんでした"),
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.render_objects(&mut encoder, &view, width, height)?;
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
        drop(view);
        if reconfigure {
            self.surface.configure(&self.device, &self.config);
        }
        Ok(())
    }
}

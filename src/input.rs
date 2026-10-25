pub mod frame;
mod services;
use crate::media::VideoAsset;
use anyhow::{Context as _, Result, bail, ensure};
use aviutl2_sys::{cache2::CACHE_HANDLE, config2::CONFIG_HANDLE, input2::*, logger2::LOG_HANDLE};
use gpui::Global;
use std::{
    collections::{HashMap, VecDeque},
    ffi::OsStr,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
};
use windows::{
    Win32::{
        Foundation::{FreeLibrary, HMODULE},
        System::{Com::*, LibraryLoader::*},
        UI::Shell::PathMatchSpecW,
    },
    core::{PCSTR, PCWSTR},
};

const VERSION: u32 = 2_011_400;
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
type Reply<T> = async_channel::Sender<Result<T>>;
pub fn directory() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .context("設定フォルダーを取得できません")?
        .join("nanavedit/Plugin"))
}
fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain([0]).collect()
}
unsafe fn string(pointer: *const u16) -> Result<String> {
    ensure!(!pointer.is_null(), "文字列ポインターがnullです");
    let mut len = 0;
    while len < 32768 && unsafe { *pointer.add(len) } != 0 {
        len += 1;
    }
    ensure!(len < 32768, "文字列が長すぎます");
    Ok(String::from_utf16_lossy(unsafe {
        std::slice::from_raw_parts(pointer, len)
    }))
}

pub struct Registry {
    pub entries: Mutex<Vec<Entry>>,
    pub scanning: std::sync::atomic::AtomicBool,
    stopping: std::sync::atomic::AtomicBool,
    scanned: async_channel::Receiver<()>,
    cache: Arc<Mutex<FrameCache>>,
}
pub struct InputPlugins(pub Arc<Registry>);
impl Global for InputPlugins {}
#[derive(Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub filters: String,
    pub error: Option<String>,
    pub client: Option<Client>,
}
#[derive(Clone, Debug)]
pub struct Client {
    sender: mpsc::Sender<Command>,
    pub opened: Arc<AtomicUsize>,
    pub can_config: bool,
}
#[derive(Debug)]
pub struct Lease {
    client: Client,
    id: u64,
}
impl Drop for Lease {
    fn drop(&mut self) {
        let _ = self.client.sender.send(Command::Close(self.id));
    }
}
enum Command {
    Open(PathBuf, (u32, u64), Client, Reply<Opened>),
    Read(u64, u64, Arc<AtomicU64>, u64, Reply<Arc<frame::Frame>>),
    Close(u64),
    Configure(usize, Reply<()>),
    Shutdown(Reply<()>),
}
impl std::fmt::Debug for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("input command")
    }
}
struct Opened {
    lease: Lease,
    format: frame::Format,
    frames: u32,
    rate: u32,
    scale: u32,
}

struct FrameCache {
    entries: VecDeque<((u64, i32), Arc<frame::Frame>)>,
    bytes: usize,
}
impl FrameCache {
    fn buffer(&mut self, bytes: usize) -> Vec<u64> {
        let length = bytes.div_ceil(8);
        let mut reusable = None;
        while self.bytes + length * 8 > frame::BUDGET {
            let (_, old) = self.entries.pop_front().expect("frame fits budget");
            self.bytes -= old.size();
            if let Ok(old) = Arc::try_unwrap(old)
                && old.data.len() == length
            {
                reusable = Some(old.data);
            }
        }
        reusable.unwrap_or_else(|| vec![0; length])
    }
    fn get(&mut self, key: (u64, i32)) -> Option<Arc<frame::Frame>> {
        let index = self.entries.iter().position(|(k, _)| *k == key)?;
        let entry = self.entries.remove(index).expect("located entry");
        let result = entry.1.clone();
        self.entries.push_back(entry);
        Some(result)
    }
    fn insert(&mut self, key: (u64, i32), frame: Arc<frame::Frame>) {
        while self.bytes + frame.size() > frame::BUDGET {
            let (_, old) = self.entries.pop_front().expect("frame fits budget");
            self.bytes -= old.size();
        }
        self.bytes += frame.size();
        self.entries.push_back((key, frame));
    }
    fn close(&mut self, id: u64) {
        self.entries.retain(|(key, _)| key.0 != id);
        self.bytes = self.entries.iter().map(|(_, frame)| frame.size()).sum();
    }
}

pub fn install(cx: &mut gpui::App) {
    let (finished, scanned) = async_channel::bounded(1);
    cx.set_quit_mode(gpui::QuitMode::Explicit);
    let registry = Arc::new(Registry {
        entries: Mutex::new(Vec::new()),
        scanning: true.into(),
        stopping: false.into(),
        scanned,
        cache: Arc::new(Mutex::new(FrameCache {
            entries: VecDeque::new(),
            bytes: 0,
        })),
    });
    cx.set_global(InputPlugins(registry.clone()));
    let shutdown = registry.clone();
    cx.on_app_quit(move |_| {
        let registry = shutdown.clone();
        async move {
            registry.shutdown().await;
        }
    })
    .detach();
    let task = cx.background_executor().spawn(async move {
        let result = registry.scan();
        if let Err(error) = result {
            registry.entries.lock().expect("registry lock").push(Entry {
                path: PathBuf::new(),
                name: "プラグイン検索".into(),
                filters: String::new(),
                error: Some(format!("{error:#}")),
                client: None,
            });
        }
        registry.scanning.store(false, Ordering::Release);
        let _ = finished.try_send(());
    });
    cx.spawn(async move |cx| {
        task.await;
        cx.update(|cx| cx.refresh_windows());
    })
    .detach();
}

pub fn quit(cx: &mut gpui::App) {
    let registry = cx.global::<InputPlugins>().0.clone();
    if registry.stopping.swap(true, Ordering::AcqRel) {
        return;
    }
    // Wait before App::quit, whose observer deadline is too short for native decoders.
    cx.spawn(async move |cx| {
        registry.shutdown().await;
        cx.update(|cx| cx.quit());
    })
    .detach();
}

impl Registry {
    fn scan(&self) -> Result<()> {
        fn visit(path: &Path, paths: &mut Vec<PathBuf>) -> Result<()> {
            for entry in std::fs::read_dir(path)? {
                let entry = entry?;
                let kind = entry.file_type()?;
                if kind.is_symlink() {
                    continue;
                }
                if kind.is_dir() {
                    visit(&entry.path(), paths)?;
                } else if entry
                    .path()
                    .extension()
                    .is_some_and(|s| s.eq_ignore_ascii_case("aui2"))
                {
                    paths.push(entry.path());
                }
            }
            Ok(())
        }
        let directory = directory()?;
        std::fs::create_dir_all(&directory)?;
        let mut paths = Vec::new();
        visit(&directory, &mut paths)?;
        paths.sort();
        for path in paths {
            if self.stopping.load(Ordering::Acquire) {
                break;
            }
            let (sender, receiver) = mpsc::channel();
            let (ready, result) = mpsc::sync_channel(1);
            let opened = Arc::new(AtomicUsize::new(0));
            let count = opened.clone();
            let cache = self.cache.clone();
            let dll = path.clone();
            std::thread::Builder::new()
                .name("aui2 input".into())
                .spawn(move || {
                    let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
                    let loaded = Plugin::load(&dll);
                    let shutdown = match loaded {
                        Ok(mut plugin) => {
                            let _ = ready.send(Ok((
                                plugin.name.clone(),
                                plugin.filters.clone(),
                                plugin.table().func_config.is_some(),
                            )));
                            plugin.run(receiver, count, cache)
                        }
                        Err(error) => {
                            let _ = ready.send(Err(format!("{error:#}")));
                            None
                        }
                    };
                    if com.is_ok() {
                        unsafe {
                            CoUninitialize();
                        }
                    }
                    if let Some(reply) = shutdown {
                        let _ = reply.try_send(Ok(()));
                    }
                })?;
            let entry = match result.recv()? {
                Ok((name, filters, can_config)) => Entry {
                    path,
                    name,
                    filters,
                    error: None,
                    client: Some(Client {
                        sender,
                        opened,
                        can_config,
                    }),
                },
                Err(error) => Entry {
                    name: path
                        .file_name()
                        .expect("plugin path")
                        .to_string_lossy()
                        .into_owned(),
                    path,
                    filters: String::new(),
                    error: Some(error),
                    client: None,
                },
            };
            self.entries.lock().expect("registry lock").push(entry);
        }
        Ok(())
    }

    pub async fn open(&self, path: PathBuf, limit: (u32, u64)) -> Result<Arc<VideoAsset>> {
        ensure!(
            !self.scanning.load(Ordering::Acquire),
            "入力プラグインを検索中です"
        );
        let filename = wide(path.file_name().context("ファイル名がありません")?);
        let entries = self.entries.lock().expect("registry lock").clone();
        let mut errors = Vec::new();
        for entry in entries {
            let Some(client) = entry.client else {
                continue;
            };
            let filters = wide(OsStr::new(&entry.filters));
            if !unsafe { PathMatchSpecW(PCWSTR(filename.as_ptr()), PCWSTR(filters.as_ptr())) }
                .as_bool()
            {
                continue;
            }
            let (reply, result) = async_channel::bounded(1);
            client
                .sender
                .send(Command::Open(path.clone(), limit, client.clone(), reply))?;
            match result.recv().await? {
                Ok(opened) => {
                    return Ok(Arc::new(VideoAsset {
                        id: opened.lease.id,
                        name: path
                            .file_name()
                            .context("ファイル名がありません")?
                            .to_string_lossy()
                            .into_owned(),
                        width: opened.format.width,
                        height: opened.format.height,
                        frames: opened.frames,
                        rate: opened.rate,
                        scale: opened.scale,
                        plugin: entry.name,
                        lease: opened.lease,
                    }));
                }
                Err(error) => errors.push(format!("{}: {error:#}", entry.name)),
            }
        }
        if errors.is_empty() {
            bail!("このファイルを扱う入力プラグインがありません");
        }
        bail!("{}", errors.join("\n"))
    }

    pub async fn shutdown(&self) {
        self.stopping.store(true, Ordering::Release);
        if self.scanning.load(Ordering::Acquire) {
            let _ = self.scanned.recv().await;
        }
        let entries = self.entries.lock().expect("registry lock").clone();
        let mut pending = Vec::new();
        for entry in entries {
            if let Some(client) = entry.client {
                let (reply, result) = async_channel::bounded(1);
                if client.sender.send(Command::Shutdown(reply)).is_ok() {
                    pending.push(result);
                }
            }
        }
        for result in pending {
            let _ = result.recv().await;
        }
    }
}
impl Client {
    pub async fn configure(&self, parent: usize) -> Result<()> {
        let (reply, result) = async_channel::bounded(1);
        self.sender.send(Command::Configure(parent, reply))?;
        result.recv().await?
    }
}
impl VideoAsset {
    pub async fn read(
        &self,
        time: u64,
        generation: Arc<AtomicU64>,
        expected: u64,
    ) -> Result<Arc<frame::Frame>> {
        let (reply, result) = async_channel::bounded(1);
        self.lease
            .client
            .sender
            .send(Command::Read(self.id, time, generation, expected, reply))?;
        result.recv().await?
    }
}

struct Plugin {
    module: HMODULE,
    table: INPUT_PLUGIN_TABLE,
    raw_table: *const INPUT_PLUGIN_TABLE,
    name: String,
    filters: String,
    initialized: bool,
    config: Box<services::Config>,
    logger: Box<services::Logger>,
    cache: Box<CACHE_HANDLE>,
}
struct Session {
    handle: INPUT_HANDLE,
    format: frame::Format,
    frames: i32,
    rate: u32,
    scale: u32,
    time_to_frame: Option<extern "C" fn(INPUT_HANDLE, f64) -> i32>,
}
impl Plugin {
    fn table(&self) -> &INPUT_PLUGIN_TABLE {
        &self.table
    }
    fn symbol(&self, name: &'static [u8]) -> Option<unsafe extern "system" fn() -> isize> {
        unsafe { GetProcAddress(self.module, PCSTR(name.as_ptr())) }
    }
    fn load(path: &Path) -> Result<Self> {
        let base = directory()?.parent().expect("plugin parent").to_path_buf();
        let config = services::Config::new(&base);
        let logger = services::Logger::new(&base.join("input-plugins.log"))?;
        let filename = wide(path.as_os_str());
        let module = unsafe {
            LoadLibraryExW(
                PCWSTR(filename.as_ptr()),
                None,
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
            )
        }?;
        let mut plugin = Self {
            module,
            table: unsafe { std::mem::zeroed() },
            raw_table: std::ptr::null(),
            name: String::new(),
            filters: String::new(),
            initialized: false,
            config,
            logger,
            cache: services::cache(),
        };
        // All exports use the SDK's C ABI. The module outlives every callback and input handle.
        unsafe {
            if let Some(function) = plugin.symbol(b"RequiredVersion\0") {
                let function: unsafe extern "C" fn() -> u32 = std::mem::transmute(function);
                ensure!(
                    function() <= VERSION,
                    "AviUtl2 2.1.14より新しいAPIが必要です"
                );
            }
            if let Some(function) = plugin.symbol(b"InitializeLogger\0") {
                let function: unsafe extern "C" fn(*mut LOG_HANDLE) = std::mem::transmute(function);
                function(&mut plugin.logger.table);
            }
            if let Some(function) = plugin.symbol(b"InitializeConfig\0") {
                let function: unsafe extern "C" fn(*mut CONFIG_HANDLE) =
                    std::mem::transmute(function);
                function(&mut plugin.config.table);
            }
            if let Some(function) = plugin.symbol(b"InitializeCache\0") {
                let function: unsafe extern "C" fn(*mut CACHE_HANDLE) =
                    std::mem::transmute(function);
                function(plugin.cache.as_mut());
            }
            if let Some(function) = plugin.symbol(b"InitializePlugin\0") {
                let function: unsafe extern "C" fn(u32) -> bool = std::mem::transmute(function);
                ensure!(function(VERSION), "InitializePluginが失敗しました");
            }
            plugin.initialized = true;
            let function = plugin
                .symbol(b"GetInputPluginTable\0")
                .context("GetInputPluginTableがありません")?;
            let function: unsafe extern "C" fn() -> *const INPUT_PLUGIN_TABLE =
                std::mem::transmute(function);
            plugin.raw_table = function();
            ensure!(!plugin.raw_table.is_null(), "入力テーブルがnullです");
            // Older SDK tables end at func_config; only read extensions when their flags opt in.
            std::ptr::copy_nonoverlapping(
                plugin.raw_table.cast::<u8>(),
                (&mut plugin.table as *mut INPUT_PLUGIN_TABLE).cast::<u8>(),
                std::mem::offset_of!(INPUT_PLUGIN_TABLE, func_set_track),
            );
            if plugin.table.flag & INPUT_PLUGIN_TABLE::FLAG_MULTI_TRACK != 0 {
                plugin.table.func_set_track =
                    std::ptr::addr_of!((*plugin.raw_table).func_set_track).read();
            }
            let table = plugin.table();
            ensure!(
                table.flag & INPUT_PLUGIN_TABLE::FLAG_VIDEO != 0,
                "映像入力に対応していません"
            );
            ensure!(
                table.func_open.is_some()
                    && table.func_close.is_some()
                    && table.func_info_get.is_some()
                    && table.func_read_video.is_some(),
                "必須の入力関数がありません"
            );
            if table.flag & INPUT_PLUGIN_TABLE::FLAG_MULTI_TRACK != 0 {
                ensure!(
                    table.func_set_track.is_some(),
                    "トラック選択関数がありません"
                );
            }
            let name = string(table.name)?;
            let mut cursor = table.filefilter;
            let mut patterns = Vec::new();
            for _ in 0..128 {
                let label = string(cursor)?;
                if label.is_empty() {
                    break;
                }
                cursor = cursor.add(label.encode_utf16().count() + 1);
                let pattern = string(cursor)?;
                ensure!(!pattern.is_empty(), "拡張子フィルターが不正です");
                cursor = cursor.add(pattern.encode_utf16().count() + 1);
                patterns.push(pattern.to_lowercase());
            }
            ensure!(*cursor == 0, "拡張子フィルターが多すぎます");
            plugin.name = name;
            plugin.filters = patterns.join(";");
        }
        Ok(plugin)
    }

    fn open(&self, path: &Path, limit: (u32, u64)) -> Result<Session> {
        let table = self.table();
        let path = wide(path.as_os_str());
        let handle = unsafe { table.func_open.expect("validated open")(path.as_ptr()) };
        ensure!(!handle.is_null(), "ファイルを開けませんでした");
        let result = (|| {
            if table.flag & INPUT_PLUGIN_TABLE::FLAG_MULTI_TRACK != 0 {
                let set = table.func_set_track.expect("validated tracks");
                ensure!(
                    set(handle, 0, -1) > 0 && set(handle, 0, 0) == 0,
                    "映像トラックを選択できません"
                );
            }
            let mut info: INPUT_INFO = unsafe { std::mem::zeroed() };
            ensure!(
                table.func_info_get.expect("validated info")(handle, &mut info),
                "映像情報を取得できません"
            );
            ensure!(
                info.flag & INPUT_INFO::FLAG_VIDEO != 0,
                "映像のないファイルです"
            );
            ensure!(
                info.rate > 0 && info.scale > 0 && info.n > 0,
                "フレーム数またはフレームレートが不正です"
            );
            let duration = (info.n as u128 * info.scale as u128 * u128::from(crate::timeline::FPS))
                .div_ceil(info.rate as u128);
            ensure!(
                duration <= u128::from(u64::MAX),
                "素材の長さがタイムラインの上限を超えています"
            );
            ensure!(
                !info.format.is_null()
                    && info.format_size >= std::mem::size_of::<BITMAPINFOHEADER>() as i32,
                "画像形式情報が不正です"
            );
            let header = unsafe { std::ptr::read_unaligned(info.format) };
            let format = frame::Format::new(header, limit)?;
            let vfr = info.flag & INPUT_INFO::FLAG_TIME_TO_FRAME != 0;
            let time_to_frame = if vfr {
                unsafe { std::ptr::addr_of!((*self.raw_table).func_time_to_frame).read() }
            } else {
                None
            };
            ensure!(!vfr || time_to_frame.is_some(), "時間変換関数がありません");
            Ok(Session {
                handle,
                format,
                frames: info.n,
                rate: info.rate as u32,
                scale: info.scale as u32,
                time_to_frame,
            })
        })();
        if result.is_err() {
            table.func_close.expect("validated close")(handle);
        }
        result
    }

    fn read(
        &self,
        session: &Session,
        id: u64,
        time: u64,
        cache: &Mutex<FrameCache>,
    ) -> Result<Arc<frame::Frame>> {
        let table = self.table();
        let number = if let Some(time_to_frame) = session.time_to_frame {
            time_to_frame(session.handle, time as f64 / crate::timeline::FPS as f64)
        } else {
            ((u128::from(time) * u128::from(session.rate))
                / (u128::from(crate::timeline::FPS) * u128::from(session.scale))) as i32
        };
        ensure!(
            (0..session.frames).contains(&number),
            "素材フレームが範囲外です: {number}"
        );
        if let Some(frame) = cache.lock().expect("frame cache").get((id, number)) {
            return Ok(frame);
        }
        let mut data = cache
            .lock()
            .expect("frame cache")
            .buffer(session.format.allocation);
        let count = table.func_read_video.expect("validated read")(
            session.handle,
            number,
            data.as_mut_ptr().cast(),
        );
        ensure!(
            count > 0
                && count as usize >= session.format.bytes
                && count as usize <= session.format.allocation,
            "フレーム{number}の読み込みサイズが不正です: {count}"
        );
        let frame = Arc::new(frame::Frame {
            number,
            format: session.format,
            data,
        });
        cache
            .lock()
            .expect("frame cache")
            .insert((id, number), frame.clone());
        Ok(frame)
    }

    fn run(
        &mut self,
        commands: mpsc::Receiver<Command>,
        count: Arc<AtomicUsize>,
        cache: Arc<Mutex<FrameCache>>,
    ) -> Option<Reply<()>> {
        let mut sessions: HashMap<u64, Session> = HashMap::new();
        let mut shutdown = None;
        while let Ok(command) = commands.recv() {
            match command {
                Command::Open(path, limit, client, reply) => {
                    let result = self.open(&path, limit).and_then(|session| {
                        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
                        if let Err(error) = self.read(&session, id, 0, &cache) {
                            self.table().func_close.expect("validated close")(session.handle);
                            return Err(error);
                        }
                        let result = Opened {
                            lease: Lease { client, id },
                            format: session.format,
                            frames: session.frames as u32,
                            rate: session.rate,
                            scale: session.scale,
                        };
                        sessions.insert(id, session);
                        count.fetch_add(1, Ordering::Relaxed);
                        Ok(result)
                    });
                    let _ = reply.try_send(result);
                }
                Command::Read(id, time, generation, expected, reply) => {
                    let result = if generation.load(Ordering::Acquire) != expected {
                        Err(anyhow::anyhow!("古いフレーム要求"))
                    } else {
                        sessions
                            .get(&id)
                            .context("素材は閉じられています")
                            .and_then(|session| self.read(session, id, time, &cache))
                    };
                    let _ = reply.try_send(result);
                }
                Command::Close(id) => {
                    if let Some(session) = sessions.remove(&id) {
                        self.table().func_close.expect("validated close")(session.handle);
                        count.fetch_sub(1, Ordering::Relaxed);
                        cache.lock().expect("cache").close(id);
                    }
                }
                Command::Configure(parent, reply) => {
                    let result = (|| {
                        ensure!(
                            sessions.is_empty(),
                            "使用中のため、再起動後に設定してください"
                        );
                        let configure = self.table().func_config.context("設定画面がありません")?;
                        ensure!(
                            configure(parent as *mut _, self.module.0),
                            "設定画面を開けませんでした"
                        );
                        Ok(())
                    })();
                    let _ = reply.try_send(result);
                }
                Command::Shutdown(reply) => {
                    shutdown = Some(reply);
                    break;
                }
            }
        }
        for (id, session) in sessions {
            self.table().func_close.expect("validated close")(session.handle);
            cache.lock().expect("cache").close(id);
        }
        count.store(0, Ordering::Relaxed);
        self.uninitialize();
        shutdown
    }
    fn uninitialize(&mut self) {
        if self.initialized {
            if let Some(function) = self.symbol(b"UninitializePlugin\0") {
                let function: unsafe extern "C" fn() = unsafe { std::mem::transmute(function) };
                unsafe {
                    function();
                }
            }
            self.initialized = false;
        }
    }
}
impl Drop for Plugin {
    fn drop(&mut self) {
        self.uninitialize();
        unsafe {
            let _ = FreeLibrary(self.module);
        }
    }
}

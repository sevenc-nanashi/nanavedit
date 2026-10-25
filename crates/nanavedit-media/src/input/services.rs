use aviutl2_sys::{cache2::*, config2::*, filter2::INPUT_PIXEL_FORMAT, logger2::LOG_HANDLE};
use std::{
    ffi::{c_char, c_void},
    path::Path,
    ptr::null_mut,
};

#[repr(C)]
pub struct Config {
    pub table: CONFIG_HANDLE,
    path: Vec<u16>,
    font_name: Vec<u16>,
    font: FONT_INFO,
}

impl Config {
    pub fn new(path: &Path, font_name: &str) -> Box<Self> {
        let path = super::wide(path.as_os_str());
        let font_name = super::wide(std::ffi::OsStr::new(font_name));
        let font = FONT_INFO {
            name: font_name.as_ptr(),
            size: 13.0,
        };
        Box::new(Self {
            table: CONFIG_HANDLE {
                app_data_path: path.as_ptr(),
                translate,
                get_language_text: language,
                get_font_info: font_info,
                get_color_code: zero,
                get_layout_size: zero,
                get_color_code_index: color_index,
            },
            path,
            font_name,
            font,
        })
    }
}
unsafe extern "C" fn translate(_: *mut CONFIG_HANDLE, text: *const u16) -> *const u16 {
    text
}
unsafe extern "C" fn language(
    _: *mut CONFIG_HANDLE,
    _: *const u16,
    text: *const u16,
) -> *const u16 {
    text
}
unsafe extern "C" fn font_info(handle: *mut CONFIG_HANDLE, _: *const c_char) -> *mut FONT_INFO {
    // CONFIG_HANDLE is the first field and is kept alive for the loaded DLL's lifetime.
    unsafe { &mut (*handle.cast::<Config>()).font }
}
unsafe extern "C" fn zero(_: *mut CONFIG_HANDLE, _: *const c_char) -> i32 {
    0
}
unsafe extern "C" fn color_index(_: *mut CONFIG_HANDLE, _: *const c_char, _: i32) -> i32 {
    0
}

#[repr(C)]
pub struct Logger {
    pub table: LOG_HANDLE,
    file: std::sync::Mutex<std::fs::File>,
}
impl Logger {
    pub fn new(path: &Path) -> std::io::Result<Box<Self>> {
        Ok(Box::new(Self {
            table: LOG_HANDLE {
                log,
                info: log,
                warn: log,
                error: log,
                verbose: log,
            },
            file: std::sync::Mutex::new(
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)?,
            ),
        }))
    }
}
unsafe extern "C" fn log(handle: *mut LOG_HANDLE, message: *const u16) {
    use std::io::Write;
    if message.is_null() {
        return;
    }
    let mut length = 0;
    while length < 1024 && unsafe { *message.add(length) } != 0 {
        length += 1;
    }
    let message = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(message, length) });
    let logger = unsafe { &*handle.cast::<Logger>() };
    if let Ok(mut file) = logger.file.lock() {
        let _ = writeln!(file, "{message}");
    }
}

fn reference() -> CACHE_REFERENCE {
    CACHE_REFERENCE {
        func_release: None,
        cache_instance: null_mut(),
    }
}
fn image() -> CACHE_IMAGE {
    CACHE_IMAGE {
        reference: reference(),
        buffer: null_mut(),
        width: 0,
        height: 0,
    }
}
fn audio() -> CACHE_AUDIO {
    CACHE_AUDIO {
        reference: reference(),
        buffer0: null_mut(),
        buffer1: null_mut(),
        sample_num: 0,
        channel_num: 0,
    }
}
fn file_image() -> CACHE_FILE_IMAGE {
    CACHE_FILE_IMAGE {
        reference: reference(),
        buffer: std::ptr::null(),
        width: 0,
        height: 0,
        pitch: 0,
        format: INPUT_PIXEL_FORMAT::HF64,
    }
}
unsafe extern "C" fn get_image(_: *mut c_void, _: *const u16) -> CACHE_IMAGE {
    image()
}
unsafe extern "C" fn create_image(_: *mut c_void, _: *const u16, _: i32, _: i32) -> CACHE_IMAGE {
    image()
}
unsafe extern "C" fn get_audio(_: *mut c_void, _: *const u16) -> CACHE_AUDIO {
    audio()
}
unsafe extern "C" fn create_audio(_: *mut c_void, _: *const u16, _: i32, _: i32) -> CACHE_AUDIO {
    audio()
}
unsafe extern "C" fn old_file(_: *const u16) -> CACHE_IMAGE {
    image()
}
unsafe extern "C" fn video_info(_: *const u16, _: *mut VIDEO_INFO, _: i32) -> bool {
    false
}
unsafe extern "C" fn audio_info(_: *const u16, _: *mut AUDIO_INFO, _: i32) -> bool {
    false
}
unsafe extern "C" fn get_file(_: *const u16) -> CACHE_FILE_IMAGE {
    file_image()
}
unsafe extern "C" fn get_frame(_: *const u16, _: i32, _: i32) -> CACHE_FILE_IMAGE {
    file_image()
}
unsafe extern "C" fn get_time(_: *const u16, _: i32, _: f64) -> CACHE_FILE_IMAGE {
    file_image()
}
unsafe extern "C" fn get_samples(
    _: *const u16,
    _: i32,
    _: i64,
    _: i32,
    _: *mut f32,
    _: *mut f32,
) -> i32 {
    0
}
unsafe extern "C" fn clear(_: *mut c_void, _: *const u16) {}
pub fn cache() -> Box<CACHE_HANDLE> {
    Box::new(CACHE_HANDLE {
        get_image_cache: get_image,
        create_image_cache: create_image,
        get_audio_cache: get_audio,
        create_audio_cache: create_audio,
        deprecated_get_image_file_cache: old_file,
        get_video_file_info: video_info,
        get_audio_file_info: audio_info,
        get_image_file_cache: get_file,
        get_video_file_cache: get_frame,
        get_video_file_cache_by_time: get_time,
        get_audio_file_data: get_samples,
        clear_image_cache: clear,
        clear_audio_cache: clear,
    })
}

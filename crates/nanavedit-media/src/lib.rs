#[cfg(target_os = "windows")]
mod image;
#[cfg(target_os = "windows")]
pub mod input;
#[cfg(target_os = "windows")]
pub use image::load_image;

#[derive(Debug, PartialEq, Eq)]
pub struct ImageAsset {
    pub id: u64,
    pub name: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug)]
pub struct VideoAsset {
    pub id: u64,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    pub rate: u32,
    pub scale: u32,
    pub plugin: String,
    #[cfg(target_os = "windows")]
    pub(crate) lease: crate::input::Lease,
}

impl PartialEq for VideoAsset {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for VideoAsset {}

impl VideoAsset {
    pub fn duration(&self, fps: u64) -> u64 {
        (u128::from(self.frames) * u128::from(self.scale) * u128::from(fps))
            .div_ceil(u128::from(self.rate)) as u64
    }
}

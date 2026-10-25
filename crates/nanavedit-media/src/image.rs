use anyhow::{Result, ensure};
use image::{ImageDecoder as _, ImageFormat, ImageReader, RgbaImage};
use std::path::Path;

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

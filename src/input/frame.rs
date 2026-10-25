use anyhow::{Result, ensure};
use aviutl2_sys::{common::*, input2::BITMAPINFOHEADER};

pub const BUDGET: usize = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub struct Format {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub kind: u32,
    pub bottom_up: bool,
    pub bytes: usize,
    pub allocation: usize,
}

impl Format {
    pub fn new(header: BITMAPINFOHEADER, limit: (u32, u64)) -> Result<Self> {
        // L-SMASH Works leaves biPlanes unset; its packed pixel format still defines the layout.
        ensure!(
            header.biSize >= 40 && matches!(header.biPlanes, 0 | 1),
            "画像ヘッダーが不正です"
        );
        ensure!(
            header.biWidth > 0 && header.biHeight != 0,
            "画像寸法が不正です"
        );
        let width = header.biWidth as u32;
        let height = header.biHeight.unsigned_abs();
        ensure!(
            width <= limit.0 && height <= limit.0,
            "画像寸法がGPUの上限を超えています"
        );
        let (kind, stride) = match (header.biCompression, header.biBitCount) {
            (BI_RGB, 24) => (0, (u64::from(width) * 3).div_ceil(4) * 4),
            (BI_RGB, 32) => (1, u64::from(width) * 4),
            (BI_YUY2, 16) => {
                ensure!(
                    width.is_multiple_of(2),
                    "YUY2の幅は偶数である必要があります"
                );
                (2, u64::from(width) * 2)
            }
            (BI_PA64, 64) => (3, u64::from(width) * 8),
            (BI_HF64, 64) => (4, u64::from(width) * 8),
            (BI_YC48, 48) => (5, u64::from(width) * 6),
            _ => anyhow::bail!(
                "未対応の画像形式です: compression={:#x}, {}bit",
                header.biCompression,
                header.biBitCount
            ),
        };
        let bytes = stride * u64::from(height);
        ensure!(
            bytes.div_ceil(4) * 4 <= limit.1,
            "フレームがGPU転送バッファの上限を超えています"
        );
        let allocation = bytes.max(u64::from(header.biSizeImage));
        ensure!(
            allocation <= BUDGET as u64,
            "1フレームが256MiBを超えています"
        );
        ensure!(
            u64::from(width) * u64::from(height) * 8 <= BUDGET as u64,
            "変換後の1フレームが256MiBを超えています"
        );
        Ok(Self {
            width,
            height,
            stride: stride as u32,
            kind,
            bottom_up: kind <= 1 && header.biHeight > 0,
            bytes: bytes as usize,
            allocation: allocation as usize,
        })
    }
}

pub struct Frame {
    pub number: i32,
    pub format: Format,
    pub data: Vec<u64>,
}

impl Frame {
    pub fn bytes(&self) -> &[u8] {
        // The plugin writes into aligned storage; only the validated image payload is uploaded.
        unsafe {
            std::slice::from_raw_parts(self.data.as_ptr().cast(), self.format.bytes.div_ceil(4) * 4)
        }
    }
    pub fn size(&self) -> usize {
        self.data.len() * 8
    }
}

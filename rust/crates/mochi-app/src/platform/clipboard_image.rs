//! 仅在明确执行粘贴时读取剪贴板。关闭系统剪贴板前，先复制大小受限的数据。
use anyhow::{bail, ensure, Result};
use windows::Win32::{
    Foundation::HGLOBAL,
    System::{
        DataExchange::{
            CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
            RegisterClipboardFormatW,
        },
        Memory::{GlobalLock, GlobalSize, GlobalUnlock},
    },
};
const RAW_LIMIT: usize = 64 * 1024 * 1024;
const PNG_LIMIT: usize = mochi_core::ai::assets::MAX_IMAGE_BYTES;
fn u32_at(b: &[u8], i: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(i..i + 4)
            .ok_or_else(|| anyhow::anyhow!("位图头不完整"))?
            .try_into()?,
    ))
}
fn u16_at(b: &[u8], i: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        b.get(i..i + 2)
            .ok_or_else(|| anyhow::anyhow!("位图头不完整"))?
            .try_into()?,
    ))
}
fn png_end(b: &[u8]) -> Result<usize> {
    ensure!(b.starts_with(b"\x89PNG\r\n\x1a\n"), "剪贴板 PNG 格式无效");
    let mut p = 8usize;
    let mut first = true;
    while p.checked_add(12).is_some_and(|end| end <= b.len()) {
        let n = u32::from_be_bytes(b[p..p + 4].try_into()?) as usize;
        let end = p
            .checked_add(n)
            .and_then(|n| n.checked_add(12))
            .ok_or_else(|| anyhow::anyhow!("PNG 长度溢出"))?;
        ensure!(
            end <= b.len() && end <= PNG_LIMIT,
            "PNG 不完整或超过图片大小限制"
        );
        if first {
            ensure!(&b[p + 4..p + 8] == b"IHDR" && n == 13, "PNG 缺少图像头");
            let w = u32::from_be_bytes(b[p + 8..p + 12].try_into()?);
            let h = u32::from_be_bytes(b[p + 12..p + 16].try_into()?);
            ensure!(
                w > 0 && h > 0 && u64::from(w) * u64::from(h) <= 64_000_000,
                "图片尺寸超出限制"
            );
            first = false;
        }
        if &b[p + 4..p + 8] == b"IEND" {
            ensure!(n == 0, "PNG 结束块无效");
            return Ok(end);
        }
        p = end;
    }
    bail!("PNG 缺少结束块")
}
/// Windows 剪贴板中的 DIB 不含 14 字节的 BMP 文件头。
pub(super) fn dib_bmp(dib: &[u8]) -> Result<Vec<u8>> {
    ensure!(dib.len() <= RAW_LIMIT, "剪贴板位图超过 64 MiB");
    let header = u32_at(dib, 0)? as usize;
    ensure!(
        [12, 40, 52, 56, 108, 124].contains(&header) && dib.len() >= header,
        "不支持的位图头"
    );
    let (width, height, bits, colors, compression, entry) = if header == 12 {
        ensure!(u16_at(dib, 8)? == 1, "位图平面数无效");
        (
            u32::from(u16_at(dib, 4)?),
            u32::from(u16_at(dib, 6)?),
            u16_at(dib, 10)?,
            0,
            0,
            3,
        )
    } else {
        let w = u32_at(dib, 4)? as i32;
        let h = u32_at(dib, 8)? as i32;
        ensure!(
            w > 0 && h != 0 && h != i32::MIN && u16_at(dib, 12)? == 1,
            "位图尺寸无效"
        );
        (
            w as u32,
            h.unsigned_abs(),
            u16_at(dib, 14)?,
            u32_at(dib, 32)?,
            u32_at(dib, 16)?,
            4,
        )
    };
    ensure!(
        width > 0 && height > 0 && u64::from(width) * u64::from(height) <= 16_777_216,
        "剪贴板位图尺寸超出限制"
    );
    ensure!(
        [1, 4, 8, 16, 24, 32].contains(&bits) && [0, 3, 6].contains(&compression),
        "不支持压缩的剪贴板位图"
    );
    ensure!(
        compression == 0 || [16, 32].contains(&bits),
        "位图掩码格式无效"
    );
    ensure!(
        compression != 6 || header == 40 || header >= 56,
        "位图透明度掩码不完整"
    );
    if header >= 108 {
        ensure!(
            ![0x4c494e4b, 0x4d424544].contains(&u32_at(dib, 56)?),
            "不支持外部或嵌入色彩配置"
        );
    }
    if header == 124 {
        ensure!(
            u32_at(dib, 112)? == 0 && u32_at(dib, 116)? == 0,
            "暂不支持带外部或嵌入色彩配置的位图"
        );
    }
    let palette = if bits <= 8 {
        let count = if colors == 0 { 1u32 << bits } else { colors };
        ensure!(count <= 1u32 << bits, "调色板长度无效");
        count as usize * entry
    } else {
        ensure!(colors <= 256, "调色板长度无效");
        colors as usize * entry
    };
    let masks = if header == 40 {
        match compression {
            3 => 12,
            6 => 16,
            _ => 0,
        }
    } else {
        0
    };
    let offset = header + palette + masks;
    let stride = (u64::from(width) * u64::from(bits)).div_ceil(32) * 4;
    let length = stride * u64::from(height);
    ensure!(length <= RAW_LIMIT as u64, "位图数据过大");
    let end = offset
        .checked_add(length as usize)
        .ok_or_else(|| anyhow::anyhow!("位图长度溢出"))?;
    ensure!(end <= dib.len(), "位图像素不完整");
    let mut bmp = Vec::with_capacity(end + 14);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&((end + 14) as u32).to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&((offset + 14) as u32).to_le_bytes());
    bmp.extend_from_slice(&dib[..end]);
    if header >= 40 {
        bmp[34..38].copy_from_slice(&(length as u32).to_le_bytes());
    }
    Ok(bmp)
}
fn dib_png(dib: &[u8]) -> Result<Vec<u8>> {
    encode_png(&dib_bmp(dib)?)
}
pub(super) fn encode_png(bytes: &[u8]) -> Result<Vec<u8>> {
    use windows::Win32::{
        Graphics::Imaging::*,
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize,
            StructuredStorage::{CreateStreamOnHGlobal, GetHGlobalFromStream},
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, STATFLAG_NONAME, STATSTG,
        },
        UI::Shell::SHCreateMemStream,
    };
    struct Com(bool);
    impl Drop for Com {
        fn drop(&mut self) {
            if self.0 {
                unsafe {
                    CoUninitialize();
                }
            }
        }
    }
    let _com = Com(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok());
    unsafe {
        let wic: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let input =
            SHCreateMemStream(Some(bytes)).ok_or_else(|| anyhow::anyhow!("无法读取位图"))?;
        let decoder =
            wic.CreateDecoderFromStream(&input, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?;
        let source = decoder.GetFrame(0)?;
        let output = CreateStreamOnHGlobal(HGLOBAL::default(), true)?;
        let encoder = wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        encoder.Initialize(&output, WICBitmapEncoderNoCache)?;
        let mut frame = None;
        encoder.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
        let frame = frame.ok_or_else(|| anyhow::anyhow!("无法创建 PNG"))?;
        frame.Initialize(None)?;
        let (mut w, mut h) = (0, 0);
        source.GetSize(&mut w, &mut h)?;
        ensure!(
            w > 0 && h > 0 && u64::from(w) * u64::from(h) <= 16_777_216,
            "图片尺寸超出限制"
        );
        frame.SetSize(w, h)?;
        let mut format = GUID_WICPixelFormat32bppBGRA;
        frame.SetPixelFormat(&mut format)?;
        let converter = wic.CreateFormatConverter()?;
        converter.Initialize(
            &source,
            &format,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )?;
        frame.WriteSource(&converter, std::ptr::null())?;
        frame.Commit()?;
        encoder.Commit()?;
        let mut stat = STATSTG::default();
        output.Stat(&mut stat, STATFLAG_NONAME)?;
        ensure!(
            stat.cbSize <= PNG_LIMIT as u64,
            "转换后的 PNG 超过图片大小限制"
        );
        let handle = GetHGlobalFromStream(&output)?;
        let locked = Locked::new(handle)?;
        ensure!(stat.cbSize as usize <= locked.len, "PNG 流长度无效");
        Ok(locked.bytes()[..stat.cbSize as usize].to_vec())
    }
}
struct Locked {
    handle: HGLOBAL,
    pointer: *const u8,
    len: usize,
}
impl Locked {
    unsafe fn new(handle: HGLOBAL) -> Result<Self> {
        let len = unsafe { GlobalSize(handle) };
        ensure!(len > 0 && len <= RAW_LIMIT, "剪贴板图像为空或过大");
        let pointer = unsafe { GlobalLock(handle) }.cast::<u8>();
        ensure!(!pointer.is_null(), "无法读取剪贴板图像");
        Ok(Self {
            handle,
            pointer,
            len,
        })
    }
    fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.pointer, self.len) }
    }
}
impl Drop for Locked {
    fn drop(&mut self) {
        unsafe {
            let _ = GlobalUnlock(self.handle);
        }
    }
}
pub fn read_png() -> Result<Option<Vec<u8>>> {
    struct Close;
    impl Drop for Close {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseClipboard();
            }
        }
    }
    let dib = unsafe {
        OpenClipboard(None).map_err(|_| anyhow::anyhow!("剪贴板正被其它程序占用，请重试"))?;
        let _close = Close;
        for name in [windows::core::w!("PNG"), windows::core::w!("image/png")] {
            let format = RegisterClipboardFormatW(name);
            if format != 0 && IsClipboardFormatAvailable(format).is_ok() {
                let data = GetClipboardData(format)?;
                let locked = Locked::new(HGLOBAL(data.0))?;
                let end = png_end(locked.bytes())?;
                return Ok(Some(locked.bytes()[..end].to_vec()));
            }
        }
        let format = if IsClipboardFormatAvailable(17).is_ok() {
            17
        } else if IsClipboardFormatAvailable(8).is_ok() {
            8
        } else {
            return Ok(None);
        };
        let data = GetClipboardData(format)?;
        let locked = Locked::new(HGLOBAL(data.0))?;
        locked.bytes().to_vec()
    };
    Ok(Some(dib_png(&dib)?))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn pixels(png: &[u8]) -> Vec<u8> {
        use windows::Win32::{
            Graphics::Imaging::*,
            System::Com::{
                CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
                COINIT_APARTMENTTHREADED,
            },
            UI::Shell::SHCreateMemStream,
        };
        struct Com(bool);
        impl Drop for Com {
            fn drop(&mut self) {
                if self.0 {
                    unsafe {
                        CoUninitialize();
                    }
                }
            }
        }
        let _com = Com(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok());
        unsafe {
            let wic: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).unwrap();
            let stream = SHCreateMemStream(Some(png)).unwrap();
            let decoder = wic
                .CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)
                .unwrap();
            let frame = decoder.GetFrame(0).unwrap();
            let (mut w, mut h) = (0, 0);
            frame.GetSize(&mut w, &mut h).unwrap();
            let converter = wic.CreateFormatConverter().unwrap();
            converter
                .Initialize(
                    &frame,
                    &GUID_WICPixelFormat32bppBGRA,
                    WICBitmapDitherTypeNone,
                    None,
                    0.0,
                    WICBitmapPaletteTypeCustom,
                )
                .unwrap();
            let mut out = vec![0; (w * h * 4) as usize];
            converter
                .CopyPixels(std::ptr::null(), w * 4, &mut out)
                .unwrap();
            out
        }
    }
    fn dib() -> Vec<u8> {
        let mut b = vec![0; 40];
        b[..4].copy_from_slice(&40u32.to_le_bytes());
        b[4..8].copy_from_slice(&2i32.to_le_bytes());
        b[8..12].copy_from_slice(&2i32.to_le_bytes());
        b[12..14].copy_from_slice(&1u16.to_le_bytes());
        b[14..16].copy_from_slice(&24u16.to_le_bytes());
        b.extend_from_slice(&[255, 0, 0, 255, 0, 0, 0, 0, 0, 0, 255, 0, 0, 255, 0, 0]);
        b
    }
    #[test]
    fn valid_dib_encodes_and_png_padding_is_not_copied() {
        let png = dib_png(&dib()).unwrap();
        assert!(png.starts_with(b"\x89PNG"));
        let mut padded = png.clone();
        padded.extend_from_slice(b"private allocation padding");
        assert_eq!(png_end(&padded).unwrap(), png.len());
        assert_eq!(
            crate::ui::imginfo::dimensions_from_bytes(&png),
            Some((2, 2))
        );
    }
    #[test]
    fn dib_orientation_and_v5_alpha_survive_conversion() {
        let data = pixels(&dib_png(&dib()).unwrap());
        assert_eq!(&data[..4], &[0, 0, 255, 255]);
        assert_eq!(&data[8..12], &[255, 0, 0, 255]);
        let mut rgb = dib();
        rgb[4..8].copy_from_slice(&1u32.to_le_bytes());
        rgb[8..12].copy_from_slice(&1u32.to_le_bytes());
        rgb[14..16].copy_from_slice(&32u16.to_le_bytes());
        rgb.truncate(40);
        rgb.extend_from_slice(&[0, 255, 0, 0]);
        assert_eq!(pixels(&dib_png(&rgb).unwrap()), vec![0, 255, 0, 255]);
        let mut v5 = vec![0; 124];
        v5[..4].copy_from_slice(&124u32.to_le_bytes());
        v5[4..8].copy_from_slice(&2u32.to_le_bytes());
        v5[8..12].copy_from_slice(&(-1i32).to_le_bytes());
        v5[12..14].copy_from_slice(&1u16.to_le_bytes());
        v5[14..16].copy_from_slice(&32u16.to_le_bytes());
        v5[16..20].copy_from_slice(&3u32.to_le_bytes());
        for (i, mask) in [0x00ff0000u32, 0x0000ff00, 0x000000ff, 0xff000000]
            .iter()
            .enumerate()
        {
            v5[40 + i * 4..44 + i * 4].copy_from_slice(&mask.to_le_bytes());
        }
        v5[56..60].copy_from_slice(&0x73524742u32.to_le_bytes());
        v5.extend_from_slice(&[0, 0, 255, 128, 255, 0, 0, 255]);
        assert_eq!(
            pixels(&dib_png(&v5).unwrap()),
            vec![0, 0, 255, 128, 255, 0, 0, 255]
        );
    }
    #[test]
    fn malformed_sizes_profiles_and_truncation_are_rejected() {
        let mut b = dib();
        b[4..8].copy_from_slice(&i32::MAX.to_le_bytes());
        assert!(dib_bmp(&b).is_err());
        assert!(dib_bmp(&dib()[..42]).is_err());
        assert!(png_end(b"\x89PNG\r\n\x1a\n").is_err());
        let mut v5 = vec![0; 140];
        v5[..4].copy_from_slice(&124u32.to_le_bytes());
        v5[4..8].copy_from_slice(&1u32.to_le_bytes());
        v5[8..12].copy_from_slice(&1u32.to_le_bytes());
        v5[12..14].copy_from_slice(&1u16.to_le_bytes());
        v5[14..16].copy_from_slice(&32u16.to_le_bytes());
        v5[112..116].copy_from_slice(&124u32.to_le_bytes());
        v5[116..120].copy_from_slice(&4u32.to_le_bytes());
        assert!(dib_bmp(&v5).is_err());
    }
}

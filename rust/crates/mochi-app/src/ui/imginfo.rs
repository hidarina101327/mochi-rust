//! 排版只读取图片头；完整解码留给 gfx。不支持的格式返回 None。

use std::fs::File;
use std::io::Read;
use std::path::Path;

/// 图片的 (宽, 高) 像素。读不出来（格式不认识、文件损坏、不存在）返回 `None`。
pub fn dimensions(path: &Path) -> Option<(u32, u32)> {
    let mut file = File::open(path).ok()?;
    // JPEG 的 SOF 段可能在几 KB 之后（EXIF 缩略图在前），多读一些
    let mut buf = vec![0u8; 64 * 1024];
    let n = file.read(&mut buf).ok()?;
    dimensions_from_bytes(&buf[..n])
}

pub fn dimensions_from_bytes(b: &[u8]) -> Option<(u32, u32)> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        return png(b);
    }
    if b.starts_with(&[0xFF, 0xD8]) {
        return jpeg(b);
    }
    if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        return gif(b);
    }
    if b.starts_with(b"BM") {
        return bmp(b);
    }
    if b.starts_with(b"RIFF") && b.get(8..12) == Some(b"WEBP") {
        return webp(b);
    }
    None
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn le16(b: &[u8], at: usize) -> Option<u32> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?) as u32)
}

fn be16(b: &[u8], at: usize) -> Option<u32> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?) as u32)
}

/// IHDR 紧跟 8 字节签名：长度(4) + "IHDR"(4) + 宽(4) + 高(4)。
fn png(b: &[u8]) -> Option<(u32, u32)> {
    if b.get(12..16)? != b"IHDR" {
        return None;
    }
    Some((be32(b, 16)?, be32(b, 20)?))
}

/// 逻辑屏幕尺寸，小端 16 位。
fn gif(b: &[u8]) -> Option<(u32, u32)> {
    Some((le16(b, 6)?, le16(b, 8)?))
}

/// BITMAPINFOHEADER：宽在 18，高在 22（高可能为负表示自上而下）。
fn bmp(b: &[u8]) -> Option<(u32, u32)> {
    let w = le32(b, 18)? as i32;
    let h = le32(b, 22)? as i32;
    Some((w.unsigned_abs(), h.unsigned_abs()))
}

/// 逐段扫到 SOF（C0..CF，除 C4/C8/CC）：段内 高(2) 宽(2) 在偏移 5。
fn jpeg(b: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2;
    while i + 4 <= b.len() {
        if b[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = b[i + 1];
        // 填充字节
        if marker == 0xFF {
            i += 1;
            continue;
        }
        // 无长度的独立标记
        if (0xD0..=0xD9).contains(&marker) || marker == 0x01 {
            i += 2;
            continue;
        }
        let len = be16(b, i + 2)? as usize;
        let is_sof = (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
        if is_sof {
            let h = be16(b, i + 5)?;
            let w = be16(b, i + 7)?;
            return Some((w, h));
        }
        i += 2 + len;
    }
    None
}

/// VP8（有损）：帧头后 14 位宽高；VP8L（无损）：14 位 + 14 位打包；VP8X：24 位减一。
fn webp(b: &[u8]) -> Option<(u32, u32)> {
    let chunk = b.get(12..16)?;
    match chunk {
        b"VP8 " => {
            // 3 字节帧标签 + 3 字节起始码 (9d 01 2a) + 宽(2) 高(2)，各取低 14 位
            let w = le16(b, 26)? & 0x3FFF;
            let h = le16(b, 28)? & 0x3FFF;
            Some((w, h))
        }
        b"VP8L" => {
            let bits = le32(b, 21)?;
            let w = (bits & 0x3FFF) + 1;
            let h = ((bits >> 14) & 0x3FFF) + 1;
            Some((w, h))
        }
        b"VP8X" => {
            let le24 = |at: usize| -> Option<u32> {
                let s = b.get(at..at + 3)?;
                Some(s[0] as u32 | (s[1] as u32) << 8 | (s[2] as u32) << 16)
            };
            Some((le24(24)? + 1, le24(27)? + 1))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_dimensions_come_from_ihdr() {
        let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
        b.extend_from_slice(&13u32.to_be_bytes());
        b.extend_from_slice(b"IHDR");
        b.extend_from_slice(&640u32.to_be_bytes());
        b.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(dimensions_from_bytes(&b), Some((640, 480)));
    }

    #[test]
    fn jpeg_dimensions_come_from_the_first_sof_after_app_segments() {
        let mut b = vec![0xFF, 0xD8];
        // APP0 段，长度 16
        b.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x10]);
        b.extend_from_slice(&[0u8; 14]);
        // DHT（C4）要跳过，不是 SOF
        b.extend_from_slice(&[0xFF, 0xC4, 0x00, 0x04, 0, 0]);
        // SOF0：长度 17，精度 8，高 300，宽 500
        b.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08]);
        b.extend_from_slice(&300u16.to_be_bytes());
        b.extend_from_slice(&500u16.to_be_bytes());
        b.extend_from_slice(&[0u8; 10]);
        assert_eq!(dimensions_from_bytes(&b), Some((500, 300)));
    }

    #[test]
    fn gif_bmp_and_webp_headers_are_read_little_endian() {
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&120u16.to_le_bytes());
        gif.extend_from_slice(&80u16.to_le_bytes());
        assert_eq!(dimensions_from_bytes(&gif), Some((120, 80)));

        let mut bmp = vec![0u8; 26];
        bmp[0] = b'B';
        bmp[1] = b'M';
        bmp[18..22].copy_from_slice(&64i32.to_le_bytes());
        bmp[22..26].copy_from_slice(&(-32i32).to_le_bytes());
        assert_eq!(
            dimensions_from_bytes(&bmp),
            Some((64, 32)),
            "负高度表示自上而下，取绝对值"
        );

        let mut webp = b"RIFF\0\0\0\0WEBPVP8L".to_vec();
        webp.extend_from_slice(&0u32.to_le_bytes()); // 数据块大小
        webp.push(0x2f); // 签名
        let bits: u32 = (99u32) | (49u32 << 14); // 宽 100，高 50（各减一存）
        webp.extend_from_slice(&bits.to_le_bytes());
        assert_eq!(dimensions_from_bytes(&webp), Some((100, 50)));
    }

    #[test]
    fn unknown_or_truncated_input_yields_none() {
        assert_eq!(dimensions_from_bytes(b"hello"), None);
        assert_eq!(dimensions_from_bytes(b"\x89PNG\r\n\x1a\n\0\0"), None);
        assert_eq!(dimensions(Path::new("Z:/does/not/exist.png")), None);
    }
}

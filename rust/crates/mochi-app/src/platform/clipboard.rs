//! CF_UNICODETEXT + CF_HTML。EmptyClipboard 之前先备好自己的缓冲。
//! <https://learn.microsoft.com/windows/win32/dataxchg/html-clipboard-format>
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
const MAX_BYTES: usize = 32 * 1024 * 1024;

fn html_bytes(fragment: &str) -> Option<Vec<u8>> {
    if fragment.len() > MAX_BYTES || fragment.contains('\0') {
        return None;
    }
    let prefix = "<html><head><meta charset=\"utf-8\"></head><body><!--StartFragment-->";
    let suffix = "<!--EndFragment--></body></html>";
    let header = |start, end, from, to| {
        format!("Version:1.0\r\nStartHTML:{start:010}\r\nEndHTML:{end:010}\r\nStartFragment:{from:010}\r\nEndFragment:{to:010}\r\n")
    };
    let start = header(0, 0, 0, 0).len();
    let from = start + prefix.len();
    let to = from + fragment.len();
    let end = to + suffix.len();
    if end >= MAX_BYTES {
        return None;
    }
    let mut result =
        format!("{}{prefix}{fragment}{suffix}", header(start, end, from, to)).into_bytes();
    result.push(0);
    Some(result)
}
struct Memory(Option<HGLOBAL>);
impl Memory {
    fn new(bytes: &[u8]) -> Option<Self> {
        unsafe {
            let mem = Self(Some(GlobalAlloc(GMEM_MOVEABLE, bytes.len()).ok()?));
            let pointer = GlobalLock(mem.0?);
            if pointer.is_null() {
                return None;
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast(), bytes.len());
            let _ = GlobalUnlock(mem.0?);
            Some(mem)
        }
    }
    fn publish(&mut self, format: u32) -> bool {
        unsafe {
            let Some(handle) = self.0 else { return false };
            if SetClipboardData(format, Some(HANDLE(handle.0))).is_err() {
                return false;
            }
            self.0 = None;
            true // SetClipboardData 成功后，数据所有权会转交给 Windows。
        }
    }
}
impl Drop for Memory {
    fn drop(&mut self) {
        if let Some(mem) = self.0 {
            unsafe {
                let _ = GlobalFree(Some(mem));
            }
        }
    }
}
struct Clipboard;
impl Drop for Clipboard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

#[derive(Clone, Copy, Default)]
struct HeaderField {
    present: bool,
    value: Option<i64>,
}

#[derive(Default)]
struct HeaderOffsets {
    start_html: HeaderField,
    end_html: HeaderField,
    start_fragment: HeaderField,
    end_fragment: HeaderField,
}

/// 即使解析提前返回，锁定的 HGLOBAL 也必须解锁。
struct LockedGlobal {
    handle: HGLOBAL,
    pointer: *const u8,
    length: usize,
}

impl LockedGlobal {
    unsafe fn new(handle: HGLOBAL, length: usize) -> Option<Self> {
        let pointer = GlobalLock(handle);
        if pointer.is_null() {
            return None;
        }
        Some(Self {
            handle,
            pointer: pointer.cast(),
            length,
        })
    }

    fn bytes(&self) -> &[u8] {
        // 这个借用存续期间，指针保持锁定。
        unsafe { std::slice::from_raw_parts(self.pointer, self.length) }
    }
}

impl Drop for LockedGlobal {
    fn drop(&mut self) {
        unsafe {
            let _ = GlobalUnlock(self.handle);
        }
    }
}

fn trim_ascii_whitespace(mut value: &[u8]) -> &[u8] {
    while value.first().is_some_and(u8::is_ascii_whitespace) {
        value = &value[1..];
    }
    while value.last().is_some_and(u8::is_ascii_whitespace) {
        value = &value[..value.len() - 1];
    }
    value
}

fn parse_offset(value: &[u8]) -> Option<i64> {
    let value = trim_ascii_whitespace(value);
    if value.is_empty() {
        return None;
    }
    let (negative, digits) = match value[0] {
        b'-' => (true, &value[1..]),
        _ => (false, value),
    };
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let mut number = 0i64;
    for digit in digits {
        number = number
            .checked_mul(10)?
            .checked_add(i64::from(*digit - b'0'))?;
    }
    if negative {
        number.checked_neg()
    } else {
        Some(number)
    }
}

fn line_bounds(bytes: &[u8], start: usize) -> (usize, usize) {
    let mut end = start;
    while end < bytes.len() && bytes[end] != b'\r' && bytes[end] != b'\n' {
        end += 1;
    }
    let next = match bytes.get(end) {
        Some(b'\r') if bytes.get(end + 1) == Some(&b'\n') => end + 2,
        Some(_) => end + 1,
        None => end,
    };
    (end, next)
}

fn is_header_key(key: &[u8]) -> bool {
    !key.is_empty()
        && key[0].is_ascii_alphabetic()
        && key
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
}

fn is_header_line(line: &[u8]) -> Option<(&[u8], &[u8])> {
    let colon = line.iter().position(|&byte| byte == b':')?;
    let key = trim_ascii_whitespace(&line[..colon]);
    let value = &line[colon + 1..];
    if !is_header_key(key) {
        return None;
    }
    Some((key, value))
}

fn is_known_header_key(key: &[u8]) -> bool {
    key.eq_ignore_ascii_case(b"Version")
        || key.eq_ignore_ascii_case(b"StartHTML")
        || key.eq_ignore_ascii_case(b"EndHTML")
        || key.eq_ignore_ascii_case(b"StartFragment")
        || key.eq_ignore_ascii_case(b"EndFragment")
        || key.eq_ignore_ascii_case(b"StartSelection")
        || key.eq_ignore_ascii_case(b"EndSelection")
}

fn set_header_field(field: &mut HeaderField, value: &[u8]) {
    field.present = true;
    field.value = parse_offset(value);
}

fn parse_header(bytes: &[u8]) -> (HeaderOffsets, usize) {
    let mut fields = HeaderOffsets::default();
    // BOM 不属于 CF_HTML 头部的语法，但在这里跳过它，
    // 对会前置 BOM 的生产者，标记回退才有用。偏移量仍然
    // 相对原始字节缓冲。
    let mut position = if bytes.starts_with(b"\xEF\xBB\xBF") {
        3
    } else {
        0
    };
    while position < bytes.len() {
        let (line_end, next) = line_bounds(bytes, position);
        let line = &bytes[position..line_end];
        if line.is_empty() {
            return (fields, next);
        }
        let Some((key, value)) = is_header_line(line) else {
            return (fields, position);
        };
        // 未知的头部扩展都是 ASCII。遇到 '<' 或非 ASCII 字节就说明
        // 进入普通的 HTML/文本；已知的偏移字段即便存在也按
        // 「存在但无效」保留，畸形值才不会触发标记回退、
        // 让坏输入意外变成可接受的内容。
        if !is_known_header_key(key) && value.iter().any(|byte| *byte >= 0x80 || *byte == b'<') {
            return (fields, position);
        }
        if key.eq_ignore_ascii_case(b"StartHTML") {
            set_header_field(&mut fields.start_html, value);
        } else if key.eq_ignore_ascii_case(b"EndHTML") {
            set_header_field(&mut fields.end_html, value);
        } else if key.eq_ignore_ascii_case(b"StartFragment") {
            set_header_field(&mut fields.start_fragment, value);
        } else if key.eq_ignore_ascii_case(b"EndFragment") {
            set_header_field(&mut fields.end_fragment, value);
        }
        position = next;
    }
    (fields, position)
}

fn checked_bound(field: HeaderField, length: usize) -> Result<Option<usize>, ()> {
    if !field.present {
        return Ok(None);
    }
    let value = field.value.ok_or(())?;
    if value == -1 {
        return Ok(None);
    }
    if value < 0 {
        return Err(());
    }
    let value = usize::try_from(value).map_err(|_| ())?;
    (value <= length).then_some(Some(value)).ok_or(())
}

fn html_range(
    fields: &HeaderOffsets,
    length: usize,
    header_end: usize,
) -> Result<(usize, usize), ()> {
    let start = checked_bound(fields.start_html, length)?;
    let end = checked_bound(fields.end_html, length)?;
    let start = start.unwrap_or(header_end);
    let end = end.unwrap_or(length);
    if start > end {
        return Err(());
    }
    Ok((start, end))
}

fn marker_length_at(bytes: &[u8], position: usize, start: bool) -> Option<usize> {
    const OPEN: &[u8] = b"<!--";
    const START: &[u8] = b"StartFragment";
    const END: &[u8] = b"EndFragment";
    if !bytes.get(position..)?.starts_with(OPEN) {
        return None;
    }
    let mut cursor = position + OPEN.len();
    while matches!(bytes.get(cursor), Some(b' ' | b'\t')) {
        cursor += 1;
    }
    let name = if start { START } else { END };
    if !bytes
        .get(cursor..)?
        .get(..name.len())?
        .eq_ignore_ascii_case(name)
    {
        return None;
    }
    cursor += name.len();
    while matches!(bytes.get(cursor), Some(b' ' | b'\t')) {
        cursor += 1;
    }
    if !bytes.get(cursor..)?.starts_with(b"-->") {
        return None;
    }
    Some(cursor + 3)
}

fn find_marker(bytes: &[u8], range: (usize, usize), start: bool) -> Option<(usize, usize)> {
    let (from, to) = range;
    let mut position = from;
    while position < to {
        if let Some(end) = marker_length_at(bytes, position, start) {
            if end <= to {
                return Some((position, end));
            }
        }
        position += 1;
    }
    None
}

fn marker_fragment(bytes: &[u8], range: (usize, usize)) -> Option<String> {
    let (_, start_end) = find_marker(bytes, range, true)?;
    let (end_start, _) = find_marker(bytes, (start_end, range.1), false)?;
    if start_end > end_start {
        return None;
    }
    std::str::from_utf8(&bytes[start_end..end_start])
        .ok()
        .map(str::to_owned)
}

fn offset_fragment(
    bytes: &[u8],
    fields: &HeaderOffsets,
    html: (usize, usize),
) -> Result<Option<String>, ()> {
    let start = fields.start_fragment;
    let end = fields.end_fragment;
    // 字段缺失，或文档约定的 -1 哨兵值，都需要走基于标记的回退。
    // 畸形/其他负值属于无效数据。
    let (Some(start), Some(end)) = (start.value, end.value) else {
        if start.present && start.value.is_none() || end.present && end.value.is_none() {
            return Err(());
        }
        return Ok(None);
    };
    if start == -1 || end == -1 {
        return Ok(None);
    }
    if start < 0 || end < 0 {
        return Err(());
    }
    let start = usize::try_from(start).map_err(|_| ())?;
    let end = usize::try_from(end).map_err(|_| ())?;
    if start > end || start < html.0 || end > html.1 {
        return Err(());
    }
    std::str::from_utf8(&bytes[start..end])
        .ok()
        .map(str::to_owned)
        .ok_or(())
        .map(Some)
}

/// CF_HTML 的偏移按 UTF-8 字节计，不是字符下标。
fn parse_html_bytes(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return None;
    }
    // GlobalAlloc 的缓冲通常以 NUL 结尾。忽略这个结尾符及其后面的
    // 分配器填充；片段内部出现 NUL 时不能作为合法 CF_HTML 返回，
    // 因为载荷是文本。
    let length = bytes
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(bytes.len());
    let bytes = &bytes[..length];
    if bytes.is_empty() {
        return None;
    }
    let (fields, header_end) = parse_header(bytes);
    let html = html_range(&fields, bytes.len(), header_end).ok()?;
    match offset_fragment(bytes, &fields, html) {
        Ok(Some(fragment)) => return Some(fragment),
        Ok(None) => {}
        Err(()) => return None,
    }
    marker_fragment(bytes, html)
}

/// 读取注册过的 Windows `HTML Format` 剪贴板条目。
pub fn read_html() -> Option<String> {
    unsafe {
        let format = RegisterClipboardFormatW(windows::core::w!("HTML Format"));
        if format == 0 || OpenClipboard(None).is_err() {
            return None;
        }
        let _clipboard = Clipboard;
        if IsClipboardFormatAvailable(format).is_err() {
            return None;
        }
        let handle = GetClipboardData(format).ok()?;
        let memory = HGLOBAL(handle.0);
        let length = GlobalSize(memory);
        if length == 0 || length > MAX_BYTES {
            return None;
        }
        let locked = LockedGlobal::new(memory, length)?;
        parse_html_bytes(locked.bytes())
    }
}

pub fn write(owner: HWND, text: &str, html: Option<&str>) -> bool {
    // 必须使用真实的应用自有窗口；无头测试请注入 sink。
    if owner.is_invalid() || text.len() > MAX_BYTES / 2 || text.contains('\0') {
        return false;
    }
    let bytes = text
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    let Some(mut plain) = Memory::new(&bytes) else {
        return false;
    };
    let mut rich = if let Some(html) = html {
        let Some(bytes) = html_bytes(html) else {
            return false;
        };
        let Some(memory) = Memory::new(&bytes) else {
            return false;
        };
        let format = unsafe { RegisterClipboardFormatW(windows::core::w!("HTML Format")) };
        if format == 0 {
            return false;
        }
        Some((format, memory))
    } else {
        None
    };
    unsafe {
        if OpenClipboard(Some(owner)).is_err() {
            return false;
        }
        let _guard = Clipboard;
        if EmptyClipboard().is_err() {
            return false;
        }
        if !plain.publish(13) {
            return false;
        }
        if let Some((format, memory)) = &mut rich {
            if !memory.publish(*format) {
                return false;
            }
        }
    }
    true
}
pub fn write_image(owner: HWND, bytes: &[u8]) -> bool {
    if owner.is_invalid() || bytes.len() > MAX_BYTES {
        return false;
    }
    let Ok(png) = super::clipboard_image::encode_png(bytes) else {
        return false;
    };
    let Some(mut memory) = Memory::new(&png) else {
        return false;
    };
    unsafe {
        let format = RegisterClipboardFormatW(windows::core::w!("PNG"));
        if format == 0 || OpenClipboard(Some(owner)).is_err() {
            return false;
        }
        let _guard = Clipboard;
        if EmptyClipboard().is_err() {
            return false;
        }
        memory.publish(format)
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn test_payload(
        fragment: &str,
        fragment_offsets: Option<(i64, i64)>,
        line_ending: &str,
    ) -> Vec<u8> {
        let before = "<html><body>";
        let start_marker = "<!--StartFragment-->";
        let end_marker = "<!--EndFragment-->";
        let context = format!("{before}{start_marker}{fragment}{end_marker}</body></html>");
        let fragment_header = fragment_offsets
            .map(|(start, end)| {
                format!("StartFragment:{start:010}{line_ending}EndFragment:{end:010}{line_ending}")
            })
            .unwrap_or_default();
        let header = |start: usize, end: usize| {
            format!(
                "Version:1.0{line_ending}StartHTML:{start:010}{line_ending}EndHTML:{end:010}{line_ending}{fragment_header}"
            )
        };
        let start_html = header(0, 0).len();
        let start_fragment = start_html + before.len() + start_marker.len();
        let end_fragment = start_fragment + fragment.len();
        let end_html = start_html + context.len();
        let mut bytes = format!("{}{context}", header(start_html, end_html)).into_bytes();
        if let Some((start, end)) = fragment_offsets.filter(|(start, end)| *start >= 0 && *end >= 0)
        {
            let mut actual_header = header(start_html, end_html);
            actual_header = actual_header
                .replace(
                    &format!("StartFragment:{start:010}"),
                    &format!("StartFragment:{start_fragment:010}"),
                )
                .replace(
                    &format!("EndFragment:{end:010}"),
                    &format!("EndFragment:{end_fragment:010}"),
                );
            bytes = format!("{actual_header}{context}").into_bytes();
        }
        bytes.push(0);
        bytes
    }

    #[test]
    fn html_offsets_are_utf8_bytes_and_point_to_only_the_fragment() {
        let fragment = "<table><tr><td>中文😀 &amp; é</td></tr></table>";
        let bytes = html_bytes(fragment).unwrap();
        let all = std::str::from_utf8(&bytes[..bytes.len() - 1]).unwrap();
        let offset = |name: &str| {
            all.lines()
                .find_map(|l| l.strip_prefix(name))
                .unwrap()
                .trim()
                .parse::<usize>()
                .unwrap()
        };
        assert_eq!(
            &all[offset("StartFragment:")..offset("EndFragment:")],
            fragment
        );
        assert!(all[offset("StartHTML:")..].starts_with("<html>"));
        assert_eq!(offset("EndHTML:"), all.len());
        assert_eq!(bytes.last(), Some(&0));
        assert_eq!(parse_html_bytes(&bytes), Some(fragment.to_owned()));
    }

    #[test]
    fn marker_fallback_handles_missing_and_minus_one_fragment_offsets() {
        let fragment = "<p>中文😀</p>";
        assert_eq!(
            parse_html_bytes(&test_payload(fragment, None, "\r\n")),
            Some(fragment.to_owned())
        );
        assert_eq!(
            parse_html_bytes(&test_payload(fragment, Some((-1, -1)), "\r")),
            Some(fragment.to_owned())
        );
    }

    #[test]
    fn parser_accepts_utf8_byte_offsets_and_all_header_line_endings() {
        let fragment = "<div>汉字😀</div>";
        for line_ending in ["\r\n", "\n", "\r"] {
            let probe = test_payload(fragment, Some((0, 0)), line_ending);
            let text = std::str::from_utf8(&probe[..probe.len() - 1]).unwrap();
            let field = |name: &str| {
                text.split(['\r', '\n'])
                    .find_map(|line| line.strip_prefix(name))
                    .unwrap()
                    .trim()
                    .parse::<usize>()
                    .unwrap()
            };
            let start = field("StartFragment:");
            let end = field("EndFragment:");
            let mut payload = test_payload(fragment, Some((start as i64, end as i64)), line_ending);
            assert_eq!(parse_html_bytes(&payload), Some(fragment.to_owned()));
            payload.pop();
            assert_eq!(parse_html_bytes(&payload), Some(fragment.to_owned()));
        }
    }

    #[test]
    fn malformed_or_out_of_range_offsets_are_rejected_without_panicking() {
        let malformed = b"Version:1.0\r\nStartHTML:bad\r\nEndHTML:0000000001\r\nStartFragment:-1\r\nEndFragment:-1\r\n<html><!--StartFragment--><b>x</b><!--EndFragment--></html>\0";
        let out_of_range = b"Version:1.0\r\nStartHTML:0000000000\r\nEndHTML:0000000001\r\nStartFragment:9999999999\r\nEndFragment:9999999999\r\n<html><!--StartFragment--><b>x</b><!--EndFragment--></html>\0";
        let invalid_utf8 = b"Version:1.0\r\nStartHTML:-1\r\nEndHTML:-1\r\nStartFragment:-1\r\nEndFragment:-1\r\n<html><!--StartFragment-->\xff<!--EndFragment--></html>\0";
        assert_eq!(parse_html_bytes(b"plain text without HTML"), None);
        assert_eq!(parse_html_bytes(malformed), None);
        assert_eq!(parse_html_bytes(out_of_range), None);
        assert_eq!(parse_html_bytes(invalid_utf8), None);
    }

    #[test]
    fn invalid_payload_is_rejected_before_opening_clipboard() {
        assert!(html_bytes("bad\0data").is_none());
        assert!(html_bytes(&"x".repeat(MAX_BYTES)).is_none());
        assert!(!write(
            HWND::default(),
            "headless",
            Some("<b>never published</b>")
        ));
    }
}

//! Optional filename lookup through the official Everything local IPC protocol.
//!
//! Everything must already be running. This module neither scans the filesystem nor
//! installs, launches, or persists a second index. Run `search` on a worker thread.

use anyhow::{bail, Context, Result};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    pub path: PathBuf,
    pub name: String,
    pub directory: bool,
}

/// Maximum user query passed to Everything, in UTF-16 code units.
const MAX_QUERY_UNITS: usize = 1024;
/// Avoid requesting an unbounded result set, even if the caller passes a very large limit.
pub const MAX_RESULTS: usize = 128;
/// Hard upper bound on a single IPC response copied from Everything.
const MAX_RESPONSE_BYTES: usize = 9 * 1024 * 1024;
const SEND_TIMEOUT_MS: u32 = 1_500;
const REPLY_TIMEOUT_MS: u64 = 2_500;

#[cfg(windows)]
mod win32 {
    use super::*;
    use std::ffi::OsString;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{
        GetLastError, ERROR_CLASS_ALREADY_EXISTS, HWND, LPARAM, LRESULT, WPARAM,
    };
    use windows::Win32::System::DataExchange::COPYDATASTRUCT;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, FindWindowW,
        PeekMessageW, RegisterClassW, SendMessageTimeoutW, HWND_MESSAGE, MSG, PM_REMOVE,
        SEND_MESSAGE_TIMEOUT_FLAGS, SMTO_ABORTIFHUNG, WINDOW_EX_STYLE, WINDOW_STYLE, WM_COPYDATA,
        WNDCLASSW,
    };

    const EVERYTHING_WINDOW_CLASS: PCWSTR = w!("EVERYTHING_TASKBAR_NOTIFICATION");
    const REPLY_WINDOW_CLASS: PCWSTR = w!("MochiEverythingIpcReplyWindow");
    const COPYDATA_QUERY2W: usize = 18;
    const QUERY2_HEADER_BYTES: usize = 7 * std::mem::size_of::<u32>();
    const QUERY2_REQUEST_FULL_PATH_AND_NAME: u32 = 0x0000_0004;
    const IPC_FOLDER: u32 = 0x0000_0001;

    static QUERY_GATE: Mutex<()> = Mutex::new(());
    static ACTIVE_REPLY_ID: AtomicU32 = AtomicU32::new(0);
    static REPLY: Mutex<Option<std::result::Result<Vec<u8>, String>>> = Mutex::new(None);
    static NEXT_REPLY_ID: AtomicU32 = AtomicU32::new(0x4d4f_0000);

    struct ReplyWindow(HWND);

    impl Drop for ReplyWindow {
        fn drop(&mut self) {
            ACTIVE_REPLY_ID.store(0, Ordering::Release);
            let mut reply = REPLY
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *reply = None;
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }

    pub(super) fn search(query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        if limit == 0 || query.trim().is_empty() {
            return Ok(Vec::new());
        }

        let query = query.trim();
        if query.contains('\0') {
            bail!("Everything 搜索词含有空字符");
        }
        let search_units = query.encode_utf16().collect::<Vec<_>>();
        if search_units.len() > MAX_QUERY_UNITS {
            bail!("Everything 搜索词不能超过 {MAX_QUERY_UNITS} 个 UTF-16 字符");
        }
        let result_limit = limit.min(MAX_RESULTS);

        let _guard = QUERY_GATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let everything = unsafe { FindWindowW(EVERYTHING_WINDOW_CLASS, PCWSTR::null()) }
            .context("Everything 未运行，或未提供本地 IPC 窗口")?;
        let reply = create_reply_window()?;
        let reply_id = next_reply_id();
        ACTIVE_REPLY_ID.store(reply_id, Ordering::Release);
        *REPLY
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;

        let mut request = build_query_request(reply.0, reply_id, &search_units, result_limit)?;
        send_query(everything, reply.0, &mut request)?;
        let bytes = wait_for_reply(reply.0)?;
        parse_query_reply(&bytes, result_limit)
    }

    fn next_reply_id() -> u32 {
        let id = NEXT_REPLY_ID.fetch_add(1, Ordering::Relaxed);
        if id == 0 {
            1
        } else {
            id
        }
    }

    fn create_reply_window() -> Result<ReplyWindow> {
        let instance = unsafe { GetModuleHandleW(None) }.context("无法读取应用程序模块")?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(reply_window_proc),
            hInstance: instance.into(),
            lpszClassName: REPLY_WINDOW_CLASS,
            ..Default::default()
        };
        if unsafe { RegisterClassW(&class) } == 0 {
            let error = unsafe { GetLastError() };
            if error != ERROR_CLASS_ALREADY_EXISTS {
                bail!("无法注册 Everything IPC 回复窗口 ({})", error.0);
            }
        }
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                REPLY_WINDOW_CLASS,
                w!("Mochi Everything IPC reply"),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(instance.into()),
                None,
            )
        }
        .context("无法创建 Everything IPC 回复窗口")?;
        Ok(ReplyWindow(hwnd))
    }

    pub(super) fn build_query_request(
        reply_window: HWND,
        reply_id: u32,
        search_units: &[u16],
        result_limit: usize,
    ) -> Result<Vec<u8>> {
        // Everything defines reply_hwnd as a 32-bit field, including on x64.
        let hwnd = reply_window.0 as usize as u32;
        if hwnd == 0 {
            bail!("Everything IPC 回复窗口句柄无效");
        }
        let result_limit = u32::try_from(result_limit).context("Everything 结果上限无效")?;
        let capacity = QUERY2_HEADER_BYTES
            .checked_add(
                search_units
                    .len()
                    .checked_add(1)
                    .and_then(|units| units.checked_mul(2))
                    .context("Everything 搜索词过长")?,
            )
            .context("Everything 查询请求过长")?;
        let mut request = Vec::with_capacity(capacity);
        for field in [
            hwnd,
            reply_id,
            0, // case-insensitive; preserve Everything's search syntax.
            0, // offset
            result_limit,
            QUERY2_REQUEST_FULL_PATH_AND_NAME,
            1, // EVERYTHING_IPC_SORT_NAME_ASCENDING.
        ] {
            request.extend_from_slice(&field.to_le_bytes());
        }
        for unit in search_units.iter().copied().chain(std::iter::once(0)) {
            request.extend_from_slice(&unit.to_le_bytes());
        }
        Ok(request)
    }

    fn send_query(everything: HWND, reply_window: HWND, request: &mut [u8]) -> Result<()> {
        let cb_data = u32::try_from(request.len()).context("Everything 查询请求超过 IPC 上限")?;
        let data = COPYDATASTRUCT {
            dwData: COPYDATA_QUERY2W,
            cbData: cb_data,
            lpData: request.as_mut_ptr().cast(),
        };
        let mut accepted = 0usize;
        let sent = unsafe {
            SendMessageTimeoutW(
                everything,
                WM_COPYDATA,
                WPARAM(reply_window.0 as usize),
                LPARAM((&data as *const COPYDATASTRUCT) as isize),
                SEND_MESSAGE_TIMEOUT_FLAGS(SMTO_ABORTIFHUNG.0),
                SEND_TIMEOUT_MS,
                Some(&mut accepted),
            )
        };
        if sent.0 == 0 {
            bail!("Everything 未在 {SEND_TIMEOUT_MS} 毫秒内接受搜索请求");
        }
        if accepted == 0 {
            bail!("Everything IPC 未接受搜索请求");
        }
        Ok(())
    }

    fn wait_for_reply(reply_window: HWND) -> Result<Vec<u8>> {
        let deadline = Instant::now() + Duration::from_millis(REPLY_TIMEOUT_MS);
        loop {
            if let Some(reply) = REPLY
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
            {
                return reply.map_err(anyhow::Error::msg);
            }

            let mut message = MSG::default();
            while unsafe { PeekMessageW(&mut message, Some(reply_window), 0, 0, PM_REMOVE) }
                .as_bool()
            {
                unsafe {
                    let _ = DispatchMessageW(&message);
                }
            }

            if let Some(reply) = REPLY
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
            {
                return reply.map_err(anyhow::Error::msg);
            }
            if Instant::now() >= deadline {
                bail!("Everything IPC 搜索在 {REPLY_TIMEOUT_MS} 毫秒内没有返回结果");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    unsafe extern "system" fn reply_window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == WM_COPYDATA && lparam.0 != 0 {
            let copy_data = unsafe { &*(lparam.0 as *const COPYDATASTRUCT) };
            let expected = ACTIVE_REPLY_ID.load(Ordering::Acquire) as usize;
            if expected != 0 && copy_data.dwData == expected {
                let result = if copy_data.cbData as usize > MAX_RESPONSE_BYTES {
                    Err(format!("Everything IPC 回复超过 {MAX_RESPONSE_BYTES} 字节"))
                } else if copy_data.cbData < 20 || copy_data.lpData.is_null() {
                    Err("Everything IPC 回复格式不完整".into())
                } else {
                    let data = unsafe {
                        std::slice::from_raw_parts(
                            copy_data.lpData.cast::<u8>(),
                            copy_data.cbData as usize,
                        )
                    };
                    Ok(data.to_vec())
                };
                *REPLY
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(result);
                return LRESULT(1);
            }
        }
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }

    pub(super) fn parse_query_reply(
        bytes: &[u8],
        result_limit: usize,
    ) -> Result<Vec<SearchResult>> {
        const LIST_HEADER_BYTES: usize = 5 * std::mem::size_of::<u32>();
        const ITEM_BYTES: usize = 2 * std::mem::size_of::<u32>();
        if bytes.len() < LIST_HEADER_BYTES {
            bail!("Everything IPC 回复头部过短");
        }
        let count = read_u32(bytes, 4)? as usize;
        let request_flags = read_u32(bytes, 12)?;
        if request_flags & QUERY2_REQUEST_FULL_PATH_AND_NAME == 0 {
            bail!("Everything IPC 未返回完整文件路径字段");
        }
        if count > result_limit || count > MAX_RESULTS {
            bail!("Everything IPC 返回条数超过请求上限");
        }
        let items_bytes = count
            .checked_mul(ITEM_BYTES)
            .and_then(|value| value.checked_add(LIST_HEADER_BYTES))
            .context("Everything IPC 条目表长度溢出")?;
        if items_bytes > bytes.len() {
            bail!("Everything IPC 条目表超出回复边界");
        }

        let mut results = Vec::with_capacity(count);
        for index in 0..count {
            let item_offset = LIST_HEADER_BYTES + index * ITEM_BYTES;
            let flags = read_u32(bytes, item_offset)?;
            let data_offset = read_u32(bytes, item_offset + 4)? as usize;
            if data_offset < items_bytes {
                bail!("Everything IPC 条目数据与索引表重叠");
            }
            let unit_count = read_u32(bytes, data_offset)? as usize;
            if unit_count == 0 || unit_count > 32_767 {
                bail!("Everything IPC 文件路径长度无效");
            }
            let text_start = data_offset
                .checked_add(std::mem::size_of::<u32>())
                .context("Everything IPC 文件路径偏移溢出")?;
            let text_bytes = unit_count
                .checked_add(1)
                .and_then(|units| units.checked_mul(std::mem::size_of::<u16>()))
                .context("Everything IPC 文件路径长度溢出")?;
            let text_end = text_start
                .checked_add(text_bytes)
                .context("Everything IPC 文件路径结束位置溢出")?;
            let path_end = text_start
                .checked_add(unit_count * std::mem::size_of::<u16>())
                .context("Everything IPC 文件路径结束位置溢出")?;
            if text_end > bytes.len() || read_u16(bytes, path_end)? != 0 {
                bail!("Everything IPC 文件路径超出回复边界或缺少终止符");
            }
            let units = bytes[text_start..path_end]
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect::<Vec<_>>();
            if units.contains(&0) {
                bail!("Everything IPC 文件路径包含内部终止符");
            }
            #[cfg(windows)]
            let path = {
                use std::os::windows::ffi::OsStringExt;
                PathBuf::from(OsString::from_wide(&units))
            };
            #[cfg(not(windows))]
            let path = PathBuf::from(String::from_utf16_lossy(&units));
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy())
                .filter(|name| !name.is_empty())
                .map(|name| name.into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned());
            results.push(SearchResult {
                path,
                name,
                directory: flags & IPC_FOLDER != 0,
            });
        }
        Ok(results)
    }

    fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
        let data = bytes
            .get(offset..offset.checked_add(4).context("IPC 字段偏移溢出")?)
            .context("Everything IPC 字段超出回复边界")?;
        Ok(u32::from_le_bytes(
            data.try_into().expect("four-byte range"),
        ))
    }

    fn read_u16(bytes: &[u8], offset: usize) -> Result<u16> {
        let data = bytes
            .get(offset..offset.checked_add(2).context("IPC 字段偏移溢出")?)
            .context("Everything IPC 字段超出回复边界")?;
        Ok(u16::from_le_bytes(data.try_into().expect("two-byte range")))
    }
}

/// Query the local Everything index without scanning or creating a parallel index.
/// The result count is clamped to [`MAX_RESULTS`]. This call is synchronous; invoke it
/// from Mochi's existing background search worker rather than the window thread.
pub fn search(query: &str, limit: usize) -> Result<Vec<SearchResult>> {
    #[cfg(windows)]
    {
        win32::search(query, limit)
    }
    #[cfg(not(windows))]
    {
        let _ = (query, limit);
        bail!("Everything 本地 IPC 仅支持 Windows")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    fn encode_full_path_item(path: &str, flags: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        for value in [1u32, 1, 0, 4, 0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let data_offset = (20 + 8) as u32;
        bytes.extend_from_slice(&flags.to_le_bytes());
        bytes.extend_from_slice(&data_offset.to_le_bytes());
        let utf16 = path.encode_utf16().collect::<Vec<_>>();
        bytes.extend_from_slice(&(utf16.len() as u32).to_le_bytes());
        for unit in utf16.into_iter().chain(std::iter::once(0)) {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
    }

    #[cfg(windows)]
    #[test]
    fn ipc_reply_parser_checks_byte_boundaries_and_extracts_file_and_folder_results() {
        let file = encode_full_path_item(r"C:\Notes\中文.md", 0);
        let parsed = win32::parse_query_reply(&file, 1).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "中文.md");
        assert_eq!(parsed[0].path, PathBuf::from(r"C:\Notes\中文.md"));
        assert!(!parsed[0].directory);

        let folder = encode_full_path_item(r"C:\Notes\Archive", 1);
        assert!(win32::parse_query_reply(&folder, 1).unwrap()[0].directory);
        assert!(win32::parse_query_reply(&file[..file.len() - 2], 1).is_err());
        let mut oversized_count = file;
        oversized_count[4..8].copy_from_slice(&2u32.to_le_bytes());
        assert!(win32::parse_query_reply(&oversized_count, 1).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn ipc_request_encoding_is_bounded_and_null_terminated_utf16() {
        use windows::Win32::Foundation::HWND;
        let units = "墨池 search".encode_utf16().collect::<Vec<_>>();
        let request = win32::build_query_request(HWND(1usize as *mut _), 7, &units, 12).unwrap();
        assert_eq!(request.len(), 28 + (units.len() + 1) * 2);
        assert_eq!(&request[16..20], &12u32.to_le_bytes());
        assert_eq!(&request[20..24], &4u32.to_le_bytes());
        assert_eq!(&request[24..28], &1u32.to_le_bytes());
        assert_eq!(&request[request.len() - 2..], &[0, 0]);
    }
}

//! WinRT .get() 不能阻塞 UI 的 STA 线程。PDF 由工作线程持有，按需返回页图。
//! WIC 位图和渲染目标资源仍在 UI 线程创建。

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use windows::core::{Result, HSTRING};
use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
use windows::Storage::StorageFile;
use windows::Storage::Streams::{DataReader, InMemoryRandomAccessStream};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::platform;

/// 工作线程发回来的事。
#[derive(Debug)]
pub enum PdfEvent {
    /// 文档打开了：页数与每页尺寸（PDF 点，1/72 英寸；`Windows.Data.Pdf` 报的是 96 DPI 下的 DIP）。
    Loaded {
        path: PathBuf,
        sizes: Vec<(f32, f32)>,
    },
    /// 某一页渲染好了：PNG 字节与位图像素尺寸。
    Page {
        path: PathBuf,
        index: u32,
        png: Vec<u8>,
        width: u32,
        height: u32,
    },
    Failed {
        path: PathBuf,
        message: String,
    },
}

/// 一份打开着的 PDF 的句柄：往工作线程点名要页。丢掉句柄即关闭请求通道，工作线程随之退出。
pub struct PdfHandle {
    requests: Sender<(u32, f32)>,
    cancelled: Arc<AtomicBool>,
}
impl Drop for PdfHandle {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

impl PdfHandle {
    /// 请求渲染第 `index` 页，`scale` 是相对页面 DIP 尺寸的放大倍数（高 DPI 屏要 ≥ 1.5 才不糊）。
    pub fn request(&self, index: u32, scale: f32) {
        let _ = self.requests.send((index, scale));
    }
}

/// 打开一份 PDF。返回句柄与事件接收端；事件到达时 UI 线程会收到 `WM_APP_PDF_EVENT`。
pub fn open(
    path: PathBuf,
    document_path: PathBuf,
    hwnd_raw: isize,
) -> (PdfHandle, Receiver<PdfEvent>) {
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancel_worker = cancelled.clone();
    let (event_tx, event_rx) = channel::<PdfEvent>();
    let (req_tx, req_rx) = channel::<(u32, f32)>();
    std::thread::Builder::new()
        .name("mochi-pdf".into())
        .spawn(move || {
            worker(
                path,
                document_path,
                hwnd_raw,
                event_tx,
                req_rx,
                cancel_worker,
            )
        })
        .expect("spawn pdf worker");
    (
        PdfHandle {
            requests: req_tx,
            cancelled,
        },
        event_rx,
    )
}

fn notify(hwnd_raw: isize) {
    unsafe {
        let _ = PostMessageW(
            Some(HWND(hwnd_raw as *mut _)),
            platform::WM_APP_PDF_EVENT,
            WPARAM(0),
            LPARAM(0),
        );
    }
}

fn worker(
    path: PathBuf,
    document_path: PathBuf,
    hwnd_raw: isize,
    events: Sender<PdfEvent>,
    requests: Receiver<(u32, f32)>,
    cancelled: Arc<AtomicBool>,
) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    crate::gfx::gfx_log(&format!("PDF 工作线程启动：{}", path.display()));
    let doc = match load_document(&document_path) {
        Ok(d) => d,
        Err(e) => {
            crate::gfx::gfx_log(&format!("PDF 打开失败：{e:?}"));
            let _ = events.send(PdfEvent::Failed {
                path,
                message: format!("{e}"),
            });
            notify(hwnd_raw);
            return;
        }
    };
    let count = doc.PageCount().unwrap_or(0);
    crate::gfx::gfx_log(&format!("PDF 打开成功：{count} 页"));
    let mut sizes = Vec::with_capacity(count as usize);
    for i in 0..count {
        if cancelled.load(Ordering::Relaxed) {
            return;
        }
        let size = doc
            .GetPage(i)
            .and_then(|p| p.Size())
            .map(|s| (s.Width, s.Height))
            .unwrap_or((612.0, 792.0));
        sizes.push(size);
    }
    let _ = events.send(PdfEvent::Loaded {
        path: path.clone(),
        sizes,
    });
    notify(hwnd_raw);

    // 逐页渲染。同一页的重复请求（滚动时会攒一堆）只渲染最后一次的倍率
    while let Ok((index, scale)) = requests.recv() {
        // 把队列里积压的请求先收完，去重
        let mut wanted: Vec<(u32, f32)> = vec![(index, scale)];
        while let Ok(more) = requests.try_recv() {
            wanted.push(more);
        }
        let mut latest = std::collections::BTreeMap::new();
        for (index, scale) in wanted {
            latest.insert(index, scale);
        }
        for (index, scale) in latest {
            if cancelled.load(Ordering::Relaxed) {
                return;
            }
            match render_page(&doc, index, scale) {
                Ok((png, w, h)) => {
                    crate::gfx::gfx_log(&format!(
                        "PDF 第 {} 页渲染完成：{w}x{h}，{} 字节",
                        index + 1,
                        png.len()
                    ));
                    if events
                        .send(PdfEvent::Page {
                            path: path.clone(),
                            index,
                            png,
                            width: w,
                            height: h,
                        })
                        .is_err()
                    {
                        return;
                    }
                }
                Err(e) => {
                    crate::gfx::gfx_log(&format!("PDF 第 {} 页渲染失败：{e:?}", index + 1));
                    let _ = events.send(PdfEvent::Failed {
                        path: path.clone(),
                        message: format!("第 {} 页渲染失败：{e}", index + 1),
                    });
                }
            }
            notify(hwnd_raw);
        }
    }
}

fn load_document(path: &std::path::Path) -> Result<PdfDocument> {
    // WinRT 的 StorageFile 只认反斜杠的规范路径；工作区里拼出来的路径常常混着 `/`（ERROR_BAD_PATHNAME）
    let normalized = path.to_string_lossy().replace('/', "\\");
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(normalized.as_str()))?.join()?;
    PdfDocument::LoadFromFileAsync(&file)?.join()
}

/// 渲染成 PNG 字节。`DestinationWidth` 决定位图分辩率。
fn render_page(doc: &PdfDocument, index: u32, scale: f32) -> Result<(Vec<u8>, u32, u32)> {
    let page = doc.GetPage(index)?;
    let size = page.Size()?;
    let width = (size.Width * scale).round().max(1.0) as u32;
    let height = (size.Height * scale).round().max(1.0) as u32;
    let options = PdfPageRenderOptions::new()?;
    options.SetDestinationWidth(width)?;
    options.SetDestinationHeight(height)?;
    let stream = InMemoryRandomAccessStream::new()?;
    page.RenderWithOptionsToStreamAsync(&stream, &options)?
        .join()?;
    let len = stream.Size()? as u32;
    let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
    reader.LoadAsync(len)?.join()?;
    let mut bytes = vec![0u8; len as usize];
    reader.ReadBytes(&mut bytes)?;
    Ok((bytes, width, height))
}

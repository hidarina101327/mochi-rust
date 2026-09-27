//! D2D 资源只在渲染线程创建；UI 通过 DrawList 提交绘制指令。

use std::collections::HashMap;
mod pdf_export;
mod text_metrics;

use windows::core::{w, Interface, Result, HSTRING};
// D2D_POINT_2F 在 windows-rs 里就是 Vector2
use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_BEZIER_SEGMENT, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED,
    D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_CLOSED, D2D1_FIGURE_END_OPEN, D2D1_GRADIENT_STOP,
    D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_SIZE_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Bitmap, ID2D1Factory, ID2D1HwndRenderTarget, ID2D1LinearGradientBrush,
    ID2D1PathGeometry, ID2D1RenderTarget, ID2D1SolidColorBrush, ID2D1StrokeStyle,
    D2D1_ANTIALIAS_MODE_ALIASED, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_ARC_SEGMENT,
    D2D1_ARC_SIZE_LARGE, D2D1_ARC_SIZE_SMALL, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
    D2D1_CAP_STYLE_ROUND, D2D1_ELLIPSE, D2D1_EXTEND_MODE_CLAMP, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_GAMMA_2_2, D2D1_HWND_RENDER_TARGET_PROPERTIES, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES,
    D2D1_LINE_JOIN_ROUND, D2D1_PRESENT_OPTIONS_NONE, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_ROUNDED_RECT,
    D2D1_STROKE_STYLE_PROPERTIES, D2D1_SWEEP_DIRECTION_CLOCKWISE,
    D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STYLE_OBLIQUE,
    DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING,
    DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory,
    WICBitmapDitherTypeNone, WICBitmapPaletteTypeMedianCut, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::Shell::SHCreateMemStream;
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
use windows_numerics::{Matrix3x2, Vector2};

use crate::ui::draw::{Align, DrawCmd, DrawList, TextStyle};
use crate::ui::icons::{self, Icon, PathCmd};
use crate::ui::layout::Rect;
use crate::ui::text::Emphasis;
use crate::ui::theme;

/// 图形层的诊断日志。设了 `MOCHI_GFX_LOG=<路径>` 才写——「窗口一片白」从画面上倒推不出原因，
/// 这是唯一能把设备丢失、位图解码失败这类事带出来的通道。
pub fn gfx_log(line: &str) {
    if let Some(path) = std::env::var_os("MOCHI_GFX_LOG") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(f, "{line}");
        }
    }
}

/// 低内存模式：D2D 走 WARP 软件光栅，绕开 D3D 设备和 GPU 用户态驱动。
/// 实测省 44MB 私有内存和 31 个线程，而 DirectWrite 排版质量完全不打折。
/// 文本应用没有动画压力，这一档应当是默认——见 docs/native-rewrite.md。
pub fn low_memory_mode() -> bool {
    std::env::var_os("MOCHI_GPU").is_none()
}

/// 画刷缓存。绘制指令带的是 `u32` 颜色，一帧里同一个颜色会用上几十次，
/// 每次 `CreateSolidColorBrush` 是一次 COM 分配——缓存住，随渲染目标一起丢弃。
struct Brushes {
    target: ID2D1RenderTarget,
    cache: HashMap<u32, ID2D1SolidColorBrush>,
}

impl Brushes {
    fn get(&mut self, color: u32) -> Option<ID2D1SolidColorBrush> {
        if let Some(b) = self.cache.get(&color) {
            return Some(b.clone());
        }
        let brush = unsafe { self.target.CreateSolidColorBrush(&rgb(color), None) }.ok()?;
        self.cache.insert(color, brush.clone());
        Some(brush)
    }
}

/// 品牌标志的资源。渐变画刷是**渲染目标**资源，随目标一起丢弃重建。
///
/// 形状照 `Logo.tsx` 转写：512 视图盒里的圆角底板、章鱼身体、两只眼睛、
/// 触手、笔记本和铅笔。绘制时用变换矩阵缩到目标盒。
struct LogoResources {
    bg: ID2D1LinearGradientBrush,
    squid: ID2D1LinearGradientBrush,
    body: ID2D1PathGeometry,
    tentacles: ID2D1PathGeometry,
    pencil_tip: ID2D1PathGeometry,
    white: ID2D1SolidColorBrush,
    tentacle_stroke: ID2D1SolidColorBrush,
    paper: ID2D1SolidColorBrush,
    lines: ID2D1SolidColorBrush,
    pencil: ID2D1SolidColorBrush,
    pencil_tip_brush: ID2D1SolidColorBrush,
    highlight: ID2D1SolidColorBrush,
}

/// 渲染器：持有工厂、渲染目标、画刷缓存和预建的文本格式。
pub struct Renderer {
    d2d: ID2D1Factory,
    /// 文本绘制、字簇测量与 PDF 文字层共用的 DirectWrite 工厂。
    dwrite: IDWriteFactory,
    target: Option<ID2D1RenderTarget>,
    brushes: Option<Brushes>,
    logo: Option<LogoResources>,
    /// 每个 `TextStyle` × 对齐 × 强调各一份格式对象。
    /// DirectWrite 的对齐是格式上的属性而不是绘制参数，只能预先分档建好。
    formats: HashMap<(TextStyle, Align, Emphasis), IDWriteTextFormat>,
    measurements: std::rc::Rc<text_metrics::Metrics>,
    /// 图标几何缓存，24 单位视图盒空间。几何是工厂资源（与渲染目标无关），
    /// 建一次用一辈子；绘制时靠变换矢量缩放到目标盒。
    icon_geometry: HashMap<Icon, Vec<ID2D1PathGeometry>>,
    /// lucide 的圆头圆角描边。
    icon_stroke: ID2D1StrokeStyle,
    /// 文档里的图片。位图是**渲染目标**资源，随目标一起丢弃。
    /// `None` 表示解码失败过——别每帧重试一个坏文件。
    bitmaps: HashMap<String, Option<ID2D1Bitmap>>,
    /// 内存里的图片字节（PDF 页面渲染结果等），键是 `mem://…`。目标丢失后从这里重建位图。
    byte_sources: HashMap<String, Vec<u8>>,
    wic: Option<IWICImagingFactory>,
    /// 已呈现的帧数（诊断日志用）。
    frames: u64,
    /// 窗口目标带预乘 alpha、清屏为全透明，让 DWM 模糊从未绘制的像素透出来。
    transparent: bool,
}

impl Renderer {
    /// 切换透明窗口目标。模式变了就丢掉目标，下一帧按新像素格式重建。
    pub fn set_transparent(&mut self, transparent: bool) {
        if self.transparent != transparent {
            self.transparent = transparent;
            self.target = None;
            self.brushes = None;
            self.logo = None;
            self.bitmaps.clear();
        }
    }
    pub fn refresh_text_formats(&mut self) -> Result<()> {
        self.formats = build_formats(&self.dwrite)?;
        self.measurements.refresh(self.formats.clone());
        Ok(())
    }
    #[cfg(debug_assertions)]
    pub fn math_bitmap_stats(&self) -> (usize, u64) {
        let mut count = 0;
        let mut pixels = 0;
        for (key, value) in &self.bitmaps {
            if key.starts_with("math://") {
                if let Some(bitmap) = value {
                    let size = unsafe { bitmap.GetPixelSize() };
                    count += 1;
                    pixels += u64::from(size.width) * u64::from(size.height);
                }
            }
        }
        (count, pixels)
    }
    pub fn prepare_snapshot(&mut self, width: u32, height: u32, dpi: f32) -> Result<Snapshot> {
        use windows::Win32::Graphics::Imaging::WICBitmapCacheOnLoad;
        let factory: IWICImagingFactory =
            unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)? };
        let bitmap = unsafe {
            factory.CreateBitmap(
                width,
                height,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapCacheOnLoad,
            )?
        };
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
            dpiX: dpi,
            dpiY: dpi,
            ..Default::default()
        };
        let target = unsafe { self.d2d.CreateWicBitmapRenderTarget(&bitmap, &props)? };
        self.brushes = Some(Brushes {
            target: target.clone(),
            cache: HashMap::new(),
        });
        self.target = Some(target);
        self.wic = Some(factory);
        self.logo = None;
        self.bitmaps.clear();
        Ok(Snapshot(bitmap))
    }
    #[cfg(debug_assertions)]
    pub fn save_snapshot(&self, snapshot: &Snapshot, path: &std::path::Path) -> Result<()> {
        use windows::Win32::{
            Foundation::GENERIC_WRITE,
            Graphics::Imaging::{
                GUID_ContainerFormatPng, IWICBitmapFrameEncode, WICBitmapEncoderNoCache,
            },
        };
        let factory = self
            .wic
            .as_ref()
            .ok_or_else(windows::core::Error::from_thread)?;
        unsafe {
            let stream = factory.CreateStream()?;
            stream.InitializeFromFilename(
                &HSTRING::from(path.to_string_lossy().as_ref()),
                GENERIC_WRITE.0,
            )?;
            let encoder = factory.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
            encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
            let mut frame: Option<IWICBitmapFrameEncode> = None;
            encoder.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
            let frame = frame.ok_or_else(windows::core::Error::from_thread)?;
            frame.Initialize(None)?;
            let mut width = 0;
            let mut height = 0;
            snapshot.0.GetSize(&mut width, &mut height)?;
            frame.SetSize(width, height)?;
            let mut format = GUID_WICPixelFormat32bppPBGRA;
            frame.SetPixelFormat(&mut format)?;
            frame.WriteSource(&snapshot.0, std::ptr::null())?;
            frame.Commit()?;
            encoder.Commit()?;
        }
        Ok(())
    }
    pub fn new() -> Result<Self> {
        let d2d: ID2D1Factory =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        let dwrite: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let formats = build_formats(&dwrite)?;
        let icon_stroke = unsafe {
            d2d.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    dashCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    miterLimit: 10.0,
                    ..Default::default()
                },
                None,
            )?
        };
        let measurements = text_metrics::Metrics::new(dwrite.clone(), formats.clone());
        Ok(Self {
            d2d,
            dwrite,
            target: None,
            brushes: None,
            logo: None,
            formats,
            measurements,
            icon_geometry: HashMap::new(),
            icon_stroke,
            bitmaps: HashMap::new(),
            byte_sources: HashMap::new(),
            wic: None,
            frames: 0,
            transparent: false,
        })
    }

    /// 登记一段内存里的图片字节（PNG/JPEG…），之后用 `mem://<key>` 当 `src` 画它。
    /// 重复登记同一个键会替换旧位图（PDF 页按新倍率重渲染时用）。
    pub fn register_bytes(&mut self, key: &str, bytes: Vec<u8>) {
        self.byte_sources.insert(key.to_owned(), bytes);
        self.bitmaps.remove(key);
    }
    pub fn has_bytes(&self, key: &str) -> bool {
        self.byte_sources.contains_key(key)
    }
    pub fn image_bytes(&self, key: &str) -> Option<&[u8]> {
        self.byte_sources.get(key).map(Vec::as_slice)
    }

    /// 丢掉一个内存图片（关掉 PDF 标签时释放页面位图）。
    pub fn forget_bytes_with_prefix(&mut self, prefix: &str) {
        self.byte_sources.retain(|k, _| !k.starts_with(prefix));
        self.bitmaps.retain(|k, _| !k.starts_with(prefix));
    }
    pub fn forget_bytes(&mut self, key: &str) {
        self.byte_sources.remove(key);
        self.bitmaps.remove(key);
    }

    /// 解码一张图片成当前渲染目标的位图。WIC 工厂懒建（没有图片的文档不该付这笔开销）。
    fn ensure_bitmap(&mut self, src: &str) {
        if self.bitmaps.contains_key(src) {
            return;
        }
        let Some(target) = self.target.clone() else {
            return;
        };
        if self.wic.is_none() {
            self.wic =
                unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }
                    .ok();
        }
        let decoded = self
            .wic
            .as_ref()
            .map(|wic| match self.byte_sources.get(src) {
                Some(bytes) => decode_bitmap_from_bytes(wic, &target, bytes),
                None => decode_bitmap(wic, &target, src),
            });
        match &decoded {
            Some(Ok(b)) => {
                let s = unsafe { b.GetSize() };
                gfx_log(&format!("位图解码成功 {src}: {}x{}", s.width, s.height));
            }
            Some(Err(e)) => gfx_log(&format!("位图解码失败 {src}: {e:?}")),
            None => gfx_log("WIC 工厂不可用"),
        }
        self.bitmaps
            .insert(src.to_owned(), decoded.and_then(Result::ok));
    }
    fn ensure_thumbnail(&mut self, src: &str, edge: u32) {
        let key = thumbnail_key(src, edge);
        if self.bitmaps.contains_key(&key) {
            return;
        }
        let Some(target) = self.target.clone() else {
            return;
        };
        if self.wic.is_none() {
            self.wic =
                unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }
                    .ok();
        }
        let value = self
            .wic
            .as_ref()
            .and_then(|wic| decode_bitmap_scaled(wic, &target, src, Some(edge)).ok());
        self.bitmaps.insert(key, value);
    }

    /// 确保一个图标的几何已建好。查不到的图标存空列表——什么都不画，
    /// 比崩掉整个窗口好；`icons::tests` 已经保证登记过的常量都查得到。
    fn ensure_icon(&mut self, icon: Icon) {
        if !self.icon_geometry.contains_key(&icon) {
            let shapes = icons::lookup(icon).unwrap_or(&[]);
            let built: Vec<ID2D1PathGeometry> = shapes
                .iter()
                .filter_map(|s| build_geometry(&self.d2d, &s.cmds).ok())
                .collect();
            self.icon_geometry.insert(icon, built);
        }
    }

    /// 按需建渲染目标。设备丢失后 [`Self::present`] 会把它清成 `None`，下一帧在这里重建。
    pub fn ensure_target(&mut self, hwnd: HWND) -> Result<()> {
        // D2D 把 dpiX/dpiY 为 0 当作「用默认值」。那个默认值是系统 DPI，
        // 可能与 GetDpiForWindow 报告的显示器 DPI 不一致
        //（比如系统 144、窗口 120）。渲染目标的 DIP 空间
        // 必须跟随 HWND 当前的 DPI。
        let window_dpi = (!hwnd.0.is_null()).then(|| crate::platform::dpi_for_window(hwnd) as f32);

        if let Some(target) = self.target.clone() {
            if let Some(dpi) = window_dpi {
                let mut target_dpi_x = 96.0_f32;
                let mut target_dpi_y = 96.0_f32;
                unsafe {
                    target.GetDpi(&mut target_dpi_x, &mut target_dpi_y);
                }
                if (target_dpi_x - dpi).abs() > 0.5 || (target_dpi_y - dpi).abs() > 0.5 {
                    unsafe {
                        target.SetDpi(dpi, dpi);
                    }
                    gfx_log(&format!(
                        "同步渲染目标 DPI {target_dpi_x:.1}x{target_dpi_y:.1} -> {dpi:.1}x{dpi:.1}"
                    ));
                }
            }
            return Ok(());
        }
        let mut rc = RECT::default();
        unsafe { GetClientRect(hwnd, &mut rc)? };
        let dpi = window_dpi.unwrap_or(96.0);
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: if low_memory_mode() {
                D2D1_RENDER_TARGET_TYPE_SOFTWARE
            } else {
                D2D1_RENDER_TARGET_TYPE_DEFAULT
            },
            dpiX: dpi,
            dpiY: dpi,
            pixelFormat: if self.transparent {
                D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                }
            } else {
                D2D1_PIXEL_FORMAT::default()
            },
            ..Default::default()
        };
        let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
            hwnd,
            pixelSize: D2D_SIZE_U {
                width: (rc.right - rc.left).max(1) as u32,
                height: (rc.bottom - rc.top).max(1) as u32,
            },
            presentOptions: D2D1_PRESENT_OPTIONS_NONE,
        };
        let target: ID2D1RenderTarget =
            unsafe { self.d2d.CreateHwndRenderTarget(&props, &hwnd_props)? }.cast()?;
        if self.transparent {
            // ClearType 需要不透明底色；在透明像素上会带出彩边。
            unsafe { target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE) };
        }
        gfx_log(&format!(
            "建渲染目标 {}x{} 物理像素，DPI {:.1}x{:.1}",
            hwnd_props.pixelSize.width, hwnd_props.pixelSize.height, dpi, dpi
        ));
        self.brushes = Some(Brushes {
            target: target.clone(),
            cache: HashMap::new(),
        });
        self.target = Some(target);
        Ok(())
    }

    /// 立即把现有窗口目标切到 WM_DPICHANGED 携带的 DPI。
    ///
    /// `ensure_target` 在每帧之前也会做同样的对账；提供一个显式入口，
    /// 窗口过程才能在让 App 重建 DIP 布局之前先更新渲染目标。
    pub fn set_window_dpi(&mut self, dpi_x: u32, dpi_y: u32) {
        let dpi_x = dpi_x.max(96) as f32;
        let dpi_y = dpi_y.max(96) as f32;
        if let Some(target) = self.target.clone() {
            unsafe {
                target.SetDpi(dpi_x, dpi_y);
            }
            gfx_log(&format!("窗口 DPI 更新为 {dpi_x:.1}x{dpi_y:.1}"));
        }
    }

    pub fn resize(&self, width: u32, height: u32) {
        if let Some(target) = &self.target {
            let size = D2D_SIZE_U {
                width: width.max(1),
                height: height.max(1),
            };
            if let Ok(window) = target.cast::<ID2D1HwndRenderTarget>() {
                let _ = unsafe { window.Resize(&size) };
            }
        }
    }

    /// 当前渲染目标的尺寸（DIP）。没有目标时是零矩形——布局照样能解算，只是解出一堆空矩形。
    pub fn viewport(&self) -> Rect {
        match &self.target {
            Some(t) => {
                let s = unsafe { t.GetSize() };
                Rect::new(0.0, 0.0, s.width, s.height)
            }
            None => Rect::ZERO,
        }
    }

    /// 把指令列表回放成一帧。清屏色单独给——它不在指令列表里，
    /// 因为 `Clear` 比铺一个全屏矩形便宜。
    pub fn present(&mut self, hwnd: HWND, background: u32, list: &DrawList) -> Result<()> {
        self.ensure_target(hwnd)?;
        let Some(target) = self.target.clone() else {
            return Ok(());
        };

        // 目标的像素尺寸必须与客户区一致。窗口隐藏着被改尺寸时 WM_SIZE 可能没赶上，
        // 结果是画面只占左上角一块、其余全黑——每帧对一次，比追那条消息可靠
        unsafe {
            let mut rc = RECT::default();
            if GetClientRect(hwnd, &mut rc).is_ok() {
                let want = D2D_SIZE_U {
                    width: (rc.right - rc.left).max(1) as u32,
                    height: (rc.bottom - rc.top).max(1) as u32,
                };
                let have = target.GetPixelSize();
                if have.width != want.width || have.height != want.height {
                    gfx_log(&format!(
                        "目标尺寸 {}x{} 与客户区 {}x{} 不一致，重设",
                        have.width, have.height, want.width, want.height
                    ));
                    if let Ok(window) = target.cast::<ID2D1HwndRenderTarget>() {
                        let _ = window.Resize(&want);
                    }
                }
            }
        }

        self.frames += 1;
        if self.frames <= 3 || list.cmds().len() < 5 {
            gfx_log(&format!(
                "第 {} 帧：{} 条指令，目标 {:?}",
                self.frames,
                list.cmds().len(),
                unsafe { target.GetPixelSize() }
            ));
        }
        unsafe {
            target.BeginDraw();
            if self.transparent {
                target.Clear(Some(&D2D1_COLOR_F::default()));
            } else {
                target.Clear(Some(&rgb(background)));
            }
            self.replay(&target, list);
            // D2DERR_RECREATE_TARGET：设备丢失，丢掉目标下次重建。
            // 其它错误（裁剪栈不平、非法几何）也会从这里报出来——debug 档打印出来，
            // 否则症状只是「窗口一片白」，从画面上倒推不出原因。
            if let Err(e) = target.EndDraw(None, None) {
                #[cfg(debug_assertions)]
                eprintln!("EndDraw 失败: {e:?}");
                gfx_log(&format!("EndDraw 失败: {e:?}"));
                self.target = None;
                self.brushes = None;
                self.logo = None;
                self.bitmaps.clear();
                return Err(e);
            }
        }
        Ok(())
    }

    #[allow(clippy::map_entry)] // 键尚未占用时，填充缓存需要调用本结构的其他 &mut self 方法。
    fn replay(&mut self, target: &ID2D1RenderTarget, list: &DrawList) {
        let (mut dpi_x, mut dpi_y) = (96.0_f32, 96.0_f32);
        unsafe {
            target.GetDpi(&mut dpi_x, &mut dpi_y);
        }
        let math_density = (dpi_x / 96.0 * 1.5).clamp(2.0, 4.0);
        let size = unsafe { target.GetSize() };
        let resources = list.visible_resources(Rect::from_size(0.0, 0.0, size.width, size.height));
        let visible = resources
            .iter()
            .filter_map(|index| match &list.cmds()[*index] {
                DrawCmd::Image {
                    src,
                    rect,
                    thumbnail,
                    ..
                } => Some(if *thumbnail {
                    thumbnail_key(src, thumbnail_edge(*rect, dpi_x))
                } else {
                    src.clone()
                }),
                DrawCmd::Math {
                    tex,
                    font_size,
                    color,
                    wrap,
                    ..
                } => Some(crate::ui::math_layout::bitmap_key_for(
                    tex,
                    *font_size,
                    *color,
                    math_density,
                    *wrap,
                )),
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>();
        self.bitmaps.retain(|key, _| visible.contains(key.as_str()));
        // 先把这一帧要用的图标几何建齐。放在循环外是借用规则的要求：
        // 循环里同时拿着画刷缓存（&mut 一个字段）和几何缓存（另一个字段）
        for (index, cmd) in list.cmds().iter().enumerate() {
            match cmd {
                DrawCmd::Image {
                    src,
                    rect,
                    thumbnail,
                    ..
                } if resources.binary_search(&index).is_ok() => {
                    if *thumbnail {
                        self.ensure_thumbnail(src, thumbnail_edge(*rect, dpi_x));
                    } else {
                        self.ensure_bitmap(src);
                    }
                }
                DrawCmd::Math {
                    tex,
                    font_size,
                    color,
                    wrap,
                    ..
                } => {
                    if resources.binary_search(&index).is_err() {
                        continue;
                    }
                    let key = crate::ui::math_layout::bitmap_key_for(
                        tex,
                        *font_size,
                        *color,
                        math_density,
                        *wrap,
                    );
                    if !self.bitmaps.contains_key(&key) {
                        match if wrap.is_none() {
                            crate::ui::math_layout::png(tex, *font_size, *color, math_density)
                        } else {
                            crate::ui::math_layout::png_for(
                                tex,
                                *font_size,
                                *color,
                                math_density,
                                *wrap,
                            )
                        } {
                            Ok(bytes) => {
                                self.register_bytes(&key, bytes);
                                self.ensure_bitmap(&key);
                                self.byte_sources.remove(&key);
                            }
                            Err(_) => {
                                self.bitmaps.insert(key, None);
                            }
                        }
                    }
                }
                DrawCmd::Icon { icon, .. } => self.ensure_icon(*icon),
                DrawCmd::Logo { .. } if self.logo.is_none() => {
                    self.logo = build_logo(&self.d2d, target).ok();
                }
                _ => {}
            }
        }
        let Some(brushes) = self.brushes.as_mut() else {
            return;
        };

        for (index, cmd) in list.cmds().iter().enumerate() {
            match cmd {
                DrawCmd::Math {
                    rect,
                    tex,
                    font_size,
                    color,
                    wrap,
                } => {
                    if resources.binary_search(&index).is_err() {
                        continue;
                    }
                    let key = if wrap.is_none() {
                        crate::ui::math_layout::bitmap_key(tex, *font_size, *color, math_density)
                    } else {
                        crate::ui::math_layout::bitmap_key_for(
                            tex,
                            *font_size,
                            *color,
                            math_density,
                            *wrap,
                        )
                    };
                    if let Some(Some(bitmap)) = self.bitmaps.get(&key) {
                        unsafe {
                            target.DrawBitmap(
                                bitmap,
                                Some(&d2d_rect(rect)),
                                1.0,
                                D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                                None,
                            );
                        }
                    } else if let (Some(brush), Some(format)) = (
                        brushes.get(*color),
                        self.formats.get(&(
                            TextStyle::DocumentMono,
                            Align::Leading,
                            Emphasis::None,
                        )),
                    ) {
                        let text = tex
                            .chars()
                            .take(400)
                            .collect::<String>()
                            .encode_utf16()
                            .collect::<Vec<_>>();
                        unsafe {
                            target.DrawText(
                                &text,
                                format,
                                &d2d_rect(rect),
                                &brush,
                                Default::default(),
                                Default::default(),
                            );
                        }
                    }
                }
                DrawCmd::Caret { visible: false, .. } => {}
                DrawCmd::ScaledText {
                    rect,
                    text,
                    style,
                    color,
                    align,
                    scale,
                } => {
                    let Some(brush) = brushes.get(*color) else {
                        continue;
                    };
                    let Some(format) = self.formats.get(&(*style, *align, Emphasis::None)) else {
                        continue;
                    };
                    let scale = scale.clamp(0.001, 64.);
                    let transform = Matrix3x2 {
                        M11: scale,
                        M12: 0.,
                        M21: 0.,
                        M22: scale,
                        M31: rect.left,
                        M32: rect.top,
                    };
                    let local =
                        Rect::from_size(0., 0., rect.width() / scale, rect.height() / scale);
                    let utf16: Vec<u16> = text.encode_utf16().collect();
                    unsafe {
                        target.SetTransform(&transform);
                        target.DrawText(
                            &utf16,
                            format,
                            &d2d_rect(&local),
                            &brush,
                            Default::default(),
                            Default::default(),
                        );
                        target.SetTransform(&Matrix3x2::identity());
                    }
                }
                DrawCmd::Polyline {
                    points,
                    color,
                    width,
                } => {
                    if let Some(brush) = brushes.get(*color) {
                        unsafe {
                            target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                            for pair in points.windows(2) {
                                target.DrawLine(
                                    pt(pair[0].0, pair[0].1),
                                    pt(pair[1].0, pair[1].1),
                                    &brush,
                                    *width,
                                    None,
                                );
                            }
                        }
                    }
                }
                DrawCmd::Rect { rect, color }
                | DrawCmd::Caret {
                    rect,
                    color,
                    visible: true,
                } => {
                    let Some(brush) = brushes.get(*color) else {
                        continue;
                    };
                    // 面板底色和 1px 分隔线要**关掉反锯齿**：D2D 默认的
                    // PER_PRIMITIVE 会把非整数边界糊成半透明的两像素灰边，
                    // 相邻面板拼接处就会出现一条比设计稿浅的缝。
                    unsafe {
                        target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
                        target.FillRectangle(&d2d_rect(rect), &brush);
                        target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                    }
                }
                DrawCmd::Text {
                    rect,
                    text,
                    style,
                    color,
                    align,
                    emphasis,
                } => {
                    if let Emphasis::Styled { bg: Some(bg), .. } = emphasis {
                        if let Some(fill) = brushes.get(*bg) {
                            unsafe {
                                target.FillRectangle(&d2d_rect(rect), &fill);
                            }
                        }
                    }
                    let color = match emphasis {
                        Emphasis::Styled { fg: Some(fg), .. } => *fg,
                        _ => *color,
                    };
                    let Some(brush) = brushes.get(color) else {
                        continue;
                    };
                    // 行内代码换等宽档；粗/斜在同一档里换字重/字形
                    let style = if emphasis.base() == Emphasis::Code && *style == TextStyle::Body {
                        TextStyle::Mono
                    } else {
                        *style
                    };
                    let Some(format) = self.formats.get(&(style, *align, emphasis.base())) else {
                        continue;
                    };
                    let utf16: Vec<u16> = text.encode_utf16().collect();
                    unsafe {
                        target.DrawText(
                            &utf16,
                            format,
                            &d2d_rect(rect),
                            &brush,
                            windows::Win32::Graphics::Direct2D::D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
                            Default::default(),
                        );
                        if let Emphasis::Styled { flags, .. } = emphasis {
                            if flags & 4 != 0 {
                                target.DrawLine(
                                    pt(rect.left, rect.bottom - 4.0),
                                    pt(rect.right, rect.bottom - 4.0),
                                    &brush,
                                    1.0,
                                    None,
                                );
                            }
                            if flags & 8 != 0 {
                                let y = (rect.top + rect.bottom) / 2.0;
                                target.DrawLine(
                                    pt(rect.left, y),
                                    pt(rect.right, y),
                                    &brush,
                                    1.0,
                                    None,
                                );
                            }
                        }
                    }
                }
                DrawCmd::RectAlpha { rect, color, alpha } => {
                    // 半透明画刷不进缓存：一帧最多一两个（遮罩），现建现用
                    let mut c = rgb(*color);
                    c.a = *alpha;
                    let Ok(brush) = (unsafe { target.CreateSolidColorBrush(&c, None) }) else {
                        continue;
                    };
                    unsafe {
                        target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
                        target.FillRectangle(&d2d_rect(rect), &brush);
                        target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                    }
                }
                DrawCmd::ShapeBorder {
                    rect,
                    ellipse,
                    width,
                    color,
                } => {
                    let Some(brush) = brushes.get(*color) else {
                        continue;
                    };
                    unsafe {
                        if *ellipse {
                            let shape = windows::Win32::Graphics::Direct2D::D2D1_ELLIPSE {
                                point: pt(
                                    (rect.left + rect.right) / 2.0,
                                    (rect.top + rect.bottom) / 2.0,
                                ),
                                radiusX: rect.width() / 2.0,
                                radiusY: rect.height() / 2.0,
                            };
                            target.DrawEllipse(&shape, &brush, *width, None);
                        } else {
                            target.DrawRectangle(&d2d_rect(rect), &brush, *width, None);
                        }
                    }
                }
                DrawCmd::GlassButton {
                    rect,
                    radius,
                    dark,
                    hovered,
                } => {
                    let base = if *dark { 0xf0f1f3 } else { 0x181b20 };
                    let tint = if *hovered {
                        crate::ui::theme::mix(base, if *dark { 0 } else { 0xffffff }, 0.96)
                    } else {
                        base
                    };
                    let mut top = rgb(crate::ui::theme::mix(
                        tint,
                        0xffffff,
                        if *dark { 0.58 } else { 0.88 },
                    ));
                    let mut middle = rgb(crate::ui::theme::mix(
                        tint,
                        0xffffff,
                        if *dark { 0.90 } else { 0.98 },
                    ));
                    let mut bottom = rgb(tint);
                    top.a = 0.94;
                    middle.a = 0.94;
                    bottom.a = 0.94;
                    let stops = [
                        D2D1_GRADIENT_STOP {
                            position: 0.0,
                            color: top,
                        },
                        D2D1_GRADIENT_STOP {
                            position: 0.46,
                            color: middle,
                        },
                        D2D1_GRADIENT_STOP {
                            position: 1.0,
                            color: bottom,
                        },
                    ];
                    let rr = D2D1_ROUNDED_RECT {
                        rect: d2d_rect(rect),
                        radiusX: *radius,
                        radiusY: *radius,
                    };
                    unsafe {
                        // 先画一层小的漫射阴影，再叠半透明色调和细内描边。
                        for (spread, alpha) in [(2.0, 0.025), (1.0, 0.045)] {
                            let mut c = rgb(0x1f3142);
                            c.a = alpha;
                            if let Ok(brush) = target.CreateSolidColorBrush(&c, None) {
                                let shadow = Rect::new(
                                    rect.left - spread,
                                    rect.top + 1.0,
                                    rect.right + spread,
                                    rect.bottom + spread + 1.0,
                                );
                                target.FillRoundedRectangle(
                                    &D2D1_ROUNDED_RECT {
                                        rect: d2d_rect(&shadow),
                                        radiusX: radius + spread,
                                        radiusY: radius + spread,
                                    },
                                    &brush,
                                );
                            }
                        }
                        if let Ok(stops) = target.CreateGradientStopCollection(
                            &stops,
                            D2D1_GAMMA_2_2,
                            D2D1_EXTEND_MODE_CLAMP,
                        ) {
                            if let Ok(brush) = target.CreateLinearGradientBrush(
                                &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                                    startPoint: Vector2 {
                                        X: rect.left,
                                        Y: rect.top,
                                    },
                                    endPoint: Vector2 {
                                        X: rect.left + rect.width() * 0.25,
                                        Y: rect.bottom,
                                    },
                                },
                                None,
                                &stops,
                            ) {
                                target.FillRoundedRectangle(&rr, &brush);
                            }
                        }
                        let mut rim = rgb(0xffffff);
                        rim.a = 0.40;
                        if let Ok(brush) = target.CreateSolidColorBrush(&rim, None) {
                            let inner = Rect::new(
                                rect.left + 0.5,
                                rect.top + 0.5,
                                rect.right - 0.5,
                                rect.bottom - 0.5,
                            );
                            target.DrawRoundedRectangle(
                                &D2D1_ROUNDED_RECT {
                                    rect: d2d_rect(&inner),
                                    radiusX: *radius,
                                    radiusY: *radius,
                                },
                                &brush,
                                1.0,
                                None,
                            );
                            rim.a = 0.48;
                            brush.SetColor(&rim);
                            target.DrawLine(
                                Vector2 {
                                    X: rect.left + radius,
                                    Y: rect.top + 1.0,
                                },
                                Vector2 {
                                    X: rect.right - radius,
                                    Y: rect.top + 1.0,
                                },
                                &brush,
                                1.0,
                                None,
                            );
                        }
                    }
                }
                DrawCmd::RoundedRect {
                    rect,
                    radius,
                    color,
                } => {
                    let Some(brush) = brushes.get(*color) else {
                        continue;
                    };
                    let rr = D2D1_ROUNDED_RECT {
                        rect: d2d_rect(rect),
                        radiusX: *radius,
                        radiusY: *radius,
                    };
                    unsafe { target.FillRoundedRectangle(&rr, &brush) };
                }
                DrawCmd::RoundedRectAlpha {
                    rect,
                    radius,
                    color,
                    alpha,
                } => {
                    let mut c = rgb(*color);
                    c.a = *alpha;
                    let Ok(brush) = (unsafe { target.CreateSolidColorBrush(&c, None) }) else {
                        continue;
                    };
                    let rr = D2D1_ROUNDED_RECT {
                        rect: d2d_rect(rect),
                        radiusX: *radius,
                        radiusY: *radius,
                    };
                    unsafe {
                        target.FillRoundedRectangle(&rr, &brush);
                    }
                }
                DrawCmd::RoundedBorder {
                    rect,
                    radius,
                    color,
                } => {
                    let Some(brush) = brushes.get(*color) else {
                        continue;
                    };
                    // 描边以路径为中心，向内缩半像素才能落在盒子里且清晰
                    let inner = Rect::new(
                        rect.left + 0.5,
                        rect.top + 0.5,
                        rect.right - 0.5,
                        rect.bottom - 0.5,
                    );
                    let rr = D2D1_ROUNDED_RECT {
                        rect: d2d_rect(&inner),
                        radiusX: *radius,
                        radiusY: *radius,
                    };
                    unsafe { target.DrawRoundedRectangle(&rr, &brush, 1.0, None) };
                }
                DrawCmd::Icon { rect, icon, color } => {
                    let Some(brush) = brushes.get(*color) else {
                        continue;
                    };
                    // 等比缩放到盒内居中：24 单位 → 盒的短边
                    let size = rect.width().min(rect.height());
                    let scale = size / icons::VIEW_BOX;
                    let dx = rect.left + (rect.width() - size) / 2.0;
                    let dy = rect.top + (rect.height() - size) / 2.0;
                    let transform = Matrix3x2 {
                        M11: scale,
                        M12: 0.0,
                        M21: 0.0,
                        M22: scale,
                        M31: dx,
                        M32: dy,
                    };
                    let Some(geometries) = self.icon_geometry.get(icon) else {
                        continue;
                    };
                    unsafe {
                        target.SetTransform(&transform);
                        for g in geometries {
                            // 描边宽度在变换后的空间里算，2 单位随图标一起缩放——与 SVG 一致
                            target.DrawGeometry(
                                g,
                                &brush,
                                icons::STROKE_WIDTH,
                                Some(&self.icon_stroke),
                            );
                        }
                        target.SetTransform(&Matrix3x2::identity());
                    }
                }
                DrawCmd::Image {
                    rect,
                    src,
                    alt,
                    rotation,
                    fill,
                    thumbnail,
                } => {
                    if resources.binary_search(&index).is_err() {
                        continue;
                    }
                    let key = if *thumbnail {
                        thumbnail_key(src, thumbnail_edge(*rect, dpi_x))
                    } else {
                        src.clone()
                    };
                    match self.bitmaps.get(&key).cloned().flatten() {
                        Some(bitmap) => {
                            let size = unsafe { bitmap.GetSize() };
                            // 查看器（已算好尺寸）填满矩形；文档里的图按宽度等比缩放、顶部左对齐
                            let dest = if *fill {
                                d2d_rect(rect)
                            } else {
                                let scale = if size.width > 0.0 {
                                    (rect.width() / size.width).min(1.0)
                                } else {
                                    1.0
                                };
                                let w = size.width * scale;
                                let h = (size.height * scale).min(rect.height());
                                D2D_RECT_F {
                                    left: rect.left,
                                    top: rect.top,
                                    right: rect.left + w,
                                    bottom: rect.top + h,
                                }
                            };
                            unsafe {
                                if *rotation != 0 {
                                    // 绕矩形中心旋转：目标矩形本身按旋转后的宽高给，这里把它转回图片的方向
                                    let cx = (rect.left + rect.right) / 2.0;
                                    let cy = (rect.top + rect.bottom) / 2.0;
                                    let angle = (*rotation as f32).to_radians();
                                    let (s, c) = angle.sin_cos();
                                    let rot = Matrix3x2 {
                                        M11: c,
                                        M12: s,
                                        M21: -s,
                                        M22: c,
                                        M31: cx - c * cx + s * cy,
                                        M32: cy - s * cx - c * cy,
                                    };
                                    target.SetTransform(&rot);
                                    // 目标矩形保持图片自己的长宽比（调用方按未旋转的尺寸给），旋转全交给变换
                                    target.DrawBitmap(
                                        &bitmap,
                                        Some(&dest),
                                        1.0,
                                        D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                                        None,
                                    );
                                    target.SetTransform(&Matrix3x2::identity());
                                } else {
                                    target.DrawBitmap(
                                        &bitmap,
                                        Some(&dest),
                                        1.0,
                                        D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                                        None,
                                    );
                                }
                            }
                        }
                        None => {
                            // 占位：虚线感的边框 + alt/路径文字，让人知道这里本该有图
                            let (Some(border), Some(muted)) = (
                                brushes.get(theme::tokens().palette(false).border),
                                brushes.get(theme::tokens().palette(false).muted),
                            ) else {
                                continue;
                            };
                            let placeholder = D2D_RECT_F {
                                left: rect.left,
                                top: rect.top,
                                right: rect.right,
                                bottom: (rect.top + 56.0).min(rect.bottom),
                            };
                            unsafe {
                                target.DrawRectangle(&placeholder, &border, 1.0, None);
                            }
                            let label = if alt.is_empty() {
                                format!("图片加载失败：{src}")
                            } else {
                                format!("{alt}（图片加载失败：{src}）")
                            };
                            if let Some(format) = self.formats.get(&(
                                TextStyle::Caption,
                                Align::Leading,
                                Emphasis::None,
                            )) {
                                let utf16: Vec<u16> = label.encode_utf16().collect();
                                let text_rect = D2D_RECT_F {
                                    left: rect.left + 12.0,
                                    top: rect.top + 8.0,
                                    right: rect.right - 12.0,
                                    bottom: rect.top + 48.0,
                                };
                                unsafe {
                                    target.DrawText(
                                        &utf16,
                                        format,
                                        &text_rect,
                                        &muted,
                                        Default::default(),
                                        Default::default(),
                                    );
                                }
                            }
                        }
                    }
                }
                DrawCmd::Logo { rect } => {
                    let Some(logo) = &self.logo else { continue };
                    let scale = rect.width().min(rect.height()) / 512.0;
                    let transform = Matrix3x2 {
                        M11: scale,
                        M12: 0.0,
                        M21: 0.0,
                        M22: scale,
                        M31: rect.left,
                        M32: rect.top,
                    };
                    unsafe {
                        target.SetTransform(&transform);
                        draw_logo(target, logo);
                        target.SetTransform(&Matrix3x2::identity());
                    }
                }
                DrawCmd::PushClip { rect } => unsafe {
                    target.PushAxisAlignedClip(&d2d_rect(rect), D2D1_ANTIALIAS_MODE_ALIASED);
                },
                DrawCmd::PopClip => unsafe { target.PopAxisAlignedClip() },
                DrawCmd::BeginOpacity { rect, opacity } => unsafe {
                    use windows::Win32::Graphics::Direct2D::{ID2D1Layer, D2D1_LAYER_PARAMETERS};
                    let parameters = D2D1_LAYER_PARAMETERS {
                        contentBounds: d2d_rect(rect),
                        maskTransform: Matrix3x2::identity(),
                        opacity: *opacity,
                        ..Default::default()
                    };
                    target.PushLayer(&parameters, None::<&ID2D1Layer>);
                },
                DrawCmd::EndOpacity => unsafe { target.PopLayer() },
            }
        }
    }
}

/// 离屏位图句柄，图形实现不泄漏到 App。
pub struct Snapshot(windows::Win32::Graphics::Imaging::IWICBitmap);

/// PDF 导出使用与屏幕相同的 DrawList/D2D 回放，逐页压缩，不依赖 WebView 或 Office。
pub fn export_pdf(
    source: &str,
    output: &std::path::Path,
    base_dir: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    pdf_export::export(source, output, base_dir)
}

/// 把一条解析好的路径灌进 D2D 几何。
///
/// SVG 的 `A` 与 D2D 的 `AddArc` 参数一一对应（半径、旋转、大小弧、方向），
/// 只有方向标志的命名不同：SVG 的 `sweep=1` 是顺时针（y 轴向下的坐标系里）。
fn build_geometry(d2d: &ID2D1Factory, cmds: &[PathCmd]) -> Result<ID2D1PathGeometry> {
    build_geometry_with(d2d, cmds, false)
}

fn build_geometry_with(
    d2d: &ID2D1Factory,
    cmds: &[PathCmd],
    filled: bool,
) -> Result<ID2D1PathGeometry> {
    let geometry = unsafe { d2d.CreatePathGeometry()? };
    let sink = unsafe { geometry.Open()? };
    let mut open = false;
    // HOLLOW：只描边不填充（lucide 全是线条图标）；标志的色块要 FILLED
    let begin = if filled {
        D2D1_FIGURE_BEGIN_FILLED
    } else {
        D2D1_FIGURE_BEGIN_HOLLOW
    };
    unsafe {
        for cmd in cmds {
            match *cmd {
                PathCmd::MoveTo(x, y) => {
                    if open {
                        sink.EndFigure(D2D1_FIGURE_END_OPEN);
                    }
                    sink.BeginFigure(pt(x, y), begin);
                    open = true;
                }
                PathCmd::LineTo(x, y) => {
                    if open {
                        sink.AddLine(pt(x, y));
                    }
                }
                PathCmd::CubicTo(x1, y1, x2, y2, x, y) => {
                    if open {
                        sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                            point1: pt(x1, y1),
                            point2: pt(x2, y2),
                            point3: pt(x, y),
                        });
                    }
                }
                PathCmd::ArcTo {
                    rx,
                    ry,
                    rotation,
                    large,
                    sweep,
                    x,
                    y,
                } => {
                    if open {
                        sink.AddArc(&D2D1_ARC_SEGMENT {
                            point: pt(x, y),
                            size: D2D_SIZE_F {
                                width: rx,
                                height: ry,
                            },
                            rotationAngle: rotation,
                            sweepDirection: if sweep {
                                D2D1_SWEEP_DIRECTION_CLOCKWISE
                            } else {
                                D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE
                            },
                            arcSize: if large {
                                D2D1_ARC_SIZE_LARGE
                            } else {
                                D2D1_ARC_SIZE_SMALL
                            },
                        });
                    }
                }
                PathCmd::Close => {
                    if open {
                        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                        open = false;
                    }
                }
            }
        }
        if open {
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
        }
        sink.Close()?;
    }
    Ok(geometry)
}

/// 预建全部文本格式。档位是 `TextStyle` × `Align` × `Emphasis` 的笛卡尔积——
/// 看着多，但一共几十个对象，建一次用一辈子，比每次绘制现建便宜得多。
fn build_formats(
    dwrite: &IDWriteFactory,
) -> Result<HashMap<(TextStyle, Align, Emphasis), IDWriteTextFormat>> {
    use windows::Win32::Graphics::DirectWrite::DWRITE_FONT_WEIGHT;
    let ui_family = crate::ui::settings_values::text("appearance.uiFontFamily", theme::UI_FONT);
    let font = HSTRING::from(if ui_family.trim().is_empty() {
        theme::UI_FONT
    } else {
        ui_family.trim()
    });
    let document_family = crate::ui::settings_values::text("typography.fontFamily", theme::UI_FONT);
    let document_font = if document_family.trim().is_empty() {
        font.clone()
    } else {
        HSTRING::from(document_family.trim())
    };
    let code_family = crate::ui::settings_values::text("code.fontFamily", "Cascadia Mono");
    let code_font = HSTRING::from(if code_family.trim().is_empty() {
        "Cascadia Mono"
    } else {
        code_family.trim()
    });
    let ai_code_family = crate::ui::settings_values::text("assistant.codeFontFamily", "Consolas");
    let ai_code_font = HSTRING::from(if ai_code_family.trim().is_empty() {
        "Consolas"
    } else {
        ai_code_family.trim()
    });
    let mut formats = HashMap::new();

    for style in TextStyle::ALL {
        let base_weight = match style {
            TextStyle::Ai { kind: 1..=4, .. } => DWRITE_FONT_WEIGHT(650),
            TextStyle::DocumentTitle => DWRITE_FONT_WEIGHT_BOLD,
            TextStyle::Title | TextStyle::Large | TextStyle::Display => {
                DWRITE_FONT_WEIGHT_SEMI_BOLD
            }
            // 文档标题用 Bold 而不是 SemiBold——正文与标题的对比度靠字重拉开，
            // 光靠字号在 13px 起步的界面里区分度不够
            TextStyle::Heading1 => {
                DWRITE_FONT_WEIGHT(crate::ui::editor_preferences::current().heading_weight[0])
            }
            TextStyle::Heading2 => {
                DWRITE_FONT_WEIGHT(crate::ui::editor_preferences::current().heading_weight[1])
            }
            TextStyle::Heading3 => {
                DWRITE_FONT_WEIGHT(crate::ui::editor_preferences::current().heading_weight[2])
            }
            TextStyle::Heading4 => {
                DWRITE_FONT_WEIGHT(crate::ui::editor_preferences::current().heading_weight[3])
            }
            TextStyle::Heading5 => {
                DWRITE_FONT_WEIGHT(crate::ui::editor_preferences::current().heading_weight[4])
            }
            TextStyle::Heading6 => {
                DWRITE_FONT_WEIGHT(crate::ui::editor_preferences::current().heading_weight[5])
            }
            _ => DWRITE_FONT_WEIGHT_NORMAL,
        };
        let size = style.font_size();
        // 代码块换等宽字族。Cascadia Mono 是 Win11 自带的，
        // 缺失时 DirectWrite 会回退，不会崩
        let family = if matches!(style, TextStyle::Ai { kind: 5, .. }) {
            ai_code_font.clone()
        } else if matches!(
            style,
            TextStyle::Mono | TextStyle::DocumentMono | TextStyle::Clock
        ) {
            code_font.clone()
        } else if matches!(
            style,
            TextStyle::Document
                | TextStyle::DocumentTitle
                | TextStyle::Heading1
                | TextStyle::Heading2
                | TextStyle::Heading3
                | TextStyle::Heading4
                | TextStyle::Heading5
                | TextStyle::Heading6
                | TextStyle::Table
        ) {
            document_font.clone()
        } else {
            font.clone()
        };

        for align in [Align::Leading, Align::Center, Align::Trailing] {
            for emphasis in [
                Emphasis::None,
                Emphasis::Bold,
                Emphasis::Italic,
                Emphasis::BoldItalic,
                Emphasis::Code,
                Emphasis::Link,
                Emphasis::Math,
            ] {
                // 粗体在原有字重上再加一档；斜体走 DirectWrite 的 ITALIC 字形
                let weight = if matches!(emphasis, Emphasis::Bold | Emphasis::BoldItalic) {
                    DWRITE_FONT_WEIGHT_BOLD
                } else {
                    base_weight
                };
                let slant = if matches!(emphasis, Emphasis::Italic | Emphasis::BoldItalic) {
                    // 若干 CJK 字体族宣称有斜体、却返回不了任何斜体字形。
                    // OBLIQUE 让 DirectWrite 从可见的正体字形合成倾斜。
                    DWRITE_FONT_STYLE_OBLIQUE
                } else {
                    DWRITE_FONT_STYLE_NORMAL
                };
                // 行内公式：Windows 自带的数学字体，字号照 KaTeX 的 1.21em
                let (family, size) = if emphasis == Emphasis::Math {
                    (
                        HSTRING::from("Cambria Math"),
                        size * crate::ui::text::math_scale(style),
                    )
                } else if emphasis == Emphasis::Code {
                    if matches!(style, TextStyle::Ai { .. }) {
                        (ai_code_font.clone(), size * 0.9)
                    } else {
                        (code_font.clone(), size)
                    }
                } else {
                    (family.clone(), size)
                };
                // zh-CN 区域标签让 DirectWrite 走中文字形（避免日文汉字变体）
                let f = unsafe {
                    dwrite.CreateTextFormat(
                        &family,
                        None,
                        weight,
                        slant,
                        DWRITE_FONT_STRETCH_NORMAL,
                        size,
                        w!("zh-CN"),
                    )?
                };
                unsafe {
                    f.SetTextAlignment(match align {
                        Align::Leading => DWRITE_TEXT_ALIGNMENT_LEADING,
                        Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                        Align::Trailing => DWRITE_TEXT_ALIGNMENT_TRAILING,
                    })?;
                    // 纵向居中：绘制矩形给的是整行，文字在其中垂直居中
                    f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
                    f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                }
                formats.insert((style, align, emphasis), f);
            }
        }
    }

    Ok(formats)
}

fn pt(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

/// 建标志的资源。坐标全部照 `Logo.tsx` 的 512 视图盒。
/// 内存字节 → WIC 流 → 位图。PDF 页面渲染结果走这里。
fn decode_bitmap_from_bytes(
    wic: &IWICImagingFactory,
    target: &ID2D1RenderTarget,
    bytes: &[u8],
) -> Result<ID2D1Bitmap> {
    unsafe {
        let stream = SHCreateMemStream(Some(bytes)).ok_or_else(|| {
            windows::core::Error::from_hresult(windows::core::HRESULT(-2147024882))
        })?;
        let decoder =
            wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?;
        let frame = decoder.GetFrame(0)?;
        let converter = wic.CreateFormatConverter()?;
        converter.Initialize(
            &frame,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeMedianCut,
        )?;
        target.CreateBitmapFromWicBitmap(&converter, None)
    }
}

/// WIC 解码 → 32bpp PBGRA → D2D 位图。只支持本地路径；URL 直接失败（占位框会写明）。
fn decode_bitmap(
    wic: &IWICImagingFactory,
    target: &ID2D1RenderTarget,
    src: &str,
) -> Result<ID2D1Bitmap> {
    decode_bitmap_scaled(wic, target, src, None)
}
fn thumbnail_edge(rect: Rect, dpi: f32) -> u32 {
    (rect.width().max(rect.height()) * dpi / 96.0)
        .ceil()
        .clamp(1.0, 2048.0) as u32
}
fn thumbnail_key(src: &str, edge: u32) -> String {
    format!("ai-thumbnail://{edge}/{src}")
}
fn decode_bitmap_scaled(
    wic: &IWICImagingFactory,
    target: &ID2D1RenderTarget,
    src: &str,
    edge: Option<u32>,
) -> Result<ID2D1Bitmap> {
    let lower = src.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("data:") {
        return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
            -2147024809,
        )));
    }
    let path = HSTRING::from(src);
    unsafe {
        let decoder = wic.CreateDecoderFromFilename(
            &path,
            None,
            GENERIC_READ,
            WICDecodeMetadataCacheOnDemand,
        )?;
        let frame = decoder.GetFrame(0)?;
        let converter = wic.CreateFormatConverter()?;
        let source: windows::Win32::Graphics::Imaging::IWICBitmapSource = if let Some(edge) = edge {
            let (mut width, mut height) = (0, 0);
            frame.GetSize(&mut width, &mut height)?;
            if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 64_000_000 {
                return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                    -2147024809,
                )));
            }
            let scale = (f64::from(edge) / f64::from(width.max(height))).min(1.0);
            let scaler = wic.CreateBitmapScaler()?;
            scaler.Initialize(
                &frame,
                (f64::from(width) * scale).round().max(1.0) as u32,
                (f64::from(height) * scale).round().max(1.0) as u32,
                windows::Win32::Graphics::Imaging::WICBitmapInterpolationModeFant,
            )?;
            scaler.cast()?
        } else {
            frame.cast()?
        };
        converter.Initialize(
            &source,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeMedianCut,
        )?;
        target.CreateBitmapFromWicBitmap(&converter, None)
    }
}

fn build_logo(d2d: &ID2D1Factory, target: &ID2D1RenderTarget) -> Result<LogoResources> {
    let gradient =
        |from: u32, to: u32, start: Vector2, end: Vector2| -> Result<ID2D1LinearGradientBrush> {
            let stops = [
                D2D1_GRADIENT_STOP {
                    position: 0.0,
                    color: rgb(from),
                },
                D2D1_GRADIENT_STOP {
                    position: 1.0,
                    color: rgb(to),
                },
            ];
            unsafe {
                let collection = target.CreateGradientStopCollection(
                    &stops,
                    D2D1_GAMMA_2_2,
                    D2D1_EXTEND_MODE_CLAMP,
                )?;
                target.CreateLinearGradientBrush(
                    &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                        startPoint: start,
                        endPoint: end,
                    },
                    None,
                    &collection,
                )
            }
        };
    let solid = |c: u32, alpha: f32| -> Result<ID2D1SolidColorBrush> {
        let mut color = rgb(c);
        color.a = alpha;
        unsafe { target.CreateSolidColorBrush(&color, None) }
    };
    Ok(LogoResources {
        // x1=0 y1=0 x2=1 y2=1：对角渐变，覆盖整个底板
        bg: gradient(0xFFE9F5, 0xD8EEFF, pt(20.0, 20.0), pt(492.0, 492.0))?,
        // x2=0 y2=1：竖直渐变，覆盖身体的纵向范围
        squid: gradient(0xC084FC, 0x8B5CF6, pt(256.0, 75.0), pt(256.0, 360.0))?,
        body: build_geometry_with(
            d2d,
            &icons::parse_path("M256 75 C165 75 105 145 105 240 C105 320 165 360 256 360 C347 360 407 320 407 240 C407 145 347 75 256 75Z"),
            true,
        )?,
        tentacles: build_geometry(
            d2d,
            &icons::parse_path("M145 315 C115 380 180 390 215 335 C240 300 260 395 300 340 C330 305 390 370 365 315"),
        )?,
        // 铅笔尖：polygon 0,0 18,0 9,-28（在铅笔自己的坐标系里，绘制时套变换）
        pencil_tip: build_geometry_with(d2d, &icons::parse_path("M0 0L18 0L9 -28Z"), true)?,
        white: solid(0xFFFFFF, 1.0)?,
        tentacle_stroke: solid(0xA78BFA, 1.0)?,
        paper: solid(0xFFFDF5, 1.0)?,
        lines: solid(0xD4C6A5, 1.0)?,
        pencil: solid(0xFFB36B, 1.0)?,
        pencil_tip_brush: solid(0xFFE0B2, 1.0)?,
        highlight: solid(0xFFFFFF, 0.25)?,
    })
}

/// 画标志。调用前已经 `SetTransform` 到 512 视图盒 → 目标盒。
unsafe fn draw_logo(target: &ID2D1RenderTarget, l: &LogoResources) {
    let mut base = Matrix3x2::identity();
    target.GetTransform(&mut base);
    let rr = |x: f32, y: f32, w: f32, h: f32, r: f32| D2D1_ROUNDED_RECT {
        rect: D2D_RECT_F {
            left: x,
            top: y,
            right: x + w,
            bottom: y + h,
        },
        radiusX: r,
        radiusY: r,
    };
    let ellipse = |cx: f32, cy: f32, rx: f32, ry: f32| D2D1_ELLIPSE {
        point: pt(cx, cy),
        radiusX: rx,
        radiusY: ry,
    };

    target.FillRoundedRectangle(&rr(20.0, 20.0, 472.0, 472.0, 110.0), &l.bg);
    target.FillGeometry(&l.body, &l.squid, None);
    target.FillEllipse(&ellipse(215.0, 230.0, 9.0, 13.0), &l.white);
    target.FillEllipse(&ellipse(297.0, 230.0, 9.0, 13.0), &l.white);
    target.DrawGeometry(&l.tentacles, &l.tentacle_stroke, 32.0, None);

    // 笔记本：translate(315 270) rotate(-12)
    let rot = |deg: f32, x: f32, y: f32| -> Matrix3x2 {
        let r = deg.to_radians();
        let (s, c) = r.sin_cos();
        Matrix3x2 {
            M11: c,
            M12: s,
            M21: -s,
            M22: c,
            M31: x,
            M32: y,
        } * base
    };
    target.SetTransform(&rot(-12.0, 315.0, 270.0));
    target.FillRoundedRectangle(&rr(0.0, 0.0, 110.0, 95.0, 14.0), &l.paper);
    target.FillRoundedRectangle(&rr(12.0, 12.0, 86.0, 70.0, 8.0), &l.white);
    for (x1, y, x2) in [(30.0, 35.0, 80.0), (30.0, 55.0, 80.0), (30.0, 75.0, 65.0)] {
        target.DrawLine(pt(x1, y), pt(x2, y), &l.lines, 5.0, None);
    }

    // 铅笔：translate(370 220) rotate(35)
    target.SetTransform(&rot(35.0, 370.0, 220.0));
    target.FillRoundedRectangle(&rr(0.0, 0.0, 18.0, 100.0, 7.0), &l.pencil);
    target.FillGeometry(&l.pencil_tip, &l.pencil_tip_brush, None);

    target.SetTransform(&base);
    target.FillEllipse(&ellipse(190.0, 140.0, 35.0, 20.0), &l.highlight);
}

fn rgb(hex: u32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((hex >> 16) & 0xFF) as f32 / 255.0,
        g: ((hex >> 8) & 0xFF) as f32 / 255.0,
        b: (hex & 0xFF) as f32 / 255.0,
        a: 1.0,
    }
}

fn d2d_rect(r: &Rect) -> D2D_RECT_F {
    D2D_RECT_F {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_target_dpi_controls_the_dip_viewport_and_can_be_updated() {
        use windows::Win32::System::Com::{
            CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED,
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
        let mut renderer = Renderer::new().unwrap();
        let _snapshot = renderer.prepare_snapshot(400, 300, 144.0).unwrap();
        let mut dpi_x = 0.0;
        let mut dpi_y = 0.0;
        let target = renderer.target.as_ref().unwrap();
        unsafe {
            target.GetDpi(&mut dpi_x, &mut dpi_y);
        }
        assert!((dpi_x - 144.0).abs() < 0.01);
        assert!((dpi_y - 144.0).abs() < 0.01);
        let viewport = renderer.viewport();
        assert!((viewport.width() - 400.0 * 96.0 / 144.0).abs() < 0.01);
        assert!((viewport.height() - 300.0 * 96.0 / 144.0).abs() < 0.01);

        renderer.set_window_dpi(192, 192);
        let viewport = renderer.viewport();
        assert!((viewport.width() - 200.0).abs() < 0.01);
        assert!((viewport.height() - 150.0).abs() < 0.01);
    }

    #[test]
    fn ai_thumbnail_decodes_to_display_pixels_without_rewriting_original() {
        use windows::Win32::System::Com::{
            CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED,
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
        let path = std::env::temp_dir().join(format!(
            "mochi-thumbnail-{}.png",
            mochi_core::paths::random_base36(12)
        ));
        let mut renderer = Renderer::new().unwrap();
        let source = renderer.prepare_snapshot(1200, 800, 96.0).unwrap();
        let mut art = DrawList::new();
        art.rect(Rect::from_size(0.0, 0.0, 1200.0, 800.0), 0x335a78);
        renderer.present(HWND::default(), 0xffffff, &art).unwrap();
        renderer.save_snapshot(&source, &path).unwrap();
        let before = std::fs::read(&path).unwrap();
        let _target = renderer.prepare_snapshot(400, 300, 96.0).unwrap();
        let mut preview = DrawList::new();
        preview.image_thumbnail(
            Rect::from_size(0.0, 0.0, 80.0, 60.0),
            path.to_string_lossy(),
            "preview",
        );
        renderer
            .present(HWND::default(), 0xffffff, &preview)
            .unwrap();
        let bitmap = renderer.bitmaps.values().next().unwrap().as_ref().unwrap();
        let size = unsafe { bitmap.GetPixelSize() };
        assert!(size.width <= 80 && size.height <= 80);
        assert!(size.width > 0 && size.height > 0);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let mut full = DrawList::new();
        full.image(
            Rect::from_size(0.0, 0.0, 400.0, 300.0),
            path.to_string_lossy(),
            "full",
        );
        renderer.present(HWND::default(), 0xffffff, &full).unwrap();
        let size = unsafe {
            renderer
                .bitmaps
                .values()
                .next()
                .unwrap()
                .as_ref()
                .unwrap()
                .GetPixelSize()
        };
        assert_eq!((size.width, size.height), (1200, 800));
        drop(renderer);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn offscreen_math_does_not_allocate_or_retain_bitmap_resources() {
        use windows::Win32::System::Com::{
            CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED,
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
        let mut renderer = Renderer::new().unwrap();
        let _snapshot = renderer.prepare_snapshot(400, 300, 96.0).unwrap();
        renderer.register_bytes(
            "mem://visibility-test",
            crate::ui::math_layout::png("z", 16.0, 0, 2.0).unwrap(),
        );
        let mut list = DrawList::new();
        list.push_clip(Rect::from_size(0.0, 0.0, 400.0, 80.0));
        list.math(Rect::from_size(0.0, 10.0, 30.0, 25.0), "x", 16.0, 0);
        for i in 0..100 {
            list.math(
                Rect::from_size(0.0, 100.0 + i as f32 * 30.0, 80.0, 25.0),
                &format!("x_{{{i}}}"),
                16.0,
                0,
            );
        }
        list.image(
            Rect::from_size(0.0, 200.0, 30.0, 30.0),
            "mem://visibility-test",
            "",
        );
        list.pop_clip();
        renderer.present(HWND::default(), 0xffffff, &list).unwrap();
        assert_eq!(renderer.bitmaps.len(), 1);
        assert!(renderer.bitmaps.values().all(Option::is_some));
        let mut next = DrawList::new();
        next.math(Rect::from_size(0.0, 10.0, 80.0, 25.0), "x_{99}", 16.0, 0);
        renderer.present(HWND::default(), 0xffffff, &next).unwrap();
        assert_eq!(renderer.bitmaps.len(), 1);
        assert!(renderer.bitmaps.keys().all(|key| key.ends_with("/x_{99}")));
        let mut image = DrawList::new();
        image.image(
            Rect::from_size(0.0, 0.0, 30.0, 30.0),
            "mem://visibility-test",
            "",
        );
        renderer.present(HWND::default(), 0xffffff, &image).unwrap();
        assert_eq!(renderer.bitmaps.len(), 1);
        assert!(renderer.bitmaps["mem://visibility-test"].is_some());
        renderer
            .present(HWND::default(), 0xffffff, &DrawList::new())
            .unwrap();
        assert!(renderer.bitmaps.is_empty());
    }

    #[test]
    fn exported_pdf_is_readable_by_windows_pdf_engine() {
        let path = std::env::temp_dir().join(format!(
            "mochi-export-{}-{}.pdf",
            std::process::id(),
            mochi_core::jstime::now_millis()
        ));
        export_pdf("# 导出测试\n\n中文 **正文** 与 $x^2$。", &path, None).unwrap();
        let (handle, rx) = crate::pdf::open(path.clone(), path.clone(), 0);
        match rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap() {
            crate::pdf::PdfEvent::Loaded { sizes, .. } => assert_eq!(sizes.len(), 1),
            other => panic!("PDF 打开失败：{other:?}"),
        }
        drop(handle);
        drop(rx);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rgb_unpacks_a_hex_color_channel_by_channel() {
        let c = rgb(0x3C5A78);
        assert!((c.r - 0x3C as f32 / 255.0).abs() < 1e-6);
        assert!((c.g - 0x5A as f32 / 255.0).abs() < 1e-6);
        assert!((c.b - 0x78 as f32 / 255.0).abs() < 1e-6);
        assert_eq!(c.a, 1.0, "颜色令牌不带 alpha，回放时一律不透明");
    }

    #[test]
    fn gpu_mode_is_opt_in() {
        // 默认走软件光栅，见 low_memory_mode 的注释；靠环境变量显式开 GPU
        assert_eq!(low_memory_mode(), std::env::var_os("MOCHI_GPU").is_none());
    }
}

//! Native Windows Shell support for desktop-folder cards.
//!
//! All Shell transfer calls are one item at a time. Callers should run the
//! potentially blocking IFileOperation functions on a worker STA thread and
//! only mark an item complete after the returned result and the core manifest
//! validation both succeed.

use anyhow::{anyhow, bail, ensure, Context, Result};
use std::ffi::OsString;
use std::mem::ManuallyDrop;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use windows::core::{implement, w, PCWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GlobalFree, DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS,
    E_ABORT, HANDLE, HGLOBAL, HWND, S_OK,
};
use windows::Win32::System::Com::{
    CoCreateInstance, IDataObject, CLSCTX_INPROC_SERVER, DVASPECT_CONTENT, FORMATETC, STGMEDIUM,
    STGMEDIUM_0, TYMED_HGLOBAL,
};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
use windows::Win32::System::Ole::{
    DoDragDrop, IDropSource, IDropSource_Impl, OleFlushClipboard, OleGetClipboard, OleInitialize,
    OleSetClipboard, OleUninitialize, ReleaseStgMedium, DROPEFFECT, DROPEFFECT_COPY,
    DROPEFFECT_LINK, DROPEFFECT_MOVE,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Shell::{
    DragQueryFileW, FileOperation, IFileOperation, IFileOperationProgressSink, IShellItem,
    SHCreateDataObject, SHCreateItemFromParsingName, FOFX_EARLYFAILURE, HDROP,
};

const CF_HDROP: u16 = 15;
const MAX_PATHS: usize = 2048;
const MAX_PATH_UNITS: usize = 32_767;
const MAX_TOTAL_PATH_UNITS: usize = 1_000_000;
const MOCHI_CLIPBOARD_MARKER: u32 = 0x4D43_4849;

/// The preferred operation associated with a Shell file clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardEffect {
    Copy,
    Move,
}

/// Files advertised by the current clipboard, including its preferred effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileClipboard {
    pub paths: Vec<PathBuf>,
    pub effect: ClipboardEffect,
    /// True only when the current object carries Mochi's private clipboard marker.
    pub owned_by_mochi: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragEffect {
    Copy,
    Move,
    Link,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragOutcome {
    Dropped(DragEffect),
    Cancelled,
}

struct ComApartment;

impl ComApartment {
    fn sta() -> Result<Self> {
        // OleInitialize also initializes the COM STA. S_OK and S_FALSE both
        // require a matching OleUninitialize.
        unsafe { OleInitialize(None) }
            .map_err(|error| anyhow!(error).context("初始化 OLE STA 失败"))?;
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { OleUninitialize() };
    }
}

struct GlobalBlock(Option<HGLOBAL>);

impl GlobalBlock {
    fn from_bytes(bytes: &[u8]) -> Result<Self> {
        ensure!(!bytes.is_empty(), "Shell 全局内存不能为空");
        let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) }
            .context("分配 Shell 全局内存失败")?;
        let block = Self(Some(memory));
        let pointer = unsafe { GlobalLock(memory) };
        if pointer.is_null() {
            bail!("锁定 Shell 全局内存失败");
        }
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast(), bytes.len());
            let _ = GlobalUnlock(memory);
        }
        Ok(block)
    }

    fn handle(&self) -> Result<HGLOBAL> {
        self.0.ok_or_else(|| anyhow!("Shell 全局内存所有权已转移"))
    }

    fn transfer_to_shell(&mut self) {
        self.0.take();
    }
}

impl Drop for GlobalBlock {
    fn drop(&mut self) {
        if let Some(memory) = self.0.take() {
            unsafe {
                let _ = GlobalFree(Some(memory));
            }
        }
    }
}

struct MediumGuard(STGMEDIUM);

impl Drop for MediumGuard {
    fn drop(&mut self) {
        unsafe { ReleaseStgMedium(&mut self.0) };
    }
}

struct Pidl(*mut windows::Win32::UI::Shell::Common::ITEMIDLIST);

impl Drop for Pidl {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                windows::Win32::UI::Shell::ILFree(Some(self.0));
            }
        }
    }
}

/// Convert only Win32 extended drive/UNC spellings to their ordinary Shell
/// spellings. Keep other extended namespaces intact and keep canonical paths
/// untouched everywhere outside the Shell boundary.
pub(super) fn normalized_shell_path(path: &Path) -> OsString {
    let wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
    let extended_prefix = [b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
    if wide.starts_with(&extended_prefix)
        && wide.len() >= 8
        && wide[4..8]
            .iter()
            .zip([b'U', b'N', b'C', b'\\'])
            .all(|(actual, expected)| {
                let actual = if (b'a' as u16..=b'z' as u16).contains(actual) {
                    *actual - 32
                } else {
                    *actual
                };
                actual == expected.to_ascii_uppercase() as u16
            })
    {
        let mut ordinary = vec![b'\\' as u16, b'\\' as u16];
        ordinary.extend_from_slice(&wide[8..]);
        return OsString::from_wide(&ordinary);
    }
    if wide.starts_with(&extended_prefix)
        && wide.len() >= 7
        && ((b'A' as u16..=b'Z' as u16).contains(&wide[4])
            || (b'a' as u16..=b'z' as u16).contains(&wide[4]))
        && wide[5] == b':' as u16
        && (wide[6] == b'\\' as u16 || wide[6] == b'/' as u16)
    {
        return OsString::from_wide(&wide[4..]);
    }
    OsString::from_wide(&wide)
}

fn nul_terminated(path: &Path) -> Vec<u16> {
    normalized_shell_path(path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn checked_paths(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    ensure!(!paths.is_empty(), "至少选择一个文件或文件夹");
    ensure!(paths.len() <= MAX_PATHS, "所选项目超过 Shell 上限");
    let mut total_units = 0usize;
    let mut checked = Vec::with_capacity(paths.len());
    for path in paths {
        let canonical = path
            .canonicalize()
            .with_context(|| format!("文件或文件夹不存在或无法访问: {}", path.display()))?;
        let metadata = std::fs::metadata(&canonical)
            .with_context(|| format!("读取项目属性失败: {}", canonical.display()))?;
        ensure!(
            metadata.is_file() || metadata.is_dir(),
            "Shell 仅支持文件和文件夹"
        );
        let units = canonical.as_os_str().encode_wide().count();
        ensure!(units > 0 && units < MAX_PATH_UNITS, "路径超过 Shell 上限");
        total_units = total_units.saturating_add(units + 1);
        ensure!(
            total_units <= MAX_TOTAL_PATH_UNITS,
            "所选路径数据超过 Shell 上限"
        );
        checked.push(canonical);
    }
    Ok(checked)
}

fn shell_data_object(paths: &[PathBuf]) -> Result<IDataObject> {
    let paths = checked_paths(paths)?;
    let mut pidls = Vec::with_capacity(paths.len());
    for path in paths {
        let wide = nul_terminated(&path);
        let pidl = unsafe { windows::Win32::UI::Shell::ILCreateFromPathW(PCWSTR(wide.as_ptr())) };
        if pidl.is_null() {
            bail!("无法为 Shell 项目创建 PIDL: {}", path.display());
        }
        pidls.push(Pidl(pidl));
    }
    let absolute_pidls = pidls
        .iter()
        .map(|pidl| pidl.0 as *const windows::Win32::UI::Shell::Common::ITEMIDLIST)
        .collect::<Vec<_>>();
    // With a null parent PIDL, Shell expects fully qualified PIDLs. This is
    // required for mixed selections from unrelated directories.
    unsafe {
        SHCreateDataObject(None, Some(&absolute_pidls), None::<&IDataObject>)
            .context("创建 Shell 文件数据对象失败")
    }
}

fn preferred_drop_effect_format() -> Result<u16> {
    let format = unsafe { RegisterClipboardFormatW(w!("Preferred DropEffect")) };
    ensure!(format != 0, "注册 Preferred DropEffect 格式失败");
    Ok(format as u16)
}

fn set_global_format(data: &IDataObject, format: u16, bytes: &[u8]) -> Result<()> {
    let mut memory = GlobalBlock::from_bytes(bytes)?;
    let medium = STGMEDIUM {
        tymed: TYMED_HGLOBAL.0 as u32,
        u: STGMEDIUM_0 {
            hGlobal: memory.handle()?,
        },
        pUnkForRelease: ManuallyDrop::new(None),
    };
    let format = FORMATETC {
        cfFormat: format,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    unsafe { data.SetData(&format, &medium, true) }
        .context("写入 Shell Preferred DropEffect 失败")?;
    memory.transfer_to_shell();
    Ok(())
}

fn set_preferred_effect(data: &IDataObject, effect: ClipboardEffect) -> Result<()> {
    let value: u32 = match effect {
        ClipboardEffect::Copy => DROPEFFECT_COPY.0,
        ClipboardEffect::Move => DROPEFFECT_MOVE.0,
    };
    set_global_format(data, preferred_drop_effect_format()?, &value.to_le_bytes())
}

fn mochi_clipboard_format() -> Result<u16> {
    let format = unsafe { RegisterClipboardFormatW(w!("Mochi.DesktopFiles.OwnerV1")) };
    ensure!(format != 0, "注册 Mochi 文件剪贴板标记失败");
    Ok(format as u16)
}

fn set_mochi_clipboard_marker(data: &IDataObject) -> Result<()> {
    set_global_format(
        data,
        mochi_clipboard_format()?,
        &MOCHI_CLIPBOARD_MARKER.to_le_bytes(),
    )
}

/// Put real filesystem items on the OLE clipboard in native Shell formats.
pub fn set_file_clipboard(paths: &[PathBuf], effect: ClipboardEffect) -> Result<()> {
    let _apartment = ComApartment::sta()?;
    let data = shell_data_object(paths)?;
    set_preferred_effect(&data, effect)?;
    set_mochi_clipboard_marker(&data)?;
    unsafe { OleSetClipboard(&data) }.context("写入文件剪贴板失败")?;
    unsafe { OleFlushClipboard() }.context("持久化文件剪贴板失败")
}

fn get_hdrop(data: &IDataObject) -> Result<Option<MediumGuard>> {
    let format = FORMATETC {
        cfFormat: CF_HDROP,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    if unsafe { data.QueryGetData(&format) }.is_err() {
        return Ok(None);
    }
    let medium = unsafe { data.GetData(&format) }.context("读取 Shell 文件剪贴板失败")?;
    ensure!(
        medium.tymed == TYMED_HGLOBAL.0 as u32,
        "Shell 文件剪贴板格式不是全局内存"
    );
    Ok(Some(MediumGuard(medium)))
}

fn read_preferred_effect(data: &IDataObject) -> ClipboardEffect {
    let Ok(format_id) = preferred_drop_effect_format() else {
        return ClipboardEffect::Copy;
    };
    let format = FORMATETC {
        cfFormat: format_id,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let Ok(medium) = (unsafe { data.GetData(&format) }) else {
        return ClipboardEffect::Copy;
    };
    let medium = MediumGuard(medium);
    if medium.0.tymed != TYMED_HGLOBAL.0 as u32 {
        return ClipboardEffect::Copy;
    }
    let memory = unsafe { medium.0.u.hGlobal };
    if memory.is_invalid() || unsafe { GlobalSize(memory) } < std::mem::size_of::<u32>() {
        return ClipboardEffect::Copy;
    }
    let pointer = unsafe { GlobalLock(memory) }.cast::<u32>();
    if pointer.is_null() {
        return ClipboardEffect::Copy;
    }
    let value = unsafe { std::ptr::read_unaligned(pointer) };
    unsafe {
        let _ = GlobalUnlock(memory);
    }
    if value & DROPEFFECT_MOVE.0 != 0 {
        ClipboardEffect::Move
    } else {
        ClipboardEffect::Copy
    }
}

fn has_mochi_clipboard_marker(data: &IDataObject) -> bool {
    let Ok(format_id) = mochi_clipboard_format() else {
        return false;
    };
    let format = FORMATETC {
        cfFormat: format_id,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let Ok(medium) = (unsafe { data.GetData(&format) }) else {
        return false;
    };
    let medium = MediumGuard(medium);
    if medium.0.tymed != TYMED_HGLOBAL.0 as u32 {
        return false;
    }
    let memory = unsafe { medium.0.u.hGlobal };
    if memory.is_invalid() || unsafe { GlobalSize(memory) } < std::mem::size_of::<u32>() {
        return false;
    }
    let pointer = unsafe { GlobalLock(memory) }.cast::<u32>();
    if pointer.is_null() {
        return false;
    }
    let marker = unsafe { std::ptr::read_unaligned(pointer) };
    unsafe {
        let _ = GlobalUnlock(memory);
    }
    marker == MOCHI_CLIPBOARD_MARKER
}

/// Read CF_HDROP and Preferred DropEffect. Non-file clipboard contents return None.
pub fn read_file_clipboard() -> Result<Option<FileClipboard>> {
    let _apartment = ComApartment::sta()?;
    let data = match unsafe { OleGetClipboard() } {
        Ok(data) => data,
        Err(_) => return Ok(None),
    };
    let Some(medium) = get_hdrop(&data)? else {
        return Ok(None);
    };
    let memory = unsafe { medium.0.u.hGlobal };
    ensure!(!memory.is_invalid(), "Shell 文件剪贴板为空");
    let byte_len = unsafe { GlobalSize(memory) };
    ensure!(
        byte_len >= 20 && byte_len <= MAX_TOTAL_PATH_UNITS * 2 + 64,
        "Shell 文件剪贴板大小无效"
    );
    let hdrop = HDROP(memory.0);
    let count = unsafe { DragQueryFileW(hdrop, u32::MAX, None) } as usize;
    ensure!(
        count > 0 && count <= MAX_PATHS,
        "Shell 文件剪贴板项目数量无效"
    );
    let mut paths = Vec::with_capacity(count);
    let mut total_units = 0usize;
    for index in 0..count {
        let length = unsafe { DragQueryFileW(hdrop, index as u32, None) } as usize;
        ensure!(
            length > 0 && length < MAX_PATH_UNITS,
            "Shell 文件剪贴板中的路径长度无效"
        );
        total_units = total_units.saturating_add(length + 1);
        ensure!(
            total_units <= MAX_TOTAL_PATH_UNITS,
            "Shell 文件剪贴板路径总量超限"
        );
        let mut wide = vec![0u16; length + 1];
        let written = unsafe { DragQueryFileW(hdrop, index as u32, Some(&mut wide)) } as usize;
        ensure!(written == length, "Shell 文件剪贴板路径读取不完整");
        paths.push(PathBuf::from(OsString::from_wide(&wide[..written])));
    }
    let effect = read_preferred_effect(&data);
    let owned_by_mochi = has_mochi_clipboard_marker(&data);
    Ok(Some(FileClipboard {
        paths,
        effect,
        owned_by_mochi,
    }))
}

/// Clear the clipboard only if it still advertises the exact cut operation we started.
/// The caller should invoke this only after every Move item has been safely published.
pub fn clear_file_clipboard_if_matches(expected: &FileClipboard) -> Result<bool> {
    if expected.effect != ClipboardEffect::Move || !expected.owned_by_mochi {
        return Ok(false);
    }
    let Some(current) = read_file_clipboard()? else {
        return Ok(false);
    };
    if &current != expected {
        return Ok(false);
    }
    let _apartment = ComApartment::sta()?;
    unsafe { OleSetClipboard(None::<&IDataObject>) }.context("清除文件剪贴板失败")?;
    Ok(true)
}

#[implement(IDropSource)]
struct FileDropSource;

impl IDropSource_Impl for FileDropSource_Impl {
    fn QueryContinueDrag(
        &self,
        escape_pressed: windows::core::BOOL,
        key_state: MODIFIERKEYS_FLAGS,
    ) -> windows::core::HRESULT {
        if escape_pressed.as_bool() {
            DRAGDROP_S_CANCEL
        } else if key_state.0 & 0x0001 == 0 {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }

    fn GiveFeedback(&self, _effect: DROPEFFECT) -> windows::core::HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

/// Start an OLE drag of filesystem paths. Must be called on the UI STA thread;
/// the Shell drop target determines the final effect.
pub fn start_drag(_owner: HWND, paths: &[PathBuf]) -> Result<DragOutcome> {
    let _apartment = ComApartment::sta()?;
    let data = shell_data_object(paths)?;
    set_preferred_effect(&data, ClipboardEffect::Copy)?;
    set_mochi_clipboard_marker(&data)?;
    let source: IDropSource = FileDropSource.into();
    let mut effect = DROPEFFECT(0);
    let hr = unsafe {
        DoDragDrop(
            &data,
            &source,
            DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK,
            &mut effect,
        )
    };
    if hr == DRAGDROP_S_CANCEL {
        return Ok(DragOutcome::Cancelled);
    }
    if hr.is_err() {
        return Err(anyhow!(windows::core::Error::from_hresult(hr)).context("Shell 拖放失败"));
    }
    let effect = if effect.0 & DROPEFFECT_MOVE.0 != 0 {
        DragEffect::Move
    } else if effect.0 & DROPEFFECT_COPY.0 != 0 {
        DragEffect::Copy
    } else if effect.0 & DROPEFFECT_LINK.0 != 0 {
        DragEffect::Link
    } else {
        DragEffect::None
    };
    Ok(DragOutcome::Dropped(effect))
}

/// Shell-copy one source to an exclusive staging path supplied by the core plan.
/// The caller must validate the plan before this call and validate the copied
/// manifest before publishing the staged item. Any partial stage is left for the
/// staging-root cleanup path rather than being mistaken for a completed transfer.
pub fn copy_to_staging(owner: HWND, source: &Path, staged_target: &Path) -> Result<()> {
    let source = source
        .canonicalize()
        .with_context(|| format!("传输来源不存在: {}", source.display()))?;
    let parent = staged_target
        .parent()
        .ok_or_else(|| anyhow!("暂存目标没有父文件夹"))?
        .canonicalize()
        .with_context(|| format!("暂存文件夹不存在: {}", staged_target.display()))?;
    let name = staged_target
        .file_name()
        .ok_or_else(|| anyhow!("暂存目标名称无效"))?;
    let name_wide = name
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let target = parent.join(name);
    ensure!(target != source, "传输来源和暂存目标相同");
    ensure!(!target.exists(), "暂存目标已存在: {}", target.display());

    let _apartment = ComApartment::sta()?;
    let shell_source = normalized_shell_path(&source);
    let shell_parent = normalized_shell_path(&parent);
    let _source_item: IShellItem =
        unsafe {
            SHCreateItemFromParsingName::<
                _,
                Option<&windows::Win32::System::Com::IBindCtx>,
                IShellItem,
            >(
                &windows::core::HSTRING::from(shell_source.to_string_lossy().as_ref()),
                None,
            )
        }
        .with_context(|| format!("打开传输来源失败: {}", source.display()))?;
    let destination: IShellItem =
        unsafe {
            SHCreateItemFromParsingName::<
                _,
                Option<&windows::Win32::System::Com::IBindCtx>,
                IShellItem,
            >(
                &windows::core::HSTRING::from(shell_parent.to_string_lossy().as_ref()),
                None,
            )
        }
        .with_context(|| format!("打开暂存文件夹失败: {}", parent.display()))?;
    let operation: IFileOperation =
        unsafe { CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER) }
            .context("创建 Shell 文件操作失败")?;
    unsafe {
        operation.SetOwnerWindow(owner)?;
        operation.SetOperationFlags(FOFX_EARLYFAILURE)?;
        operation.CopyItem(
            &_source_item,
            &destination,
            PCWSTR(name_wide.as_ptr()),
            None::<&IFileOperationProgressSink>,
        )?;
        operation.PerformOperations()?;
        if operation.GetAnyOperationsAborted()?.as_bool() {
            return Err(anyhow!(windows::core::Error::new(
                E_ABORT,
                "Shell 复制已取消"
            )));
        }
    }
    ensure!(
        target.exists(),
        "Shell 未在预期暂存路径创建项目: {}",
        target.display()
    );
    let source_is_dir = std::fs::metadata(&source)?.is_dir();
    let target_is_dir = std::fs::metadata(&target)?.is_dir();
    ensure!(source_is_dir == target_is_dir, "暂存项目类型与来源不一致");
    Ok(())
}

/// Recycle a path using the existing guarded IFileOperation implementation.
/// Never falls back to permanent deletion.
pub fn move_to_trash(owner: HWND, path: &Path) -> Result<()> {
    ensure!(path.exists(), "要回收的项目不存在: {}", path.display());
    crate::platform::move_to_trash(owner, path).context("移到回收站失败")
}

/// Execute a validated folder transfer item by item. Copy/move both stage and
/// verify the complete tree before the core atomically publishes it. For Move,
/// the original source is recycled only after its manifest is revalidated.
pub fn execute_transfer(
    owner: HWND,
    config: &mochi_core::desktop_cards::folder::FolderConfig,
    plan: &mochi_core::desktop_cards::folder_operations::TransferPlan,
) -> Vec<mochi_core::desktop_cards::folder_operations::OperationResult> {
    use mochi_core::desktop_cards::folder_operations as files;

    let mut results = plan.failures.clone();
    if let Err(error) = files::prepare_transfer_staging(config, plan) {
        results.extend(plan.items.iter().map(|item| files::OperationResult {
            source: item.source.clone(),
            target: Some(item.target.clone()),
            succeeded: false,
            error: Some(format!("准备暂存目录失败：{error:#}")),
        }));
        return results;
    }

    for (index, item) in plan.items.iter().enumerate() {
        let attempt = (|| -> Result<()> {
            files::validate_transfer_item(config, plan, index)?;
            copy_to_staging(owner, &item.source, &item.staged_target)?;
            let published = files::publish_staged_item(config, plan, index)?;
            if plan.kind == files::TransferKind::Move {
                files::validate_source_manifest(config, plan, index)
                    .context("来源在复制期间发生变化，已保留原文件")?;
                move_to_trash(owner, &item.source).context("副本已发布，但原文件移到回收站失败")?;
                ensure!(
                    !item.source.exists(),
                    "副本已发布，但原文件仍存在；请检查回收站操作"
                );
            }
            debug_assert_eq!(published, item.target);
            Ok(())
        })();
        match attempt {
            Ok(()) => results.push(files::OperationResult {
                source: item.source.clone(),
                target: Some(item.target.clone()),
                succeeded: true,
                error: None,
            }),
            Err(error) => {
                let cancelled = is_shell_cancelled(&error);
                results.push(files::OperationResult {
                    source: item.source.clone(),
                    target: Some(item.target.clone()),
                    succeeded: false,
                    error: Some(format!("{error:#}")),
                });
                if cancelled {
                    results.extend(plan.items.iter().skip(index + 1).map(|pending| {
                        files::OperationResult {
                            source: pending.source.clone(),
                            target: Some(pending.target.clone()),
                            succeeded: false,
                            error: Some("已取消，未开始此项目".into()),
                        }
                    }));
                    break;
                }
            }
        }
    }

    if let Err(error) = files::cleanup_transfer_staging(config, plan) {
        results.push(files::OperationResult {
            source: plan.staging_root.clone(),
            target: Some(plan.staging_root.clone()),
            succeeded: false,
            error: Some(format!("暂存目录仍保留，未清理其内容：{error:#}")),
        });
    }
    results
}

pub fn is_shell_cancelled(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<windows::core::Error>()
            .is_some_and(|error| error.code() == E_ABORT || error.code().0 == 0x8007_04C7u32 as i32)
    })
}

/// Reveal a local item in Explorer using the existing PIDL-based implementation.
pub fn reveal(path: &Path) {
    crate::platform::show_in_explorer(path);
}

/// Optional QuickLook integration is deliberately best effort and never starts
/// or installs the third-party preview application.
pub fn preview_if_quicklook_running(path: &Path) -> Result<bool> {
    let Some(executable) = running_quicklook_executable()? else {
        return Ok(false);
    };
    let path = path
        .canonicalize()
        .with_context(|| format!("预览目标不存在或无法访问: {}", path.display()))?;
    std::process::Command::new(executable)
        .arg(path)
        .spawn()
        .context("向已运行的 QuickLook 请求预览失败")?;
    Ok(true)
}

fn running_quicklook_executable() -> Result<Option<PathBuf>> {
    struct Snapshot(HANDLE);
    impl Drop for Snapshot {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    let snapshot = Snapshot(
        unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
            .context("枚举 QuickLook 进程失败")?,
    );
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    if unsafe { Process32FirstW(snapshot.0, &mut entry) }.is_err() {
        return Ok(None);
    }
    loop {
        let end = entry
            .szExeFile
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..end]);
        if name.eq_ignore_ascii_case("QuickLook.exe") {
            let process = unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION,
                    false,
                    entry.th32ProcessID,
                )
            };
            if let Ok(process) = process {
                struct Process(HANDLE);
                impl Drop for Process {
                    fn drop(&mut self) {
                        unsafe {
                            let _ = CloseHandle(self.0);
                        }
                    }
                }
                let process = Process(process);
                let mut buffer = vec![0u16; 32_768];
                let mut length = buffer.len() as u32;
                if unsafe {
                    QueryFullProcessImageNameW(
                        process.0,
                        PROCESS_NAME_WIN32,
                        windows::core::PWSTR(buffer.as_mut_ptr()),
                        &mut length,
                    )
                }
                .is_ok()
                {
                    buffer.truncate(length as usize);
                    let executable = PathBuf::from(OsString::from_wide(&buffer));
                    if executable.is_file() {
                        return Ok(Some(executable));
                    }
                }
            }
        }
        if unsafe { Process32NextW(snapshot.0, &mut entry) }.is_err() {
            break;
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mochi_core::desktop_cards::{
        folder::FolderConfig,
        folder_operations::{self, TransferKind, TransferLimits},
    };

    struct TemporaryDirectory(PathBuf);

    impl TemporaryDirectory {
        fn create() -> Self {
            let path = std::env::temp_dir().join(format!(
                "mochi-shell-transfer-smoke-{}-{}",
                std::process::id(),
                mochi_core::paths::random_base36(10)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TemporaryDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn path_limits_count_utf16_units_and_preserve_unicode_paths() {
        let root = std::env::temp_dir().join(format!(
            "mochi-desktop-files-paths-{}-{}",
            std::process::id(),
            mochi_core::paths::random_base36(8)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("会议记录 😀.txt");
        std::fs::write(&file, "temporary test data").unwrap();
        let paths = checked_paths(std::slice::from_ref(&file)).unwrap();
        assert_eq!(paths[0], file.canonicalize().unwrap());
        assert!(checked_paths(&[]).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn preferred_effect_mapping_uses_shell_copy_and_move_bits() {
        assert_eq!(DROPEFFECT_COPY.0, 1);
        assert_eq!(DROPEFFECT_MOVE.0, 2);
        assert_eq!(DROPEFFECT_LINK.0, 4);
        assert_ne!(DRAGDROP_S_CANCEL, DRAGDROP_S_DROP);
    }

    #[test]
    fn shell_path_normalization_handles_drive_and_unc_without_rewriting_other_namespaces() {
        for (input, expected) in [
            (r"\\?\C:\会议\资料.txt", r"C:\会议\资料.txt"),
            (r"\\?\unc\server\share\资料", r"\\server\share\资料"),
            (
                r"\\?\Volume{01234567-89ab-cdef-0123-456789abcdef}\资料",
                r"\\?\Volume{01234567-89ab-cdef-0123-456789abcdef}\资料",
            ),
            (r"C:\普通路径", r"C:\普通路径"),
        ] {
            assert_eq!(
                normalized_shell_path(Path::new(input)),
                OsString::from(expected),
                "input={input}"
            );
        }
    }

    #[test]
    #[ignore = "runs real Shell copy and recycle operations on temporary test files"]
    fn i_file_operation_copy_then_move_smoke_uses_only_temporary_items() {
        use windows::Win32::System::Com::{
            CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED,
        };

        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
            .ok()
            .expect("initialize the test STA");
        struct Apartment(bool);
        impl Drop for Apartment {
            fn drop(&mut self) {
                if self.0 {
                    unsafe { CoUninitialize() };
                }
            }
        }
        let _apartment = Apartment(true);

        let temporary = TemporaryDirectory::create();
        let destination = temporary.0.join("destination");
        std::fs::create_dir(&destination).unwrap();
        let config = FolderConfig {
            path: destination.to_string_lossy().into_owned(),
            ..Default::default()
        };
        let owner = HWND::default();

        let copy_source = temporary.0.join("copy source 😀.txt");
        let copy_bytes = b"copy smoke payload";
        std::fs::write(&copy_source, copy_bytes).unwrap();
        let copy_plan = folder_operations::plan_transfer(
            &config,
            std::slice::from_ref(&copy_source),
            TransferKind::Copy,
            TransferLimits::default(),
        )
        .unwrap();
        let copied = execute_transfer(owner, &config, &copy_plan);
        assert_eq!(copied.len(), 1, "{copied:?}");
        assert!(copied[0].succeeded, "{copied:?}");
        assert!(copy_source.exists(), "copy must preserve its source");
        assert_eq!(
            std::fs::read(destination.join(copy_source.file_name().unwrap())).unwrap(),
            copy_bytes
        );

        let move_source = temporary.0.join("move source.txt");
        let move_bytes = b"move smoke payload";
        std::fs::write(&move_source, move_bytes).unwrap();
        let move_plan = folder_operations::plan_transfer(
            &config,
            std::slice::from_ref(&move_source),
            TransferKind::Move,
            TransferLimits::default(),
        )
        .unwrap();
        let moved = execute_transfer(owner, &config, &move_plan);
        assert_eq!(moved.len(), 1, "{moved:?}");
        assert!(moved[0].succeeded, "{moved:?}");
        assert!(
            !move_source.exists(),
            "move must recycle the temporary source"
        );
        assert_eq!(
            std::fs::read(destination.join(move_source.file_name().unwrap())).unwrap(),
            move_bytes
        );
    }
}

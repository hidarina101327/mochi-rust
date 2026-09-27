//! 定义应用共享状态、拖动状态及侧边栏树拖放数据。
use super::*;

/// 一次进行中的面板宽度拖动。
///
/// 记的是**指针与面板边缘的偏移**而不是起始宽度：按下手柄中间再拖，
/// 面板宽度不该先跳几个像素去对齐指针。
#[derive(Debug, Clone, Copy)]
pub(super) struct Drag {
    pub(super) target: DragTarget,
    /// 按下时指针相对面板边缘的偏移（DIP）。
    pub(super) grab_offset: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DragTarget {
    EditorImage,
    EditorTable,
    EditorTableColumn,
    Split,
    Navigation,
    Sidebar,
    SettingsNav,
    AiPanel,
    /// 按住左键在编辑器里拖动：扩展选区。
    EditorSelect,
    ProviderFieldSelect,
    SettingsFieldSelect,
    AiTextSelect,
    ObjectPickerQuery,
    DialogFieldSelect,
    /// 图片查看器里拖动平移。
    ViewerPan,
    CanvasCard,
    CanvasPan,
    PdfAnnotation,
    /// 文件树按下后等待越过阈值；松开时仍保留普通单击行为。
    SidebarTree,
    /// 日程时间轴里拖动时间块改期 / 改时长。
    ScheduleBlock,
}

pub(super) const TREE_DRAG_THRESHOLD: f32 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SidebarTreeDropKind {
    Before,
    After,
    Into,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SidebarTreeDrop {
    pub(super) path: PathBuf,
    pub(super) kind: SidebarTreeDropKind,
}

#[derive(Debug, Clone)]
pub(super) struct SidebarTreeDrag {
    pub(super) source_path: PathBuf,
    pub(super) source_is_dir: bool,
    pub(super) start_x: f32,
    pub(super) start_y: f32,
    pub(super) active: bool,
    pub(super) drop: Option<SidebarTreeDrop>,
}

/// 侧栏拖动区间。Sidebar.tsx 的 `handleMouseMove` 夹在 180..600。
pub(super) const SIDEBAR_MIN: f32 = 180.0;

pub(super) const SIDEBAR_MAX: f32 = 600.0;

// 设置覆盖层的最近位置属于应用设置，关闭窗口后跨进程启动也能恢复。
pub(super) const SETTINGS_LAST_TAB_KEY: &str = "settings.lastTab";

pub(super) const SETTINGS_LAST_SECTION_KEY: &str = "settings.lastSection";

pub(super) const SETTINGS_SCROLL_KEY: &str = "settings.scroll";

/// 点击时 Shift 是否按着（Shift+点击 = 扩展选区）。鼠标消息里没有这一位，只能问键盘状态。
pub(super) fn shift_down() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_SHIFT};
    unsafe { GetKeyState(VK_SHIFT.0 as i32) < 0 }
}

/// 从工具回执中提取 Electron 会挂到会话消息上的局部编辑提案。
///
/// 工具回执本身仍作为隐藏消息保存；这里仅识别稳定的
/// `data.pendingEdit`（以及 headless host 使用的 `pendingEdit`）形状，
/// 不把任意工具输出当成可执行文件操作。
pub(super) fn pending_edits_from_transcript(transcript: &[AiMessage]) -> Vec<serde_json::Value> {
    transcript
        .iter()
        .filter(|message| message.role == "tool")
        .filter_map(|message| message.content.as_deref())
        .filter_map(|content| serde_json::from_str::<serde_json::Value>(content).ok())
        .filter_map(|value| {
            value
                .get("data")
                .and_then(|data| data.get("pendingEdit"))
                .or_else(|| value.get("pendingEdit"))
                .filter(|edit| edit.is_object())
                .cloned()
        })
        .collect()
}

/// 文件树右键菜单里的动作。对应 `ContextMenu.tsx` 的各个 `onSelect`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MenuAction {
    AiSession(String, ai_sessions::SessionAction),
    ToggleFavorite(PathBuf),
    AiMessage(String, String, assistant::MessageAction),
    MountAiMessage(String, String, PathBuf),
    PickAiMessageDocuments(String, String),
    SelectAgent(String),
    SetAgentFollowUpFrequency(mochi_core::ai::FollowUpFrequency),
    OpenAgentDefinitions,
    MountAiSession(PathBuf),
    PickAiMountDocument,
    LibraryScope(Option<usize>, String),
    HideNavigation(NavItem),
    HideTitleBarEntry(&'static str),
    CloseTabs(usize, &'static str),
    ExportDocument(PathBuf, String),
    FileIcon(PathBuf),
    SetFileIcon(PathBuf, Option<String>),
    SortFiles,
    SetSort(String),
    ImportFiles(PathBuf),
    ExportFile(PathBuf),
    SaveAsTemplate(PathBuf),
    NewSubdocument(PathBuf),
    Properties(PathBuf),
    AiPermission(PathBuf),
    SetAiPermission(PathBuf, String),
    FileToAssistant(PathBuf),
    BaseCellToAssistant(PathBuf, base_view::AiCellContext),
    BaseCellInsert(usize, usize, base_view::InsertDirection),
    SelectionAi(String, bool),
    SelectionToAssistant,
    SplitRight(PathBuf),
    PinTab(usize),
    CloseTab(usize),
    NewLink(PathBuf),
    NewBase(PathBuf),
    NewCanvas(PathBuf),
    AgendaPick(agenda::panel::PickSlot, Option<String>),
    CodeLanguage(usize, String),
    NewFile(PathBuf),
    NewFolder(PathBuf),
    NewMappedFolder(PathBuf),
    DuplicateFile(PathBuf),
    RenamePath(PathBuf),
    DeletePath(PathBuf),
    Refresh,
    RefreshTab(PathBuf),
    Rename(usize),
    Delete(usize),
    CopyPath(PathBuf),
    CopyRelativePath(PathBuf),
    CopyMochiUrl(PathBuf, bool),
    ObjectPresentation(usize, bool),
    CopyObjectLink(String),
    InsertObject,
    ShowInExplorer(PathBuf),
    ShowInFileTree(PathBuf),
    OpenTab(PathBuf),
    /// 设置页下拉：把第 `0` 项描述符设成 `1`。
    SetEnum(usize, String),
    /// 收件箱：归档目标库。
    PickInboxLibrary(usize),
    /// 工具栏下拉（段落格式 / 插入）选中的格式化操作。
    Format(Format),
    /// 服务方设置中的模型选择器；服务方 ID 可防止表单切换到其他服务方后，
    /// 迟到的结果被错误采用。
    SelectProviderModel(String, String),
    /// 服务方设置中的协议选择器；服务方 ID 可防止表单切换或关闭后，
    /// 菜单项仍被触发。
    SelectProviderProtocol(String, String),
    SelectProviderPreset(String, Option<usize>),
    WorkflowProvider(String, Option<String>),
    WorkflowReference(String, String, String),
    WorkflowAction(crate::ui::workflows::Hit),
    InsertMath,
    Container(usize, crate::ui::containers::Action),
    Table(usize, table_edit::Action),
    ImageWidth(usize),
    PreviewEditorImage(usize),
    CopyEditorImage(usize),
    CommentSelection,
    BlockParagraphMenu,
    BlockFormatMenu,
    BlockAiMenu,
    BlockAlign(Align),
    BlockIndent(bool),
    BlockColorMenu(bool),
    Block(usize, bool),
    EditorCopy,
    EditorCut,
    EditorPaste,
    EditorPastePlain,
    EditorSelectAll,
    TextColor(bool, u32),
    SetColorSetting(usize, String),
}

/// 工具栏上打开着的下拉。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ToolbarMenu {
    Heading,
    Insert,
}

/// 模态对话框的动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DialogAction {
    DesktopFiles {
        card: String,
        page: String,
        command: crate::desktop_window::folder::Command,
        paths: Vec<String>,
        move_files: bool,
    },
    ImportPackage {
        workspace: PathBuf,
        package: Box<mochi_core::transfer::Package>,
        remaining: Vec<PathBuf>,
    },
    RememberFolderDrop,
    ImportDropped {
        paths: Vec<PathBuf>,
        target: PathBuf,
        mapping: bool,
    },
    WorkflowApprove {
        id: String,
        revision: i64,
        enable: bool,
        run: bool,
    },
    WorkflowDiscard,
    WorkflowDiscardEditor,
    WorkflowDelete,
    /// 原 Electron 快捷导航的原生表单入口。内容保存在插件兼容的 data.json。
    QuickNavAddItem,
    QuickNavAddGroup,
    QuickNavEditItem(String),
    QuickNavRenameGroup(String),
    QuickNavDeleteGroup(String),
    QuickNavSetBrowser,
    EnglishAddWord,
    EnglishAddArticle,
    EnglishAddSentence,
    EnglishRemoveDictionary(String),
    EnglishGradeDictation {
        sentence_id: i64,
    },
    RestoreBlockDocument(PathBuf, String),
    RenameFile(PathBuf),
    RenameAiSession(String),
    NewAiSessionProject(String),
    ClearAiSession(String),
    ReplaceAiDraft {
        session_id: String,
        text: String,
    },
    DeleteAiRound,
    ConflictReload(PathBuf),
    ConflictOverwrite(PathBuf),
    ConflictCopy(PathBuf),
    SetShortcut(&'static str),
    EnableAutomaticAiEdits,
    FileIcon(PathBuf),
    CreateSubdocument(PathBuf),
    CreateTemplate(String),
    CreateTemplateGroup,
    ExamText {
        path: PathBuf,
        block: usize,
        question: Option<usize>,
        note: bool,
    },
    CreateBase(PathBuf),
    AutomationText(base_automation::TextEdit),
    BaseEdit {
        path: PathBuf,
        edit: base_view::Edit,
        original: String,
    },
    DeleteProvider(String),
    DeleteAiSession(String),
    CodeTitle(usize),
    ContainerTitle {
        path: PathBuf,
        start: usize,
        original: String,
    },
    PickEditorImage,
    ImageWidth {
        path: PathBuf,
        start: usize,
        original: String,
    },
    InsertEditorLink {
        image: bool,
        path: PathBuf,
        range: (usize, usize),
        original: String,
        label: String,
    },
    EditEditorMath {
        path: PathBuf,
        range: (usize, usize),
        original: String,
        display: bool,
    },
    DownloadAppUpdate,
    SkipAppUpdate,
    DismissAppUpdate,
    Dismiss,
    /// 新建文件/文件夹对话框（空白处右键用的是对话框，目录上右键用内联编辑）。
    CreateInDialog {
        parent: PathBuf,
        folder: bool,
    },
    DeleteToTrash(PathBuf),
    DeletePermanently(PathBuf),
    /// 版本历史：把文件恢复到某次提交（先自动创建恢复前快照，`restore_file` 里做）。
    RestoreVersion {
        path: PathBuf,
        oid: String,
    },
    /// 日程：删除一条任务/日程。
    ReviewScheduleDiff {
        raw: serde_json::Value,
        approve: bool,
    },
    ReviewConsoleAction {
        raw: serde_json::Value,
        approve: bool,
    },
    PdfText,
    PdfPage,
}

#[derive(Debug, Clone)]
pub(super) enum QuickNavHit {
    Select(String),
    Open(String),
    RunAsAdmin(String),
    ShowInFolder(String),
    ToggleFavorite(String),
    Remove(String),
    MoveItem(String, i32),
    MoveGroup(String, i32),
    EditItem(String),
    MoveItemToSelectedGroup(String),
    RenameGroup(String),
    DeleteGroup(String),
    MoveGroupToRoot(String),
    ConfigureBrowser,
    ImportItem,
    SetSort(String),
    SetLayout(String),
    SetGridColumns(i8),
    SetGridRows(i8),
    ToggleField(String),
    AddItem,
    AddGroup,
}

#[derive(Debug, Clone)]
pub(super) enum EnglishLabHit {
    AddWord,
    AddArticle,
    AddSentence,
    OpenArticle(i64),
    CloseArticle,
    ImportDictionary,
    RemoveDictionary(String),
    Listen(String),
    Dictate(i64),
    Grade(i64, u8),
    PracticeGroup(i64),
    Page(EnglishLabPage),
}

/// Electron English Lab 的工作区路由。原生版保持在同一个插件视图内切换，
/// 不创建知识库标签，也不会把插件点击带到设置页。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EnglishLabPage {
    Dashboard,
    Practice,
    Listening,
    Reading,
    Library,
    Dictionaries,
    Stats,
    History,
    Settings,
}

/// 「删除的版本」只是隐藏，记在设置里（TSX 存 localStorage，键按路径小写）。
pub(super) fn deleted_versions_key(path: &std::path::Path) -> String {
    format!(
        "mochi:deleted-versions:{}",
        path.to_string_lossy().replace('\\', "/").to_lowercase()
    )
}

pub(super) fn update_notes_for_dialog(notes: &str) -> String {
    let notes = notes.trim().replace("\r\n", "\n");
    if notes.is_empty() {
        "此版本没有填写更新说明。".into()
    } else {
        notes
    }
}

/// 启动独立于应用进程的更新助手。它只接收已通过 SHA-256 校验的本地安装包路径，
/// 等待本进程退出后才执行安装，因此不会尝试覆盖仍被 Windows 锁定的 exe。
pub(super) fn launch_installer_after_exit(installer: &Path) -> std::result::Result<(), String> {
    if !installer.is_file() {
        return Err("已下载的安装包不存在".into());
    }
    let helper = installer.with_file_name(format!(
        "apply-update-{}-{}.ps1",
        std::process::id(),
        chrono::Local::now().timestamp_millis()
    ));
    let script = r#"
param(
  [Parameter(Mandatory = $true)][int]$TargetPid,
  [Parameter(Mandatory = $true)][string]$InstallerPath
)
try {
  while (Get-Process -Id $TargetPid -ErrorAction SilentlyContinue) {
    Start-Sleep -Milliseconds 200
  }
  $ErrorActionPreference = 'Stop'
  $log = $InstallerPath + '.install.log'
  $installArgs = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/MOCHIRESTART=1', ('/LOG="' + $log + '"'))
  $process = Start-Process -FilePath $InstallerPath -ArgumentList $installArgs -WindowStyle Hidden -Wait -PassThru
  if ($process.ExitCode -ne 0) {
    throw "Mochi installer failed ($($process.ExitCode)); see $log"
  }
}
catch {
  $_ | Out-String | Set-Content -LiteralPath ($InstallerPath + '.error.txt') -Encoding UTF8
  Add-Type -AssemblyName System.Windows.Forms
  [System.Windows.Forms.MessageBox]::Show($_.ToString(), 'Mochi update failed') | Out-Null
}
finally {
  Remove-Item -LiteralPath $PSCommandPath -Force -ErrorAction SilentlyContinue
}
"#;
    std::fs::write(&helper, script).map_err(|error| format!("创建更新助手失败：{error}"))?;
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    match std::process::Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&helper)
        .arg("-TargetPid")
        .arg(std::process::id().to_string())
        .arg("-InstallerPath")
        .arg(installer)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
    {
        Ok(_) => Ok(()),
        Err(error) => {
            let _ = std::fs::remove_file(&helper);
            Err(error.to_string())
        }
    }
}

/// 评论 id 用 `crypto.randomUUID()` 的形状。没有 uuid 依赖，用时间戳 + 随机数拼一个
/// RFC 4122 v4 形状的字串——只要求唯一与同形，不要求密码学随机。
pub(super) fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut x = nanos as u64 ^ 0x9E37_79B9_7F4A_7C15 ^ (std::process::id() as u64).rotate_left(32);
    let mut next = || {
        // xorshift64*
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    let a = next();
    let b = next();
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        (a >> 32) as u32,
        (a >> 16) as u16,
        (a & 0xFFF) as u16,
        ((b >> 48) as u16 & 0x3FFF) | 0x8000,
        b & 0xFFFF_FFFF_FFFF
    )
}

/// 点击标题栏按钮后要让窗口做的事。`App` 不该自己调 `ShowWindow`——
/// 那是窗口过程的职责，这里只把意图传出去。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptionAction {
    Minimize,
    ToggleMaximize,
    Close,
}

/// 键盘输入此刻送到哪。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Focus {
    CanvasText,
    Workflow,
    DocumentTitle,
    /// 主区（源码编辑面）。
    Main,
    /// 导航轨里的新建库输入框。
    NavNewLibrary,
    /// 侧栏顶部的搜索框。
    SidebarSearch,
    /// 侧栏里的内联编辑框（新建/重命名）。
    SidebarEditor,
    /// 模态对话框（它的输入框，或只是拦截 Esc/Enter）。
    Dialog,
    /// 全局搜索覆盖层的输入框。
    Search,
    SettingsField,
    SettingsSearch,
    /// 版本历史面板的提交信息框。
    VersionMessage,
    /// 评论面板的撰写框。
    CommentCompose,
    /// AI 助手的输入框。
    AiInput,
    AiMessageQuery,
    AiSessionQuery,
    ProviderField,
    TableCell,
    /// 日程的快速添加框 / 搜索框。
    ScheduleQuick,
    ScheduleQuery,
    ScheduleForm,
    /// 收件箱里正在编辑的条目。
    InboxEdit,
    /// 最近视图的搜索框。
    RecentQuery,
    /// 文档查找条的查找框。
    FindQuery,
    /// 文档查找条的替换框。
    FindReplacement,
    /// 命令面板 / 快速打开的输入框。
    Command,
    /// AI 定义独享视图中的源码编辑器。
    AgentSource,
}

/// 拖动悬浮窗：移动记指针相对左上角的偏移，缩放记指针相对右下角的偏移。
#[derive(Debug, Clone, Copy)]
pub(super) enum AiFloatDrag {
    Move { dx: f32, dy: f32 },
    Resize { right: f32, bottom: f32 },
}

/// 墨池窗口内的 AI 助手悬浮窗。内容和右侧栏是同一份面板，只是画在一块更小的区域里。
#[derive(Debug, Clone, Copy)]
pub(super) struct AiFloat {
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) w: f32,
    pub(super) h: f32,
    pub(super) drag: Option<AiFloatDrag>,
}

impl AiFloat {
    pub(super) fn rect(self) -> Rect {
        Rect::from_size(self.x, self.y, self.w, self.h)
    }

    pub(super) fn clamp_into(&mut self, view: Rect) {
        let min_w = 320.0_f32.min(view.width().max(1.0));
        let min_h = 420.0_f32.min(view.height().max(1.0));
        self.w = self.w.clamp(min_w, view.width().max(min_w));
        self.h = self.h.clamp(min_h, view.height().max(min_h));
        self.x = self
            .x
            .clamp(view.left, (view.right - self.w).max(view.left));
        self.y = self.y.clamp(view.top, (view.bottom - self.h).max(view.top));
    }
}

/// AI 助手的运行期状态（面板状态在 `assistant::State`）。
#[derive(Default)]
pub(super) struct AiState {
    pub(super) pending_memory: Option<crate::memory_runtime::Exchange>,
    pub(super) memory_jobs: Vec<crate::memory_runtime::Job>,
    pub(super) follow_up_jobs: Vec<crate::follow_up_runtime::FollowUpJob>,
    pub(super) title_jobs: Vec<super::ai_titles::Job>,
    pub(super) export_requests: Option<Arc<crate::export_requests::Queue>>,
    pub(super) workspace: ai_workspace::State,
    pub(super) workspace_side: ai_workspace::Layout,
    pub(super) panel: assistant::State,
    pub(super) layout: assistant::Layout,
    /// 窗口内悬浮的 AI 助手。打开时右侧栏收起，内容和侧栏是同一份。
    pub(super) float: Option<AiFloat>,
    pub(super) run: Option<RunHandle>,
    pub(super) animation_timer_armed: bool,
    /// 后台线程共享的宿主快照。
    pub(super) snapshot: Arc<Mutex<HostSnapshot>>,
    pub(super) host: Option<Arc<AppHost>>,
    /// 权限服务与 `host` 共享同一份；单独留一个句柄给将来的「AI 权限设置」面板用。
    #[allow(dead_code)]
    pub(super) permissions: Option<Arc<AiPermissionService>>,
}

/// 一次即将落盘的文本文件保存，用于把保存前后的字数写入活动日志。
#[derive(Debug, Clone)]
pub(super) struct TextSaveActivity {
    pub(super) path: PathBuf,
    pub(super) before_words: usize,
    pub(super) after_words: usize,
}

/// Agent 配置页的状态与上一帧布局。
#[derive(Default)]
pub(super) struct AgentConfigState {
    pub(super) updates: Vec<mochi_core::ai::agent_config::updates::AgentUpdate>,
    pub(super) update_review: Option<mochi_core::ai::agent_config::updates::AgentUpdate>,
    pub(super) update_diff: String,
    pub(super) update_upstream: bool,
    pub(super) update_banner: Rect,
    pub(super) mcp_rx: Option<Receiver<(String, std::result::Result<usize, String>)>>,
    pub(super) data: agent_config::Data,
    pub(super) loaded: bool,
    pub(super) error: Option<String>,
    pub(super) scroll: f32,
    pub(super) hover: Option<agent_config::Hit>,
    pub(super) layout: agent_config::Layout,
    pub(super) nav_layout: agent_config::NavLayout,
    pub(super) nav_scroll: f32,
    pub(super) nav_hover: Option<agent_config::NavHit>,
    // Agent 配置分区独立于文档标签页。
    pub(super) section: usize,
    pub(super) source_editor: Option<AgentSourceEditor>,
    pub(super) source_drafts: HashMap<PathBuf, AgentSourceEditor>,
}

pub(super) struct AgentSourceEditor {
    pub(super) path: PathBuf,
    pub(super) field: TextField,
    pub(super) area: Rect,
    pub(super) dirty: bool,
}

/// 日程待办页的状态与上一帧布局（主区 + 左侧栏）。
#[derive(Default)]
pub(super) struct ScheduleState {
    pub(super) timer_armed: bool,
    pub(super) reminder_workspace: Option<PathBuf>,
    pub(super) reminder_cursor: Option<chrono::NaiveDateTime>,
    pub(super) delivered: HashSet<String>,
    /// 数据属于哪个工作区；换工作区时清空撤销栈和视图状态。
    pub(super) workspace: Option<PathBuf>,
    /// 上次读取时的文件指纹，用来发现别处的修改。
    pub(super) stamp: Option<mochi_core::agenda::store::Stamp>,
    /// 用户修改产生的精确变更集；Ctrl+Z / Ctrl+Y 在两个栈之间移动。
    pub(super) undo: Vec<mochi_core::agenda::ChangeSet>,
    pub(super) redo: Vec<mochi_core::agenda::ChangeSet>,
    pub(super) view: agenda::State,
    pub(super) layout: agenda::Layout,
    pub(super) side_layout: agenda::side::SideLayout,
}

/// 收件箱 / 最近 两个独立视图的状态与上一帧布局。
#[derive(Default)]
pub(super) struct ViewsState {
    pub(super) favorites: views::favorites::State,
    pub(super) inbox: views::inbox::State,
    pub(super) inbox_layout: views::inbox::Layout,
    pub(super) recent: views::recent::State,
    pub(super) recent_layout: views::recent::Layout,
    pub(super) templates: crate::ui::templates::State,
    pub(super) templates_layout: crate::ui::templates::Layout,
}

/// 右侧栏五个数据面板的状态与上一帧布局。
#[derive(Default)]
pub(super) struct PanelsState {
    pub(super) annotation_scroll: f32,
    pub(super) annotation_layout: pdf_annotations::Layout,
    pub(super) version: version::State,
    pub(super) version_layout: version::Layout,
    pub(super) version_rx: Option<
        Receiver<(
            Option<PathBuf>,
            std::result::Result<Vec<mochi_core::git::GitCommitInfo>, String>,
        )>,
    >,
    pub(super) pomodoro: pomodoro::State,
    pub(super) pomodoro_layout: Vec<(Rect, pomodoro::Hit)>,
    pub(super) comments: comments::State,
    pub(super) comments_layout: comments::Layout,
    pub(super) mounts: mounts::State,
    pub(super) mounts_layout: mounts::Layout,
    pub(super) inbox: inbox::State,
    pub(super) inbox_layout: inbox::Layout,
    /// 番茄钟计时器是否已在跑（避免重复 SetTimer）。
    pub(super) pomodoro_timer_armed: bool,
}

pub(super) struct SettingsState {
    pub(super) providers: providers::State,
    pub(super) provider_layout: providers::Layout,
    /// 连接测试结果附带代数编号。后续点击所对应的状态优先，
    /// 并会忽略过期工作线程的结果。
    pub(super) provider_rx: Option<Receiver<(u64, bool, Option<String>)>>,
    pub(super) provider_generation: u64,
    pub(super) provider_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// 模型选择器要使用服务方真实返回的 `/models` 数据。
    pub(super) provider_models_rx: Option<
        Receiver<(
            u64,
            String,
            String,
            std::result::Result<Vec<String>, String>,
        )>,
    >,
    pub(super) provider_models_generation: u64,
    pub(super) provider_models_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub(super) scroll: f32,
    pub(super) search: TextField,
    pub(super) editing: Option<EditingField>,
    pub(super) nav_layout: SettingsNavLayout,
    pub(super) nav_scroll: f32,
    pub(super) content_layout: SettingsContentLayout,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            providers: providers::State::default(),
            provider_layout: providers::Layout::default(),
            provider_rx: None,
            provider_generation: 0,
            provider_cancel: None,
            provider_models_rx: None,
            provider_models_generation: 0,
            provider_models_cancel: None,
            scroll: 0.0,
            search: TextField::new("搜索设置..."),
            editing: None,
            nav_layout: SettingsNavLayout::default(),
            nav_scroll: 0.0,
            content_layout: SettingsContentLayout::default(),
        }
    }
}

/// 后台搜索：每次发出去带一个代号，晚到的旧结果直接丢——
/// 否则快速打字时先前那次慢查询会把新结果覆盖掉。
pub(super) struct SearchJob {
    pub(super) generation: u64,
    pub(super) rx: Option<Receiver<(u64, SearchQueryResult)>>,
    pub(super) cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// 查询词变了、等去抖计时器到点再搜。
    pub(super) timer_pending: bool,
}

impl SearchJob {
    pub(super) fn invalidate(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.generation = self.generation.wrapping_add(1);
        self.rx = None;
    }
}

impl Drop for SearchJob {
    fn drop(&mut self) {
        self.invalidate();
    }
}

/// 打开工作区后的全量搜索索引。任务只持有索引服务，结果以 generation 标记，
/// 用户快速切换工作区时旧任务的 UI 消息不会污染新工作区。
pub(super) enum WorkspaceIndexEvent {
    Progress(mochi_core::metadata_index::FullIndexProgress),
    Finished(std::result::Result<usize, String>),
}

/// 侧栏自己的状态（Electron 里是 `Sidebar` 的若干 `useState`）。
pub(super) struct SidebarState {
    pub(super) scroll: f32,
    pub(super) search: TextField,
    pub(super) hover: Option<usize>,
    pub(super) editing: Option<Editing>,
    pub(super) layout: SidebarLayout,
    pub(super) tree_drag: Option<SidebarTreeDrag>,
}

impl Default for SidebarState {
    fn default() -> Self {
        let mut search = TextField::new("搜索文件...");
        search.style = TextStyle::Label;
        SidebarState {
            scroll: 0.0,
            search,
            hover: None,
            editing: None,
            layout: SidebarLayout::default(),
            tree_drag: None,
        }
    }
}

/// 导航轨自己的状态（Electron 里是 `GlobalNavigation` 的 `useState`）。
pub(super) struct NavState {
    pub(super) unread_root: Option<PathBuf>,
    pub(super) unread: mochi_core::document_unread::Snapshot,
    pub(super) library_drag: Option<navigation_drag::LibraryDrag>,
    pub(super) drag_timer: Option<u32>,
    /// 最近一次绘制的导航列表中各行的稳定 ID。
    pub(super) painted_libraries: Vec<String>,
    /// 展开的库类型。默认只展开知识库。
    pub(super) expanded_types: HashSet<String>,
    pub(super) creating: Option<CreatingLibrary>,
    pub(super) scroll: f32,
    pub(super) inbox_count: usize,
}

pub(super) struct CreatingLibrary {
    pub(super) type_id: String,
    pub(super) field: TextField,
    pub(super) error: String,
}

impl Default for NavState {
    fn default() -> Self {
        NavState {
            unread_root: None,
            unread: Default::default(),
            library_drag: None,
            drag_timer: None,
            painted_libraries: Vec::new(),
            expanded_types: [navigation::DEFAULT_EXPANDED_TYPE.to_owned()]
                .into_iter()
                .collect(),
            creating: None,
            scroll: 0.0,
            inbox_count: 0,
        }
    }
}

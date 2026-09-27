//! 定义工作流编辑器状态、命中区域和画布数据。
mod chrome;
mod editing;
mod forms;
mod interaction;
mod library;
mod modern;
mod painting;
mod ports;
mod results;
use super::{layout::Rect, widgets::TextField};
pub use forms::parameter_choices;
use mochi_core::workflows::{
    Edge, Node, Run, RunSummary, SavedWorkflow, Workflow, WorkflowSummary,
};
pub use painting::paint;
pub use ports::{input_keys, node_height, output_keys};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    OpenArtifact(String),
    ResultOverview,
    ResultTab(u8),
    ClosePanel,
    NewFolder,
    Folder(Option<String>),
    FolderMenu(String),
    RenameFolder(String),
    DeleteFolder(String),
    WorkflowMenu(usize),
    MoveWorkflow(String, Option<String>),
    VariablePicker(String),
    SetVariable(String, String, String),
    InputVariable(usize, String),
    OutputVariable(usize, usize),
    Binding(usize, String),
    RemoveBinding(usize, String),
    Back,
    New,
    Import,
    Save,
    Run,
    RunOptions,
    Authorize,
    Pause,
    History,
    Export,
    Definition,
    Add,
    Undo,
    Redo,
    Fit,
    ZoomIn,
    ZoomOut,
    Open(usize),
    Node(usize),
    Port(usize, Option<bool>),
    Input(usize),
    Edge(usize),
    Kind(usize),
    Parameters,
    Apply,
    CloseEditor,
    Delete,
    PickObject,
    RunHistory(usize),
    CloseRun,
    CancelRun,
    Trigger,
    Rename,
    DeleteFlow,
    Sample(bool),
    CopyResult,
    ImportFile,
    ResultPage(bool),
    AutoLayout,
    Minimap,
    ResetZoom,
    Settings,
    Duplicate,
    Advanced,
    Parameter(String),
    PaletteSearch,
    Insert(usize),
    Map,
    Resize,
    Variables,
    More,
    Validate,
    Section(String),
    ConfigTab(bool),
    NodeMenu(usize),
    Choice(String, String),
}
#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Editor {
    Folder,
    Node,
    Definition,
    Import,
    Trigger,
    RunInput,
    Rename,
    Result,
    Parameter,
}
pub enum Drag {
    Variable(usize, usize),
    Node(usize, f32, f32, bool),
    Pan(f32, f32, f32, f32),
    Wire(usize, Option<bool>),
    Template(usize, f32, f32),
    Resize(f32, f32),
    Box(f32, f32),
}
pub struct State {
    pub panel_scroll: f32,
    pub panel_height: f32,
    pub panel_body: Rect,
    pub result_tab: u8,
    pub tooltips: Vec<(Rect, String)>,
    pub run_palette: bool,
    pub result_hidden: bool,
    pub folders: Vec<mochi_core::workflows::WorkflowFolder>,
    pub folder: Option<String>,
    pub editing_folder: Option<String>,
    pub library_height: f32,
    pub selected_binding: Option<(usize, String)>,
    pub workflows: Vec<WorkflowSummary>,
    pub selected: Option<SavedWorkflow>,
    pub draft: Option<Workflow>,
    pub dirty: bool,
    pub selected_node: Option<usize>,
    pub selected_edge: Option<usize>,
    pub editor: Option<Editor>,
    pub field: TextField,
    pub field_rect: Rect,
    pub history: Vec<RunSummary>,
    pub history_open: bool,
    pub run: Option<Run>,
    pub error: String,
    pub status: String,
    pub hover: Option<Hit>,
    pub hits: Vec<(Rect, Hit)>,
    pub area: Rect,
    pub canvas: Rect,
    pub inspector: Rect,
    pub pan: (f32, f32),
    pub zoom: f32,
    pub drag: Option<Drag>,
    pub pointer: (f32, f32),
    pub palette: bool,
    pub scroll: f32,
    undo: Vec<Workflow>,
    redo: Vec<Workflow>,
    pub field_before: String,
    pub needs_fit: bool,
    pub result_text: String,
    pub result_page: usize,
    pub minimap: bool,
    pub map_rect: Rect,
    pub panel_width: f32,
    pub settings_open: bool,
    pub advanced: bool,
    pub parameter: String,
    pub form_scroll: f32,
    pub form_height: f32,
    pub search: TextField,
    pub search_rect: Rect,
    pub searching: bool,
    pub insert_edge: Option<usize>,
    pub provider_labels: Vec<(String, String)>,
    pub palette_rect: Rect,
    pub folded: std::collections::HashSet<String>,
    pub node_history: bool,
    pub group: std::collections::BTreeSet<usize>,
    pub shift: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            panel_scroll: 0.,
            panel_height: 0.,
            panel_body: Rect::ZERO,
            result_tab: 0,
            tooltips: vec![],
            run_palette: false,
            result_hidden: false,
            folders: vec![],
            folder: None,
            editing_folder: None,
            library_height: 0.,
            selected_binding: None,
            workflows: vec![],
            selected: None,
            draft: None,
            dirty: false,
            selected_node: None,
            selected_edge: None,
            editor: None,
            field: TextField::new("填写内容"),
            field_rect: Rect::ZERO,
            history: vec![],
            history_open: false,
            run: None,
            error: String::new(),
            status: String::new(),
            hover: None,
            hits: vec![],
            area: Rect::ZERO,
            canvas: Rect::ZERO,
            inspector: Rect::ZERO,
            pan: (0., 0.),
            zoom: 1.,
            drag: None,
            pointer: (0., 0.),
            palette: false,
            scroll: 0.,
            undo: vec![],
            redo: vec![],
            field_before: String::new(),
            needs_fit: true,
            result_text: String::new(),
            result_page: 0,
            minimap: false,
            map_rect: Rect::ZERO,
            panel_width: 360.,
            settings_open: false,
            advanced: false,
            parameter: String::new(),
            form_scroll: 0.,
            form_height: 0.,
            search: TextField::new("搜索节点"),
            search_rect: Rect::ZERO,
            searching: false,
            insert_edge: None,
            provider_labels: Vec::new(),
            palette_rect: Rect::ZERO,
            folded: ["运行设置".to_string()].into_iter().collect(),
            node_history: false,
            group: Default::default(),
            shift: false,
        }
    }
}
impl State {
    pub fn graph(&self) -> Option<&Workflow> {
        self.run
            .as_ref()
            .map(|r| &r.definition)
            .or(self.draft.as_ref())
    }
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        self.hits
            .iter()
            .rev()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, h)| h.clone())
    }
    pub fn open(&mut self, saved: SavedWorkflow) {
        self.panel_scroll = 0.;
        self.settings_open = false;
        self.result_tab = 0;
        self.selected_binding = None;
        self.group.clear();
        self.drag = None;
        self.draft = Some(saved.definition.clone());
        self.selected = Some(saved);
        self.dirty = false;
        self.selected_node = None;
        self.selected_edge = None;
        self.run = None;
        self.editor = None;
        self.field_before = self.field.text().into();
        self.palette = true;
        self.history_open = false;
        self.pan = (0., 0.);
        self.zoom = 1.;
        self.needs_fit = true;
        self.undo.clear();
        self.redo.clear();
        self.error.clear();
        self.status.clear();
        if self.has_overlaps() {
            self.auto_layout();
            self.status = "旧卡片间距已整理，可撤销；保存后保留新布局".into();
        }
    }
    pub fn set_editor(&mut self, editor: Editor, text: String) {
        self.field.set_text(&text);
        self.field_before = text;
        self.editor = Some(editor);
        self.error.clear();
    }
    pub fn select_node(&mut self, i: usize) {
        self.result_hidden = false;
        self.selected_binding = None;
        self.group.clear();
        self.node_history = false;
        if self.selected_node != Some(i) {
            self.form_scroll = 0.;
            self.panel_scroll = 0.;
            self.result_tab = 0;
        }
        self.history_open = false;
        self.advanced = false;
        self.selected_node = Some(i);
        self.selected_edge = None;
        let Some(n) = self.graph().and_then(|g| g.nodes.get(i)).cloned() else {
            return;
        };
        if self.run.is_some() {
            self.refresh_result_text();
            if self.result_tab == 2 {
                self.show_result_page();
            } else {
                self.editor = None;
            }
        } else {
            let value = serde_json::json!({"label":n.label,"inputs":n.inputs,"config":n.config,"enabled":n.enabled,"on_error":n.on_error,"retries":n.retries,"timeout_ms":n.timeout_ms,"for_each":n.for_each});
            self.set_editor(
                Editor::Node,
                serde_json::to_string_pretty(&value).unwrap_or_default(),
            );
        }
    }
    pub fn show_result_page(&mut self) {
        let text = self
            .result_text
            .chars()
            .skip(self.result_page * 8000)
            .take(8000)
            .collect();
        self.set_editor(Editor::Result, text);
    }
    pub fn apply_node(&mut self) -> Result<(), String> {
        if self.editor == Some(Editor::Parameter) {
            return self.apply_parameter();
        }
        if self.editor != Some(Editor::Node) || self.field.text() == self.field_before {
            return Ok(());
        }
        let value: serde_json::Value =
            serde_json::from_str(self.field.text()).map_err(|e| e.to_string())?;
        let i = self.selected_node.ok_or("请先选择节点")?;
        let mut node = self
            .draft
            .as_ref()
            .and_then(|d| d.nodes.get(i))
            .ok_or("节点不存在")?
            .clone();
        let mut raw = serde_json::to_value(&node).map_err(|e| e.to_string())?;
        for (k, v) in value.as_object().ok_or("参数必须是对象")? {
            if ["id", "type", "position"].contains(&k.as_str()) {
                return Err("节点 id、类型和位置不能在参数面板中修改".into());
            }
            raw[k] = v.clone();
        }
        node = serde_json::from_value(raw).map_err(|e| e.to_string())?;
        self.checkpoint();
        self.draft.as_mut().unwrap().nodes[i] = node;
        self.field_before = self.field.text().into();
        Ok(())
    }
    pub fn node_rect(&self, n: &Node) -> Rect {
        let x = self.canvas.left + self.pan.0 + n.position.x * self.zoom;
        let y = self.canvas.top + self.pan.1 + n.position.y * self.zoom;
        Rect::from_size(x, y, 292. * self.zoom, node_height(n) * self.zoom)
    }
}

#[cfg(test)]
mod tests;

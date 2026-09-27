//! 定义数据表界面的状态、操作和弹出菜单数据。
use super::*;

#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub document: BaseDocument,
    pub saved_raw: String,
    pub dirty: bool,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub query: String,
    pub error: String,
    pub(super) popup: Option<Popup>,
    pub(super) popup_anchor: Option<Hit>,
    pub(super) date_time_picker: Option<DateTimePicker>,
    pub popup_scroll: f32,
    pub editing: bool,
    pub selected: Option<(usize, usize)>,
    pub detail: Option<usize>,
    pub detail_scroll: f32,
    pub located: Option<BaseLocation>,
    pub hover: Option<Hit>,
    pub reference_candidates: Vec<ReferenceCandidate>,
    pub reference_query: String,
    pub action: Option<Action>,
    pub(super) drag: Option<Drag>,
    pub(super) scrollbars: Interaction<Axis>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceCandidate {
    pub url: String,
    pub label: String,
}

/// 可作为 AI 上下文卡片发送的单元格快照。
///
/// `content` 是给模型的结构化记录内容；界面则使用 `title` 渲染为一枚可移除
/// 的上下文小片，而不是把整段快照直接塞进输入框。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiCellContext {
    pub title: String,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertDirection {
    Above,
    Below,
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    ExportXlsx,
    Automations,
    Open(String),
    CopyRecord(usize, Option<usize>),
    LoadReferences(usize, usize),
    /// 请求宿主在 `<base>.documents` 里创建一张表专属的文档。
    CreateTemporaryDocument,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Drag {
    Resize {
        field: usize,
        start_x: f32,
        width: f32,
    },
    Progress {
        record: usize,
        field: usize,
        left: f32,
        width: f32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Popup {
    Table,
    View,
    NewField,
    NewView,
    Field(usize),
    Options(usize),
    Option(usize, usize),
    Color(usize, usize),
    Select(usize, usize),
    DateTime(usize, usize),
    Record(usize),
    Sort,
    RowHeight,
    Columns,
    Freeze,
    Filter,
    FilterOperator(usize),
    Group,
    Reference(usize, usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DateTimePicker {
    pub(super) year: i32,
    pub(super) month: u32,
    pub(super) day: u32,
    pub(super) hour: u32,
    pub(super) minute: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateTimePart {
    Year,
    Month,
    Day,
    Hour,
    Minute,
}

impl DateTimePicker {
    pub(super) fn now() -> Self {
        let now = Local::now();
        Self {
            year: now.year(),
            month: now.month(),
            day: now.day(),
            hour: now.hour(),
            minute: now.minute(),
        }
    }
    pub(super) fn days_in_month(year: i32, month: u32) -> u32 {
        (28..=31)
            .rev()
            .find(|day| NaiveDate::from_ymd_opt(year, month, *day).is_some())
            .unwrap_or(28)
    }
    pub(super) fn normalize_day(&mut self) {
        self.day = self.day.min(Self::days_in_month(self.year, self.month));
    }
    pub(super) fn adjust(&mut self, part: DateTimePart, delta: i32) {
        match part {
            DateTimePart::Year => self.year = (self.year + delta).clamp(1, 9999),
            DateTimePart::Month => self.month = (self.month as i32 + delta).clamp(1, 12) as u32,
            DateTimePart::Day => {
                self.day = (self.day as i32 + delta)
                    .clamp(1, Self::days_in_month(self.year, self.month) as i32)
                    as u32
            }
            DateTimePart::Hour => self.hour = (self.hour as i32 + delta).clamp(0, 23) as u32,
            DateTimePart::Minute => self.minute = (self.minute as i32 + delta).clamp(0, 59) as u32,
        }
        self.normalize_day();
    }
    pub(super) fn display(self) -> String {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute
        )
    }
    pub(super) fn as_rfc3339(self) -> String {
        let local = NaiveDate::from_ymd_opt(self.year, self.month, self.day)
            .and_then(|date| date.and_hms_opt(self.hour, self.minute, 0))
            .and_then(|value| {
                Local
                    .from_local_datetime(&value)
                    .single()
                    .or_else(|| Local.from_local_datetime(&value).earliest())
            })
            .unwrap_or_else(Local::now);
        local.to_rfc3339_opts(SecondsFormat::Millis, true)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edit {
    FrozenRows,
    FrozenColumns,
    NewTable,
    TableName,
    NewField(FieldType),
    FieldName(usize),
    FieldType(usize, FieldType),
    NewOption(usize),
    OptionName(usize, usize),
    NewView(ViewType),
    ViewName,
    Cell(usize, usize),
    Search,
    Filter(usize, FilterOperator),
    DeleteTable,
    DeleteView,
    DeleteField(usize),
    DeleteRecord(usize),
    DeleteOption(usize, usize),
    ReferenceSearch(usize, usize),
    ReferenceLink(usize, usize),
}

#[derive(Debug, Clone)]
pub struct Prompt {
    pub edit: Edit,
    pub title: String,
    pub description: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    ExportXlsx,
    Columns,
    Freeze,
    Automations,
    Body,
    Table(usize),
    View(usize),
    TablesMenu,
    ViewsMenu,
    NewTable,
    NewView,
    NewField,
    NewRecord,
    NewTemporaryDocument,
    Field(usize),
    Cell(usize, usize),
    ProgressInput(usize, usize),
    Record(usize),
    Save,
    #[allow(dead_code)]
    Search,
    Sort,
    RowHeight,
    Filter,
    Group,
    Menu(usize),
    Dismiss,
    ToggleEditing,
    Resize(usize),
    ScrollTrack,
    ScrollThumb,
    DetailBody,
    DetailBackdrop,
    CloseDetail,
    OpenReference(usize, usize, usize),
    RemoveReference(usize, usize, usize),
    AddReference(usize, usize),
    CopyRecord(usize),
    RecordMenu(usize),
    ClearLocation,
    DateTimeAdjust(DateTimePart, i32),
    DateTimeConfirm,
    DateTimeClear,
}

#[derive(Debug, Clone)]
pub(super) enum Choice {
    ToggleColumn(usize),
    ShowAllColumns,
    ClearFreeze,
    ExportXlsx,
    Popup(Popup),
    Edit(Edit),
    Table(usize),
    View(usize),
    SetType(usize, FieldType),
    SetColor(usize, usize, OptionColor),
    ToggleOption(usize, usize, usize),
    ClearCell(usize, usize),
    Sort(usize, SortDirection),
    ClearSort,
    SetRowHeight(f32),
    EmptyFilter(usize, FilterOperator),
    ClearFilter,
    Group(usize),
    DuplicateRecord(usize),
    CopyRecord(usize),
    ShowRecord(usize),
    ToggleReference(usize, usize, usize),
    SetDateTime(usize, usize, String),
}

pub(super) struct MenuItem {
    pub(super) label: String,
    pub(super) choice: Choice,
    pub(super) color: Option<OptionColor>,
    pub(super) checked: bool,
}

impl MenuItem {
    pub(super) fn new(label: impl Into<String>, choice: Choice) -> Self {
        Self {
            label: label.into(),
            choice,
            color: None,
            checked: false,
        }
    }
}

#[derive(Default)]
pub struct Layout {
    pub(super) grid: Option<grid::Grid>,
    pub entries: Vec<(Rect, Hit)>,
    pub body: Rect,
    pub max_x: f32,
    pub max_y: f32,
    pub menu: Option<Rect>,
    pub menu_max: f32,
    pub scroll_track: Option<Rect>,
    pub scroll_thumb: Option<Rect>,
    pub scrollbars: Vec<(Axis, Bar)>,
    pub detail: Option<Rect>,
    pub detail_max: f32,
}

impl Layout {
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        self.entries
            .iter()
            .rev()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, h)| *h)
    }
    pub(super) fn rect_of(&self, hit: Hit) -> Option<Rect> {
        self.entries
            .iter()
            .rev()
            .find(|(_, entry)| *entry == hit)
            .map(|(rect, _)| *rect)
    }
}

pub(super) const TYPES: [(FieldType, &str); 12] = [
    (FieldType::Text, "文本"),
    (FieldType::Number, "数字"),
    (FieldType::SingleSelect, "下拉菜单"),
    (FieldType::MultiSelect, "多选标签"),
    (FieldType::Date, "日期"),
    (FieldType::DateTime, "时间点"),
    (FieldType::DateRange, "时间段"),
    (FieldType::Checkbox, "复选框"),
    (FieldType::Url, "链接"),
    (FieldType::Progress, "进度"),
    (FieldType::Reference, "引用"),
    (FieldType::Document, "文档"),
];

pub(super) const COLORS: [(OptionColor, &str); 8] = [
    (OptionColor::Gray, "灰色"),
    (OptionColor::Red, "红色"),
    (OptionColor::Orange, "橙色"),
    (OptionColor::Yellow, "黄色"),
    (OptionColor::Green, "绿色"),
    (OptionColor::Blue, "蓝色"),
    (OptionColor::Purple, "紫色"),
    (OptionColor::Pink, "粉色"),
];

pub(super) const OPERATORS: [(FilterOperator, &str); 5] = [
    (FilterOperator::Contains, "包含"),
    (FilterOperator::Equals, "等于"),
    (FilterOperator::NotEquals, "不等于"),
    (FilterOperator::IsEmpty, "为空"),
    (FilterOperator::IsNotEmpty, "不为空"),
];

pub(super) const DEFAULT_GRID_ROW_HEIGHT: f32 = 38.0;

pub(super) const GRID_ROW_HEIGHTS: [(f32, &str); 4] = [
    (32.0, "紧凑"),
    (38.0, "默认"),
    (48.0, "舒适"),
    (64.0, "宽松"),
];

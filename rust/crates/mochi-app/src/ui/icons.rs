//! 图标源是 lucide-react，数据嵌入自 assets/icons.json。

use std::collections::HashMap;
use std::sync::OnceLock;

const RAW: &str = include_str!("../../assets/icons.json");

/// lucide 的视图盒边长。
pub const VIEW_BOX: f32 = 24.0;
/// lucide 的描边宽度（视图盒单位）。
pub const STROKE_WIDTH: f32 = 2.0;

/// 一个图标的名字。与 lucide-react 的导出名一致（PascalCase）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Icon(pub &'static str);

/// 应用图标常量；新增项必须在内嵌资源表中存在。
///
/// 共享图标集合允许未引用项；`every_icon_constant_exists` 校验常量与资源表一致。
#[allow(dead_code)]
impl Icon {
    pub const CROSSHAIR: Icon = Icon("Crosshair");
    // 导航轨（GlobalNavigation.tsx）
    pub const HOME: Icon = Icon("Home");
    pub const INBOX: Icon = Icon("Inbox");
    pub const FOLDER: Icon = Icon("Folder");
    pub const CALENDAR_DAYS: Icon = Icon("CalendarDays");
    pub const BOT: Icon = Icon("Bot");
    pub const BOOK_OPEN: Icon = Icon("BookOpen");
    pub const CLOCK: Icon = Icon("Clock");
    pub const SPARKLES: Icon = Icon("Sparkles");
    pub const SETTINGS: Icon = Icon("Settings");
    pub const SEARCH: Icon = Icon("Search");
    pub const PLUS: Icon = Icon("Plus");
    pub const CHEVRON_DOWN: Icon = Icon("ChevronDown");
    pub const CHEVRON_RIGHT: Icon = Icon("ChevronRight");
    pub const CHEVRON_LEFT: Icon = Icon("ChevronLeft");
    pub const LINK2: Icon = Icon("Link2");
    // 右侧栏工具条（MainLayout.tsx）
    pub const LIST: Icon = Icon("List");
    pub const MESSAGE_SQUARE: Icon = Icon("MessageSquare");
    pub const TIMER: Icon = Icon("Timer");
    pub const PENCIL_LINE: Icon = Icon("PencilLine");
    pub const HISTORY: Icon = Icon("History");
    pub const X: Icon = Icon("X");
    // 侧栏（Sidebar.tsx / FileTreeNode.tsx）
    pub const CHEVRONS_UP_DOWN: Icon = Icon("ChevronsUpDown");
    pub const CHEVRONS_DOWN_UP: Icon = Icon("ChevronsDownUp");
    pub const ROTATE_CW: Icon = Icon("RotateCw");
    pub const FILE_TEXT: Icon = Icon("FileText");
    pub const FILE: Icon = Icon("File");
    pub const FOLDER_OPEN: Icon = Icon("FolderOpen");
    pub const FILE_PLUS: Icon = Icon("FilePlus");
    pub const FOLDER_PLUS: Icon = Icon("FolderPlus");
    pub const PENCIL: Icon = Icon("Pencil");
    pub const TRASH2: Icon = Icon("Trash2");
    pub const COPY: Icon = Icon("Copy");
    pub const CHECK: Icon = Icon("Check");
    pub const IMAGE: Icon = Icon("Image");
    pub const FILE_CODE: Icon = Icon("FileCode");
    // 标签栏、编辑器
    pub const PIN: Icon = Icon("Pin");
    pub const CODE: Icon = Icon("Code");
    pub const EYE: Icon = Icon("Eye");
    pub const SAVE: Icon = Icon("Save");
    pub const BOLD: Icon = Icon("Bold");
    pub const ITALIC: Icon = Icon("Italic");
    pub const STRIKETHROUGH: Icon = Icon("Strikethrough");
    pub const HEADING1: Icon = Icon("Heading1");
    pub const HEADING2: Icon = Icon("Heading2");
    pub const HEADING3: Icon = Icon("Heading3");
    pub const LIST_ORDERED: Icon = Icon("ListOrdered");
    pub const LIST_TODO: Icon = Icon("ListTodo");
    pub const QUOTE: Icon = Icon("Quote");
    pub const TABLE: Icon = Icon("Table");
    pub const MINUS: Icon = Icon("Minus");
    // 日程 / 收件箱 / 番茄钟 / AI
    pub const CALENDAR: Icon = Icon("Calendar");
    pub const CHECK_CIRCLE2: Icon = Icon("CheckCircle2");
    pub const CIRCLE: Icon = Icon("Circle");
    pub const PLAY: Icon = Icon("Play");
    pub const PAUSE: Icon = Icon("Pause");
    pub const SQUARE: Icon = Icon("Square");
    pub const SEND: Icon = Icon("Send");
    pub const ARCHIVE: Icon = Icon("Archive");
    pub const STAR: Icon = Icon("Star");
    pub const ALERT_CIRCLE: Icon = Icon("AlertCircle");
    pub const ALERT_TRIANGLE: Icon = Icon("AlertTriangle");
    pub const BLOCKS: Icon = Icon("Blocks");
    pub const FILE_CLOCK: Icon = Icon("FileClock");
    pub const ROTATE_CCW: Icon = Icon("RotateCcw");
    pub const USER_ROUND: Icon = Icon("UserRound");
    pub const REPLY: Icon = Icon("Reply");
    pub const PAPERCLIP: Icon = Icon("Paperclip");
    pub const CASE_SENSITIVE: Icon = Icon("CaseSensitive");
    pub const WHOLE_WORD: Icon = Icon("WholeWord");
    pub const REGEX: Icon = Icon("Regex");
    pub const UPLOAD: Icon = Icon("Upload");
    pub const LINK: Icon = Icon("Link");
    pub const FILE_PLUS2: Icon = Icon("FilePlus2");
    pub const EDIT2: Icon = Icon("Edit2");
    pub const FILE_DOWN: Icon = Icon("FileDown");
    pub const SHIELD: Icon = Icon("Shield");
    pub const INFO: Icon = Icon("Info");
    pub const SMILE: Icon = Icon("Smile");
    pub const UNLINK: Icon = Icon("Unlink");
    pub const LOADER2: Icon = Icon("Loader2");
    pub const REFRESH_CW: Icon = Icon("RefreshCw");
    pub const ARROW_LEFT: Icon = Icon("ArrowLeft");
    pub const ARROW_RIGHT: Icon = Icon("ArrowRight");
    pub const ARROW_UP: Icon = Icon("ArrowUp");
    pub const ARROW_DOWN: Icon = Icon("ArrowDown");
    // 日程（ScheduleSidebar.tsx / ScheduleWorkspace.tsx）
    pub const SQUARE_KANBAN: Icon = Icon("SquareKanban");
    pub const ACTIVITY: Icon = Icon("Activity");
    pub const FOLDER_KANBAN: Icon = Icon("FolderKanban");
    pub const REPEAT: Icon = Icon("Repeat");
    pub const FLAG: Icon = Icon("Flag");
    pub const BELL: Icon = Icon("Bell");
    // 收件箱（InboxWorkspace.tsx）
    pub const ARCHIVE_RESTORE: Icon = Icon("ArchiveRestore");
    pub const CHECK_SQUARE: Icon = Icon("CheckSquare");
    pub const PEN_LINE: Icon = Icon("PenLine");
    pub const MORE_HORIZONTAL: Icon = Icon("MoreHorizontal");
    pub const EXTERNAL_LINK: Icon = Icon("ExternalLink");
    pub const HASH: Icon = Icon("Hash");
    pub const TAG: Icon = Icon("Tag");
    pub const ZAP: Icon = Icon("Zap");
    pub const GIT_COMMIT: Icon = Icon("GitCommit");
    pub const TRENDING_UP: Icon = Icon("TrendingUp");
    pub const FLAME: Icon = Icon("Flame");
    pub const TARGET: Icon = Icon("Target");
    pub const KANBAN: Icon = Icon("Kanban");
    pub const LAYOUT_GRID: Icon = Icon("LayoutGrid");
    pub const SUN: Icon = Icon("Sun");
    pub const MOON: Icon = Icon("Moon");
    pub const MONITOR: Icon = Icon("Monitor");
    pub const KEYBOARD: Icon = Icon("Keyboard");
    pub const PALETTE: Icon = Icon("Palette");
    pub const GIT_BRANCH: Icon = Icon("GitBranch");
    pub const PUZZLE: Icon = Icon("Puzzle");
    pub const FOLDER_COG: Icon = Icon("FolderCog");
    pub const SLIDERS_HORIZONTAL: Icon = Icon("SlidersHorizontal");
    // 编辑器工具栏（EditorToolbar.tsx）
    pub const UNDERLINE: Icon = Icon("Underline");
    pub const HEADING4: Icon = Icon("Heading4");
    pub const HEADING5: Icon = Icon("Heading5");
    pub const HEADING6: Icon = Icon("Heading6");
    pub const CODE2: Icon = Icon("Code2");
    pub const ALIGN_LEFT: Icon = Icon("AlignLeft");
    pub const ALIGN_CENTER: Icon = Icon("AlignCenter");
    pub const ALIGN_RIGHT: Icon = Icon("AlignRight");
    pub const DOWNLOAD: Icon = Icon("Download");
    pub const PILCROW: Icon = Icon("Pilcrow");
    pub const REMOVE_FORMATTING: Icon = Icon("RemoveFormatting");
    pub const REPLACE: Icon = Icon("Replace");
    pub const CHEVRON_UP: Icon = Icon("ChevronUp");
    // Agent 配置页（AIPromptsManager.tsx）
    pub const BOXES: Icon = Icon("Boxes");
    pub const FILE_CODE2: Icon = Icon("FileCode2");
    pub const NETWORK: Icon = Icon("Network");
    pub const SETTINGS2: Icon = Icon("Settings2");
    // 命令面板（CommandPalette.tsx）
    pub const PANEL_LEFT_CLOSE: Icon = Icon("PanelLeftClose");
    // 查看器（Viewer/*）
    pub const ZOOM_IN: Icon = Icon("ZoomIn");
    pub const ZOOM_OUT: Icon = Icon("ZoomOut");
    pub const MAXIMIZE2: Icon = Icon("Maximize2");
    pub const GLOBE: Icon = Icon("Globe");
    pub const MOUSE_POINTER2: Icon = Icon("MousePointer2");
    pub const TYPE: Icon = Icon("Type");

    /// 全部常量，供测试与调试用。**漏登记的常量测不到**——加常量时同步加这里。
    pub const ALL: &'static [Icon] = &[
        Self::UNDERLINE,
        Self::HEADING4,
        Self::HEADING5,
        Self::HEADING6,
        Self::CODE2,
        Self::ALIGN_LEFT,
        Self::ALIGN_CENTER,
        Self::ALIGN_RIGHT,
        Self::DOWNLOAD,
        Self::PILCROW,
        Self::REMOVE_FORMATTING,
        Self::REPLACE,
        Self::CHEVRON_UP,
        Self::BOXES,
        Self::FILE_CODE2,
        Self::NETWORK,
        Self::SETTINGS2,
        Self::PANEL_LEFT_CLOSE,
        Self::ZOOM_IN,
        Self::ZOOM_OUT,
        Self::MAXIMIZE2,
        Self::GLOBE,
        Self::MOUSE_POINTER2,
        Self::TYPE,
        Self::HOME,
        Self::INBOX,
        Self::FOLDER,
        Self::CALENDAR_DAYS,
        Self::BOT,
        Self::BOOK_OPEN,
        Self::CLOCK,
        Self::SPARKLES,
        Self::SETTINGS,
        Self::SEARCH,
        Self::PLUS,
        Self::CHEVRON_DOWN,
        Self::CHEVRON_RIGHT,
        Self::CHEVRON_LEFT,
        Self::LINK2,
        Self::LIST,
        Self::MESSAGE_SQUARE,
        Self::TIMER,
        Self::PENCIL_LINE,
        Self::HISTORY,
        Self::X,
        Self::CHEVRONS_UP_DOWN,
        Self::CHEVRONS_DOWN_UP,
        Self::ROTATE_CW,
        Self::FILE_TEXT,
        Self::FILE,
        Self::FOLDER_OPEN,
        Self::FILE_PLUS,
        Self::FOLDER_PLUS,
        Self::PENCIL,
        Self::TRASH2,
        Self::COPY,
        Self::CHECK,
        Self::IMAGE,
        Self::FILE_CODE,
        Self::PIN,
        Self::CODE,
        Self::EYE,
        Self::SAVE,
        Self::BOLD,
        Self::ITALIC,
        Self::STRIKETHROUGH,
        Self::HEADING1,
        Self::HEADING2,
        Self::HEADING3,
        Self::LIST_ORDERED,
        Self::LIST_TODO,
        Self::QUOTE,
        Self::TABLE,
        Self::MINUS,
        Self::CALENDAR,
        Self::CHECK_CIRCLE2,
        Self::CIRCLE,
        Self::PLAY,
        Self::PAUSE,
        Self::SQUARE,
        Self::SEND,
        Self::ARCHIVE,
        Self::STAR,
        Self::ALERT_CIRCLE,
        Self::LOADER2,
        Self::REFRESH_CW,
        Self::ARROW_LEFT,
        Self::ARROW_RIGHT,
        Self::MORE_HORIZONTAL,
        Self::EXTERNAL_LINK,
        Self::HASH,
        Self::TAG,
        Self::ZAP,
        Self::GIT_COMMIT,
        Self::TRENDING_UP,
        Self::FLAME,
        Self::TARGET,
        Self::KANBAN,
        Self::LAYOUT_GRID,
        Self::SUN,
        Self::MOON,
        Self::MONITOR,
        Self::KEYBOARD,
        Self::PALETTE,
        Self::GIT_BRANCH,
        Self::PUZZLE,
        Self::FOLDER_COG,
        Self::SLIDERS_HORIZONTAL,
        Self::ALERT_TRIANGLE,
        Self::UPLOAD,
        Self::LINK,
        Self::FILE_PLUS2,
        Self::EDIT2,
        Self::FILE_DOWN,
        Self::SHIELD,
        Self::INFO,
        Self::SMILE,
        Self::UNLINK,
        Self::CASE_SENSITIVE,
        Self::WHOLE_WORD,
        Self::REGEX,
        Self::BLOCKS,
        Self::FILE_CLOCK,
        Self::ROTATE_CCW,
        Self::USER_ROUND,
        Self::REPLY,
        Self::PAPERCLIP,
        Self::ARROW_UP,
        Self::ARROW_DOWN,
        Self::SQUARE_KANBAN,
        Self::ACTIVITY,
        Self::FOLDER_KANBAN,
        Self::REPEAT,
        Self::FLAG,
        Self::BELL,
        Self::ARCHIVE_RESTORE,
        Self::CHECK_SQUARE,
        Self::PEN_LINE,
    ];
}

/// 一段路径。坐标全是**绝对**的视图盒坐标——相对命令在解析时就已展开。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathCmd {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    /// 三次贝塞尔：两个控制点 + 终点。二次贝塞尔在解析时升阶成三次。
    CubicTo(f32, f32, f32, f32, f32, f32),
    /// SVG 椭圆弧：半径、旋转角（度）、大弧标志、顺时针标志、终点。
    ArcTo {
        rx: f32,
        ry: f32,
        rotation: f32,
        large: bool,
        sweep: bool,
        x: f32,
        y: f32,
    },
    Close,
}

/// 一个图标 = 若干条子路径。每个 SVG 元素（path/circle/rect/…）解析成一条。
#[derive(Debug, Clone, PartialEq)]
pub struct IconShape {
    pub cmds: Vec<PathCmd>,
}

/// 按名字查图标。查不到返回 `None`——调用方应当画一个占位而不是崩。
pub fn lookup(icon: Icon) -> Option<&'static [IconShape]> {
    table().get(icon.0).map(|v| v.as_slice())
}
pub fn named(name: &str) -> Option<Icon> {
    table()
        .get_key_value(name)
        .map(|(name, _)| Icon(name.as_str()))
}

/// 原生端已嵌入、可供用户选择的 Lucide 图标。名称保持与 Electron 版一致。
pub fn catalog() -> Vec<Icon> {
    let mut icons = table()
        .keys()
        .map(|name| Icon(name.as_str()))
        .collect::<Vec<_>>();
    icons.sort_by(|left, right| left.0.cmp(right.0));
    icons
}

fn table() -> &'static HashMap<String, Vec<IconShape>> {
    static CELL: OnceLock<HashMap<String, Vec<IconShape>>> = OnceLock::new();
    CELL.get_or_init(|| {
        parse_table(RAW)
            .expect("icons.json 解析失败（编译期嵌入的资源，解不开说明抽取脚本产出坏了）")
    })
}

fn parse_table(raw: &str) -> Option<HashMap<String, Vec<IconShape>>> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    let mut out = HashMap::new();
    for (name, nodes) in v["icons"].as_object()? {
        let mut shapes = Vec::new();
        for node in nodes.as_array()? {
            let tag = node[0].as_str()?;
            let attrs = &node[1];
            let num = |k: &str| -> f32 {
                attrs[k]
                    .as_str()
                    .and_then(|s| s.parse::<f32>().ok())
                    .or_else(|| attrs[k].as_f64().map(|n| n as f32))
                    .unwrap_or(0.0)
            };
            let cmds = match tag {
                "path" => parse_path(attrs["d"].as_str()?),
                "circle" => ellipse_path(num("cx"), num("cy"), num("r"), num("r")),
                "ellipse" => ellipse_path(num("cx"), num("cy"), num("rx"), num("ry")),
                "rect" => {
                    let rx = if attrs.get("rx").is_some() {
                        num("rx")
                    } else {
                        0.0
                    };
                    let ry = if attrs.get("ry").is_some() {
                        num("ry")
                    } else {
                        rx
                    };
                    rect_path(num("x"), num("y"), num("width"), num("height"), rx, ry)
                }
                "line" => vec![
                    PathCmd::MoveTo(num("x1"), num("y1")),
                    PathCmd::LineTo(num("x2"), num("y2")),
                ],
                "polyline" | "polygon" => {
                    let pts = parse_numbers(attrs["points"].as_str()?);
                    let mut cmds = Vec::new();
                    for (i, pair) in pts.chunks(2).enumerate() {
                        if pair.len() < 2 {
                            break;
                        }
                        cmds.push(if i == 0 {
                            PathCmd::MoveTo(pair[0], pair[1])
                        } else {
                            PathCmd::LineTo(pair[0], pair[1])
                        });
                    }
                    if tag == "polygon" {
                        cmds.push(PathCmd::Close);
                    }
                    cmds
                }
                // 抽取脚本已经拒绝了未支持的元素；到这儿说明资源被手改过
                _ => return None,
            };
            shapes.push(IconShape { cmds });
        }
        out.insert(name.clone(), shapes);
    }
    Some(out)
}

/// 圆/椭圆拆成两段半圆弧——单段 360° 的弧在 SVG 里是退化的，D2D 也画不出来。
fn ellipse_path(cx: f32, cy: f32, rx: f32, ry: f32) -> Vec<PathCmd> {
    vec![
        PathCmd::MoveTo(cx + rx, cy),
        PathCmd::ArcTo {
            rx,
            ry,
            rotation: 0.0,
            large: false,
            sweep: true,
            x: cx - rx,
            y: cy,
        },
        PathCmd::ArcTo {
            rx,
            ry,
            rotation: 0.0,
            large: false,
            sweep: true,
            x: cx + rx,
            y: cy,
        },
        PathCmd::Close,
    ]
}

/// 圆角矩形。`rx = 0` 时就是四条直线。
fn rect_path(x: f32, y: f32, w: f32, h: f32, rx: f32, ry: f32) -> Vec<PathCmd> {
    let rx = rx.min(w / 2.0);
    let ry = ry.min(h / 2.0);
    if rx <= 0.0 || ry <= 0.0 {
        return vec![
            PathCmd::MoveTo(x, y),
            PathCmd::LineTo(x + w, y),
            PathCmd::LineTo(x + w, y + h),
            PathCmd::LineTo(x, y + h),
            PathCmd::Close,
        ];
    }
    let arc = |ex: f32, ey: f32| PathCmd::ArcTo {
        rx,
        ry,
        rotation: 0.0,
        large: false,
        sweep: true,
        x: ex,
        y: ey,
    };
    vec![
        PathCmd::MoveTo(x + rx, y),
        PathCmd::LineTo(x + w - rx, y),
        arc(x + w, y + ry),
        PathCmd::LineTo(x + w, y + h - ry),
        arc(x + w - rx, y + h),
        PathCmd::LineTo(x + rx, y + h),
        arc(x, y + h - ry),
        PathCmd::LineTo(x, y + ry),
        arc(x + rx, y),
        PathCmd::Close,
    ]
}

/// 把 `"1 2,3.5-4e1"` 这类 SVG 数字串切成数字。负号和指数都能当分隔。
fn parse_numbers(s: &str) -> Vec<f32> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let bytes: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        let prev_is_exp = cur.ends_with('e') || cur.ends_with('E');
        if c.is_ascii_digit() || c == '.' && !cur.contains('.') && !cur.contains('e') {
            cur.push(c);
        } else if c == '-' || c == '+' {
            if !cur.is_empty() && !prev_is_exp {
                flush(&mut cur, &mut out);
            }
            cur.push(c);
        } else if c == 'e' || c == 'E' {
            cur.push(c);
        } else {
            // 分隔符（空格、逗号）或第二个小数点开头的新数字
            flush(&mut cur, &mut out);
            if c == '.' {
                cur.push(c);
            }
        }
        i += 1;
    }
    flush(&mut cur, &mut out);
    out
}

fn flush(cur: &mut String, out: &mut Vec<f32>) {
    if !cur.is_empty() {
        if let Ok(n) = cur.parse::<f32>() {
            out.push(n);
        }
        cur.clear();
    }
}

/// 解析 SVG `d` 属性。支持 M/L/H/V/C/S/Q/T/A/Z 及其相对形式。
///
/// 相对坐标、`H`/`V` 单轴、`S`/`T` 的反射控制点全在这里展开，
/// 输出只有四种绝对段——回放层因此不需要知道 SVG。
pub fn parse_path(d: &str) -> Vec<PathCmd> {
    let mut cmds = Vec::new();
    // 当前点、子路径起点、上一段的第二控制点（S/T 反射用）、上一命令
    let (mut cx, mut cy) = (0.0f32, 0.0f32);
    let (mut sx, mut sy) = (0.0f32, 0.0f32);
    let mut last_ctrl: Option<(f32, f32)> = None;
    let mut last_cmd = ' ';

    // 先按命令字母切段
    let mut segments: Vec<(char, Vec<f32>)> = Vec::new();
    let mut letter = ' ';
    let mut buf = String::new();
    for ch in d.chars() {
        if ch.is_ascii_alphabetic() && ch != 'e' && ch != 'E' {
            if letter != ' ' {
                segments.push((letter, parse_numbers(&buf)));
            }
            letter = ch;
            buf.clear();
        } else {
            buf.push(ch);
        }
    }
    if letter != ' ' {
        segments.push((letter, parse_numbers(&buf)));
    }

    for (cmd, nums) in segments {
        let rel = cmd.is_ascii_lowercase();
        let upper = cmd.to_ascii_uppercase();
        let mut i = 0;
        // 一条命令后面可以跟多组参数（隐式重复）。M 的重复视为 L。
        let mut first = true;
        loop {
            match upper {
                'M' => {
                    if i + 2 > nums.len() {
                        break;
                    }
                    let (mut x, mut y) = (nums[i], nums[i + 1]);
                    if rel {
                        x += cx;
                        y += cy;
                    }
                    if first {
                        cmds.push(PathCmd::MoveTo(x, y));
                        sx = x;
                        sy = y;
                    } else {
                        cmds.push(PathCmd::LineTo(x, y));
                    }
                    cx = x;
                    cy = y;
                    i += 2;
                    last_ctrl = None;
                }
                'L' => {
                    if i + 2 > nums.len() {
                        break;
                    }
                    let (mut x, mut y) = (nums[i], nums[i + 1]);
                    if rel {
                        x += cx;
                        y += cy;
                    }
                    cmds.push(PathCmd::LineTo(x, y));
                    cx = x;
                    cy = y;
                    i += 2;
                    last_ctrl = None;
                }
                'H' => {
                    if i + 1 > nums.len() {
                        break;
                    }
                    let x = if rel { cx + nums[i] } else { nums[i] };
                    cmds.push(PathCmd::LineTo(x, cy));
                    cx = x;
                    i += 1;
                    last_ctrl = None;
                }
                'V' => {
                    if i + 1 > nums.len() {
                        break;
                    }
                    let y = if rel { cy + nums[i] } else { nums[i] };
                    cmds.push(PathCmd::LineTo(cx, y));
                    cy = y;
                    i += 1;
                    last_ctrl = None;
                }
                'C' => {
                    if i + 6 > nums.len() {
                        break;
                    }
                    let mut p = [
                        nums[i],
                        nums[i + 1],
                        nums[i + 2],
                        nums[i + 3],
                        nums[i + 4],
                        nums[i + 5],
                    ];
                    if rel {
                        for k in 0..3 {
                            p[k * 2] += cx;
                            p[k * 2 + 1] += cy;
                        }
                    }
                    cmds.push(PathCmd::CubicTo(p[0], p[1], p[2], p[3], p[4], p[5]));
                    last_ctrl = Some((p[2], p[3]));
                    cx = p[4];
                    cy = p[5];
                    i += 6;
                }
                'S' => {
                    if i + 4 > nums.len() {
                        break;
                    }
                    // 第一控制点是上一段第二控制点关于当前点的反射；上一段不是曲线时就是当前点
                    let (c1x, c1y) = match (last_cmd_is_cubic(last_cmd), last_ctrl) {
                        (true, Some((lx, ly))) => (2.0 * cx - lx, 2.0 * cy - ly),
                        _ => (cx, cy),
                    };
                    let mut p = [nums[i], nums[i + 1], nums[i + 2], nums[i + 3]];
                    if rel {
                        p[0] += cx;
                        p[1] += cy;
                        p[2] += cx;
                        p[3] += cy;
                    }
                    cmds.push(PathCmd::CubicTo(c1x, c1y, p[0], p[1], p[2], p[3]));
                    last_ctrl = Some((p[0], p[1]));
                    cx = p[2];
                    cy = p[3];
                    i += 4;
                }
                'Q' | 'T' => {
                    let (qx, qy, x, y);
                    if upper == 'Q' {
                        if i + 4 > nums.len() {
                            break;
                        }
                        let mut p = [nums[i], nums[i + 1], nums[i + 2], nums[i + 3]];
                        if rel {
                            p[0] += cx;
                            p[1] += cy;
                            p[2] += cx;
                            p[3] += cy;
                        }
                        (qx, qy, x, y) = (p[0], p[1], p[2], p[3]);
                        i += 4;
                    } else {
                        if i + 2 > nums.len() {
                            break;
                        }
                        let (c1x, c1y) = match (last_cmd_is_quad(last_cmd), last_ctrl) {
                            (true, Some((lx, ly))) => (2.0 * cx - lx, 2.0 * cy - ly),
                            _ => (cx, cy),
                        };
                        let mut p = [nums[i], nums[i + 1]];
                        if rel {
                            p[0] += cx;
                            p[1] += cy;
                        }
                        (qx, qy, x, y) = (c1x, c1y, p[0], p[1]);
                        i += 2;
                    }
                    // 二次升三次：C1 = P0 + 2/3 (Q - P0)，C2 = P + 2/3 (Q - P)
                    let c1x = cx + 2.0 / 3.0 * (qx - cx);
                    let c1y = cy + 2.0 / 3.0 * (qy - cy);
                    let c2x = x + 2.0 / 3.0 * (qx - x);
                    let c2y = y + 2.0 / 3.0 * (qy - y);
                    cmds.push(PathCmd::CubicTo(c1x, c1y, c2x, c2y, x, y));
                    last_ctrl = Some((qx, qy));
                    cx = x;
                    cy = y;
                }
                'A' => {
                    if i + 7 > nums.len() {
                        break;
                    }
                    let (mut x, mut y) = (nums[i + 5], nums[i + 6]);
                    if rel {
                        x += cx;
                        y += cy;
                    }
                    cmds.push(PathCmd::ArcTo {
                        rx: nums[i].abs(),
                        ry: nums[i + 1].abs(),
                        rotation: nums[i + 2],
                        large: nums[i + 3] != 0.0,
                        sweep: nums[i + 4] != 0.0,
                        x,
                        y,
                    });
                    cx = x;
                    cy = y;
                    i += 7;
                    last_ctrl = None;
                }
                'Z' => {
                    cmds.push(PathCmd::Close);
                    cx = sx;
                    cy = sy;
                    last_ctrl = None;
                    break;
                }
                _ => break,
            }
            first = false;
            last_cmd = upper;
            if i >= nums.len() {
                break;
            }
        }
        last_cmd = upper;
    }
    cmds
}

fn last_cmd_is_cubic(c: char) -> bool {
    matches!(c, 'C' | 'S')
}

fn last_cmd_is_quad(c: char) -> bool {
    matches!(c, 'Q' | 'T')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_constant_exists_in_the_extracted_table() {
        // 常量拼错在编译期发现不了（它只是个字符串），这条把常量与资源钉在一起
        for icon in Icon::ALL {
            assert!(lookup(*icon).is_some(), "资源表里没有 {}", icon.0);
        }
    }

    #[test]
    fn the_table_has_the_icons_the_electron_navigation_uses() {
        for name in [
            "Home",
            "Inbox",
            "Folder",
            "CalendarDays",
            "Bot",
            "BookOpen",
            "Clock",
            "Sparkles",
            "Settings",
            "Search",
        ] {
            assert!(lookup(Icon(name)).is_some(), "{name}");
        }
    }

    #[test]
    fn an_unknown_icon_returns_none_instead_of_panicking() {
        assert!(lookup(Icon("DefinitelyNotAnIcon")).is_none());
    }

    #[test]
    fn numbers_split_on_signs_and_commas() {
        assert_eq!(parse_numbers("1 2,3.5-4"), vec![1.0, 2.0, 3.5, -4.0]);
        // 连续小数点开头是两个数：`.5.5` = 0.5 0.5
        assert_eq!(parse_numbers(".5.5"), vec![0.5, 0.5]);
        // 指数里的负号不是分隔
        assert_eq!(parse_numbers("1e-2 3"), vec![0.01, 3.0]);
    }

    #[test]
    fn relative_commands_are_expanded_to_absolute_coordinates() {
        // lucide 的 house：`M15 21v-8a1 1 0 0 0-1-1h-4a1 1 0 0 0-1 1v8`
        let cmds = parse_path("M15 21v-8a1 1 0 0 0-1-1h-4a1 1 0 0 0-1 1v8");
        assert_eq!(cmds[0], PathCmd::MoveTo(15.0, 21.0));
        assert_eq!(cmds[1], PathCmd::LineTo(15.0, 13.0));
        assert_eq!(
            cmds[2],
            PathCmd::ArcTo {
                rx: 1.0,
                ry: 1.0,
                rotation: 0.0,
                large: false,
                sweep: false,
                x: 14.0,
                y: 12.0
            }
        );
        assert_eq!(cmds[3], PathCmd::LineTo(10.0, 12.0));
        assert_eq!(
            cmds[4],
            PathCmd::ArcTo {
                rx: 1.0,
                ry: 1.0,
                rotation: 0.0,
                large: false,
                sweep: false,
                x: 9.0,
                y: 13.0
            }
        );
        assert_eq!(cmds[5], PathCmd::LineTo(9.0, 21.0));
    }

    #[test]
    fn implicit_repeats_of_l_and_m_are_handled() {
        // `M1 1 2 2` 第二组是隐式 L
        let cmds = parse_path("M1 1 2 2L3 3 4 4");
        assert_eq!(
            cmds,
            vec![
                PathCmd::MoveTo(1.0, 1.0),
                PathCmd::LineTo(2.0, 2.0),
                PathCmd::LineTo(3.0, 3.0),
                PathCmd::LineTo(4.0, 4.0),
            ]
        );
    }

    #[test]
    fn close_returns_the_pen_to_the_subpath_start() {
        // Z 之后的相对命令要从子路径起点算
        let cmds = parse_path("M10 10h5v5zl1 1");
        assert_eq!(cmds.last(), Some(&PathCmd::LineTo(11.0, 11.0)));
    }

    #[test]
    fn smooth_cubic_reflects_the_previous_control_point() {
        let cmds = parse_path("M0 0C1 1 2 1 3 0S5 -1 6 0");
        match cmds[2] {
            PathCmd::CubicTo(c1x, c1y, ..) => {
                // 上一段第二控制点 (2,1) 关于当前点 (3,0) 的反射是 (4,-1)
                assert_eq!((c1x, c1y), (4.0, -1.0));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn quadratic_curves_are_raised_to_cubic() {
        let cmds = parse_path("M0 0Q3 3 6 0");
        match cmds[1] {
            PathCmd::CubicTo(c1x, c1y, c2x, c2y, x, y) => {
                assert!((c1x - 2.0).abs() < 1e-5 && (c1y - 2.0).abs() < 1e-5);
                assert!((c2x - 4.0).abs() < 1e-5 && (c2y - 2.0).abs() < 1e-5);
                assert_eq!((x, y), (6.0, 0.0));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_circle_becomes_two_half_arcs_not_one_degenerate_full_arc() {
        let shapes = lookup(Icon::CIRCLE).unwrap();
        let arcs = shapes[0]
            .cmds
            .iter()
            .filter(|c| matches!(c, PathCmd::ArcTo { .. }))
            .count();
        assert_eq!(arcs, 2);
    }

    #[test]
    fn every_shape_in_the_table_starts_with_a_move() {
        // 没有起点的路径 D2D 会拒绝（BeginFigure 之前不能 AddLine）
        for (name, shapes) in table() {
            for shape in shapes {
                assert!(
                    matches!(shape.cmds.first(), Some(PathCmd::MoveTo(..))),
                    "{name} 的子路径不是以 M 开头: {:?}",
                    shape.cmds.first()
                );
            }
        }
    }

    #[test]
    fn all_coordinates_stay_inside_the_view_box_with_a_little_slack() {
        // 相对坐标展开错了通常会让点飘到几十几百之外——用视图盒边界兜住
        for (name, shapes) in table() {
            for shape in shapes {
                for cmd in &shape.cmds {
                    let pts: Vec<f32> = match *cmd {
                        PathCmd::MoveTo(x, y) | PathCmd::LineTo(x, y) => vec![x, y],
                        PathCmd::CubicTo(a, b, c, d, e, f) => vec![a, b, c, d, e, f],
                        PathCmd::ArcTo { x, y, .. } => vec![x, y],
                        PathCmd::Close => vec![],
                    };
                    for p in pts {
                        assert!(
                            (-2.0..=26.0).contains(&p),
                            "{name} 的坐标 {p} 跑出了视图盒: {cmd:?}"
                        );
                    }
                }
            }
        }
    }
}

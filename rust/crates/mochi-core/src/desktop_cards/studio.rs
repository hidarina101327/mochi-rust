//! 可移植的桌面卡片布局。位置以基点表示（0..10000）。
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Studio {
    pub height: u16,
    pub nodes: Vec<Node>,
    pub snap: u16,
    pub accept_drop: bool,
}
impl Default for Studio {
    fn default() -> Self {
        Self {
            height: 100,
            nodes: vec![],
            snap: 250,
            accept_drop: true,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Text,
    Clock,
    AnalogClock,
    Date,
    Button,
    Shortcut,
    Timer,
    Data,
    AiChat,
    Divider,
}
impl Kind {
    pub const ALL: [Self; 10] = [
        Self::Text,
        Self::Clock,
        Self::AnalogClock,
        Self::Date,
        Self::Button,
        Self::Shortcut,
        Self::Timer,
        Self::Data,
        Self::AiChat,
        Self::Divider,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Text => "文本",
            Self::Clock => "时钟",
            Self::AnalogClock => "圆形时钟",
            Self::Date => "日期",
            Self::Button => "按钮",
            Self::Shortcut => "快捷方式",
            Self::Timer => "番茄钟",
            Self::Data => "数据列表",
            Self::AiChat => "AI 对话",
            Self::Divider => "分隔线",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Trigger {
    Click,
    DoubleClick,
    MouseEnter,
    MouseLeave,
    Interval,
    PageEnter,
    Custom,
}
impl Trigger {
    pub const ALL: [Self; 7] = [
        Self::Click,
        Self::DoubleClick,
        Self::MouseEnter,
        Self::MouseLeave,
        Self::Interval,
        Self::PageEnter,
        Self::Custom,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Click => "单击",
            Self::DoubleClick => "双击",
            Self::MouseEnter => "鼠标进入",
            Self::MouseLeave => "鼠标离开",
            Self::Interval => "每隔 N 秒",
            Self::PageEnter => "进入分页",
            Self::Custom => "自定义事件",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    OpenTarget,
    OpenModule,
    SwitchPage,
    ToggleTimer,
    ResetTimer,
    SetText,
    ToggleVisible,
    Emit,
}
impl Action {
    pub const ALL: [Self; 8] = [
        Self::OpenTarget,
        Self::OpenModule,
        Self::SwitchPage,
        Self::ToggleTimer,
        Self::ResetTimer,
        Self::SetText,
        Self::ToggleVisible,
        Self::Emit,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenTarget => "打开文件 / 网址",
            Self::OpenModule => "进入墨池功能",
            Self::SwitchPage => "切换分页",
            Self::ToggleTimer => "开始 / 暂停计时",
            Self::ResetTimer => "重置计时",
            Self::SetText => "设置组件文字",
            Self::ToggleVisible => "切换组件可见",
            Self::Emit => "发送自定义事件",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Binding {
    pub name: String,
    pub trigger: Trigger,
    pub action: Action,
    pub target: String,
    pub value: String,
    pub seconds: u32,
}
impl Default for Binding {
    fn default() -> Self {
        Self {
            name: String::new(),
            trigger: Trigger::Click,
            action: Action::OpenModule,
            target: "home".into(),
            value: String::new(),
            seconds: 60,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Node {
    pub id: String,
    pub kind: Kind,
    pub title: String,
    pub target: String,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub font_size: u16,
    pub foreground: Option<u32>,
    pub background: Option<u32>,
    pub border: bool,
    pub show_icon: bool,
    pub hidden: bool,
    pub events: Vec<Binding>,
}
impl Default for Node {
    fn default() -> Self {
        Self {
            id: super::new_id("node"),
            kind: Kind::Text,
            title: "新组件".into(),
            target: String::new(),
            x: 500,
            y: 500,
            width: 4000,
            height: 2000,
            font_size: 24,
            foreground: None,
            background: None,
            border: false,
            show_icon: true,
            hidden: false,
            events: vec![],
        }
    }
}
impl Node {
    pub fn new(kind: Kind) -> Self {
        let mut n = Self {
            kind,
            title: kind.label().into(),
            ..Self::default()
        };
        match kind {
            Kind::AnalogClock => {
                n.width = 6000;
                n.height = 6000;
            }
            Kind::Clock => {
                n.font_size = 56;
                n.width = 9000;
                n.target = "%H:%M:%S".into()
            }
            Kind::Date => n.target = "%Y年%m月%d日 %A".into(),
            Kind::Button => {
                n.border = true;
                n.events.push(Binding::default());
            }
            Kind::Shortcut => {
                n.width = 2200;
                n.height = 2200;
                n.font_size = 14;
            }
            Kind::Timer => {
                n.font_size = 48;
                n.width = 9000;
                n.events.push(Binding {
                    action: Action::ToggleTimer,
                    ..Binding::default()
                });
            }
            Kind::Data => {
                n.target = "favorites".into();
                n.width = 9000;
                n.height = 5000;
                n.font_size = 16;
            }
            Kind::AiChat => {
                n.width = 9000;
                n.height = 8000;
            }
            Kind::Divider => n.height = 250,
            _ => {}
        }
        n
    }
    pub fn clamp(&mut self) {
        self.width = self.width.clamp(250, 10000);
        self.height = self.height.clamp(100, 10000);
        self.x = self.x.min(10000 - self.width);
        self.y = self.y.min(10000 - self.height);
    }
}
impl Studio {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (100..=1600).contains(&self.height) && self.nodes.len() <= 256 && self.snap <= 2500,
            "组件最多 256 个；吸附步长须为 0–25%"
        );
        ensure!(
            self.nodes.iter().filter(|n| n.kind == Kind::AiChat).count() <= 1,
            "每页最多一个 AI 对话组件"
        );
        let mut ids = std::collections::HashSet::new();
        for n in &self.nodes {
            ensure!(
                !n.id.is_empty() && n.id.len() <= 160 && ids.insert(&n.id),
                "组件 ID 无效或重复"
            );
            ensure!(
                (1..=10000).contains(&n.width)
                    && (1..=10000).contains(&n.height)
                    && n.x as u32 + n.width as u32 <= 10000
                    && n.y as u32 + n.height as u32 <= 10000,
                "组件须位于画布内"
            );
            ensure!(
                (8..=240).contains(&n.font_size) && n.title.len() <= 4096 && n.target.len() <= 4096,
                "组件字号或内容超出范围"
            );
            ensure!(
                n.foreground.is_none_or(|c| c <= 0xffffff)
                    && n.background.is_none_or(|c| c <= 0xffffff),
                "组件颜色无效"
            );
            ensure!(n.events.len() <= 32, "每个组件最多 32 个事件");
            for b in &n.events {
                ensure!(
                    b.seconds >= 1
                        && b.seconds <= 86400
                        && b.target.len() <= 4096
                        && b.value.len() <= 4096
                        && b.name.len() <= 160,
                    "事件参数超出范围"
                );
            }
        }
        Ok(())
    }
    pub fn arrange_shortcuts(&mut self) {
        let count = self
            .nodes
            .iter()
            .filter(|n| n.kind == Kind::Shortcut)
            .count();
        let rows = count.div_ceil(4).max(4);
        self.height = (rows * 25).min(1600) as u16;
        let mut i: usize = 0;
        for n in self.nodes.iter_mut().filter(|n| n.kind == Kind::Shortcut) {
            n.x = ((i % 4) * 2500) as u16;
            n.y = ((i / 4) * 10000 / rows) as u16;
            n.width = 2200;
            n.height = 9000 / rows as u16;
            i += 1;
        }
    }
    pub fn add_shortcuts(&mut self, paths: &[String]) {
        for path in paths {
            if self.nodes.len() >= 256 {
                break;
            }
            if self
                .nodes
                .iter()
                .any(|n| n.kind == Kind::Shortcut && n.target.eq_ignore_ascii_case(path))
            {
                continue;
            }
            let name = std::path::Path::new(path)
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let index = self.nodes.len();
            let mut n = Node::new(Kind::Shortcut);
            n.title = name;
            n.target = path.clone();
            n.x = (index % 4) as u16 * 2500;
            n.y = ((index / 4) % 4) as u16 * 2500;
            self.nodes.push(n);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_studio_validates_coordinates_and_duplicates() {
        let mut s = Studio::default();
        s.nodes.push(Node::new(Kind::Clock));
        assert!(s.validate().is_ok());
        s.nodes.push(s.nodes[0].clone());
        assert!(s.validate().is_err());
        s.nodes.pop();
        s.nodes[0].width = 10001;
        assert!(s.validate().is_err());
    }
    #[test]
    fn desktop_shortcuts_preserve_targets_without_moving_files() {
        let mut s = Studio::default();
        s.add_shortcuts(&[
            "C:/Desktop/readme.lnk".into(),
            "C:/Desktop/readme.lnk".into(),
        ]);
        assert_eq!(s.nodes.len(), 1);
        assert_eq!(s.nodes[0].target, "C:/Desktop/readme.lnk");
        assert_eq!(s.nodes[0].title, "readme");
        assert!(s.validate().is_ok());
    }
}

/// 将快捷方式移动到当前网格中的插入位置。
pub fn move_shortcut(studio: &mut Studio, id: &str, target: usize) {
    let positions: Vec<_> = studio
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.kind == Kind::Shortcut)
        .map(|(i, _)| i)
        .collect();
    let Some(ordinal) = positions.iter().position(|&i| studio.nodes[i].id == id) else {
        return;
    };
    let from = positions[ordinal];
    let to = positions.get(target).copied().unwrap_or(studio.nodes.len());
    let node = studio.nodes.remove(from);
    studio.nodes.insert(
        to.saturating_sub(usize::from(to > from))
            .min(studio.nodes.len()),
        node,
    );
}

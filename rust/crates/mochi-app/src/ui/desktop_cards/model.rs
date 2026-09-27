//! 用于持久化桌面卡片协议的界面辅助函数。
//!
//! 权威实现位于 `mochi_core::desktop_cards`。这里刻意只保留轻量的
//! 重新导出；管理器编辑的 `DesktopConfig` 与桌面窗口和持久化层使用的
//! 是同一个类型。

pub use mochi_core::desktop_cards::{
    Card as DesktopCard, DesktopConfig, Interaction, Module as ModuleKind, ModuleOption,
    Page as DesktopPage,
};

pub type PageRoute = Interaction;

pub trait ModuleExt {
    fn default_limit(self) -> usize;
    fn requires_source(self) -> bool;
}

impl ModuleExt for ModuleKind {
    fn default_limit(self) -> usize {
        match self {
            ModuleKind::Home | ModuleKind::QuickNote | ModuleKind::Pomodoro => 4,
            ModuleKind::Inbox | ModuleKind::Ai | ModuleKind::Automations => 5,
            _ => 8,
        }
    }

    fn requires_source(self) -> bool {
        matches!(
            self,
            ModuleKind::Document | ModuleKind::Base | ModuleKind::Canvas
        )
    }
}

pub trait PageExt {
    fn has_source(&self) -> bool;
}

impl PageExt for DesktopPage {
    fn has_source(&self) -> bool {
        self.module.requires_source()
    }
}

pub fn fields_for(module: ModuleKind) -> Vec<ModuleOption> {
    module
        .options()
        .into_iter()
        .filter(|option| module != ModuleKind::Schedule || option.key != "completed")
        .collect()
}

pub const ROUTES: [Interaction; 2] = [Interaction::Smart, Interaction::OpenOnly];

pub fn route_label(route: Interaction) -> &'static str {
    match route {
        Interaction::Smart => "智能操作",
        Interaction::OpenOnly => "仅跳转墨池",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardSizePreset {
    Compact,
    Medium,
    Spacious,
}

impl CardSizePreset {
    pub const ALL: [Self; 3] = [Self::Compact, Self::Medium, Self::Spacious];

    pub fn size(self) -> (u32, u32) {
        match self {
            Self::Compact => (400, 400),
            Self::Medium => (480, 480),
            Self::Spacious => (560, 600),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Compact => "紧凑 400 × 400",
            Self::Medium => "标准 480 × 480",
            Self::Spacious => "宽松 560 × 600",
        }
    }

    pub fn matches(self, width: u32, height: u32) -> bool {
        let (target_width, target_height) = self.size();
        width == target_width && height == target_height
    }
}

pub fn interactive(module: ModuleKind) -> bool {
    matches!(
        module,
        ModuleKind::Ai | ModuleKind::Pomodoro | ModuleKind::Custom | ModuleKind::Clock
    )
}

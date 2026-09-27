//! 定义通知类别、设置项和通知状态标签。
use serde::{Deserialize, Serialize};

pub const CAPACITY: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Category {
    Document,
    Assistant,
    Schedule,
    System,
    Automation,
}

impl Category {
    pub const ALL: [Self; 5] = [
        Self::Document,
        Self::Assistant,
        Self::Schedule,
        Self::System,
        Self::Automation,
    ];

    pub fn setting_key(self) -> &'static str {
        match self {
            Self::Document => "notifications.documentEnabled",
            Self::Assistant => "notifications.assistantEnabled",
            Self::Schedule => "notifications.scheduleEnabled",
            Self::System => "notifications.systemEnabled",
            Self::Automation => "notifications.automationEnabled",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Document => "文档",
            Self::Assistant => "AI",
            Self::Schedule => "日程",
            Self::System => "系统",
            Self::Automation => "自动化",
        }
    }

    pub fn for_status(message: &str) -> Self {
        if ["自动化", "工作流"].iter().any(|s| message.contains(s)) {
            Self::Automation
        } else if ["AI", "Agent", "模型", "会话", "审批"]
            .iter()
            .any(|s| message.contains(s))
        {
            Self::Assistant
        } else if ["日程", "提醒", "番茄钟"]
            .iter()
            .any(|s| message.contains(s))
        {
            Self::Schedule
        } else if ["文档", "文件", "保存", "导入", "导出", "表格", "知识库"]
            .iter()
            .any(|s| message.contains(s))
        {
            Self::Document
        } else {
            Self::System
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    pub id: u64,
    pub category: Category,
    pub title: String,
    pub message: String,
    pub created_at: i64,
    pub read: bool,
    pub occurrences: u32,
}

#[derive(Debug, Default)]
pub struct History {
    pub entries: Vec<Notice>,
    next_id: u64,
}

impl History {
    pub fn restore(raw: Option<&str>) -> Self {
        let mut entries: Vec<Notice> = raw
            .filter(|s| s.len() <= 2 * 1024 * 1024)
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();
        entries.sort_by_key(|n| std::cmp::Reverse(n.created_at));
        let mut ids = std::collections::HashSet::new();
        entries.retain(|n| !n.message.trim().is_empty() && n.id < u64::MAX && ids.insert(n.id));
        entries.truncate(CAPACITY);
        for n in &mut entries {
            n.title = bounded(&n.title, 80);
            n.message = bounded(&n.message, 2048);
            n.occurrences = n.occurrences.max(1);
        }
        let next_id = entries.iter().map(|n| n.id).max().unwrap_or(0) + 1;
        Self { entries, next_id }
    }

    pub fn push(
        &mut self,
        category: Category,
        title: &str,
        message: &str,
        now: i64,
    ) -> Option<u64> {
        if message.trim().is_empty() {
            return None;
        }
        let message = bounded(message.trim(), 2048);
        let title = bounded(title.trim(), 80);
        // 只合并连续且短时间重复的同源消息；已读后再次发生仍需提醒。
        if let Some(last) = self.entries.first_mut().filter(|n| {
            n.category == category
                && n.title == title
                && n.message == message
                && (0..=5000).contains(&now.saturating_sub(n.created_at))
        }) {
            last.occurrences = last.occurrences.saturating_add(1);
            last.created_at = now;
            last.read = false;
            return Some(last.id);
        }
        let id = self.next_id.max(1);
        self.next_id = id.saturating_add(1);
        self.entries.insert(
            0,
            Notice {
                id,
                category,
                title,
                message,
                created_at: now,
                read: false,
                occurrences: 1,
            },
        );
        self.entries.truncate(CAPACITY);
        Some(id)
    }

    pub fn unread(&self) -> usize {
        self.entries.iter().filter(|n| !n.read).count()
    }

    pub fn mark_read(&mut self, id: u64) -> bool {
        self.entries
            .iter_mut()
            .find(|n| n.id == id)
            .is_some_and(|n| {
                let changed = !n.read;
                n.read = true;
                changed
            })
    }

    pub fn mark_all_read(&mut self) -> bool {
        let changed = self.unread() > 0;
        for n in &mut self.entries {
            n.read = true;
        }
        changed
    }
}

fn bounded(s: &str, limit: usize) -> String {
    let mut chars = s.chars();
    let mut out: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        out.push('…');
    }
    out
}

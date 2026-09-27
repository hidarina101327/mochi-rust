//! 产品知识由 extract-product-knowledge.js 从 TS 提取，不在 Rust 中维护副本。

use std::sync::LazyLock;

use serde::Deserialize;

const RAW: &str = include_str!("../assets/product-knowledge.json");

#[derive(Debug, Deserialize)]
pub struct DocTopic {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub body: String,
}

#[derive(Debug, Deserialize)]
struct Knowledge {
    brief: String,
    topics: Vec<DocTopic>,
}

static KNOWLEDGE: LazyLock<Knowledge> = LazyLock::new(|| {
    let mut knowledge: Knowledge = serde_json::from_str(RAW)
        .expect("产品知识 JSON 损坏——assets/product-knowledge.json 需重新生成");
    if let Some(topic) = knowledge
        .topics
        .iter_mut()
        .find(|t| t.id == "multidimensional-base")
    {
        topic.body.push_str("\n\n## Rust 原生版视图自动化\n\n视图工具栏的「自动化」可配置触发器、条件和多动作，试跑、保存草稿、启停并查看日志；规则随 .mcb 保存，持续授权只在本机。应用和工作区打开时执行，关闭表格标签不影响运行；退出应用后不执行。Electron 能保留配置，但暂未提供此执行器。\n\n");
        topic
            .body
            .push_str(crate::ai::tools::base_automation_tools::GUIDE);
    }
    if let Some(topic) = knowledge.topics.iter_mut().find(|t| t.id == "ai-system") {
        topic.body.push_str("\n\n## Rust 原生版内置脚本批处理\n\n通用助手默认具备 script_environment / script_run；自定义 Agent 可以选择这两个工具。内置独立 Python，无需用户安装，支持 PowerShell/CMD。脚本需要“执行命令”权限和逐次审批，结果写回原会话。当前不会在结果保存后自动发起新一轮模型请求；可要求 Agent 继续核验。\n\n");
        topic.body.push_str(crate::script::GUIDE);
    }
    knowledge
});

/// 精简产品简报，注入每个 Agent 的系统提示词。
pub fn brief() -> &'static str {
    &KNOWLEDGE.brief
}

pub fn topics() -> &'static [DocTopic] {
    &KNOWLEDGE.topics
}

pub fn topic(id: &str) -> Option<&'static DocTopic> {
    KNOWLEDGE.topics.iter().find(|t| t.id == id)
}

/// 主题相关度：命中 id/标题/摘要记 10 分，正文每命中一次加 1 分（最多 8 分）。
/// 对齐 TS 的 `scoreTopic`——标题命中远比正文里提一嘴重要。
pub fn score(topic: &DocTopic, needle: &str) -> u32 {
    let mut score = 0;
    let head = format!("{} {} {}", topic.id, topic.title, topic.summary).to_lowercase();
    if head.contains(needle) {
        score += 10;
    }
    score += count_occurrences(&topic.body.to_lowercase(), needle).min(8);
    score
}

/// 不重叠计数，等价于 JS 的 `s.split(needle).length - 1`。
fn count_occurrences(haystack: &str, needle: &str) -> u32 {
    if needle.is_empty() {
        return 0;
    }
    haystack.matches(needle).count() as u32
}

/// 按关键词取最相关的前 3 个主题。无命中返回空。
pub fn search(query: &str) -> Vec<&'static DocTopic> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(&'static DocTopic, u32)> = topics()
        .iter()
        .map(|t| (t, score(t, &needle)))
        .filter(|(_, s)| *s > 0)
        .collect();
    // 稳定排序：同分时保持声明顺序，与 TS 的 Array.sort 一致
    scored.sort_by_key(|b| std::cmp::Reverse(b.1));
    scored.into_iter().take(3).map(|(t, _)| t).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knowledge_loads_and_covers_every_declared_topic() {
        assert!(brief().contains("墨池"), "简报里总该提到产品名");
        // 工具定义里的 topic 枚举必须与这里一一对应，否则模型会点名一个不存在的主题
        let definition =
            crate::ai::tools::definitions::find("mochi_docs").expect("缺 mochi_docs 定义");
        let declared: Vec<&str> = definition.function.parameters["properties"]["topic"]["enum"]
            .as_array()
            .expect("topic 应是枚举")
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let actual: Vec<&str> = topics().iter().map(|t| t.id.as_str()).collect();
        assert_eq!(declared, actual, "工具定义里的主题枚举与说明书对不上");
    }

    #[test]
    fn every_topic_has_content() {
        for item in topics() {
            assert!(!item.title.trim().is_empty(), "{} 缺标题", item.id);
            assert!(!item.summary.trim().is_empty(), "{} 缺摘要", item.id);
            assert!(item.body.len() > 50, "{} 正文太短，像是抽取出错", item.id);
        }
    }

    #[test]
    fn lookup_by_id() {
        assert_eq!(topic("shortcuts").unwrap().id, "shortcuts");
        assert!(topic("不存在的主题").is_none());
    }

    #[test]
    fn title_hits_outrank_body_mentions() {
        let settings = topic("settings").unwrap();
        let overview = topic("product-overview").unwrap();
        assert!(score(settings, "设置") > 10, "标题命中该拿到 10 分以上");
        assert!(score(settings, "设置") > score(overview, "设置"));
    }

    #[test]
    fn body_match_count_is_capped() {
        let topic = topics().iter().max_by_key(|t| t.body.len()).unwrap();
        assert!(
            score(topic, "的") <= 18,
            "正文分最多 8 分，加标题 10 分封顶"
        );
    }

    #[test]
    fn search_returns_at_most_three_topics() {
        let hits = search("设置");
        assert!(!hits.is_empty());
        assert!(hits.len() <= 3);
    }

    #[test]
    fn search_without_a_match_is_empty() {
        assert!(search("绝无可能出现的关键词zzz").is_empty());
        assert!(search("   ").is_empty());
    }
}

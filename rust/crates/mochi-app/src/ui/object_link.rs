//! 工作区文档、日程项目和 AI 链接共用同一套显示规则。
use mochi_core::{
    ai::locator::Locator,
    mochi_url::{self, ResourceKind},
};

#[derive(Clone, Debug, PartialEq)]
pub struct ObjectLink {
    pub target: String,
    pub label: String,
    pub detail: String,
    pub kind: String,
    pub text_style: bool,
}

impl ObjectLink {
    pub fn parse(source: &str) -> Option<Self> {
        let source = source.trim();
        let (label, target) = if let Some((label, target)) =
            source.strip_prefix('[').and_then(|s| s.split_once("]("))
        {
            (Some(label), target.strip_suffix(')')?)
        } else {
            (None, source)
        };
        let reference = mochi_core::object_reference::ObjectReference::parse(target)?;
        let text_style = target
            .split('?')
            .nth(1)
            .is_some_and(|query| query.split('&').any(|pair| pair == "view=text"));
        if let Some(locator) = Locator::parse(target) {
            return Some(Self {
                target: target.into(),
                label: label
                    .filter(|s| !s.is_empty())
                    .unwrap_or(if locator.title.is_empty() {
                        "AI 问答"
                    } else {
                        &locator.title
                    })
                    .into(),
                detail: locator.snippet,
                kind: "AI 问答".into(),
                text_style,
            });
        }
        if reference.kind == mochi_core::object_reference::ObjectKind::Block {
            let path = reference.path.as_deref().unwrap_or_default();
            let block_id = reference.block_id.as_deref().unwrap_or_default();
            let default_label = reference.display_label();
            let label = label
                .or(reference.label.as_deref())
                .map(str::to_owned)
                .unwrap_or(default_label);
            return Some(Self {
                target: target.into(),
                label,
                detail: format!("{path} · {block_id}"),
                kind: "文档块".into(),
                text_style,
            });
        }
        if target
            .split_once('?')
            .is_some_and(|(head, _)| head.eq_ignore_ascii_case("mochi://ai-session"))
        {
            let reference = mochi_core::object_reference::ObjectReference::parse(target)?;
            return Some(Self {
                target: target.into(),
                label: label
                    .or(reference.label.as_deref())
                    .unwrap_or("AI 会话")
                    .into(),
                detail: "打开会话".into(),
                kind: "AI 会话".into(),
                text_style,
            });
        }
        let resource = mochi_url::parse_mochi_resource_url(target)?;
        let kind = match resource.kind {
            ResourceKind::Directory => "目录",
            ResourceKind::Task => "任务",
            ResourceKind::Event => "日程",
            ResourceKind::Project => "项目",
            ResourceKind::AiSession => "AI 会话",
            _ => {
                if resource.table_id.is_some() {
                    "多维表格记录"
                } else {
                    "文档"
                }
            }
        };
        let default_label = resource.path.rsplit('/').next().unwrap_or(&resource.path);
        let label = label
            .or(resource.label.as_deref())
            .filter(|s| !s.is_empty())
            .unwrap_or(default_label)
            .to_owned();
        Some(Self {
            target: target.into(),
            label,
            detail: resource.path,
            kind: kind.into(),
            text_style,
        })
    }

    pub fn with_presentation(&self, text: bool) -> String {
        let (head, query) = self.target.split_once('?').unwrap();
        let mut fields = query
            .split('&')
            .filter(|pair| pair.split('=').next() != Some("view"))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let title_key = if head.eq_ignore_ascii_case("mochi://ai-locate") {
            "title"
        } else {
            "label"
        };
        if !fields
            .iter()
            .any(|pair| pair.split('=').next() == Some(title_key))
        {
            fields.push(format!(
                "{title_key}={}",
                mochi_url::form_encode(&self.label)
            ));
        }
        fields.push(format!("view={}", if text { "text" } else { "card" }));
        format!("{head}?{}", fields.join("&"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn presentation_keeps_reference_identity_and_handles_legacy_ai_links() {
        let link =
            ObjectLink::parse("[参考文档](mochi://open?path=notes%2Fa.mc&future=x%2fy)").unwrap();
        assert_eq!(link.label, "参考文档");
        let text = link.with_presentation(true);
        assert!(text.contains("future=x%2fy"));
        assert!(ObjectLink::parse(&text).unwrap().text_style);
        let ai = ObjectLink::parse("mochi://ai-locate?session=s&message=m&title=问答&view=text")
            .unwrap();
        assert!(ai.text_style);
        assert_eq!(ai.kind, "AI 问答");
        assert!(ObjectLink::parse("https://example.com").is_none());
        assert_eq!(
            ObjectLink::parse("MOCHI://AI-SESSION?session=s&label=test")
                .unwrap()
                .kind,
            "AI 会话"
        );
        assert_eq!(
            ObjectLink::parse("MOCHI://AI-LOCATE?session=s&message=m&view=text")
                .unwrap()
                .kind,
            "AI 问答"
        );
    }

    #[test]
    fn block_links_render_as_document_block_cards_and_keep_the_id() {
        let link = ObjectLink::parse(
            "mochi://block?path=notes%2Fplan.md&id=block_00000000-0000-4000-8000-000000000001&label=发布计划",
        )
        .unwrap();
        assert_eq!(link.kind, "文档块");
        assert_eq!(link.label, "发布计划");
        assert!(link
            .detail
            .contains("block_00000000-0000-4000-8000-000000000001"));
        let text = link.with_presentation(true);
        assert!(text.contains("id=block_00000000-0000-4000-8000-000000000001"));
        assert!(text.ends_with("&view=text"));
        assert!(ObjectLink::parse(&text).unwrap().text_style);
    }
}

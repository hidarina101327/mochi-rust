//! 与实现单元测试分开的产品级验收测试。
use mochi_blocks::{model::Document, slash};

const SOURCE: &str = "# 发布计划\r\n\r\n产品将在十月份左右上线。\r\n\r\n重复段落🙂\r\n\r\n重复段落🙂\r\n\r\n- [ ] 审核发布说明\r\n- [ ] 同步部署清单\r\n\r\n```rust\r\nlet literal = \"<!-- mochi:block invalid -->\";\r\nprintln!(\"/rewrite\");\r\n```\r\n\r\n| 项目 | 状态 |\r\n| --- | --- |\r\n| 发布 | 准备中 |\r\n";

#[test]
fn references_and_pending_edits_survive_roundtrip_and_unrelated_changes() -> anyhow::Result<()> {
    let original = Document::import("acceptance", SOURCE)?;
    let block = original
        .blocks()
        .iter()
        .find(|b| b.content.contains("产品将在"))
        .expect("release paragraph");
    let id = block.id.clone();
    let before = block.content.clone();
    let persisted = original.render_with_ids();
    let reopened = Document::import("acceptance", &persisted)?;
    assert_eq!(
        reopened.blocks().iter().map(|b| &b.id).collect::<Vec<_>>(),
        original.blocks().iter().map(|b| &b.id).collect::<Vec<_>>()
    );
    let duplicates = reopened
        .blocks()
        .iter()
        .filter(|b| b.content.contains("重复段落🙂"))
        .collect::<Vec<_>>();
    assert_eq!(duplicates.len(), 2);
    assert_ne!(duplicates[0].id, duplicates[1].id);
    let unrelated_id = duplicates[0].id.clone();
    let unrelated_before = duplicates[0].content.clone();
    let concurrently_edited =
        reopened.replace_block(&unrelated_id, &unrelated_before, "仅修改另一个块🙂")?;
    let approved =
        concurrently_edited.replace_block(&id, &before, "产品计划于 10 月 15 日正式上线。")?;
    assert_eq!(
        approved
            .blocks()
            .iter()
            .find(|b| b.id == id)
            .unwrap()
            .content,
        "产品计划于 10 月 15 日正式上线。"
    );
    assert!(approved
        .blocks()
        .iter()
        .any(|b| b.content == "仅修改另一个块🙂"));
    assert!(approved
        .replace_block(&id, &before, "过期 AI 建议不得覆盖新内容")
        .is_err());
    assert!(approved
        .render_with_ids()
        .contains("let literal = \"<!-- mochi:block invalid -->\";"));
    Ok(())
}

#[test]
fn slash_ai_actions_request_proposals_instead_of_writing_generated_text() -> anyhow::Result<()> {
    for command in [
        slash::Command::Ai,
        slash::Command::Rewrite,
        slash::Command::Summarize,
    ] {
        let source = format!("原块必须保留\n\n/{}", command.name());
        let trigger = slash::detect(&source, source.len()).unwrap();
        let action = slash::complete(&source, &trigger, command)?;
        assert_eq!(action.ai_intent, Some(command));
        assert!(action.replacement.is_empty());
        let mut after = source;
        after.replace_range(action.range, &action.replacement);
        assert_eq!(after, "原块必须保留\n\n");
    }
    Ok(())
}

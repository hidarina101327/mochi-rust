use super::*;
#[test]
fn cards_preserve_the_original_url_and_never_escape_code_fences() {
    let url = "mochi://ai-locate?session=s&message=m&title=%E6%A0%87%E9%A2%98&snippet=hello";
    let parsed = parse_ranged(url);
    assert!(matches!(&parsed.blocks[0].block,Block::AiLocator(l) if l.title=="标题"));
    assert_eq!(&url[parsed.blocks[0].start..parsed.blocks[0].end], url);
    for source in [
        format!("~~~text\n{url}\n~~~"),
        format!("````markdown\n```\n{url}\n```\n````"),
    ] {
        let parsed = parse_ranged(&source);
        assert_eq!(parsed.blocks.len(), 1);
        assert!(
            matches!(&parsed.blocks[0].block,Block::Code{lines,..} if lines.iter().any(|l|l==url))
        );
        let range = super::super::code_blocks::language_range(&source, 0).unwrap();
        let mut edited = source.clone();
        edited.replace_range(range, "rust");
        assert!(edited.starts_with("~~~rust") || edited.starts_with("````rust"));
    }
}

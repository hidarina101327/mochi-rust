use super::*;
#[test]
fn active_block_crlf_and_soft_spaces_keep_exact_source_ranges() {
    let source = "  first words words 中文 😀\r\nsecond  words and tail\r\nthird line";
    let parsed = parse_ranged(source);
    let laid = layout_live(&parsed.blocks, source, Some(0), 65.0, &|_| None);
    for line in &laid.lines {
        if let Some((start, end)) = line.source {
            assert_eq!(
                line.runs
                    .iter()
                    .map(|r| r.text.as_str())
                    .collect::<String>(),
                &source[start..end]
            );
        }
    }
}
#[test]
fn rendered_paragraph_wrap_offsets_include_skipped_spaces() {
    let value = "first words words 中文 and tail";
    let parsed = parse_ranged(value);
    let laid = layout_live(&parsed.blocks, value, None, 65.0, &|_| None);
    let chars = value.chars().collect::<Vec<_>>();
    for line in &laid.lines {
        let drawn = line
            .runs
            .iter()
            .map(|r| r.text.as_str())
            .collect::<String>();
        assert_eq!(
            drawn,
            chars
                .iter()
                .skip(line.visible_start)
                .take(drawn.chars().count())
                .collect::<String>()
        );
    }
}

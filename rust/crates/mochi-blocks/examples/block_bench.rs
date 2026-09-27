//! 手动的规模探测；计时结果只作观察参考，不是脆弱的 CI 断言。
use mochi_blocks::model::Document;
use std::time::Instant;
fn main() -> anyhow::Result<()> {
    let count = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "2000".into())
        .parse::<usize>()?
        .min(10000);
    let source = (0..count)
        .map(|n| format!("第 {n} 段：验证中文🙂段落的稳定 ID 和修改定位。\n\n"))
        .collect::<String>();
    let start = Instant::now();
    let document = Document::import("scale.md", source)?;
    let imported = start.elapsed();
    let start = Instant::now();
    let source = document.save()?;
    let saved = start.elapsed();
    let start = Instant::now();
    let document = Document::import("scale.md", source)?;
    let reopened = start.elapsed();
    let block = &document.blocks()[count / 2];
    let start = Instant::now();
    let next = document.replace_block(&block.id, &block.content, "中间块改动，保持其他块不变。")?;
    let edited = start.elapsed();
    assert_eq!(document.blocks().len(), next.blocks().len());
    assert_eq!(
        document.blocks().iter().map(|b| &b.id).collect::<Vec<_>>(),
        next.blocks().iter().map(|b| &b.id).collect::<Vec<_>>()
    );
    println!("{count} blocks | import {imported:?} | save {saved:?} | reopen {reopened:?} | edit {edited:?}");
    Ok(())
}

//! 仅使用合成数据进行负载测试。绝不打开、迁移或修改用户工作区。
use anyhow::{ensure, Context, Result};
use mochi_core::{
    metadata_index::MetadataIndexService,
    search::{SearchOptions, SearchService},
};
use std::{
    fs,
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

fn main() -> Result<()> {
    let count: usize = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "5000".into())
        .parse()?;
    ensure!(
        (100..=20_000).contains(&count),
        "note count must be 100..20000"
    );
    let root = std::env::temp_dir().join(format!(
        "mochi-scale-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    fs::create_dir(&root)?;
    let notes = root.join("知识库").join("规模验收");
    fs::create_dir_all(&notes)?;
    let mut bytes = 0;
    for i in 0..count {
        let body = format!("---\ntags: [原生, 验收]\n---\n# 笔记{i:05}\n\nneedle{i:05} 中文检索🙂\n\n[[笔记{:05}]]\n\n{}", (i + 1) % count,
            "这是隔离生成的笔记正文，测试中文、链接、索引和搜索。native workspace benchmark.\n".repeat(40));
        bytes += body.len();
        fs::write(notes.join(format!("笔记{i:05}.md")), body)?;
    }
    eprintln!("generated {count} notes in {}", root.display());
    let index = Arc::new(MetadataIndexService::new(&root));
    index.open()?;
    let start = Instant::now();
    let indexed = index.run_full_index()?;
    let cold_ms = start.elapsed().as_millis();
    ensure!(indexed == count, "cold index count: {indexed}");
    let start = Instant::now();
    ensure!(
        index.run_full_index()? == 0,
        "warm index unexpectedly rewrote notes"
    );
    let warm_ms = start.elapsed().as_millis();
    let search = SearchService::new(&root, index.clone());
    let mut queries = Vec::new();
    for (query, regex, expected) in [
        (format!("needle{:05}", count - 1), false, 1),
        ("中文检索".into(), false, count),
        ("needle[0-9]{5}".into(), true, count),
    ] {
        let result = search.search(
            &query,
            &SearchOptions {
                use_regex: regex,
                ..Default::default()
            },
        );
        ensure!(result.error.is_none(), "search error: {:?}", result.error);
        // 范围较广的 FTS 查询可能会有意限制候选项数量；正则表达式则应扫描所有文件。
        if regex || expected == 1 {
            ensure!(
                result.stats.total_files == expected,
                "query {query}: {} != {expected}",
                result.stats.total_files
            );
        }
        queries.push(serde_json::json!({"query":query,"regex":regex,"durationMs":result.stats.duration_ms,"matchedFiles":result.stats.total_files,"shownGroups":result.groups.len(),"truncated":result.stats.truncated}));
    }
    let first = notes.join("笔记00000.md");
    let start = Instant::now();
    let backlinks = index.get_backlinks(&first)?;
    ensure!(backlinks.len() == 1, "ring backlink missing");
    let backlinks_ms = start.elapsed().as_millis();
    let start = Instant::now();
    for i in 0..20 {
        let path = notes.join(format!("笔记{i:05}.md"));
        let mut content = fs::read_to_string(&path)?;
        content.push_str("\n增量更新验证 marker_updated\n");
        fs::write(&path, content)?;
        ensure!(index.index_single_file(&path)?, "single-file update failed");
    }
    let incremental_ms = start.elapsed().as_millis();
    let result = search.search("marker_updated", &SearchOptions::default());
    ensure!(
        result.stats.total_files == 20,
        "incremental search mismatch"
    );
    let mut batch = Vec::new();
    for i in 20..40 {
        let path = notes.join(format!("笔记{i:05}.md"));
        let mut content = fs::read_to_string(&path)?;
        content.push_str("\n批量更新验证 marker_batched\n");
        fs::write(&path, content)?;
        batch.push(path);
    }
    let start = Instant::now();
    ensure!(index.index_files(&batch)? == 20, "batch update mismatch");
    let batch_ms = start.elapsed().as_millis();
    ensure!(
        search
            .search("marker_batched", &SearchOptions::default())
            .stats
            .total_files
            == 20,
        "batch search mismatch"
    );
    let report = serde_json::json!({"scenario":"synthetic notes; isolated temp workspace; core index/search only", "root":root, "notes":count,"sourceBytes":bytes,"coldIndexMs":cold_ms,"warmIndexMs":warm_ms,"backlinksMs":backlinks_ms,"twentyIncrementalUpdatesMs":incremental_ms,"twentyBatchedUpdatesMs":batch_ms,"queries":queries});
    let report_path = root.join("benchmark.json");
    fs::write(&report_path, serde_json::to_string_pretty(&report)?)
        .context("write benchmark report")?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    // 保留带名称的临时数据集和报告，便于复现测试结果。
    Ok(())
}

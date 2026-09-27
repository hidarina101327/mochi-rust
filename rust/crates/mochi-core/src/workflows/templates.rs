//! 定义可直接创建的内置工作流模板。
use super::*;
use serde_json::json;

pub fn daily_web() -> Workflow {
    let mut f = chain(&["start", "http", "ai", "file_write", "notify", "end"]);
    f.name = "每日网页整理".into();
    f.description = "抓取网页 → AI 清洗整理 → 写入指定文件夹 → 通知".into();
    f.trigger = Trigger::Daily {
        time: "09:00".into(),
        utc_offset_minutes: 480,
    };
    f.defaults = json!({"url":"https://example.com","output_dir":"知识库/日报"});
    f.input_schema = json!({"type":"object","required":["url","output_dir"],"properties":{"url":{"type":"string"},"output_dir":{"type":"string"}}});
    f.nodes[1].inputs = json!({"url":"$input.url"});
    f.nodes[2].inputs = json!({"prompt":"将以下网页整理为中文 Markdown 日报，去除导航和广告，不捏造来源未提供的信息。标注来源 {{ $nodes.n1.output.url }}。\n网页内容：\n{{ $nodes.n1.output.text }}"});
    f.nodes[3].inputs = json!({"path":"{{ $input.output_dir }}/{{ $run.date }}.md","content":"$nodes.n2.output.text"});
    f.nodes[4].inputs =
        json!({"title":"日报已生成","message":"已写入 {{ $nodes.n3.output.path }}"});
    f.nodes[5].inputs = json!({"path":"$nodes.n3.output.path"});
    f
}
pub fn weekly_documents() -> Workflow {
    let mut f = chain(&[
        "start",
        "folder_list",
        "file_read",
        "ai",
        "chart",
        "file_write",
        "file_write",
        "notify",
        "end",
    ]);
    f.name = "文档周报与统计图".into();
    f.description = "读取文件夹内的 Markdown 文档，归纳周报并生成 SVG 图表".into();
    f.trigger = Trigger::Weekly {
        time: "18:00".into(),
        weekdays: vec![5],
        utc_offset_minutes: 480,
    };
    f.defaults = json!({"source_dir":"知识库/日报","output_dir":"知识库/周报"});
    f.input_schema = json!({"type":"object","required":["source_dir","output_dir"],"properties":{"source_dir":{"type":"string"},"output_dir":{"type":"string"}}});
    f.nodes[1].inputs = json!({"path":"$input.source_dir"});
    f.nodes[1].config = json!({"extensions":["md","mc"],"modified_within_days":7});
    f.nodes[2].for_each = Some("$nodes.n1.output.items".into());
    f.nodes[2].inputs = json!({"path":"$item.path"});
    f.nodes[3].config["response_format"] = json!("json");
    f.nodes[3].inputs = json!({"prompt":"根据以下文档生成周报。返回 JSON：report（Markdown 字符串）、labels（统计项目字符串数组）、values（同长度数字数组）。统计必须来自原文；资料为空时 report 说明无资料，labels=[\"文档数\"]，values=[0]。资料：\n{{ $nodes.n2.output.items }}"});
    f.nodes[4].inputs = json!({"title":"本周统计","labels":"$nodes.n3.output.data.labels","values":"$nodes.n3.output.data.values"});
    f.nodes[5].inputs = json!({"path":"{{ $input.output_dir }}/{{ $run.week }}.svg","content":"$nodes.n4.output.svg"});
    f.nodes[5].config = json!({"overwrite":true});
    f.nodes[6].inputs = json!({"path":"{{ $input.output_dir }}/{{ $run.week }}.md","content":"{{ $nodes.n3.output.data.report }}\n\n![本周统计]({{ $run.week }}.svg)"});
    f.nodes[6].config = json!({"overwrite":true});
    f.nodes[7].inputs =
        json!({"title":"周报已更新","message":"已写入 {{ $nodes.n6.output.path }}"});
    f.nodes[8].inputs = json!({"report":"$nodes.n6.output.path","chart":"$nodes.n5.output.path"});
    f
}
fn chain(kinds: &[&str]) -> Workflow {
    let mut f = Workflow::blank();
    f.nodes.clear();
    f.edges.clear();
    for (i, kind) in kinds.iter().enumerate() {
        f.nodes.push(Node::new(
            &format!("n{i}"),
            kind,
            60. + i as f32 * 260.,
            100.,
        ));
        if i > 0 {
            f.edges.push(Edge {
                id: format!("e{i}"),
                source: format!("n{}", i - 1),
                target: format!("n{i}"),
                source_handle: None,
            });
        }
    }
    f
}

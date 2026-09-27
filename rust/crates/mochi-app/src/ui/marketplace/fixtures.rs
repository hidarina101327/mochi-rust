//! 离线测试数据，不会放入发布版本，也不会用作市场内容。
use super::*;
pub fn catalog() -> Catalog {
    let titles = [
        "每日灵感手记",
        "项目进度管理",
        "创意画布",
        "代码审查工作流",
        "每周复盘模版",
        "研究助手 Agent",
        "个人知识库",
        "GitHub 插件",
    ];
    let summaries = [
        "记录灵感与思考，让零散笔记逐渐成为清晰的知识。",
        "从任务、里程碑到进度，在一个表格中组织项目。",
        "在自由画布上连接想法、文档和参考资料。",
        "整理代码变更，生成结构化的审查清单。",
        "回顾本周成果，梳理问题并规划下一步。",
        "把研究问题拆成步骤，收集资料并整理结论。",
        "用清晰的目录结构开启自己的知识空间。",
        "将代码仓库信息带入你的日常工作区。",
    ];
    let packages = (0..20).map(|i| {
        let category = Category::ALL[i % 8];
        let kind = if i % 8 == 3 { "workflow" } else if i % 8 == 7 { "plugin" } else { "template" };
        Package { id: format!("sample-{i}"), kind: kind.into(), category, title: titles[i % 8].into(), version: "1.0.0".into(), summary: summaries[i % 8].into(), description: format!("{}\n\n适用场景\n为学习、工作和个人项目提供一个清晰的起点。\n\n使用方式\n下载独立资源包，或复制安装命令，在目标工作区目录中运行。安装完成后即可开始使用，并根据自己的习惯调整内容。\n\n{}", summaries[i % 8], "支持整理信息、保存思路，并持续完善你的工作方式。\n".repeat(8)), image: None, official: i % 3 != 2, author: if i % 3 != 2 { "Mochi" } else { "Community" }.into(), asset: format!("sample-{i}-{kind}.zip"), sha256: "a".repeat(64), download_url: String::new(), size: 1450000 }
    }).collect();
    Catalog {
        release: "market-v1.0.0".into(),
        packages,
    }
}

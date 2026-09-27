//! 内置的可编辑文档起手模板。正文放在普通 MD/MC 资源文件里。
pub struct BuiltinTemplate {
    pub group: &'static str,
    pub name: &'static str,
    pub extension: &'static str,
    pub description: &'static str,
    pub features: &'static [&'static str],
    pub content: &'static str,
}

macro_rules! template {
    ($group:literal, $name:literal, $ext:literal, $file:literal, $description:literal, [$($feature:literal),*]) => {
        BuiltinTemplate {
            group: $group, name: $name, extension: $ext, description: $description,
            features: &[$($feature),*],
            content: include_str!(concat!("../../assets/templates/", $file)),
        }
    };
}

pub const BUILTINS: &[BuiltinTemplate] = &[
    template!(
        "日常",
        "每日复盘",
        "md",
        "daily-review.md",
        "整理今天的进展、精力与收获，留下一步行动。",
        ["待办清单", "复盘表格", "引用"]
    ),
    template!(
        "日常",
        "周计划",
        "md",
        "weekly-plan.md",
        "把本周目标拆成具体安排，周末对照结果复盘。",
        ["目标清单", "周安排", "复盘"]
    ),
    template!(
        "学习",
        "学习笔记",
        "md",
        "study-notes.md",
        "从概念、例子到自测，把听懂变成真正掌握。",
        ["知识表格", "自测清单", "引用"]
    ),
    template!(
        "学习",
        "读书笔记",
        "md",
        "reading-notes.md",
        "保留阅读线索、关键摘录，以及自己的理解与实践。",
        ["摘录", "观点对照", "行动清单"]
    ),
    template!(
        "工作",
        "会议纪要",
        "md",
        "meeting-notes.md",
        "围绕结论记录会议，让每个行动都有负责人和期限。",
        ["议程", "决策表格", "行动清单"]
    ),
    template!(
        "工作",
        "项目计划",
        "md",
        "project-plan.md",
        "定义目标与边界，跟踪里程碑、风险和交付标准。",
        ["里程碑", "风险表格", "验收清单"]
    ),
    template!(
        "日常",
        "晨间日程",
        "mc",
        "morning-plan.mc",
        "用三个优先事项和时间块，安排有余量的一天。",
        ["高亮块", "时间表", "折叠备忘"]
    ),
    template!(
        "日常",
        "月度复盘",
        "mc",
        "monthly-review.mc",
        "回看目标与生活状态，为下个月做有依据的取舍。",
        ["高亮块", "目标对照", "折叠归档"]
    ),
    template!(
        "学习",
        "康奈尔笔记",
        "mc",
        "cornell-notes.mc",
        "按线索、笔记、总结整理课堂，折叠答案主动回忆。",
        ["线索表格", "折叠自测", "高亮总结"]
    ),
    template!(
        "学习",
        "错题复盘",
        "mc",
        "mistake-review.mc",
        "记录错因与关键推导，先独立重做，再展开解析。",
        ["数学公式", "折叠解析", "复习清单"]
    ),
    template!(
        "工作",
        "项目周报",
        "mc",
        "project-report.mc",
        "用一页说明本周成果、风险、协助事项与下周计划。",
        ["状态高亮", "进展表格", "折叠明细"]
    ),
    template!(
        "工作",
        "产品需求",
        "mc",
        "product-brief.mc",
        "从用户问题出发，写清流程、边界和验收条件。",
        ["目标高亮", "需求表格", "验收清单"]
    ),
    template!(
        "研究",
        "论文精读",
        "mc",
        "paper-review.mc",
        "拆解研究问题、方法与证据，形成可复用的阅读判断。",
        ["观点高亮", "证据表格", "折叠摘录"]
    ),
    template!(
        "研究",
        "实验记录",
        "mc",
        "experiment-log.mc",
        "保存假设、控制条件和结果，让实验可以被复现。",
        ["数学公式", "数据表格", "代码卡片"]
    ),
    template!(
        "研究",
        "方案对比",
        "mc",
        "decision-matrix.mc",
        "用加权指标比较候选方案，记录取舍与复查条件。",
        ["评分表格", "数学公式", "决策高亮"]
    ),
    template!(
        "研究",
        "访谈记录",
        "mc",
        "interview-notes.mc",
        "区分受访者原话、观察与推断，提炼待验证的洞察。",
        ["引用摘录", "洞察高亮", "折叠提纲"]
    ),
    template!(
        "开发",
        "技术方案",
        "mc",
        "technical-design.mc",
        "写清设计约束、接口与失败路径，方便评审和实施。",
        ["代码卡片", "方案表格", "风险高亮"]
    ),
    template!(
        "开发",
        "接口文档",
        "mc",
        "api-reference.mc",
        "把请求、响应、错误处理和联调检查放在同一页。",
        ["参数表格", "代码卡片", "折叠示例"]
    ),
    template!(
        "开发",
        "问题排查",
        "mc",
        "incident-log.mc",
        "按时间线收集证据，跟踪假设、修复与回归验证。",
        ["时间线", "折叠日志", "验证清单"]
    ),
    template!(
        "开发",
        "代码笔记",
        "mc",
        "code-notes.mc",
        "记录代码为何这样写、适用边界和最小验证用例。",
        ["代码卡片", "复杂度公式", "折叠推演"]
    ),
    template!(
        "写作",
        "文章策划",
        "mc",
        "article-brief.mc",
        "明确读者、核心论点和素材缺口，再开始写初稿。",
        ["主旨高亮", "素材表格", "写作清单"]
    ),
    template!(
        "写作",
        "内容大纲",
        "mc",
        "content-outline.mc",
        "以问题组织章节，将素材与旁支想法分层收纳。",
        ["多级标题", "折叠素材", "引用"]
    ),
    template!(
        "写作",
        "故事设定",
        "mc",
        "story-bible.mc",
        "梳理人物动机、世界规则与关键冲突，保持叙事一致。",
        ["人物表格", "冲突高亮", "折叠伏笔"]
    ),
    template!(
        "写作",
        "发布检查",
        "mc",
        "publishing-checklist.mc",
        "从事实核对到发布回看，为内容交付保留检查记录。",
        ["待办清单", "发布表格", "折叠记录"]
    ),
];

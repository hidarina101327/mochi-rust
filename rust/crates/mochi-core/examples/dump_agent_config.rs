//! 只读检查 Agent 配置；不初始化或迁移工作区。

use mochi_core::ai::agent_config::{AgentConfigService, Selection};

fn main() {
    let Some(root) = std::env::args().nth(1) else {
        eprintln!("用法: dump_agent_config <工作区路径>");
        std::process::exit(2);
    };
    let svc = AgentConfigService::new(&root);
    println!("配置根目录: {}\n", svc.root().display());

    let agents = svc.load_agents();
    println!("Agents ({}):", agents.len());
    for a in &agents {
        let tools = match &a.tools {
            Selection::All => "全部".to_owned(),
            Selection::List(t) => format!("{} 个", t.len()),
        };
        let _mcps = match &a.mcps {
            Selection::All => "全部".to_owned(),
            Selection::List(m) if m.is_empty() => "—".to_owned(),
            Selection::List(m) => m.join(","),
        };
        println!(
            "  {:<14} 工具={:<6} 技能={:<2} 可路由={:<2} 路由={:<8} 模式={:<16} 提示词={} 字",
            a.id,
            tools,
            a.skills.len(),
            a.routable_skills.as_list().len(),
            a.skill_routing,
            a.execution_mode,
            a.system_prompt.chars().count()
        );
    }

    // 通用助手配置加载时补齐必需的内置工具。
    if let Some(general) = agents.iter().find(|a| a.id == "通用助手") {
        let injected: Vec<&str> = mochi_core::ai::agent_config::REQUIRED_GENERAL_ASSISTANT_TOOLS
            .iter()
            .copied()
            .filter(|t| general.tools.contains(t))
            .collect();
        println!(
            "\n  通用助手强制补齐的内置工具 ({}/8): {}",
            injected.len(),
            injected.join(", ")
        );
    }

    let skills = svc.load_skills();
    println!("\nSkills ({}):", skills.len());
    for s in &skills {
        println!(
            "  {:<16} 别名={:<2} 正文={} 字  {}",
            s.id,
            s.aliases.len(),
            s.body.chars().count(),
            s.description.as_deref().unwrap_or("—")
        );
    }

    let tools = svc.load_prompt_tools();
    let overrides = svc.load_tool_overrides();
    println!(
        "\nTools ({} 个定义，{} 个产生描述覆盖)",
        tools.len(),
        overrides.len()
    );
    for t in tools.iter().take(5) {
        println!("  {:<28} kind={:<8} enabled={}", t.id, t.kind, t.enabled);
    }
    if tools.len() > 5 {
        println!("  …另有 {} 个", tools.len() - 5);
    }

    let mcps = svc.load_mcp_servers();
    println!("\nMCPs ({}):", mcps.len());
    for m in &mcps {
        println!(
            "  {:<16} transport={:<6} enabled={}  {}",
            m.id,
            m.transport,
            m.enabled,
            m.url.as_deref().or(m.command.as_deref()).unwrap_or("—")
        );
    }

    let actions = svc.load_quick_actions();
    println!("\nQuickActions ({}):", actions.len());
    for a in &actions {
        println!("  {:<12} order={:<4} apply={}", a.id, a.order, a.apply);
    }
}

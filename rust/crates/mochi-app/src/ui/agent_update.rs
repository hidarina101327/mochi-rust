//! 只读的统一差异视图；所有操作都基于评审时的磁盘快照。
use super::*;
use mochi_core::ai::agent_config::updates::AgentUpdate;

pub fn paint(
    list: &mut DrawList,
    area: Rect,
    update: &AgentUpdate,
    diff: &str,
    upstream: bool,
    error: Option<&str>,
    scroll: &mut f32,
    p: &Palette,
) -> Layout {
    let mut layout = Layout::default();
    let left = area.left + 16.0;
    let right = area.right - 16.0;
    list.rect(area, p.surface);
    list.text(
        Rect::new(left, area.top + 8.0, right, area.top + 40.0),
        format!(
            "更新预览 · {}",
            update.relative_path.trim_start_matches("Agents/")
        ),
        TextStyle::Label,
        p.foreground,
    );
    let mut buttons = vec![
        ("返回", Hit::UpdateBack),
        ("上一项", Hit::UpdatePrevious),
        ("下一项", Hit::UpdateNext),
    ];
    if update.baseline.is_some() {
        buttons.push((
            if upstream {
                "查看应用差异"
            } else {
                "查看官方变更"
            },
            Hit::UpdateDiffMode,
        ));
    }
    buttons.push(("跳过此版本", Hit::UpdateSkip));
    buttons.push((
        if upstream {
            "查看应用差异后更新"
        } else {
            "备份并更新"
        },
        Hit::UpdateApply,
    ));
    let (mut x, mut y) = (left, area.top + 46.0);
    for (label, hit) in buttons {
        let width = (text::measure(label, TextStyle::Caption) + 24.0).min((right - left).max(1.0));
        if x > left && x + width > right {
            x = left;
            y += 40.0;
        }
        let rect = Rect::new(x, y, x + width, y + 32.0);
        list.rounded_rect(
            rect,
            5.0,
            if hit == Hit::UpdateApply {
                p.accent
            } else {
                p.surface_muted
            },
        );
        list.text_aligned(
            rect,
            label,
            TextStyle::Caption,
            if hit == Hit::UpdateApply {
                p.accent_foreground
            } else {
                p.foreground
            },
            Align::Center,
        );
        layout.entries.push((rect, hit));
        x += width + 8.0;
    }
    layout.body = Rect::new(area.left, y + 42.0, area.right, area.bottom);
    let old = update
        .previous_revision()
        .map(|s| s[..12].to_owned())
        .unwrap_or_else(|| "未知（旧工作区）".into());
    let mut lines = vec![(
        format!(
            "定义版本：{old} → {}{}",
            &update.revision[..12],
            if update.skipped { " · 已跳过" } else { "" }
        ),
        p.muted,
    )];
    lines.push((
        if upstream {
            "官方变更：上次安装的定义 → 安装包新版。切回应用差异后可更新。".into()
        } else {
            "应用差异：当前本地文件 → 安装包新版。− 红色删除，+ 绿色新增。".into()
        },
        p.foreground,
    ));
    if update.baseline.is_none() {
        lines.push((
            "旧工作区没有版本记录，无法区分历史版本与个人修改，请核对下面的完整替换差异。".into(),
            p.danger,
        ));
    } else if update.locally_modified() {
        lines.push((
            "此定义含本地修改。更新会用新版替换当前文件；原文件将保留在更新备份目录。".into(),
            p.danger,
        ));
    }
    if let Some(error) = error {
        lines.push((error.into(), p.danger));
    }
    lines.push((String::new(), p.muted));
    let green = if theme::is_dark(p) {
        0x86efac
    } else {
        0x166534
    };
    for line in diff.lines() {
        let color = if line.starts_with('+') {
            green
        } else if line.starts_with('-') {
            p.danger
        } else if line.starts_with("@@") {
            p.accent
        } else {
            p.foreground
        };
        lines.push((line.to_owned(), color));
    }
    if diff.is_empty() {
        lines.push(("没有文本差异。".into(), p.muted));
    }
    let rows: Vec<_> = lines
        .into_iter()
        .flat_map(|(line, color)| {
            if line.is_empty() {
                return vec![(String::new(), color)];
            }
            text::wrap_source(&line, TextStyle::Mono, (right - left).max(1.0))
                .into_iter()
                .map(|runs| {
                    (
                        runs.iter().map(|run| run.text.as_str()).collect::<String>(),
                        color,
                    )
                })
                .collect()
        })
        .collect();
    layout.content_height = rows.len() as f32 * 22.0 + 24.0;
    *scroll = (*scroll).clamp(0.0, layout.max_scroll());
    list.push_clip(layout.body);
    for (i, (line, color)) in rows.into_iter().enumerate() {
        let top = layout.body.top + 8.0 + i as f32 * 22.0 - *scroll;
        if top + 22.0 < layout.body.top || top > layout.body.bottom {
            continue;
        }
        if color == green || color == p.danger {
            list.rect(
                Rect::new(left - 4.0, top, right + 4.0, top + 22.0),
                theme::mix(color, p.surface, 0.08),
            );
        }
        list.text(
            Rect::new(left, top, right, top + 22.0),
            line,
            TextStyle::Mono,
            color,
        );
    }
    list.pop_clip();
    layout
}

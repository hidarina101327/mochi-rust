//! 把有界的绘图指令编译成普通、可编辑的画布对象。
use super::{CanvasDocument, Point, Stroke, TextNote};
use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

const MAX_POINTS: usize = 20_000;
const MAX_OBJECTS: usize = 256;

#[cfg(test)]
mod tests;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Drawing {
    #[serde(default)]
    pub paths: Vec<DrawPath>,
    #[serde(default)]
    pub ellipses: Vec<Ellipse>,
    #[serde(default)]
    pub texts: Vec<DrawText>,
    #[serde(default)]
    pub replace_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrawPath {
    pub commands: Vec<Vec<Value>>,
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default = "default_width")]
    pub width: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ellipse {
    pub cx: f64,
    pub cy: f64,
    pub rx: f64,
    pub ry: f64,
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default = "default_width")]
    pub width: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DrawText {
    pub text: String,
    pub x: f64,
    pub y: f64,
    #[serde(default = "default_text_width")]
    pub width: f64,
    #[serde(default = "default_font_size")]
    pub font_size: f64,
    #[serde(default = "default_color")]
    pub color: String,
}

fn default_color() -> String {
    "#334155".into()
}
fn default_width() -> f64 {
    3.0
}
fn default_text_width() -> f64 {
    320.0
}
fn default_font_size() -> f64 {
    24.0
}

/// 平移/缩放刻意不算失效条件：动一下视图并不会让绘图作废。
pub fn revision(document: &CanvasDocument) -> String {
    let content = serde_json::to_vec(&(&document.cards, &document.texts, &document.strokes))
        .expect("validated canvas is serializable");
    Sha256::digest(content)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn coordinate(value: f64) -> Result<f64> {
    ensure!(
        value.is_finite() && value.abs() <= 10_000_000.0,
        "绘图坐标必须是绝对值不超过 10000000 的有限数"
    );
    Ok(value)
}
fn color(value: &str) -> Result<u32> {
    let hex = value.strip_prefix('#').context("颜色必须为 #RRGGBB")?;
    ensure!(
        hex.len() == 6 && hex.bytes().all(|c| c.is_ascii_hexdigit()),
        "颜色必须为 #RRGGBB"
    );
    Ok(u32::from_str_radix(hex, 16)?)
}
fn pen(width: f64, ink: &str) -> Result<u32> {
    ensure!(
        width.is_finite() && (0.5..=64.0).contains(&width),
        "笔宽必须为 0.5..64"
    );
    color(ink)
}
fn push(points: &mut Vec<Point>, point: Point, budget: &mut usize) -> Result<()> {
    ensure!(
        *budget < MAX_POINTS,
        "一次绘图最多 20000 个采样点，请简化曲线或分批绘制"
    );
    points.push(point);
    *budget += 1;
    Ok(())
}
fn mid(a: Point, b: Point) -> Point {
    Point {
        x: (a.x + b.x) / 2.0,
        y: (a.y + b.y) / 2.0,
    }
}
fn distance(p: Point, a: Point, b: Point) -> f64 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let t = if dx * dx + dy * dy == 0.0 {
        0.0
    } else {
        ((p.x - a.x) * dx + (p.y - a.y) * dy) / (dx * dx + dy * dy)
    }
    .clamp(0.0, 1.0);
    (p.x - a.x - t * dx).hypot(p.y - a.y - t * dy)
}
fn cubic(points: &mut Vec<Point>, p: [Point; 4], depth: u8, budget: &mut usize) -> Result<()> {
    if distance(p[1], p[0], p[3]).max(distance(p[2], p[0], p[3])) <= 0.5 {
        return push(points, p[3], budget);
    }
    ensure!(depth < 16, "曲线过于复杂，请缩小坐标范围");
    let a = mid(p[0], p[1]);
    let b = mid(p[1], p[2]);
    let c = mid(p[2], p[3]);
    let d = mid(a, b);
    let e = mid(b, c);
    let f = mid(d, e);
    cubic(points, [p[0], a, d, f], depth + 1, budget)?;
    cubic(points, [f, e, c, p[3]], depth + 1, budget)
}
fn append_stroke(
    doc: &mut CanvasDocument,
    points: &mut Vec<Point>,
    ink: u32,
    width: f64,
    ids: &mut Vec<String>,
) -> Result<()> {
    if points.is_empty() {
        return Ok(());
    }
    ensure!(ids.len() < MAX_OBJECTS, "一次绘图最多 256 个对象");
    let id = doc.next_id("stroke");
    doc.strokes.push(Stroke {
        id: id.clone(),
        points: std::mem::take(points),
        color: ink,
        width,
    });
    ids.push(id);
    Ok(())
}

/// 全批次校验通过才返回新文档，输入永不就地修改。
pub fn apply(
    current: &CanvasDocument,
    expected: &str,
    drawing: Drawing,
) -> Result<(CanvasDocument, Value)> {
    ensure!(
        revision(current) == expected,
        "画布内容已变化，请重新调用 canvas_get 后按最新 revision 绘制"
    );
    ensure!(
        drawing.paths.len() + drawing.ellipses.len() + drawing.texts.len() <= MAX_OBJECTS,
        "一次绘图最多 256 个对象"
    );
    ensure!(
        drawing.replace_ids.len() <= MAX_OBJECTS,
        "一次最多替换 256 个对象"
    );
    ensure!(
        drawing.texts.iter().map(|t| t.text.len()).sum::<usize>() <= 32_000,
        "文字总长度超过 32000 字节"
    );
    let replace: HashSet<_> = drawing.replace_ids.iter().collect();
    ensure!(
        replace.len() == drawing.replace_ids.len(),
        "replaceIds 不能重复"
    );
    for id in &replace {
        ensure!(
            current.strokes.iter().any(|s| &s.id == *id)
                || current.texts.iter().any(|t| &t.id == *id),
            "未找到可替换的笔迹或文字：{id}"
        );
    }
    let mut next = current.clone();
    // 先分配新 ID 再删旧对象：替换后的 ID 不能与旧 ID 重名。
    let mut ids = Vec::new();
    let mut budget = 0;
    for path in drawing.paths {
        ensure!(
            !path.commands.is_empty() && path.commands.len() <= 512,
            "每条路径需要 1..512 条命令"
        );
        let ink = pen(path.width, &path.color)?;
        let mut points = Vec::new();
        for command in path.commands {
            let op = command
                .first()
                .and_then(Value::as_str)
                .context("绘图命令首项必须为 M/L/Q/C/Z")?;
            let count = match op {
                "M" | "L" => 2,
                "Q" => 4,
                "C" => 6,
                "Z" => 0,
                _ => bail!("不支持的绘图命令 {op}，请使用绝对坐标 M/L/Q/C/Z"),
            };
            ensure!(
                command.len() == count + 1,
                "命令 {op} 需要 {count} 个坐标数值"
            );
            let values = command[1..]
                .iter()
                .map(|v| coordinate(v.as_f64().context("坐标必须为数值")?))
                .collect::<Result<Vec<_>>>()?;
            let p = |i: usize| Point {
                x: values[i],
                y: values[i + 1],
            };
            if op == "M" {
                append_stroke(&mut next, &mut points, ink, path.width, &mut ids)?;
                push(&mut points, p(0), &mut budget)?;
                continue;
            }
            let start = *points.last().context("路径必须先以 M 开始")?;
            match op {
                "L" => push(&mut points, p(0), &mut budget)?,
                "Z" => {
                    let first = points[0];
                    push(&mut points, first, &mut budget)?;
                }
                "C" => cubic(&mut points, [start, p(0), p(2), p(4)], 0, &mut budget)?,
                "Q" => {
                    let control = p(0);
                    let end = p(2);
                    cubic(
                        &mut points,
                        [
                            start,
                            Point {
                                x: start.x + (control.x - start.x) * 2.0 / 3.0,
                                y: start.y + (control.y - start.y) * 2.0 / 3.0,
                            },
                            Point {
                                x: end.x + (control.x - end.x) * 2.0 / 3.0,
                                y: end.y + (control.y - end.y) * 2.0 / 3.0,
                            },
                            end,
                        ],
                        0,
                        &mut budget,
                    )?;
                }
                _ => unreachable!(),
            }
        }
        append_stroke(&mut next, &mut points, ink, path.width, &mut ids)?;
    }
    for e in drawing.ellipses {
        coordinate(e.cx)?;
        coordinate(e.cy)?;
        ensure!(
            e.rx.is_finite()
                && e.ry.is_finite()
                && e.rx > 0.0
                && e.ry > 0.0
                && e.rx <= 10000.0
                && e.ry <= 10000.0,
            "椭圆半径必须大于 0 且不超过 10000"
        );
        let ink = pen(e.width, &e.color)?;
        let steps = ((std::f64::consts::PI / (1.0 - 0.5 / e.rx.max(e.ry)).clamp(-1.0, 1.0).acos())
            .ceil() as usize)
            .clamp(24, 512);
        let mut points = Vec::new();
        for i in 0..steps {
            let angle = i as f64 * std::f64::consts::TAU / steps as f64;
            push(
                &mut points,
                Point {
                    x: coordinate(e.cx + e.rx * angle.cos())?,
                    y: coordinate(e.cy + e.ry * angle.sin())?,
                },
                &mut budget,
            )?;
        }
        let first = points[0];
        push(&mut points, first, &mut budget)?;
        append_stroke(&mut next, &mut points, ink, e.width, &mut ids)?;
    }
    for t in drawing.texts {
        ensure!(!t.text.trim().is_empty(), "文字不能为空");
        coordinate(t.x)?;
        coordinate(t.y)?;
        ensure!(
            (40.0..=10000.0).contains(&t.width) && (12.0..=96.0).contains(&t.font_size),
            "文字宽度必须为 40..10000，字号必须为 12..96"
        );
        ensure!(ids.len() < MAX_OBJECTS, "一次绘图最多 256 个对象");
        let id = next.next_id("text");
        // 无头环境的保守回退；原生编辑器保存前会重新量一遍文本。
        let columns = (t.width / t.font_size - 2.0).floor().max(1.0) as usize;
        let lines = t
            .text
            .split('\n')
            .map(|line| line.chars().count().max(1).div_ceil(columns))
            .sum::<usize>();
        let height = (lines as f64 * t.font_size * 1.5 + t.font_size * 2.0).max(48.0);
        next.texts.push(TextNote {
            id: id.clone(),
            x: t.x,
            y: t.y,
            width: t.width,
            height,
            text: t.text,
            color: color(&t.color)?,
            font_size: t.font_size,
        });
        ids.push(id);
    }
    ensure!(
        !ids.is_empty() || !replace.is_empty(),
        "绘图内容为空，请提供 paths、ellipses 或 texts"
    );
    next.strokes.retain(|s| !replace.contains(&s.id));
    next.texts.retain(|t| !replace.contains(&t.id));
    next.validate()?;
    let result = json!({"addedIds":ids,"removedIds":drawing.replace_ids,"revision":revision(&next),"added":ids.len()});
    Ok((next, result))
}

//! 试卷阅读/作答/解析/历史回放。答案草稿由 App 存原生设置，提交历史才改 .exam。
use super::{
    ai_markdown,
    draw::{DrawList, TextStyle},
    layout::Rect,
    text::{self, Run},
    theme::{self, Palette},
};
use mochi_core::exam;
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::rc::Rc;
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub raw: String,
    pub blocks: Vec<exam::Block>,
    pub answers: Vec<Map<String, Value>>,
    pub notes: Vec<Map<String, Value>>,
    pub revealed: Vec<bool>,
    pub replay: Vec<Option<usize>>,
    pub analysis: HashSet<(usize, usize)>,
    pub scroll: f32,
    pub error: String,
    pub drafts_loaded: bool,
}
impl State {
    pub fn new(raw: String) -> Self {
        let blocks = exam::parse(&raw);
        let n = blocks.len();
        Self {
            raw,
            blocks,
            answers: vec![Map::new(); n],
            notes: vec![Map::new(); n],
            revealed: vec![false; n],
            replay: vec![None; n],
            analysis: HashSet::new(),
            scroll: 0.0,
            error: String::new(),
            drafts_loaded: false,
        }
    }
    pub fn question(&self, b: usize, q: usize) -> Option<&Value> {
        self.blocks.get(b)?.model["questions"].get(q)
    }
    pub fn answers(&self, b: usize) -> &Map<String, Value> {
        self.replay[b]
            .and_then(|h| self.blocks[b].model["history"].get(h))
            .and_then(|h| h["answers"].as_object())
            .unwrap_or(&self.answers[b])
    }
    pub fn locked(&self, b: usize) -> bool {
        self.replay.get(b).is_some_and(Option::is_some) || self.revealed.get(b) == Some(&true)
    }
    pub fn pick(&mut self, b: usize, q: usize, option: usize) {
        if self.locked(b) {
            return;
        }
        let Some(question) = self.question(b, q) else {
            return;
        };
        let id = question["id"].as_str().unwrap_or("").to_owned();
        let value = match question["type"].as_str() {
            Some("judge") => Value::Bool(option == 0),
            Some("multi_choice") => {
                let mut selected = self.answers[b]
                    .get(&id)
                    .and_then(|a| a.as_array())
                    .cloned()
                    .unwrap_or_default();
                let value = Value::from(option);
                if selected.contains(&value) {
                    selected.retain(|v| *v != value);
                } else {
                    selected.push(value);
                }
                selected.sort_by_key(|v| v.as_u64());
                Value::Array(selected)
            }
            _ => Value::from(option),
        };
        self.answers[b].insert(id, value);
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Source,
    Template,
    Pick(usize, usize, usize),
    Short(usize, usize),
    Note(usize, Option<usize>),
    Analysis(usize, usize),
    Submit(usize),
    Reset(usize),
    Replay(usize, usize),
}
struct Line {
    rect: Rect,
    runs: Vec<Run>,
    style: TextStyle,
    muted: bool,
}
#[derive(Default)]
pub struct Layout {
    pub entries: Vec<(Rect, Hit)>,
    pub body: Rect,
    pub height: f32,
    lines: Vec<Line>,
    buttons: Vec<(Rect, String, bool, bool)>,
    markdown: Vec<(Rect, Rc<ai_markdown::Layout>)>,
}
impl Layout {
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        self.entries
            .iter()
            .rev()
            .find(|(r, h)| {
                r.contains(x, y)
                    && (matches!(h, Hit::Source | Hit::Template) || self.body.contains(x, y))
            })
            .map(|(_, h)| *h)
    }
    pub fn max_scroll(&self) -> f32 {
        (self.height - self.body.height()).max(0.0)
    }
    fn text(
        &mut self,
        x: f32,
        right: f32,
        y: &mut f32,
        value: &str,
        style: TextStyle,
        muted: bool,
    ) {
        for runs in text::wrap_runs(&text::parse_inline(value), style, (right - x).max(40.0)) {
            self.lines.push(Line {
                rect: Rect::new(x, *y, right, *y + style.line_height()),
                runs,
                style,
                muted,
            });
            *y += style.line_height();
        }
    }
    fn button(&mut self, r: Rect, label: String, hit: Hit, selected: bool) {
        self.entries.push((r, hit));
        self.buttons.push((r, label, selected, hit == Hit::Source));
    }
    fn markdown(&mut self, x: f32, right: f32, y: &mut f32, source: &str) {
        if source.trim().is_empty() {
            return;
        }
        let layout = ai_markdown::layout(source, (right - x).max(1.0), true);
        let bottom = *y + layout.height;
        self.markdown
            .push((Rect::new(x, *y, right, bottom), layout));
        *y = bottom;
    }
}
pub fn layout(s: &State, area: Rect) -> Layout {
    let mut l = Layout {
        body: Rect::new(area.left, area.top + 49.0, area.right, area.bottom),
        ..Default::default()
    };
    l.button(
        Rect::new(
            area.right - 96.0,
            area.top + 8.0,
            area.right - 16.0,
            area.top + 40.0,
        ),
        "源码".into(),
        Hit::Source,
        false,
    );
    let width = (area.width() - 64.0).clamp(100.0, 796.0);
    let left = area.left + (area.width() - width) / 2.0;
    let right = left + width;
    let mut y = l.body.top + 40.0 - s.scroll;
    let mut end = 0;
    for (b, block) in s.blocks.iter().enumerate() {
        let before = s.raw.get(end..block.start).unwrap_or("");
        if !before.trim().is_empty() {
            l.markdown(left, right, &mut y, before);
            y += 20.0;
        }
        let model = &block.model;
        if let Some(title) = model["title"].as_str().filter(|s| !s.is_empty()) {
            l.text(left, right, &mut y, title, TextStyle::Heading2, false);
        }
        if let Some(desc) = model["description"].as_str() {
            l.markdown(left, right, &mut y, desc);
        }
        y += 16.0;
        if s.locked(b) {
            l.text(
                left,
                right,
                &mut y,
                &format!(
                    "{} 分 · 客观题得分 · 简答题需人工评分{}",
                    exam::score(model, s.answers(b)),
                    if s.replay[b].is_some() {
                        " · 历史回放"
                    } else {
                        ""
                    }
                ),
                TextStyle::Label,
                false,
            );
            l.button(
                Rect::new(left, y, left + 110.0, y + 32.0),
                "重新作答".into(),
                Hit::Reset(b),
                false,
            );
            y += 44.0;
        }
        for (q, question) in model["questions"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            let id = question["id"].as_str().unwrap_or("");
            let answer = s.answers(b).get(id).unwrap_or(&Value::Null);
            let status = if s.locked(b) {
                match exam::score_question(question, answer) {
                    "correct" => " · 回答正确",
                    "incorrect" => " · 回答错误",
                    "manual" => " · 待评分",
                    _ => " · 未作答",
                }
            } else {
                ""
            };
            l.text(
                left,
                right,
                &mut y,
                &format!(
                    "第 {} 题 · {}{}",
                    q + 1,
                    match question["type"].as_str() {
                        Some("single_choice") => "单选题",
                        Some("multi_choice") => "多选题",
                        Some("judge") => "判断题",
                        _ => "简答题",
                    },
                    status
                ),
                TextStyle::Body,
                false,
            );
            y += 8.0;
            l.markdown(left, right, &mut y, question["stem"].as_str().unwrap_or(""));
            y += 8.0;
            let options = if question["type"] == "judge" {
                vec!["正确".into(), "错误".into()]
            } else {
                question["options"]
                    .as_array()
                    .map(|o| {
                        o.iter()
                            .map(|o| o.as_str().unwrap_or("").to_owned())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            };
            for (i, option) in options.iter().enumerate() {
                let selected = if question["type"] == "judge" {
                    answer.as_bool() == Some(i == 0)
                } else if question["type"] == "multi_choice" {
                    answer
                        .as_array()
                        .is_some_and(|a| a.contains(&Value::from(i)))
                } else {
                    answer.as_u64() == Some(i as u64)
                };
                let top = y;
                let marker = if question["type"] == "multi_choice" {
                    if selected {
                        "☑"
                    } else {
                        "□"
                    }
                } else if selected {
                    "●"
                } else {
                    "○"
                };
                let label = if question["type"] == "judge" {
                    marker.to_owned()
                } else {
                    format!("{} {}.", marker, option_letter(i))
                };
                let mut marker_y = top + 10.0;
                l.text(
                    left + 10.0,
                    left + 66.0,
                    &mut marker_y,
                    &label,
                    TextStyle::Label,
                    false,
                );
                y += 10.0;
                l.markdown(left + 68.0, right - 12.0, &mut y, option);
                y = (y + 10.0).max(top + 40.0);
                l.button(
                    Rect::new(left, top, right, y),
                    String::new(),
                    Hit::Pick(b, q, i),
                    selected,
                );
                y += 6.0;
            }
            if question["type"] == "short_answer" {
                let top = y;
                y += 12.0;
                let value = answer
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("在此作答……");
                for paragraph in value.split('\n') {
                    for runs in text::wrap_runs(
                        &[Run::plain(paragraph)],
                        TextStyle::Body,
                        (width - 24.0).max(1.0),
                    ) {
                        push_answer_line(&mut l, left + 12.0, right - 12.0, &mut y, runs);
                    }
                }
                y = (y + 12.0).max(top + 96.0);
                l.button(
                    Rect::new(left, top, right, y),
                    String::new(),
                    Hit::Short(b, q),
                    false,
                );
                y += 12.0;
            }
            l.button(
                Rect::new(left, y, left + 72.0, y + 28.0),
                "解析".into(),
                Hit::Analysis(b, q),
                s.analysis.contains(&(b, q)),
            );
            l.button(
                Rect::new(left + 84.0, y, left + 156.0, y + 28.0),
                "备注".into(),
                Hit::Note(b, Some(q)),
                false,
            );
            y += 40.0;
            if s.analysis.contains(&(b, q)) || s.locked(b) {
                if let Some(answer) = answer_summary(question) {
                    l.text(left, right, &mut y, "正确答案", TextStyle::Caption, true);
                    l.markdown(left, right, &mut y, &answer);
                }
                for key in ["answer_guide", "analysis"] {
                    if let Some(value) = question[key].as_str() {
                        l.markdown(left, right, &mut y, value);
                    }
                }
                y += 12.0;
            }
            y += 20.0;
        }
        if !s.locked(b) {
            l.button(
                Rect::new(left, y, left + 120.0, y + 36.0),
                "提交作答".into(),
                Hit::Submit(b),
                true,
            );
            l.button(
                Rect::new(left + 132.0, y, left + 252.0, y + 36.0),
                "整卷备注".into(),
                Hit::Note(b, None),
                false,
            );
            y += 48.0;
        }
        l.text(left, right, &mut y, "作答历史", TextStyle::Label, true);
        for (i, h) in model["history"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .enumerate()
            .rev()
        {
            l.button(
                Rect::new(left, y, right, y + 32.0),
                format!("{} · {} 分 · 回放", h["submitted_at"], h["score"]),
                Hit::Replay(b, i),
                false,
            );
            y += 38.0;
        }
        y += 32.0;
        end = block.end;
    }
    if end < s.raw.len() {
        l.markdown(left, right, &mut y, &s.raw[end..]);
    }
    if s.blocks.is_empty() {
        l.text(
            left,
            right,
            &mut y,
            "未找到有效试卷块，可切换源码修复或插入模板。",
            TextStyle::Label,
            true,
        );
        l.button(
            Rect::new(left, y, left + 128.0, y + 36.0),
            "插入试卷模板".into(),
            Hit::Template,
            true,
        );
        y += 48.0;
    }
    l.height = y - l.body.top + s.scroll + 200.0;
    l
}
pub fn paint(list: &mut DrawList, area: Rect, s: &State, l: &Layout, p: &Palette) {
    list.push_clip(area);
    list.rect(area, p.background);
    list.text(
        Rect::new(
            area.left + 16.0,
            area.top,
            area.right - 110.0,
            area.top + 48.0,
        ),
        "试卷 · 阅读与作答",
        TextStyle::Label,
        p.foreground,
    );
    list.hline(area.left, area.right, area.top + 48.0, p.border);
    for (r, label, selected, toolbar) in &l.buttons {
        if *toolbar {
            list.rounded_border(*r, 6.0, p.border);
            list.text(*r, label, TextStyle::Caption, p.foreground);
        } else if r.top < area.bottom && r.bottom > l.body.top {
            list.push_clip(l.body);
            list.rounded_rect(
                *r,
                6.0,
                if *selected {
                    theme::mix(p.accent, p.background, 0.12)
                } else {
                    p.surface
                },
            );
            list.rounded_border(*r, 6.0, if *selected { p.accent } else { p.border });
            list.text(
                Rect::new(r.left + 10.0, r.top, r.right - 10.0, r.bottom),
                text::ellipsize(label, TextStyle::Label, (r.width() - 20.0).max(0.0)),
                TextStyle::Label,
                p.foreground,
            );
            list.pop_clip();
        }
    }
    list.push_clip(l.body);
    for (rect, markdown) in &l.markdown {
        if rect.bottom >= l.body.top && rect.top <= l.body.bottom {
            markdown.paint_scrolled(
                list,
                (rect.left, rect.top),
                l.body,
                p,
                None,
                false,
                None,
                None,
            );
        }
    }
    for line in &l.lines {
        if line.rect.bottom < l.body.top || line.rect.top > l.body.bottom {
            continue;
        }
        let mut x = line.rect.left;
        for run in &line.runs {
            let w = text::measure_runs(std::slice::from_ref(run), line.style);
            list.text_run(
                Rect::new(x, line.rect.top, line.rect.right, line.rect.bottom),
                &run.text,
                line.style,
                if line.muted { p.muted } else { p.foreground },
                super::draw::Align::Leading,
                run.emphasis,
            );
            x += w;
        }
    }
    if !s.error.is_empty() {
        list.text(
            Rect::new(
                area.left + 16.0,
                area.bottom - 32.0,
                area.right - 16.0,
                area.bottom,
            ),
            &s.error,
            TextStyle::Caption,
            p.danger,
        );
    }
    list.pop_clip();
    list.pop_clip();
}
fn option_letter(index: usize) -> String {
    u32::try_from(index)
        .ok()
        .and_then(|i| i.checked_add(65))
        .and_then(char::from_u32)
        .map(|c| c.to_string())
        .unwrap_or_else(|| (index + 1).to_string())
}

fn push_answer_line(l: &mut Layout, left: f32, right: f32, y: &mut f32, runs: Vec<Run>) {
    let style = TextStyle::Body;
    l.lines.push(Line {
        rect: Rect::new(left, *y, right, *y + style.line_height()),
        runs,
        style,
        muted: false,
    });
    *y += style.line_height();
}

fn answer_summary(question: &Value) -> Option<String> {
    let choice = |value: &Value| {
        let index = value.as_u64().and_then(|i| usize::try_from(i).ok());
        index.map(|i| {
            format!(
                "{}. {}",
                option_letter(i),
                question["options"]
                    .get(i)
                    .and_then(Value::as_str)
                    .unwrap_or("")
            )
        })
    };
    match question["type"].as_str() {
        Some("single_choice") => choice(&question["answer"]),
        Some("multi_choice") => Some(
            question["answer"]
                .as_array()?
                .iter()
                .filter_map(choice)
                .collect::<Vec<_>>()
                .join("\n\n"),
        ),
        Some("judge") => Some(
            if question["answer"].as_bool()? {
                "正确"
            } else {
                "错误"
            }
            .into(),
        ),
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rich_content_and_long_options_keep_their_full_height() {
        let option = "σ² 未知时，应使用样本标准差构造均值的置信区间。".repeat(8);
        let model = serde_json::json!({"questions": [{"id":"q1", "type":"single_choice", "stem": "**均值**\n\n$\\frac{a}{b}$", "options":[option, "第二项"], "answer": 1}]});
        let s = State::new(format!("# 参数估计\n\n:::exam\n{model}\n:::\n\n## 后文"));
        let wide = layout(&s, Rect::new(0.0, 0.0, 900.0, 900.0));
        let narrow = layout(&s, Rect::new(0.0, 0.0, 420.0, 900.0));
        let choice = |l: &Layout, i| {
            l.entries
                .iter()
                .find(|(_, hit)| *hit == Hit::Pick(0, 0, i))
                .unwrap()
                .0
        };
        assert!(choice(&narrow, 0).height() > choice(&wide, 0).height());
        assert!(choice(&narrow, 0).bottom < choice(&narrow, 1).top);
        let r = choice(&narrow, 0);
        assert_eq!(
            narrow.hit(r.left + 1.0, r.bottom - 1.0),
            Some(Hit::Pick(0, 0, 0))
        );
        let contents = narrow
            .markdown
            .iter()
            .flat_map(|(_, m)| m.selectable_text(None))
            .collect::<Vec<_>>();
        assert!(contents
            .iter()
            .any(|t| t.text == "参数估计" && matches!(t.style, TextStyle::Ai { kind: 1, .. })));
        assert!(contents
            .iter()
            .any(|t| t.emphasis.base() == text::Emphasis::Math));
        let full_text = contents.iter().map(|t| t.text.as_str()).collect::<String>();
        assert!(full_text.contains(&option));
        assert!(!full_text.contains("# 参数估计"));
        assert!(full_text.contains("后文"));
    }

    #[test]
    fn answer_labels_match_the_visible_options() {
        assert_eq!(
            answer_summary(
                &serde_json::json!({"type":"single_choice","answer":1,"options":["甲","乙"]})
            )
            .as_deref(),
            Some("B. 乙")
        );
        assert_eq!(answer_summary(&serde_json::json!({"type":"multi_choice","answer":[0,2],"options":["甲","乙","丙"]})).as_deref(), Some("A. 甲\n\nC. 丙"));
        assert_eq!(
            answer_summary(&serde_json::json!({"type":"judge","answer":false})).as_deref(),
            Some("错误")
        );
    }
    #[test]
    fn zero_option_is_selected_and_replay_locks_answers() {
        let mut s = State::new(exam::template());
        s.pick(0, 0, 0);
        assert_eq!(s.answers[0]["q1"], 0);
        s.revealed[0] = true;
        s.pick(0, 0, 1);
        assert_eq!(s.answers[0]["q1"], 0);
        let area = Rect::new(0.0, 0.0, 800.0, 900.0);
        let l = layout(&s, area);
        let mut d = DrawList::new();
        paint(
            &mut d,
            area,
            &s,
            &l,
            super::super::theme::tokens().palette(false),
        );
        assert!(d.finish().is_ok());
    }
}

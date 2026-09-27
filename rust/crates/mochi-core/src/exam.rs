//! .exam：Markdown 中的 JSON 容器、归一化与客观题判分。写回仅替换容器内 JSON 字节。
use serde_json::{json, Map, Value};
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub start: usize,
    pub end: usize,
    pub json_start: usize,
    pub json_end: usize,
    pub model: Value,
}
pub fn parse(source: &str) -> Vec<Block> {
    let mut starts = Vec::new();
    let mut offset = 0;
    let lines = source
        .split_inclusive('\n')
        .inspect(|s| {
            starts.push(offset);
            offset += s.len();
        })
        .collect::<Vec<_>>();
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim_end_matches(['\r', '\n', ' ', '\t']) != ":::exam" {
            i += 1;
            continue;
        }
        let Some(close) = lines[i + 1..]
            .iter()
            .position(|l| l.trim_end_matches(['\r', '\n', ' ', '\t']) == ":::")
        else {
            break;
        };
        let close = i + 1 + close;
        let json_start = starts.get(i + 1).copied().unwrap_or(source.len());
        let json_end = starts[close].saturating_sub(if starts[close] > json_start { 1 } else { 0 });
        if let Ok(value) = serde_json::from_str(source.get(json_start..json_end).unwrap_or("")) {
            blocks.push(Block {
                start: starts[i],
                end: starts[close] + lines[close].len(),
                json_start,
                json_end,
                model: normalize(value),
            });
        }
        i = close + 1;
    }
    blocks
}
pub fn normalize(input: Value) -> Value {
    let mut questions = Vec::new();
    if let Some(raw) = input["questions"].as_array() {
        for (i, q) in raw.iter().enumerate() {
            if !q.is_object() {
                continue;
            }
            let kind = match q["type"].as_str() {
                Some(t @ ("single_choice" | "multi_choice" | "judge" | "short_answer")) => t,
                _ if q["answer"].is_boolean() => "judge",
                _ if q["answer"].is_array() && q["options"].is_array() => "multi_choice",
                _ if q["answer"].is_number() && q["options"].is_array() => "single_choice",
                _ => "short_answer",
            };
            let mut question = json!({"id":q["id"].as_str().filter(|s|!s.trim().is_empty()).map(str::to_owned).unwrap_or_else(||format!("q{}",i+1)),"type":kind,"stem":q["stem"].as_str().unwrap_or("")});
            let map = question.as_object_mut().unwrap();
            match kind {
                "single_choice" | "multi_choice" => {
                    let options = q["options"]
                        .as_array()
                        .map(|o| {
                            o.iter()
                                .map(|s| {
                                    if s.is_null() {
                                        "".into()
                                    } else if let Some(s) = s.as_str() {
                                        Value::String(s.into())
                                    } else {
                                        Value::String(s.to_string())
                                    }
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    map.insert("options".into(), options.into());
                    map.insert(
                        "answer".into(),
                        if kind == "single_choice" {
                            q["answer"]
                                .as_number()
                                .cloned()
                                .map(Value::Number)
                                .unwrap_or(json!(-1))
                        } else {
                            q["answer"]
                                .as_array()
                                .map(|a| {
                                    a.iter()
                                        .filter(|v| v.is_number())
                                        .cloned()
                                        .collect::<Vec<_>>()
                                })
                                .unwrap_or_default()
                                .into()
                        },
                    );
                }
                "judge" => {
                    map.insert(
                        "answer".into(),
                        q["answer"].as_bool().unwrap_or(false).into(),
                    );
                }
                _ => {
                    if let Some(s) = q["answer_guide"].as_str() {
                        map.insert("answer_guide".into(), s.into());
                    }
                    map.insert(
                        "scoring_mode".into(),
                        if q["scoring_mode"] == "ai" {
                            "ai"
                        } else {
                            "manual"
                        }
                        .into(),
                    );
                }
            }
            if let Some(s) = q["analysis"].as_str() {
                map.insert("analysis".into(), s.into());
            }
            questions.push(question);
        }
    }
    let history=input["history"].as_array().map(|rows|rows.iter().filter(|h|h.is_object()).map(|h|{let mut entry=json!({"submitted_at":if h["submitted_at"].is_number(){h["submitted_at"].clone()}else{json!(0)},"answers":h["answers"].as_object().cloned().unwrap_or_default(),"score":if h["score"].is_number(){h["score"].clone()}else{json!(0)}});if h["notes"].is_object(){entry["notes"]=h["notes"].clone();}entry}).collect::<Vec<_>>()).unwrap_or_default();
    let mut model = json!({"version":input["version"].as_str().unwrap_or("1.0"),"questions":questions,"history":history});
    for key in ["title", "description"] {
        if let Some(s) = input[key].as_str() {
            model[key] = s.into();
        }
    }
    model
}
pub fn provided(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(_) | Value::Number(_) => true,
        Value::String(s) => !s.trim().is_empty(),
        Value::Array(a) => !a.is_empty(),
        _ => false,
    }
}
pub fn score_question(q: &Value, a: &Value) -> &'static str {
    if !provided(a) {
        return "unanswered";
    }
    if q["type"] == "short_answer" {
        return "manual";
    }
    let correct = if q["type"] == "multi_choice" {
        let empty = Vec::new();
        let a = a.as_array().unwrap_or(&empty);
        let b = q["answer"].as_array().unwrap_or(&empty);
        a.iter().all(|v| b.contains(v)) && b.iter().all(|v| a.contains(v))
    } else {
        *a == q["answer"]
    };
    if correct {
        "correct"
    } else {
        "incorrect"
    }
}
pub fn score(model: &Value, answers: &Map<String, Value>) -> u32 {
    let questions = model["questions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|q| q["type"] != "short_answer")
        .collect::<Vec<_>>();
    if questions.is_empty() {
        return 0;
    }
    let correct = questions
        .iter()
        .filter(|q| {
            score_question(
                q,
                answers
                    .get(q["id"].as_str().unwrap_or(""))
                    .unwrap_or(&Value::Null),
            ) == "correct"
        })
        .count();
    (correct as f64 / questions.len() as f64 * 100.0).round() as u32
}
pub fn update(source: &str, index: usize, model: &Value) -> anyhow::Result<String> {
    let blocks = parse(source);
    let block = blocks
        .get(index)
        .ok_or_else(|| anyhow::anyhow!("试卷块不存在"))?;
    let mut next = source.to_owned();
    next.replace_range(
        block.json_start..block.json_end,
        &crate::json2::serialize(model)?,
    );
    Ok(next)
}
pub fn template() -> String {
    format!(":::exam\n{}\n:::\n",crate::json2::serialize(&json!({"version":"1.0","questions":[{"id":"q1","type":"single_choice","stem":"请编辑题干","options":["选项 A","选项 B"],"answer":0}],"history":[],"title":"新试卷"})).unwrap())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_json_changes_and_crlf_markdown_survives() {
        let s = format!("# 中文\r\n\r\n{}后文\r\n", template().replace('\n', "\r\n"));
        let b = parse(&s);
        assert_eq!(b.len(), 1);
        let mut m = b[0].model.clone();
        m["title"] = "改名".into();
        let next = update(&s, 0, &m).unwrap();
        assert!(next.starts_with("# 中文\r\n\r\n:::exam\r\n"));
        assert!(next.ends_with("\n:::\r\n后文\r\n"));
    }
    #[test]
    fn false_zero_and_sets_score_while_short_answers_are_manual() {
        let m = normalize(
            json!({"questions":[{"options":["a","b"],"answer":0},{"answer":false},{"options":["a","b"],"answer":[0,1]},{"stem":"简答"}]}),
        );
        let answers =
            serde_json::from_value(json!({"q1":0,"q2":false,"q3":[1,0],"q4":"回答"})).unwrap();
        assert_eq!(score(&m, &answers), 100);
        assert_eq!(score_question(&m["questions"][3], &json!("回答")), "manual");
        assert_eq!(
            score_question(&m["questions"][1], &Value::Null),
            "unanswered"
        );
    }
    #[test]
    fn invalid_and_unclosed_blocks_preserve_original() {
        assert!(parse(":::exam\n{bad}\n:::").is_empty());
        assert!(parse(":::exam\n{}").is_empty());
    }
}

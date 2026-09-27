//! 对齐 JSON.stringify(x, null, 2)：两空格缩进、LF、UTF-8 无 BOM，非 ASCII 不转义。
//! 字段顺序、camelCase 和 None 省略由模型保证。

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;

/// 序列化为与 `JSON.stringify(value, null, 2)` 字节一致的字符串（结尾无换行）。
pub fn serialize<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string_pretty(value).context("JSON 序列化失败")
}

pub fn deserialize<T: DeserializeOwned>(json: &str) -> Result<T> {
    serde_json::from_str(json).context("JSON 反序列化失败")
}

/// 写入文件（UTF-8 无 BOM），自动创建父目录。
pub fn write_to_file(path: impl AsRef<Path>, json: &str) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("创建目录失败: {}", parent.display()))?;
    }
    fs::write(path, json.as_bytes()).with_context(|| format!("写入失败: {}", path.display()))
}

/// 序列化并落盘。
pub fn write<T: Serialize>(path: impl AsRef<Path>, value: &T) -> Result<()> {
    write_to_file(path, &serialize(value)?)
}

/// 读取并反序列化；文件不存在返回 `None`（对齐 C# 版 `ReadFromFile` 的 `default`）。
pub fn read_from_file<T: DeserializeOwned>(path: impl AsRef<Path>) -> Result<Option<T>> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(path).with_context(|| format!("读取失败: {}", path.display()))?;
    deserialize(&text).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Sample {
        id: String,
        display_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        order: i64,
        builtin: bool,
    }

    /// 期望值取自 Node 的 `JSON.stringify(x, null, 2)`，逐字节抄写。
    #[test]
    fn matches_js_stringify_two_space_lf() {
        let value = Sample {
            id: "knowledge-base".into(),
            display_name: "知识库".into(),
            description: None,
            order: 0,
            builtin: true,
        };
        let expected = "{\n  \"id\": \"knowledge-base\",\n  \"displayName\": \"知识库\",\n  \"order\": 0,\n  \"builtin\": true\n}";
        assert_eq!(serialize(&value).unwrap(), expected);
    }

    #[test]
    fn cjk_is_not_escaped() {
        let json = serialize(&serde_json::json!({ "名称": "计算机通识" })).unwrap();
        assert_eq!(json, "{\n  \"名称\": \"计算机通识\"\n}");
        assert!(!json.contains("\\u"), "中文被转义了：{json}");
    }

    #[test]
    fn uses_lf_never_crlf() {
        let json = serialize(&serde_json::json!({ "a": 1, "b": 2 })).unwrap();
        assert!(json.contains('\n'));
        assert!(!json.contains('\r'), "出现了 CRLF：{json:?}");
    }

    #[test]
    fn nested_indentation_is_two_spaces_per_level() {
        let json = serialize(&serde_json::json!({ "outer": { "inner": [1, 2] } })).unwrap();
        assert_eq!(
            json,
            "{\n  \"outer\": {\n    \"inner\": [\n      1,\n      2\n    ]\n  }\n}"
        );
    }

    /// JS: `JSON.stringify({a:{},b:[]}, null, 2)` → 空容器不展开成多行。
    #[test]
    fn empty_containers_stay_on_one_line() {
        let json = serialize(&serde_json::json!({ "a": {}, "b": [] })).unwrap();
        assert_eq!(json, "{\n  \"a\": {},\n  \"b\": []\n}");
    }

    /// 转义集必须与 JS 一致：`"` `\` 和控制字符，且 `/` 不转义。
    #[test]
    fn escape_set_matches_js() {
        let json = serialize(&serde_json::json!({ "s": "a\"b\\c\nd\te/f" })).unwrap();
        assert_eq!(json, "{\n  \"s\": \"a\\\"b\\\\c\\nd\\te/f\"\n}");
    }

    #[test]
    fn none_fields_are_omitted_not_written_as_null() {
        let value = Sample {
            id: "x".into(),
            display_name: "x".into(),
            description: None,
            order: 1,
            builtin: false,
        };
        assert!(!serialize(&value).unwrap().contains("description"));
    }

    #[test]
    fn some_fields_are_written() {
        let value = Sample {
            id: "x".into(),
            display_name: "x".into(),
            description: Some("说明".into()),
            order: 1,
            builtin: false,
        };
        assert!(serialize(&value)
            .unwrap()
            .contains("\"description\": \"说明\""));
    }

    #[test]
    fn file_roundtrip_writes_utf8_without_bom() {
        let dir = std::env::temp_dir().join(format!("mochi-json2-{}", std::process::id()));
        let path = dir.join("nested").join("libraries.json");
        let value = serde_json::json!({ "库": "知识库" });

        write(&path, &value).unwrap();

        let bytes = fs::read(&path).unwrap();
        assert_ne!(&bytes[..3.min(bytes.len())], b"\xEF\xBB\xBF", "写出了 BOM");
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            serialize(&value).unwrap()
        );

        let back: serde_json::Value = read_from_file(&path).unwrap().unwrap();
        assert_eq!(back, value);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_missing_file_returns_none() {
        let missing = std::env::temp_dir().join("mochi-does-not-exist-9e3f.json");
        let got: Option<serde_json::Value> = read_from_file(missing).unwrap();
        assert!(got.is_none());
    }
}

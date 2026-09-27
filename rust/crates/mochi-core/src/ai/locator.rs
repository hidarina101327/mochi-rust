//! 共享的 mochi://ai-locate 协议。引用不会被转换成任意文件路径。
use super::session::{AiConversation, AiSessionService, AiStoredMessage};
use std::path::Path;
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Locator {
    pub session_id: String,
    pub message_id: String,
    pub title: String,
    pub snippet: String,
}
impl Locator {
    pub fn parse(value: &str) -> Option<Self> {
        let (scheme, query) = value.trim().split_once('?')?;
        if !scheme.eq_ignore_ascii_case("mochi://ai-locate") {
            return None;
        }
        if query.is_empty() || query.chars().any(char::is_whitespace) {
            return None;
        }
        let fields = url::form_urlencoded::parse(query.as_bytes()).collect::<Vec<_>>();
        let get = |name: &str| {
            fields
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, v)| v.to_string())
                .unwrap_or_default()
        };
        let result = Self {
            session_id: get("session"),
            message_id: get("message"),
            title: get("title"),
            snippet: get("snippet"),
        };
        (!result.session_id.is_empty() && !result.message_id.is_empty()).then_some(result)
    }
    pub fn to_url(&self) -> String {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query
            .append_pair("session", &self.session_id)
            .append_pair("message", &self.message_id);
        if !self.title.is_empty() {
            query.append_pair("title", &self.title);
        }
        if !self.snippet.is_empty() {
            query.append_pair("snippet", &self.snippet);
        }
        format!("mochi://ai-locate?{}", query.finish())
    }
}
pub fn safe_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 240
        && id != "."
        && id != ".."
        && !id.ends_with(['.', ' '])
        && !id
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
}
pub fn load_session(root: &Path, id: &str) -> Option<AiConversation> {
    if !safe_session_id(id) {
        return None;
    }
    let root = root.canonicalize().ok()?;
    let directory = AiSessionService::new(&root)
        .base_path()
        .join("sessions")
        .canonicalize()
        .ok()?;
    let file = directory.join(format!("{id}.json")).canonicalize().ok()?;
    if !crate::paths::path_is_within(&root, &directory)
        || !crate::paths::path_is_within(&directory, &file)
        || std::fs::metadata(&file).ok()?.len() > 64 * 1024 * 1024
    {
        return None;
    }
    let value: AiConversation =
        crate::json2::deserialize(&std::fs::read_to_string(file).ok()?).ok()?;
    (value.id == id).then_some(value)
}
pub fn visible(m: &AiStoredMessage) -> bool {
    let role = m.get("role").and_then(|v| v.as_str());
    let hidden = m
        .get("hidden")
        .is_some_and(|v| !v.is_null() && v != &serde_json::Value::Bool(false));
    !hidden
        && (role == Some("user")
            || (role == Some("assistant")
                && m.get("tool_calls")
                    .and_then(|v| v.as_array())
                    .is_none_or(|v| v.is_empty())))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn url_round_trips_unicode_and_matches_url_search_params_order() {
        let r = Locator {
            session_id: "s".into(),
            message_id: "m".into(),
            title: "中文 & title".into(),
            snippet: "a+b\n😀".into(),
        };
        assert!(r
            .to_url()
            .starts_with("mochi://ai-locate?session=s&message=m&title="));
        assert_eq!(Locator::parse(&r.to_url()), Some(r));
        assert_eq!(
            Locator::parse("mochi://ai-locate?session=first&session=second&message=m")
                .unwrap()
                .session_id,
            "first"
        );
        for bad in [
            "mochi://ai-locate?session=s",
            "mochi://ai-locate?session=s&message=",
            "mochi://ai-locate?session=has space&message=m",
        ] {
            assert!(Locator::parse(bad).is_none());
        }
    }
}

//! 获取网页内容并提取可供工作流处理的文本。
use super::*;
use std::{
    io::Read,
    time::{Duration, Instant},
};

pub(super) fn fetch(node: &Node, input: &Value, cancel: Arc<AtomicBool>) -> Result<Value> {
    let mut url =
        url::Url::parse(input["url"].as_str().ok_or("缺少 url")?).map_err(|e| e.to_string())?;
    let method = node.config["method"].as_str().unwrap_or("GET");
    if !["GET", "POST"].contains(&method) {
        return Err("HTTP 节点支持 GET 或 POST".into());
    }
    let deadline = Instant::now() + Duration::from_millis(node.timeout_ms);
    for hop in 0..=4 {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("HTTP 节点已超时")?;
        let agent = ureq::Agent::with_parts(
            ureq::Agent::config_builder()
                .timeout_global(Some(remaining))
                .tls_config(
                    ureq::tls::TlsConfig::builder()
                        .provider(ureq::tls::TlsProvider::NativeTls)
                        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                        .build(),
                )
                .max_redirects(0)
                .http_status_as_error(false)
                .build(),
            ureq::unversioned::transport::DefaultConnector::default(),
            ureq::unversioned::resolver::DefaultResolver::default(),
        );
        if cancel.load(Ordering::Relaxed) {
            return Err("运行已取消".into());
        }
        if !["http", "https"].contains(&url.scheme())
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err("只支持不含凭据的 HTTP(S) URL".into());
        }
        let mut request = ureq::http::Request::builder()
            .method(method)
            .uri(url.as_str());
        if let Some(headers) = input["headers"].as_object() {
            for (k, v) in headers {
                if hop > 0
                    && ["authorization", "cookie", "proxy-authorization"]
                        .contains(&k.to_ascii_lowercase().as_str())
                {
                    continue;
                }
                request = request.header(k, v.as_str().ok_or("HTTP 请求头必须为文本")?);
            }
        }
        let body = input["body"].as_str().unwrap_or("");
        let mut response = agent
            .run(request.body(body).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            if method != "GET" {
                return Err("POST 重定向需显式使用最终 URL，避免重复提交".into());
            }
            let location = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or("重定向缺少 location")?;
            url = url.join(location).map_err(|e| e.to_string())?;
            continue;
        }
        if !(200..300).contains(&status) {
            return Err(format!("HTTP 返回 {status}"));
        }
        let mut bytes = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(262145)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 262144 {
            return Err("网页超过 256 KiB，请使用分页接口或脚本流式处理".into());
        }
        if cancel.load(Ordering::Relaxed) {
            return Err("运行已取消".into());
        }
        let body = String::from_utf8(bytes).map_err(|_| "网页不是 UTF-8，请使用脚本指定编码")?;
        let value = match node.config["extract"].as_str().unwrap_or("text") {
            "json" => {
                json!({"data":serde_json::from_str::<Value>(&body).map_err(|e|e.to_string())?})
            }
            "raw" => json!({"body":body}),
            "text" => json!({"text":html_text(&body)}),
            _ => return Err("extract 应为 text / raw / json".into()),
        };
        let mut output = value;
        output["status"] = json!(status);
        output["url"] = json!(url.as_str());
        return Ok(output);
    }
    Err("重定向超过 4 次".into())
}
pub(super) fn html_text(source: &str) -> String {
    use std::sync::LazyLock;
    static HIDDEN: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"(?is)<(?:script|style|noscript)\b[^>]*>.*?</(?:script|style|noscript)\s*>|<!--.*?-->",
        )
        .unwrap()
    });
    static TAGS: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?s)<[^>]+>").unwrap());
    let visible = HIDDEN.replace_all(source, "");
    TAGS.replace_all(&visible, " ")
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

//! `.link.json` 与网页纯文本缓存；键顺序及 URL hash 对齐 src/services/linkfile.ts。
use anyhow::{bail, Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cache {
    pub url: String,
    pub title: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub favicon: Option<String>,
    pub content: String,
    pub cached_at: String,
}
pub fn hash_url(url: &str) -> String {
    let h = url
        .encode_utf16()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32));
    let mut n = (h as i64).unsigned_abs();
    let mut out = Vec::new();
    loop {
        out.push(b"0123456789abcdefghijklmnopqrstuvwxyz"[(n % 36) as usize] as char);
        n /= 36;
        if n == 0 {
            break;
        }
    }
    out.into_iter().rev().collect()
}
/// 浏览笔记时的网络图片。不发送应用凭据、Cookie 或工作区路径。
pub fn fetch_image(url: &str) -> Result<Vec<u8>> {
    use std::io::Read;
    let parsed = url::Url::parse(url)?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        bail!("不支持的图片 URL")
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(20)))
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .build(),
        )
        .build()
        .into();
    let mut response = agent.get(url).header("Accept", "image/*").call()?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 16 * 1024 * 1024 {
        bail!("图片超过 16 MB 限制")
    }
    Ok(bytes)
}
pub fn cache_path(root: &Path, url: &str) -> PathBuf {
    root.join(".mochi")
        .join("link-cache")
        .join(format!("{}.json", hash_url(url)))
}
pub fn read_cache(root: &Path, url: &str) -> Option<Cache> {
    crate::json2::read_from_file(cache_path(root, url))
        .ok()
        .flatten()
}
/// 导入时保留链接文件的复合后缀，并通过 create_new 避免检查与复制间覆盖文件。
pub fn import_file(source: &Path, directory: &Path) -> Result<PathBuf> {
    use std::io::Write;
    let mut input = std::fs::File::open(source)?;
    let name = source.file_name().context("文件名无效")?.to_string_lossy();
    let (base, ext) = if name.to_ascii_lowercase().ends_with(".link.json") {
        name.split_at(name.len() - ".link.json".len())
    } else {
        match name.rfind('.') {
            Some(i) if i > 0 => name.split_at(i),
            _ => (name.as_ref(), ""),
        }
    };
    for n in 0..=1000 {
        let target = directory.join(if n == 0 {
            name.to_string()
        } else {
            format!("{base} {n}{ext}")
        });
        let mut output = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
        {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        };
        let copied = (|| -> Result<()> {
            std::io::copy(&mut input, &mut output)?;
            output.flush()?;
            output.sync_all()?;
            Ok(())
        })();
        drop(output);
        if let Err(e) = copied {
            let _ = std::fs::remove_file(&target);
            return Err(e);
        }
        return Ok(target);
    }
    bail!("无法生成唯一文件名")
}
fn capture(html: &str, re: &str) -> Option<String> {
    Regex::new(re)
        .ok()?
        .captures(html)?
        .get(1)
        .map(|v| v.as_str().to_owned())
}
pub fn extract_content(html: &str) -> String {
    let scripts = Regex::new(r"(?is)<script[^>]*>.*?</script>|<style[^>]*>.*?</style>").unwrap();
    let stripped = scripts.replace_all(html, "");
    let content = capture(&stripped, r"(?is)<main[^>]*>(.*?)</main>")
        .or_else(|| capture(&stripped, r"(?is)<article[^>]*>(.*?)</article>"))
        .unwrap_or_else(|| stripped.to_string());
    let tags = Regex::new(r"<[^>]+>").unwrap();
    let content = tags
        .replace_all(&content, " ")
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    content.split_whitespace().collect::<Vec<_>>().join(" ")
}
pub fn from_html(url: &str, html: &str) -> Cache {
    let title = capture(
        html,
        r#"(?is)<meta[^>]*property=["']og:title["'][^>]*content=["']([^"']+)["']"#,
    )
    .or_else(|| capture(html, r"(?is)<title[^>]*>([^<]+)</title>"))
    .unwrap_or_else(|| url.into());
    let description = capture(
        html,
        r#"(?is)<meta[^>]*property=["']og:description["'][^>]*content=["']([^"']+)["']"#,
    )
    .or_else(|| {
        capture(
            html,
            r#"(?is)<meta[^>]*name=["']description["'][^>]*content=["']([^"']+)["']"#,
        )
    })
    .unwrap_or_default();
    let favicon = capture(
        html,
        r#"(?is)<link[^>]*rel=["'](?:icon|shortcut icon)["'][^>]*href=["']([^"']+)["']"#,
    )
    .and_then(|p| {
        url::Url::parse(url)
            .ok()?
            .join(&p)
            .ok()
            .map(|u| u.to_string())
    });
    Cache {
        url: url.into(),
        title,
        description,
        favicon,
        content: extract_content(html),
        cached_at: crate::jstime::now(),
    }
}
fn fetch(url: &str) -> Result<Cache> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        bail!("仅支持 HTTP/HTTPS 网页")
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .build()
        .into();
    let mut response = agent.get(url).header("User-Agent", "Mochi/1.0").call()?;
    use std::io::Read;
    let mut html = String::new();
    response
        .body_mut()
        .as_reader()
        .take(8 * 1024 * 1024 + 1)
        .read_to_string(&mut html)?;
    if html.len() > 8 * 1024 * 1024 {
        bail!("网页超过 8 MB 缓存限制")
    }
    Ok(from_html(url, &html))
}
pub fn cache(root: &Path, path: &Path) -> Result<Cache> {
    let original: Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let url = original["url"].as_str().context("链接缺少 URL")?;
    let cache = fetch(url)?;
    crate::json2::write(cache_path(root, url), &cache)?;
    // 网络请求期间文件可能被编辑；更新最新版本，只改缓存相关字段。
    let mut link: Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    if link["url"] != original["url"] {
        bail!("链接 URL 已改变，请重新缓存")
    }
    let object = link.as_object_mut().context("链接 JSON 必须是对象")?;
    object.insert("cached".into(), true.into());
    object.insert("cachedAt".into(), cache.cached_at.clone().into());
    object.insert("title".into(), cache.title.clone().into());
    object.insert("description".into(), cache.description.clone().into());
    if let Some(icon) = &cache.favicon {
        object.insert("favicon".into(), icon.clone().into());
    } else {
        object.remove("favicon");
    }
    crate::files::FileService::new().write_file_safe(path, &crate::json2::serialize(&link)?)?;
    Ok(cache)
}
pub fn create(directory: &Path, url: &str, title: Option<&str>) -> Result<PathBuf> {
    let url = url.trim();
    if !url.starts_with("http://") && !url.starts_with("https://") {
        bail!("请输入 HTTP/HTTPS URL")
    }
    let data = match title {
        Some(t) => Cache {
            url: url.into(),
            title: t.into(),
            description: String::new(),
            favicon: None,
            content: String::new(),
            cached_at: String::new(),
        },
        None => fetch(url).unwrap_or_else(|_| from_html(url, "")),
    };
    let name = Regex::new(r#"[<>:"/\\|?*]|\s+"#)
        .unwrap()
        .replace_all(&data.title, "-")
        .chars()
        .take(50)
        .collect::<String>();
    let name = if name.trim_matches('.').is_empty() {
        "链接"
    } else {
        &name
    };
    let mut path = directory.join(format!("{name}.link.json"));
    let mut i = 1;
    while path.exists() {
        path = directory.join(format!("{name} ({i}).link.json"));
        i += 1;
    }
    let mut file = serde_json::Map::new();
    file.insert("type".into(), "link".into());
    file.insert("url".into(), url.into());
    file.insert("title".into(), data.title.into());
    file.insert("description".into(), data.description.into());
    if let Some(icon) = data.favicon {
        file.insert("favicon".into(), icon.into());
    }
    file.insert("createdAt".into(), crate::jstime::now().into());
    file.insert("cached".into(), false.into());
    use std::io::Write;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    output.write_all(crate::json2::serialize(&file)?.as_bytes())?;
    Ok(path)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_fetch_uses_http_without_application_headers() {
        use std::io::{BufRead, Write};
        assert!(fetch_image("file:///C:/secret.png").is_err());
        assert!(fetch_image("https://user:secret@example.test/image").is_err());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/picture", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut headers = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                headers.push_str(&line);
            }
            let headers = headers.to_ascii_lowercase();
            assert!(!headers.contains("authorization:"));
            assert!(!headers.contains("cookie:"));
            assert!(!headers.contains("referer:"));
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nPNG"
            )
            .unwrap();
        });
        assert_eq!(fetch_image(&url).unwrap(), b"PNG");
        server.join().unwrap();
    }
    #[test]
    fn imports_preserve_compound_suffix_and_never_overwrite_source() {
        let root = std::env::temp_dir().join(format!(
            "mochi-import-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("中文.link.json");
        std::fs::write(&source, b"original").unwrap();
        let target = import_file(&source, &root).unwrap();
        assert_eq!(target.file_name().unwrap(), "中文 1.link.json");
        assert_eq!(std::fs::read(&source).unwrap(), b"original");
        assert_eq!(std::fs::read(&target).unwrap(), b"original");
        assert!(import_file(&root.join("missing"), &root).is_err());
        assert!(!root.join("missing").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn explicit_title_becomes_the_link_file_display_name() {
        let root = std::env::temp_dir().join(format!(
            "mochi-link-title-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = create(&root, "https://example.test/", Some("项目主页")).unwrap();
        assert_eq!(path.file_name().unwrap(), "项目主页.link.json");
        let created: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(created["title"], "项目主页");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn hash_uses_signed_utf16_js_arithmetic() {
        assert_eq!(hash_url("abc"), "22ci");
        assert_ne!(hash_url("😀"), hash_url(""));
    }
    #[test]
    fn cache_extracts_main_and_omits_scripts() {
        let html="<title>标题</title><nav>不要</nav><main>甲 <b>乙</b>&amp;丙<script>坏内容</script></main>";
        let c = from_html("https://example.test/", html);
        assert_eq!(c.title, "标题");
        assert_eq!(c.content, "甲 乙 &丙");
        let json = crate::json2::serialize(&c).unwrap();
        assert!(!json.contains("favicon"));
        assert!(!json.ends_with('\n'));
    }
}

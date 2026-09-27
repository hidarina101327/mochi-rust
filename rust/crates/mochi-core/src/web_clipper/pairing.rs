//! 调用 `approve` 前，必须先在桌面应用中明确批准。
use super::*;

pub fn extension_from_url(raw: &str) -> Result<String> {
    let url = url::Url::parse(raw)?;
    if url.scheme() != "mochi-clipper"
        || url.host_str() != Some("connect")
        || !matches!(url.path(), "" | "/")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
    {
        bail!("无效的网页剪藏连接请求");
    }
    let params = url.query_pairs().collect::<Vec<_>>();
    if params.len() != 1 || params[0].0 != "extension" || !valid_extension_id(&params[0].1) {
        bail!("无效的浏览器扩展");
    }
    Ok(params[0].1.to_string())
}

pub fn valid_extension_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| (b'a'..=b'p').contains(&b))
}

/// 串行处理并发浏览器授权请求中的读取、追加和刷新操作。
pub fn approve(settings: &SettingsService, id: &str) -> Result<()> {
    update(settings, id, true)
}

pub fn revoke(settings: &SettingsService, id: &str) -> Result<()> {
    update(settings, id, false)
}

fn update(settings: &SettingsService, id: &str, add: bool) -> Result<()> {
    if !valid_extension_id(id) {
        bail!("无效的浏览器扩展");
    }
    let _guard = crate::settings::file::Lock::acquire(
        &settings.file_path().with_extension("clipper-bindings"),
    )?;
    settings.reload()?;
    let mut ids = extension_ids(settings)?;
    if add && !ids.iter().any(|existing| existing == id) {
        ids.push(id.into());
    }
    if !add {
        ids.retain(|existing| existing != id);
    }
    settings.set("webClipper.extensionIds", &ids.join(","));
    if add {
        settings.set("webClipper.enabled", "true");
    }
    settings.flush()
}

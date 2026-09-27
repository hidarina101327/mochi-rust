//! 带版本号的本地浏览器剪藏协议。不监听网络，也不接受任意路径。
#[cfg(windows)]
pub mod native;
pub mod pairing;
mod storage;
#[cfg(test)]
mod tests;

use crate::{settings::SettingsService, workspace::WorkspaceService};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const VERSION: u32 = 1;
pub const MAX_TOTAL: u64 = 100 * 1024 * 1024;
pub const MAX_IMAGE: u64 = 20 * 1024 * 1024;
pub const MAX_FRAME: usize = 768 * 1024;
pub const HOST: &str = "com.mochi.web_clipper";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClipMetadata {
    pub id: String,
    pub title: String,
    pub url: String,
    pub author: Option<String>,
    pub captured_at: i64,
    pub mode: String,
    pub format: String,
    pub document: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UploadFile {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClipInput {
    pub clip_id: String,
    pub workspace: String,
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub author: Option<String>,
    pub captured_at: i64,
    pub mode: String,
    pub format: String,
    pub destination: String,
    #[serde(default)]
    pub library_id: Option<String>,
    #[serde(default)]
    pub folder: String,
    #[serde(default)]
    pub excerpt: String,
    pub files: Vec<UploadFile>,
}

pub struct Receiver {
    pub workspace: Arc<Mutex<Option<PathBuf>>>,
    pub settings: Arc<SettingsService>,
    gate: Mutex<()>,
}

impl Receiver {
    pub fn new(settings: Arc<SettingsService>) -> Self {
        Self {
            workspace: Arc::new(Mutex::new(None)),
            settings,
            gate: Mutex::new(()),
        }
    }
    pub fn handle(&self, origin: &str, request: Value) -> Value {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let result = (|| {
            if request.get("version").and_then(Value::as_u64) != Some(VERSION as u64) {
                bail!("协议版本不兼容，请更新扩展和墨池");
            }
            if self.settings.get("webClipper.enabled").as_deref() != Some("true") {
                bail!("请先在墨池设置中启用网页剪藏");
            }
            if !allowed(&self.settings, origin) {
                bail!("尚未连接此浏览器，请在扩展设置中点击“连接墨池”");
            }
            let _guard = self.gate.lock().unwrap_or_else(|e| e.into_inner());
            if request["op"] == "disconnect" {
                let id = origin
                    .strip_prefix("chrome-extension://")
                    .unwrap_or("")
                    .trim_end_matches('/');
                pairing::revoke(&self.settings, id)?;
                #[cfg(windows)]
                native::register(&self.settings)?;
                return Ok(json!({"disconnected":true}));
            }
            self.dispatch(&request)
        })();
        match result {
            Ok(value) => json!({"version":VERSION,"id":id,"ok":true,"result":value}),
            Err(error) => {
                json!({"version":VERSION,"id":id,"ok":false,"error":format!("{error:#}")})
            }
        }
    }
    fn dispatch(&self, r: &Value) -> Result<Value> {
        let op = field(r, "op")?;
        if op == "ping" {
            return Ok(json!({"version":VERSION}));
        }
        let root = self
            .workspace
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .context("请在墨池中打开工作区，然后重试")?;
        let token = workspace_token(&root);
        if op == "context" {
            let libraries = libraries(&root)?;
            return Ok(
                json!({"workspace":token,"name":root.file_name().unwrap_or_default().to_string_lossy(),"libraries":libraries,"defaults":self.defaults()}),
            );
        }
        if field(r, "workspace")? != token {
            bail!("工作区已切换，请重新选择保存位置后重试");
        }
        match op {
            "folders" => {
                let parent = library_target(
                    &root,
                    field(r, "libraryId")?,
                    r["folder"].as_str().unwrap_or(""),
                )?;
                let mut names = Vec::new();
                for entry in std::fs::read_dir(parent)? {
                    let entry = entry?;
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if entry.file_type()?.is_dir()
                        && !name.starts_with('.')
                        && name != "assets"
                        && !is_reparse(&entry.path())?
                    {
                        names.push(name);
                    }
                }
                names.sort();
                Ok(json!(names))
            }
            "defaults" => {
                let library_id = field(r, "libraryId")?;
                let folder = r["folder"].as_str().unwrap_or("");
                let target = library_target(&root, library_id, folder)?;
                let rel = target
                    .strip_prefix(&root)?
                    .to_string_lossy()
                    .replace('\\', "/");
                self.settings.set("webClipper.defaultDirectory", &rel);
                self.settings.flush()?;
                Ok(self.defaults())
            }
            "begin" => storage::begin(&root, serde_json::from_value(r["clip"].clone())?),
            "chunk" => storage::chunk(
                &root,
                field(r, "clipId")?,
                field(r, "name")?,
                r["offset"].as_u64().context("缺少 offset")?,
                field(r, "data")?,
            ),
            "commit" => storage::commit(&root, field(r, "clipId")?, &self.settings),
            "result" => storage::result(&root, field(r, "clipId")?),
            _ => bail!("未知网页剪藏操作"),
        }
    }
    fn defaults(&self) -> Value {
        json!({"directory":self.settings.get("webClipper.defaultDirectory").unwrap_or_else(||"知识库/浏览器收藏".into())})
    }
}

fn field<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key].as_str().with_context(|| format!("缺少字段 {key}"))
}
pub fn valid_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 80 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}
pub fn extension_ids(settings: &SettingsService) -> Result<Vec<String>> {
    let raw = settings.get("webClipper.extensionIds").unwrap_or_default();
    let ids: Vec<String> = raw
        .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if ids.iter().any(|s| !pairing::valid_extension_id(s)) {
        bail!("扩展 ID 应为 32 位 a–p 小写字母，多个 ID 用逗号分隔");
    }
    Ok(ids)
}
pub fn allowed(settings: &SettingsService, origin: &str) -> bool {
    extension_ids(settings)
        .unwrap_or_default()
        .iter()
        .any(|id| origin == format!("chrome-extension://{id}/"))
}
pub fn workspace_token(root: &Path) -> String {
    use sha2::{Digest, Sha256};
    hex(&Sha256::digest(
        root.to_string_lossy()
            .replace('\\', "/")
            .to_lowercase()
            .as_bytes(),
    ))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn libraries(root: &Path) -> Result<Vec<crate::domain::Library>> {
    let all: Vec<crate::domain::Library> =
        crate::json2::read_from_file(&crate::paths::libraries_file(root))?.unwrap_or_default();
    Ok(all
        .into_iter()
        .filter(|l| l.kind == "knowledge-base" && contained(root, Path::new(&l.path)).is_ok())
        .collect())
}
fn library_target(root: &Path, id: &str, folder: &str) -> Result<PathBuf> {
    let lib = libraries(root)?
        .into_iter()
        .find(|l| l.id == id)
        .context("知识库不存在或位于工作区外")?;
    let relative = safe_relative(folder, true)?;
    let path = Path::new(&lib.path).join(relative);
    contained(root, &path)?;
    if !path.is_dir() {
        bail!("目标文件夹不存在，请重新选择");
    }
    Ok(path)
}
pub fn safe_relative(s: &str, empty: bool) -> Result<PathBuf> {
    if (!empty && s.is_empty()) || s.contains('\\') || s.contains(':') {
        bail!("非法相对路径");
    }
    let path = Path::new(s);
    if path
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
        || s.split('/').any(|p| p == ".." || p == ".")
    {
        bail!("路径不能越过目标目录");
    }
    Ok(path.to_path_buf())
}
pub fn contained(root: &Path, path: &Path) -> Result<()> {
    let root = root.canonicalize()?;
    let real = path.canonicalize()?;
    if !real.starts_with(&root) {
        bail!("目标位于工作区外");
    }
    let mut ancestor = Some(path);
    while let Some(p) = ancestor {
        if is_reparse(p)? {
            bail!("不支持符号链接或映射目录");
        }
        if p.canonicalize()? == root {
            break;
        }
        ancestor = p.parent();
    }
    Ok(())
}
fn is_reparse(path: &Path) -> Result<bool> {
    let m = std::fs::symlink_metadata(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        Ok(m.file_attributes() & 0x400 != 0)
    }
    #[cfg(not(windows))]
    {
        Ok(m.file_type().is_symlink())
    }
}
pub fn default_target(root: &Path, settings: &SettingsService) -> Result<PathBuf> {
    let rel = settings
        .get("webClipper.defaultDirectory")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "知识库/浏览器收藏".into());
    safe_relative(&rel, false)?;
    if rel == "知识库/浏览器收藏" && !root.join(&rel).exists() {
        contained(root, &root.join("知识库"))?;
        WorkspaceService::new(root)?.create_library(
            "knowledge-base",
            "知识库",
            "浏览器收藏",
            None,
        )?;
    }
    let path = root.join(rel);
    contained(root, &path)?;
    if !libraries(root)?.iter().any(|l| path.starts_with(&l.path)) {
        bail!("默认位置不是工作区内的知识库，请重新设置");
    }
    Ok(path)
}

fn create_directory(root: &Path, relative: &str) -> Result<PathBuf> {
    let relative = safe_relative(relative, false)?;
    contained(root, root)?;
    let mut path = root.to_path_buf();
    for component in relative.components() {
        path.push(component.as_os_str());
        if !path.exists() {
            std::fs::create_dir(&path)?;
        }
        contained(root, &path)?;
        if !path.is_dir() {
            bail!("目标不是文件夹");
        }
    }
    Ok(path)
}

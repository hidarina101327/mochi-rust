//! 管理网页剪藏分段上传任务及其输入、结果数据。
use super::*;
use base64::Engine;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
};

fn job(root: &Path, id: &str) -> Result<PathBuf> {
    if !valid_id(id) {
        bail!("非法剪藏 ID");
    }
    create_directory(root, &format!(".mochi/web-clipper/{id}"))
}
fn read_input(dir: &Path) -> Result<ClipInput> {
    Ok(serde_json::from_slice(&fs::read(dir.join("input.json"))?)?)
}
fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    crate::files::FileService::new().write_file_safe(path, &serde_json::to_string_pretty(value)?)
}
pub fn begin(root: &Path, input: ClipInput) -> Result<Value> {
    if input.workspace != workspace_token(root) {
        bail!("剪藏工作区不匹配");
    }
    if !["markdown", "html", "pdf"].contains(&input.format.as_str())
        || !["article", "selection", "screenshot"].contains(&input.mode.as_str())
    {
        bail!("未知剪藏模式或格式");
    }
    if !["inbox", "default", "custom"].contains(&input.destination.as_str()) {
        bail!("未知保存入口");
    }
    let url = url::Url::parse(&input.url)?;
    if !["http", "https"].contains(&url.scheme()) {
        bail!("只支持 HTTP/HTTPS 网页");
    }
    if input.title.trim().is_empty()
        || input.title.len() > 2048
        || input.excerpt.len() > 16000
        || input.files.is_empty()
        || input.files.len() > 1000
    {
        bail!("标题或文件清单不合法");
    }
    let extension = match input.format.as_str() {
        "markdown" => "md",
        "html" => "html",
        _ => "pdf",
    };
    let expected = format!("document.{extension}");
    if input.files[0].name != expected {
        bail!("正文文件格式不匹配");
    }
    let mut names = std::collections::HashSet::new();
    let mut total = 0u64;
    for (i, file) in input.files.iter().enumerate() {
        safe_relative(&file.name, false)?;
        if i > 0 && (!file.name.starts_with("assets/") || file.name.matches('/').count() != 1) {
            bail!("附件必须位于 assets 目录");
        }
        if !names.insert(file.name.to_lowercase())
            || file.sha256.len() != 64
            || !file.sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            bail!("文件名重复或哈希无效");
        }
        if i > 0 && file.size > MAX_IMAGE {
            bail!("单张图片不能超过 20 MiB");
        }
        total = total.checked_add(file.size).context("文件过大")?;
        if total > MAX_TOTAL {
            bail!("单次剪藏不能超过 100 MiB");
        }
    }
    let dir = job(root, &input.clip_id)?;
    if dir.join("input.json").exists() {
        if serde_json::to_value(read_input(&dir)?)? != serde_json::to_value(&input)? {
            bail!("剪藏 ID 已用于其他内容");
        }
    } else {
        write_json(&dir.join("input.json"), &input)?;
    }
    create_directory(
        root,
        &format!(".mochi/web-clipper/{}/bundle/assets", input.clip_id),
    )?;
    Ok(json!({"clipId":input.clip_id}))
}
pub fn chunk(root: &Path, id: &str, name: &str, offset: u64, data: &str) -> Result<Value> {
    let dir = job(root, id)?;
    if dir.join("receipt.json").exists() {
        return result(root, id);
    }
    let input = read_input(&dir)?;
    let file = input
        .files
        .iter()
        .find(|f| f.name == name)
        .context("文件不在上传清单中")?;
    if data.len() > 384 * 1024 {
        bail!("分块过大");
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(data)?;
    let end = offset.checked_add(bytes.len() as u64).context("偏移溢出")?;
    if end > file.size {
        bail!("上传超出声明大小");
    }
    let path = dir.join("bundle").join(safe_relative(name, false)?);
    contained(root, path.parent().unwrap())?;
    if path.exists() {
        contained(root, &path)?;
    }
    let mut out = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    let current = out.metadata()?.len();
    if offset < current {
        if end > current {
            bail!("分块重叠，请重试");
        }
        out.seek(SeekFrom::Start(offset))?;
        let mut existing = vec![0; bytes.len()];
        out.read_exact(&mut existing)?;
        if existing != bytes {
            bail!("重复分块内容不一致");
        }
    } else {
        if offset != current {
            bail!("分块顺序错误");
        }
        out.seek(SeekFrom::End(0))?;
        out.write_all(&bytes)?;
        out.sync_data()?;
    }
    Ok(json!({"offset":end}))
}
pub fn result(root: &Path, id: &str) -> Result<Value> {
    let dir = job(root, id)?;
    if dir.join("receipt.json").exists() {
        return Ok(serde_json::from_slice(&fs::read(
            dir.join("receipt.json"),
        )?)?);
    }
    Ok(json!({"status":"pending"}))
}
pub fn commit(root: &Path, id: &str, settings: &SettingsService) -> Result<Value> {
    let dir = job(root, id)?;
    let _lock = crate::settings::file::Lock::acquire(&dir.join("commit"))?;
    if dir.join("receipt.json").exists() {
        return result(root, id);
    }
    let input = read_input(&dir)?;
    let destination = if dir.join("target.json").exists() {
        let relative: String = serde_json::from_slice(&fs::read(dir.join("target.json"))?)?;
        root.join(safe_relative(&relative, false)?)
    } else {
        let parent = match input.destination.as_str() {
            "inbox" => create_directory(root, "收件箱/web-clips")?,
            "default" => default_target(root, settings)?,
            _ => library_target(
                root,
                input.library_id.as_deref().context("请选择知识库")?,
                &input.folder,
            )?,
        };
        contained(root, &parent)?;
        // 资源包会保留图片的相对路径，并支持一次性原子发布。
        let clean: String = input
            .title
            .chars()
            .filter(|c| !c.is_control() && !"<>:\"/\\|?*".contains(*c))
            .take(60)
            .collect();
        let clean = clean.trim().trim_end_matches('.');
        let name = format!(
            "{}--{}",
            if clean.is_empty() {
                "网页收藏"
            } else {
                clean
            },
            id
        );
        let target = parent.join(name);
        write_json(
            &dir.join("target.json"),
            &target
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/"),
        )?;
        target
    };
    contained(root, destination.parent().unwrap())?;
    if !destination.exists() {
        let bundle = dir.join("bundle");
        for file in &input.files {
            let path = bundle.join(&file.name);
            contained(root, &path)?;
            if fs::metadata(&path)?.len() != file.size {
                bail!("文件未上传完成：{}", file.name);
            }
            let mut reader = fs::File::open(path)?;
            let mut hash = Sha256::new();
            let mut buf = [0u8; 65536];
            loop {
                let n = reader.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hash.update(&buf[..n]);
            }
            if hex(&hash.finalize()) != file.sha256.to_lowercase() {
                bail!("文件校验失败：{}", file.name);
            }
        }
        let document = bundle.join(&input.files[0].name);
        if input.format == "pdf" {
            let mut bytes = [0; 5];
            fs::File::open(document)?.read_exact(&mut bytes)?;
            if &bytes != b"%PDF-" {
                bail!("无效 PDF 文件");
            }
        }
        write_json(&bundle.join(".clip.json"), &input)?;
        fs::rename(&bundle, &destination)?;
    } else {
        contained(root, &destination)?;
        let original: ClipInput =
            serde_json::from_slice(&fs::read(destination.join(".clip.json"))?)?;
        if original.clip_id != id {
            bail!("目标已存在");
        }
    }
    let document = destination.join(&input.files[0].name);
    let meta = ClipMetadata {
        id: id.into(),
        title: input.title.clone(),
        url: input.url.clone(),
        author: input.author.clone(),
        captured_at: input.captured_at,
        mode: input.mode.clone(),
        format: input.format.clone(),
        document: document
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/"),
    };
    if input.destination == "inbox" {
        crate::capture::CaptureService::new(root).add_web_clip(meta.clone(), &input.excerpt)?;
    }
    let receipt = json!({"status":"saved","clipId":id,"document":meta.document,"destination":input.destination});
    write_json(&dir.join("receipt.json"), &receipt)?;
    Ok(receipt)
}

//! 工作区内保存相对 POSIX 路径，区外保存绝对路径。
//! 编码遵循 URLSearchParams：空格转 +，斜杠和非 ASCII 按 UTF-8 百分号转义。

use std::path::Path;

pub const MOCHI_OPEN_PREFIX: &str = "mochi://open";
pub const MOCHI_BLOCK_PREFIX: &str = "mochi://block";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    File,
    Directory,
    Task,
    Event,
    Project,
    /// 不指向具体消息的 AI 会话。归入同一资源 URL 家族，base 单元格和
    /// 文档块就能共用一个解析器。具体提问继续走 mochi://ai-locate。
    AiSession,
}

fn to_posix(s: &str) -> String {
    s.replace('\\', "/")
}

fn strip_trailing_separators(s: &str) -> &str {
    s.trim_end_matches(['/', '\\'])
}

/// 目标在工作区内则返回 POSIX 相对路径。Windows 路径大小写不敏感，盘符大小写不一致不该让链接失效。
pub fn to_workspace_relative_path(target: &str, workspace: Option<&str>) -> Option<String> {
    let workspace = workspace?;
    let target = to_posix(target);
    let target = strip_trailing_separators(&target);
    let root = to_posix(workspace);
    let root = strip_trailing_separators(&root);
    if target.is_empty() || root.is_empty() {
        return None;
    }
    let prefix = format!("{}/", root.to_lowercase());
    if !target.to_lowercase().starts_with(&prefix) {
        return None;
    }
    Some(target[prefix.len()..].to_owned())
}

/// `URLSearchParams` 的值编码。
pub fn form_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn build_mochi_resource_url(
    target: &Path,
    kind: ResourceKind,
    workspace: Option<&Path>,
) -> String {
    let target_s = target.to_string_lossy();
    let workspace_s = workspace.map(|w| w.to_string_lossy().into_owned());
    let relative = to_workspace_relative_path(&target_s, workspace_s.as_deref());
    let path = relative.unwrap_or_else(|| to_posix(strip_trailing_separators(&target_s)));
    let mut url = format!("{MOCHI_OPEN_PREFIX}?path={}", form_encode(&path));
    match kind {
        ResourceKind::Directory => url.push_str("&kind=directory"),
        ResourceKind::Task => url.push_str("&kind=task"),
        ResourceKind::Event => url.push_str("&kind=event"),
        ResourceKind::Project => url.push_str("&kind=project"),
        ResourceKind::AiSession => url.push_str("&kind=ai-session"),
        ResourceKind::File => {}
    }
    url
}

/// 构造指向已持久化 Markdown 块的引用。块 ID 单独放一个查询参数：
/// 路径重命名时只需更新 `path`，稳定身份和呈现元数据原样保留。
pub fn build_mochi_block_url(target: &Path, block_id: &str, workspace: Option<&Path>) -> String {
    let target_s = target.to_string_lossy();
    let workspace_s = workspace.map(|w| w.to_string_lossy().into_owned());
    let relative = to_workspace_relative_path(&target_s, workspace_s.as_deref());
    let path = relative.unwrap_or_else(|| to_posix(strip_trailing_separators(&target_s)));
    format!(
        "{MOCHI_BLOCK_PREFIX}?path={}&id={}",
        form_encode(&path),
        form_encode(block_id)
    )
}

/// 从 `mochi://block` URL 中取出的路径与稳定 ID。ID 的 UUID 形状校验
/// 归 `object_reference` 管——对象类型是在那里才知道的；这个底层解析器
/// 只对齐 URL 编码契约。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockRef {
    pub path: String,
    pub block_id: String,
    pub label: Option<String>,
}

pub fn parse_mochi_block_url(text: &str) -> Option<BlockRef> {
    let trimmed = text.trim();
    let prefix = format!("{MOCHI_BLOCK_PREFIX}?");
    if !trimmed
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(&prefix))
    {
        return None;
    }
    let query = &trimmed[prefix.len()..];
    if query.is_empty() || query.contains('#') || query.chars().any(char::is_whitespace) {
        return None;
    }
    let mut path = None;
    let mut block_id = None;
    let mut label = None;
    for pair in query.split('&') {
        if !valid_percent_encoding(pair) {
            return None;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match form_decode(key).as_str() {
            "path" if path.is_none() => path = Some(form_decode(value)),
            "path" => return None,
            "id" if block_id.is_none() => block_id = Some(form_decode(value)),
            "id" => return None,
            "label" if label.is_none() => label = Some(form_decode(value)),
            "label" => return None,
            _ => {}
        }
    }
    Some(BlockRef {
        path: to_posix(&path.filter(|p| !p.is_empty())?),
        block_id: block_id.filter(|id| !id.is_empty())?,
        label,
    })
}

fn valid_percent_encoding(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.iter().enumerate().any(|(index, byte)| {
        *byte == b'%'
            && !bytes
                .get(index + 1..index + 3)
                .is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit))
    })
}

/// 一条 Mochi 链接解析出来的目标。`path` 是 POSIX 形式（相对工作区，或绝对路径）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceRef {
    pub path: String,
    pub kind: ResourceKind,
    pub table_id: Option<String>,
    pub record_id: Option<String>,
    pub field_id: Option<String>,
    pub item_id: Option<String>,
    pub label: Option<String>,
}

pub fn build_mochi_record_url(
    target: &Path,
    table_id: &str,
    record_id: &str,
    field_id: Option<&str>,
    workspace: Option<&Path>,
) -> String {
    let mut url = build_mochi_resource_url(target, ResourceKind::File, workspace);
    url.push_str(&format!(
        "&table={}&record={}",
        form_encode(table_id),
        form_encode(record_id)
    ));
    if let Some(field) = field_id.filter(|field| !field.is_empty()) {
        url.push_str(&format!("&field={}", form_encode(field)));
    }
    url
}

/// `URLSearchParams` 的值解码：`+` 是空格，`%XX` 是字节。非法的百分号序列原样保留。
pub fn form_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => {
                let decoded = bytes
                    .get(i + 1..i + 3)
                    .and_then(|hex| std::str::from_utf8(hex).ok())
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok());
                match decoded {
                    Some(b) => {
                        out.push(b);
                        i += 3;
                        continue;
                    }
                    None => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 解析 Mochi 链接。允许首尾空白；query 内部不允许空白，必须是一个完整值，
/// 缺少 path 视为无效。前缀大小写不敏感，与 TS 版一致。
pub fn parse_mochi_resource_url(text: &str) -> Option<ResourceRef> {
    let trimmed = text.trim();
    let head = format!("{MOCHI_OPEN_PREFIX}?");
    if !trimmed.to_lowercase().starts_with(&head) {
        return None;
    }
    let query = &trimmed[head.len()..];
    if query.is_empty() || query.contains('#') || query.chars().any(char::is_whitespace) {
        return None;
    }
    let mut path = None;
    let mut kind = ResourceKind::File;
    let mut kind_seen = false;
    let (mut table_id, mut record_id, mut field_id, mut item_id, mut label) =
        (None, None, None, None, None);
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        match form_decode(k).as_str() {
            // 与 URLSearchParams.get 一致：同名取第一个
            "path" if path.is_none() => path = Some(form_decode(v)),
            "kind" if !kind_seen => {
                kind = match form_decode(v).as_str() {
                    "file" | "" => ResourceKind::File,
                    "directory" => ResourceKind::Directory,
                    "task" => ResourceKind::Task,
                    "event" => ResourceKind::Event,
                    "project" => ResourceKind::Project,
                    "ai-session" => ResourceKind::AiSession,
                    _ => return None,
                };
                kind_seen = true;
            }
            "table" if table_id.is_none() => table_id = Some(form_decode(v)),
            "record" if record_id.is_none() => record_id = Some(form_decode(v)),
            "field" if field_id.is_none() => field_id = Some(form_decode(v)),
            "item" if item_id.is_none() => item_id = Some(form_decode(v)),
            "label" if label.is_none() => label = Some(form_decode(v)),
            _ => {}
        }
    }
    let path = path.filter(|p| !p.is_empty())?;
    Some(ResourceRef {
        path: to_posix(&path),
        kind,
        table_id,
        record_id,
        field_id,
        item_id,
        label,
    })
}

fn is_absolute_path(value: &str) -> bool {
    let b = value.as_bytes();
    (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'/' || b[2] == b'\\'))
        || value.starts_with('/')
        || value.starts_with('\\')
}

/// 沿用路径本身的分隔符风格：同一个文件若出现正反斜杠两种写法会被标签页去重当成两个文件。
fn separator_of(value: &str) -> char {
    if value.contains('\\') {
        '\\'
    } else if value.contains('/') {
        '/'
    } else if is_absolute_path(value) {
        '\\'
    } else {
        '/'
    }
}

/// 把链接里的路径还原成本地绝对路径，分隔符跟随工作区路径的风格。
pub fn resolve_workspace_path(ref_path: &str, workspace: Option<&str>) -> Option<String> {
    let raw = strip_trailing_separators(ref_path.trim());
    if raw.is_empty() {
        return None;
    }
    // 工作区之外的目标只能原样用
    if is_absolute_path(raw) {
        return Some(raw.to_owned());
    }
    let root = strip_trailing_separators(workspace?);
    let sep = separator_of(root);
    let tail: Vec<&str> = raw.split('/').collect();
    Some(format!("{root}{sep}{}", tail.join(&sep.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    #[test]
    fn record_links_preserve_stable_identifiers_and_workspace_relative_paths() {
        let url = build_mochi_record_url(
            Path::new("D:/ws/成长.mcb"),
            "table /甲",
            "record+乙",
            Some("field &丙"),
            Some(Path::new("D:/ws")),
        );
        let reference = parse_mochi_resource_url(&url).unwrap();
        assert_eq!(reference.path, "成长.mcb");
        assert_eq!(reference.table_id.as_deref(), Some("table /甲"));
        assert_eq!(reference.record_id.as_deref(), Some("record+乙"));
        assert_eq!(reference.field_id.as_deref(), Some("field &丙"));
        assert!(crate::base::is_valid_base_reference(&url));
    }

    #[test]
    fn block_links_keep_the_path_and_id_as_separate_encoded_values() {
        let url = build_mochi_block_url(
            Path::new(r"D:\ws\知识库\a b.md"),
            "block_00000000-0000-4000-8000-000000000001",
            Some(Path::new(r"D:\ws")),
        );
        assert_eq!(
            url,
            "mochi://block?path=%E7%9F%A5%E8%AF%86%E5%BA%93%2Fa+b.md&id=block_00000000-0000-4000-8000-000000000001"
        );
        assert_eq!(
            parse_mochi_block_url(&format!(
                "{url}&label=%E5%8F%91%E5%B8%83+%E8%AE%A1%E5%88%92"
            )),
            Some(BlockRef {
                path: "知识库/a b.md".into(),
                block_id: "block_00000000-0000-4000-8000-000000000001".into(),
                label: Some("发布 计划".into()),
            })
        );
    }

    #[test]
    fn block_parser_rejects_missing_identity_and_path_fragments() {
        assert!(parse_mochi_block_url("mochi://block?path=note.md").is_none());
        assert!(parse_mochi_block_url("mochi://block?id=block_1").is_none());
        assert!(parse_mochi_block_url("mochi://block?path=note.md&id=block_1#part").is_none());
        assert!(parse_mochi_block_url("mochi://block?path=note.md&id=block_1%ZZ").is_none());
    }

    #[test]
    fn parsing_round_trips_what_building_produced() {
        let url = build_mochi_resource_url(
            &PathBuf::from(r"D:\mochi\知识库\a b.md"),
            ResourceKind::File,
            Some(&PathBuf::from(r"D:\mochi")),
        );
        let r = parse_mochi_resource_url(&url).unwrap();
        assert_eq!(r.path, "知识库/a b.md");
        assert_eq!(r.kind, ResourceKind::File);
        assert!(r.table_id.is_none());
        let dir = parse_mochi_resource_url("  MOCHI://open?path=x%2Fy&kind=directory  ").unwrap();
        assert_eq!(dir.kind, ResourceKind::Directory);
        assert_eq!(dir.path, "x/y");
    }

    #[test]
    fn invalid_links_are_rejected_like_the_ts_version() {
        assert!(
            parse_mochi_resource_url("mochi://open?").is_none(),
            "空 query"
        );
        assert!(
            parse_mochi_resource_url("mochi://open?path=a b").is_none(),
            "query 里有空白"
        );
        assert!(
            parse_mochi_resource_url("mochi://open?kind=directory").is_none(),
            "缺 path"
        );
        assert!(parse_mochi_resource_url("https://example.com").is_none());
    }

    #[test]
    fn resolving_follows_the_workspace_separator_style() {
        assert_eq!(
            resolve_workspace_path("知识库/a.md", Some(r"D:\mochi")),
            Some(r"D:\mochi\知识库\a.md".to_owned())
        );
        assert_eq!(
            resolve_workspace_path("a/b", Some("/home/u/ws/")),
            Some("/home/u/ws/a/b".to_owned())
        );
        assert_eq!(
            resolve_workspace_path("C:/tmp/x.md", None),
            Some("C:/tmp/x.md".to_owned()),
            "绝对路径原样"
        );
        assert_eq!(
            resolve_workspace_path("a.md", None),
            None,
            "相对路径没有工作区就无解"
        );
        assert_eq!(resolve_workspace_path("   ", Some("D:\\w")), None);
    }

    #[test]
    fn a_file_inside_the_workspace_gets_a_relative_posix_path() {
        let url = build_mochi_resource_url(
            &PathBuf::from(r"D:\mochi\知识库\计算机通识\操作系统.md"),
            ResourceKind::File,
            Some(&PathBuf::from(r"D:\mochi")),
        );
        // URLSearchParams：`/` → %2F，中文 → UTF-8 百分号
        assert_eq!(
            url,
            "mochi://open?path=%E7%9F%A5%E8%AF%86%E5%BA%93%2F%E8%AE%A1%E7%AE%97%E6%9C%BA%E9%80%9A%E8%AF%86%2F%E6%93%8D%E4%BD%9C%E7%B3%BB%E7%BB%9F.md"
        );
    }

    #[test]
    fn a_directory_carries_the_kind_parameter_and_drive_letter_case_is_ignored() {
        let url = build_mochi_resource_url(
            &PathBuf::from(r"d:\Mochi\知识库\面试经历\"),
            ResourceKind::Directory,
            Some(&PathBuf::from(r"D:\mochi")),
        );
        assert!(url.ends_with("&kind=directory"));
        assert!(url.contains("path=%E7%9F%A5%E8%AF%86%E5%BA%93%2F"), "{url}");
    }

    #[test]
    fn a_target_outside_the_workspace_falls_back_to_the_absolute_posix_path() {
        let url = build_mochi_resource_url(
            &PathBuf::from(r"C:\tmp\a b.md"),
            ResourceKind::File,
            Some(&PathBuf::from(r"D:\mochi")),
        );
        assert_eq!(url, "mochi://open?path=C%3A%2Ftmp%2Fa+b.md");
    }

    #[test]
    fn form_encoding_keeps_the_unreserved_set_and_turns_space_into_plus() {
        assert_eq!(form_encode("a-b_c.d*e f/g"), "a-b_c.d*e+f%2Fg");
    }
}

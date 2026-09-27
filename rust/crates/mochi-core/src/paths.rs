//! 目录名是磁盘协议的一部分，中文名称不能改成英文。

use std::path::{Path, PathBuf};

pub const MOCHI_DIR_NAME: &str = ".mochi";

// 库类型 / Agent 配置约定（src/services/library.ts、aiPrompts.ts）
pub const KNOWLEDGE_BASE_TYPE_NAME: &str = "知识库";
pub const AGENT_CONFIG_DIR: &str = "Agent配置";
pub const LEGACY_AGENT_CONFIG_DIR: &str = "AI提示词";
pub const SCHEDULE_DIR_NAME: &str = "schedule";
pub const INBOX_DIR_NAME: &str = "收件箱";

pub const AGENT_CONFIG_SECTIONS: [&str; 5] = ["Agents", "Skills", "Tools", "MCPs", "QuickActions"];

pub fn mochi_dir(ws: impl AsRef<Path>) -> PathBuf {
    ws.as_ref().join(MOCHI_DIR_NAME)
}

pub fn config_file(ws: impl AsRef<Path>) -> PathBuf {
    mochi_dir(ws).join("config.json")
}

pub fn sidebar_file(ws: impl AsRef<Path>) -> PathBuf {
    mochi_dir(ws).join("sidebar.json")
}

pub fn ai_permissions_file(ws: impl AsRef<Path>) -> PathBuf {
    mochi_dir(ws).join("ai-permissions.json")
}

pub fn library_types_file(ws: impl AsRef<Path>) -> PathBuf {
    mochi_dir(ws).join("library-types.json")
}

pub fn libraries_file(ws: impl AsRef<Path>) -> PathBuf {
    mochi_dir(ws).join("libraries.json")
}

/// 工作区内映射到外部目录的只读展示配置。外部文件本身不写入工作区。
pub fn mapped_folders_file(ws: impl AsRef<Path>) -> PathBuf {
    mochi_dir(ws).join("mapped-folders.json")
}

/// Git 仓库目录。注意是 `.mochi/git` 而非 `.git`——打开时需写 `core.worktree` 自愈。
pub fn git_dir(ws: impl AsRef<Path>) -> PathBuf {
    mochi_dir(ws).join("git")
}

pub fn index_db(ws: impl AsRef<Path>) -> PathBuf {
    mochi_dir(ws).join("index.db")
}

/// 反斜杠 → 正斜杠（旧版所有持久化相对路径均为正斜杠）。
pub fn to_forward_slashes(path: &str) -> String {
    path.replace('\\', "/")
}

fn path_component_eq(left: std::path::Component<'_>, right: std::path::Component<'_>) -> bool {
    if cfg!(windows) {
        left.as_os_str()
            .to_string_lossy()
            .to_lowercase()
            .eq(&right.as_os_str().to_string_lossy().to_lowercase())
    } else {
        left == right
    }
}

/// 按路径组件判断包含关系，并遵循宿主文件系统的大小写规则。
/// `Path::starts_with` 在 Windows 上也区分大小写；而字符串前缀判断
/// 又会混淆 `C:\\notes` 和 `C:\\notes-old` 这种兄弟目录名。
pub fn path_is_within(root: &Path, target: &Path) -> bool {
    let mut target_components = target.components();
    root.components().all(|root_component| {
        target_components
            .next()
            .is_some_and(|target_component| path_component_eq(root_component, target_component))
    })
}

pub fn paths_equal(left: &Path, right: &Path) -> bool {
    let mut left_components = left.components();
    let mut right_components = right.components();
    loop {
        match (left_components.next(), right_components.next()) {
            (Some(left), Some(right)) if path_component_eq(left, right) => {}
            (None, None) => return true,
            _ => return false,
        }
    }
}

/// 生成与旧版同形的随机库 ID：`${typeId}-${Date.now()}-${rand9}`。
pub fn new_library_id(type_id: &str) -> String {
    format!(
        "{type_id}-{}-{}",
        crate::jstime::now_millis(),
        random_base36(9)
    )
}

/// 随机 base36 串。对齐 JS 的 `Math.random().toString(36).slice(2, N)`。
///
/// 用 `RandomState` 取 OS 播种的哈希器，省掉一个 `rand` 依赖——
/// 这些 ID 只需避免同一毫秒内的碰撞，不是密码学用途。
pub fn random_base36(len: usize) -> String {
    const ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    (0..len)
        .map(|i| ALPHABET[rand_index(ALPHABET.len(), i)] as char)
        .collect()
}

fn rand_index(len: usize, salt: usize) -> usize {
    use std::hash::{BuildHasher, Hasher, RandomState};
    let mut h = RandomState::new().build_hasher();
    h.write_usize(len ^ salt);
    (h.finish() % len as u64) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_matches_electron() {
        let ws = Path::new("D:\\mochi");
        assert!(mochi_dir(ws).ends_with(".mochi"));
        assert!(config_file(ws).ends_with("config.json"));
        assert!(git_dir(ws).ends_with("git"));
        assert!(index_db(ws).ends_with("index.db"));
        // 全部挂在 .mochi 下，不散落到工作区根目录
        for p in [
            config_file(ws),
            sidebar_file(ws),
            ai_permissions_file(ws),
            library_types_file(ws),
            libraries_file(ws),
            git_dir(ws),
            index_db(ws),
        ] {
            assert_eq!(
                p.parent().unwrap(),
                mochi_dir(ws),
                "{} 不在 .mochi 下",
                p.display()
            );
        }
    }

    #[test]
    fn forward_slashes() {
        assert_eq!(
            to_forward_slashes("知识库\\计算机通识\\a.md"),
            "知识库/计算机通识/a.md"
        );
        assert_eq!(to_forward_slashes("already/ok"), "already/ok");
    }

    #[cfg(windows)]
    #[test]
    fn windows_containment_is_case_insensitive_and_component_bounded() {
        let root = Path::new(r"C:\\Users\\Example\\Notes");
        assert!(path_is_within(
            root,
            Path::new(r"c:\\users\\example\\notes\\inside.md")
        ));
        assert!(paths_equal(root, Path::new(r"c:\\USERS\\EXAMPLE\\NOTES")));
        assert!(!path_is_within(
            root,
            Path::new(r"C:\\Users\\Example\\Notes-old\\outside.md")
        ));
    }

    #[test]
    fn library_id_shape() {
        let id = new_library_id("knowledge-base");
        let rest = id.strip_prefix("knowledge-base-").expect("缺前缀: {id}");
        let (ts, rand) = rest.rsplit_once('-').expect("缺随机段: {id}");
        assert!(ts.parse::<i64>().is_ok(), "时间戳段不是数字: {ts}");
        assert_eq!(rand.len(), 9, "随机段应为 9 位: {rand}");
        assert!(rand
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()));
    }

    #[test]
    fn library_ids_do_not_collide_within_one_millisecond() {
        let ids: std::collections::HashSet<_> = (0..200).map(|_| new_library_id("kb")).collect();
        assert!(
            ids.len() > 190,
            "随机性不足，200 次生成只得到 {} 个不同 ID",
            ids.len()
        );
    }
}

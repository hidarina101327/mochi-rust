//! 字段名和声明顺序保持与 TS 落盘格式一致；缺字段取默认值，None 字段省略。

use serde::{Deserialize, Serialize};

fn is_none<T>(v: &Option<T>) -> bool {
    v.is_none()
}

/// 库类型（如「知识库」「Agent配置」）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LibraryType {
    pub id: String,
    pub name: String,
    pub icon: String,
    #[serde(skip_serializing_if = "is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "is_none")]
    pub default_sidebar_config: Option<LibrarySidebarConfig>,
    pub created_at: String,
    pub is_built_in: bool,
}

/// 库侧栏配置。注意 `show_file_tree` 默认 **true**，不是 `bool::default()`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LibrarySidebarConfig {
    pub show_file_tree: bool,
    pub show_outline: bool,
    #[serde(skip_serializing_if = "is_none")]
    pub show_tags: Option<bool>,
}

impl Default for LibrarySidebarConfig {
    fn default() -> Self {
        Self {
            show_file_tree: true,
            show_outline: false,
            show_tags: None,
        }
    }
}

/// 库实例（如「计算机通识」）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Library {
    pub id: String,
    pub name: String,
    /// TS 侧字段名是 `type`，Rust 关键字冲突，改名 `kind` 并显式 rename。
    #[serde(rename = "type")]
    pub kind: String,
    pub path: String,
    #[serde(skip_serializing_if = "is_none")]
    pub icon: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "is_none")]
    pub sidebar_config: Option<LibrarySidebarConfig>,
}

/// 文件树节点。`id` 即绝对路径。仅运行时使用（不落盘），但保持形状一致以便复用。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FileNode {
    pub id: String,
    pub name: String,
    pub path: String,
    /// `"file"` | `"directory"` | `"link"`
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "is_none")]
    pub children: Option<Vec<FileNode>>,
    #[serde(skip_serializing_if = "is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "is_none")]
    pub mtime: Option<String>,
    #[serde(skip_serializing_if = "is_none")]
    pub size: Option<i64>,
    #[serde(skip_serializing_if = "is_none")]
    pub icon: Option<String>,
    /// 子文档挂载时的父文档路径
    #[serde(skip_serializing_if = "is_none")]
    pub sub_doc_parent: Option<String>,
    /// 目录子项尚未枚举；仅运行时使用，避免首次打开时递归扫描整库。
    #[serde(skip)]
    pub lazy: bool,
}

impl Default for FileNode {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            path: String::new(),
            kind: "file".into(),
            children: None,
            created_at: None,
            mtime: None,
            size: None,
            icon: None,
            sub_doc_parent: None,
            lazy: false,
        }
    }
}

impl FileNode {
    pub fn is_directory(&self) -> bool {
        self.kind == "directory" || self.kind == "mapped-directory"
    }
    pub fn is_link(&self) -> bool {
        self.kind == "link"
    }
}

/// `.mochi/config.json`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MochiConfig {
    pub workspace_name: String,
    pub created_at: String,
    pub last_modified: String,
    pub version: String,
}

impl Default for MochiConfig {
    fn default() -> Self {
        Self {
            workspace_name: String::new(),
            created_at: String::new(),
            last_modified: String::new(),
            version: "1.0.0".into(),
        }
    }
}

/// `.mochi/sidebar.json` 的分区。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SidebarSection {
    pub id: String,
    pub title: String,
    pub path: String,
    pub collapsed: bool,
    pub pinned: bool,
    #[serde(skip_serializing_if = "is_none")]
    pub order: Option<i32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SidebarConfig {
    pub sections: Vec<SidebarSection>,
}

/// `.mochi/ai-permissions.json` 的单条文件夹权限。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AiFolderPermission {
    pub path: String,
    /// `"invisible"` | `"readonly"` | `"suggest"` | `"modify"`
    pub level: String,
    #[serde(skip_serializing_if = "is_none")]
    pub inherited: Option<bool>,
}

impl Default for AiFolderPermission {
    fn default() -> Self {
        Self {
            path: String::new(),
            level: "suggest".into(),
            inherited: None,
        }
    }
}

/// `.mochi/ai-permissions.json`
///
/// `version` 是 `Option` 而非 `String`，因为 Electron 版**自己就写了两种形状**：
/// - `metadata.ts` 的种子路径写 `{ permissions }`，**不带** version（真实工作区里就是这个）
/// - `ai-permissions.ts` 的保存路径写 `{ version: '1.0', permissions }`
///
/// 用 `Option` + `skip_serializing_if` 两种都能原样往返：读到什么形状就写回什么形状，
/// 不会给用户凭空制造一次 Git 变更。C# 版 `WorkspaceService.cs:113` 用保存路径的形状
/// 去做种子，会把已有的种子文件改写掉。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AiPermissionsConfig {
    #[serde(skip_serializing_if = "is_none")]
    pub version: Option<String>,
    pub permissions: Vec<AiFolderPermission>,
}

/// 工作区打开结果。运行时结构，不落盘，因此不带 serde。
#[derive(Debug, Clone)]
pub struct WorkspaceInfo {
    pub root_path: std::path::PathBuf,
    pub name: String,
    pub library_types: Vec<LibraryType>,
    pub libraries: Vec<Library>,
    /// true = 本次打开时执行了结构迁移（旧目录 → 知识库/）
    pub migrated_this_open: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json2;

    /// 字段顺序是契约。期望值按 TS 的声明顺序逐字抄写。
    #[test]
    fn library_type_field_order_and_names() {
        let t = LibraryType {
            id: "knowledge-base".into(),
            name: "知识库".into(),
            icon: "BookOpen".into(),
            description: None,
            default_sidebar_config: None,
            created_at: "2026-08-22T03:14:15.926Z".into(),
            is_built_in: true,
        };
        assert_eq!(
            json2::serialize(&t).unwrap(),
            "{\n  \"id\": \"knowledge-base\",\n  \"name\": \"知识库\",\n  \"icon\": \"BookOpen\",\n  \"createdAt\": \"2026-08-22T03:14:15.926Z\",\n  \"isBuiltIn\": true\n}"
        );
    }

    #[test]
    fn library_type_writes_nested_sidebar_config_in_place() {
        let t = LibraryType {
            id: "kb".into(),
            name: "库".into(),
            icon: "B".into(),
            description: Some("说明".into()),
            default_sidebar_config: Some(LibrarySidebarConfig::default()),
            created_at: "T".into(),
            is_built_in: false,
        };
        let json = json2::serialize(&t).unwrap();
        // description 排在 icon 与 defaultSidebarConfig 之间
        let i_icon = json.find("\"icon\"").unwrap();
        let i_desc = json.find("\"description\"").unwrap();
        let i_cfg = json.find("\"defaultSidebarConfig\"").unwrap();
        assert!(i_icon < i_desc && i_desc < i_cfg, "字段顺序不对:\n{json}");
        assert!(json.contains("\"showFileTree\": true"));
    }

    #[test]
    fn library_type_field_is_renamed_to_type() {
        let l = Library {
            id: "kb-1".into(),
            name: "计算机通识".into(),
            kind: "knowledge-base".into(),
            path: "知识库/计算机通识".into(),
            ..Default::default()
        };
        let json = json2::serialize(&l).unwrap();
        assert!(json.contains("\"type\": \"knowledge-base\""), "{json}");
        assert!(!json.contains("\"kind\""), "泄漏了 Rust 侧字段名:\n{json}");
    }

    #[test]
    fn sidebar_config_show_file_tree_defaults_true() {
        // 空对象读出来必须是 true，不是 bool::default()
        let cfg: LibrarySidebarConfig = json2::deserialize("{}").unwrap();
        assert!(cfg.show_file_tree);
        assert!(!cfg.show_outline);
        assert_eq!(cfg.show_tags, None);
    }

    #[test]
    fn file_node_kind_defaults_to_file() {
        let n: FileNode = json2::deserialize(r#"{"id":"a","name":"a.md","path":"a.md"}"#).unwrap();
        assert_eq!(n.kind, "file");
        assert!(!n.is_directory());
        assert!(!n.is_link());
    }

    #[test]
    fn missing_fields_do_not_error() {
        // 旧数据里确实有缺字段的记录，读取必须宽容
        let t: LibraryType = json2::deserialize(r#"{"id":"x"}"#).unwrap();
        assert_eq!(t.id, "x");
        assert_eq!(t.name, "");
        assert!(!t.is_built_in);
    }

    #[test]
    fn defaults_of_persisted_configs() {
        assert_eq!(MochiConfig::default().version, "1.0.0");
        assert_eq!(AiFolderPermission::default().level, "suggest");
    }

    /// 两种历史形状都必须原样往返，否则会给用户制造无意义的 Git 变更。
    #[test]
    fn ai_permissions_preserves_presence_or_absence_of_version() {
        let seeded = "{\n  \"permissions\": [\n    {\n      \"path\": \"/\",\n      \"level\": \"suggest\"\n    }\n  ]\n}";
        let parsed: AiPermissionsConfig = json2::deserialize(seeded).unwrap();
        assert_eq!(parsed.version, None);
        assert_eq!(
            json2::serialize(&parsed).unwrap(),
            seeded,
            "种子形状被改写了"
        );

        let saved = "{\n  \"version\": \"1.0\",\n  \"permissions\": []\n}";
        let parsed: AiPermissionsConfig = json2::deserialize(saved).unwrap();
        assert_eq!(parsed.version.as_deref(), Some("1.0"));
        assert_eq!(
            json2::serialize(&parsed).unwrap(),
            saved,
            "保存形状被改写了"
        );
    }

    #[test]
    fn ai_permissions_roundtrip() {
        let cfg = AiPermissionsConfig {
            version: Some("1.0".into()),
            permissions: vec![AiFolderPermission {
                path: "知识库/私密".into(),
                level: "invisible".into(),
                inherited: Some(true),
            }],
        };
        let json = json2::serialize(&cfg).unwrap();
        assert_eq!(
            json2::deserialize::<AiPermissionsConfig>(&json).unwrap(),
            cfg
        );
    }

    #[test]
    fn empty_permissions_list_stays_inline() {
        let json = json2::serialize(&AiPermissionsConfig::default()).unwrap();
        assert_eq!(json, "{\n  \"permissions\": []\n}");
    }
}

//! 权限语义以 ai-permission-gate.ts 为准；动作开关读取持久设置。
//! Windows 下 / 规范化为空，不是匹配所有路径的根规则。

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::domain::{AiFolderPermission, AiPermissionsConfig};
use crate::{json2, paths};

const CACHE_TTL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiPermissionLevel {
    /// AI 完全看不到，读写一律拒绝
    Invisible,
    /// 可读，禁止写入/删除/重命名
    ReadOnly,
    /// 可读，写入降级为待批准提案
    Suggest,
    /// 完全放行
    Modify,
}

impl AiPermissionLevel {
    pub fn from_wire(value: &str) -> Self {
        match value {
            "invisible" => Self::Invisible,
            "readonly" => Self::ReadOnly,
            "suggest" => Self::Suggest,
            _ => Self::Modify,
        }
    }
    pub fn to_wire(self) -> &'static str {
        match self {
            Self::Invisible => "invisible",
            Self::ReadOnly => "readonly",
            Self::Suggest => "suggest",
            Self::Modify => "modify",
        }
    }
}

/// 工具动作。与 TS 的 `AIToolAction` 一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiToolAction {
    ReadFile,
    WriteFile,
    DeleteFile,
    ExecuteCommand,
    NetworkAccess,
    ModifySettings,
}

impl AiToolAction {
    pub fn from_wire(value: &str) -> Option<Self> {
        Some(match value {
            "read_file" => Self::ReadFile,
            "write_file" => Self::WriteFile,
            "delete_file" => Self::DeleteFile,
            "execute_command" => Self::ExecuteCommand,
            "network_access" => Self::NetworkAccess,
            "modify_settings" => Self::ModifySettings,
            _ => return None,
        })
    }
    pub fn to_wire(self) -> &'static str {
        match self {
            Self::ReadFile => "read_file",
            Self::WriteFile => "write_file",
            Self::DeleteFile => "delete_file",
            Self::ExecuteCommand => "execute_command",
            Self::NetworkAccess => "network_access",
            Self::ModifySettings => "modify_settings",
        }
    }
    /// 与 TS 的 `ACTION_LABELS` 一致，用于拼错误信息。
    pub fn label(self) -> &'static str {
        match self {
            Self::ReadFile => "读取文件",
            Self::WriteFile => "写入文件",
            Self::DeleteFile => "删除文件",
            Self::ExecuteCommand => "执行命令",
            Self::NetworkAccess => "访问网络",
            Self::ModifySettings => "修改设置",
        }
    }
    fn is_write(self) -> bool {
        matches!(self, Self::WriteFile | Self::DeleteFile)
    }
    /// 与 TS `defaultPermissions` 一致：只有「执行命令」默认关闭。
    pub fn default_allowed(self) -> bool {
        !matches!(self, Self::ExecuteCommand)
    }

    pub const ALL: [Self; 6] = [
        Self::ReadFile,
        Self::WriteFile,
        Self::DeleteFile,
        Self::ExecuteCommand,
        Self::NetworkAccess,
        Self::ModifySettings,
    ];
}

/// 动作级权限开关。核心层只负责权限判定，持久化由调用方处理。
#[derive(Debug, Clone, PartialEq)]
pub struct ActionPermissions {
    allowed: [bool; 6],
}

impl Default for ActionPermissions {
    fn default() -> Self {
        let mut allowed = [true; 6];
        for (i, action) in AiToolAction::ALL.iter().enumerate() {
            allowed[i] = action.default_allowed();
        }
        Self { allowed }
    }
}

impl ActionPermissions {
    fn index(action: AiToolAction) -> usize {
        AiToolAction::ALL
            .iter()
            .position(|a| *a == action)
            .unwrap_or(0)
    }

    pub fn is_allowed(&self, action: AiToolAction) -> bool {
        self.allowed[Self::index(action)]
    }

    pub fn set(&mut self, action: AiToolAction, allowed: bool) {
        let i = Self::index(action);
        self.allowed[i] = allowed;
    }

    /// 序列化成 `read_file=1,write_file=0,…` 便于塞进扁平设置存储。
    pub fn to_setting(&self) -> String {
        AiToolAction::ALL
            .iter()
            .map(|a| format!("{}={}", a.to_wire(), u8::from(self.is_allowed(*a))))
            .collect::<Vec<_>>()
            .join(",")
    }

    /// 解析 `to_setting` 的输出；未出现的动作沿用默认值，脏值忽略。
    pub fn from_setting(raw: &str) -> Self {
        let mut out = Self::default();
        for pair in raw.split(',') {
            let Some((k, v)) = pair.split_once('=') else {
                continue;
            };
            let Some(action) = AiToolAction::from_wire(k.trim()) else {
                continue;
            };
            match v.trim() {
                "1" | "true" => out.set(action, true),
                "0" | "false" => out.set(action, false),
                _ => {}
            }
        }
        out
    }
}

/// 权限判定失败的原因。工具执行器把它转成工具错误回给模型。
#[derive(Debug, Clone, PartialEq)]
pub struct PermissionDenied {
    pub message: String,
}

impl std::fmt::Display for PermissionDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PermissionDenied {}

/// 路径归一：反斜杠→斜杠、去掉非根路径的尾部斜杠。
pub fn normalize(input: &str) -> String {
    let slashed = input.replace('\\', "/");
    if slashed == "/"
        || (slashed.len() == 3 && slashed.as_bytes()[1] == b':' && slashed.as_bytes()[2] == b'/')
    {
        slashed
    } else {
        slashed.trim_end_matches('/').to_owned()
    }
}

/// `target` 是否落在 `folder` 之内（或就是 `folder` 本身）。
///
/// 严格按分段匹配，不用裸 `starts_with`——否则 `/kb/note-old` 会命中 `/kb/note` 的规则。
/// TS 的注释里专门点名了这个坑。
fn is_within(target: &str, folder: &str) -> bool {
    if folder == "/" {
        return target.starts_with('/')
            || (target.len() >= 3 && target.as_bytes()[1] == b':' && target.as_bytes()[2] == b'/');
    }
    let windows_path = target.len() >= 2 && target.as_bytes()[1] == b':'
        || folder.len() >= 2 && folder.as_bytes()[1] == b':';
    if windows_path {
        let target = target.to_lowercase();
        let folder = folder.to_lowercase();
        target == folder || target.starts_with(&format!("{folder}/"))
    } else {
        target == folder || target.starts_with(&format!("{folder}/"))
    }
}

pub struct AiPermissionService {
    config_path: std::path::PathBuf,
    cache: Mutex<Cache>,
    actions: Mutex<ActionPermissions>,
}

struct Cache {
    permissions: Vec<AiFolderPermission>,
    loaded_at: Option<Instant>,
}

impl AiPermissionService {
    pub fn new(workspace_path: impl AsRef<Path>) -> Self {
        Self {
            config_path: paths::ai_permissions_file(workspace_path),
            cache: Mutex::new(Cache {
                permissions: Vec::new(),
                loaded_at: None,
            }),
            actions: Mutex::new(ActionPermissions::default()),
        }
    }

    pub fn set_action_permissions(&self, actions: ActionPermissions) {
        *self.actions.lock().unwrap_or_else(|e| e.into_inner()) = actions;
    }

    pub fn action_permissions(&self) -> ActionPermissions {
        self.actions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn is_action_allowed(&self, action: AiToolAction) -> bool {
        self.action_permissions().is_allowed(action)
    }

    /// 加载文件夹权限（5 秒缓存）。路径在加载时就归一，与 TS 的 `loadInto` 一致。
    pub fn load_permissions(&self) -> Vec<AiFolderPermission> {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let fresh = cache.loaded_at.is_some_and(|t| t.elapsed() < CACHE_TTL);
        if fresh {
            return cache.permissions.clone();
        }

        let loaded: Vec<AiFolderPermission> = std::fs::read_to_string(&self.config_path)
            .ok()
            .and_then(|t| json2::deserialize::<AiPermissionsConfig>(&t).ok())
            .map(|c| c.permissions)
            .unwrap_or_default()
            .into_iter()
            .map(|mut p| {
                p.path = normalize(&p.path);
                p
            })
            .collect();

        cache.permissions = loaded.clone();
        cache.loaded_at = Some(Instant::now());
        loaded
    }

    pub fn invalidate_cache(&self) {
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .loaded_at = None;
    }

    /// 保存文件夹权限。写入时带 `version`（对齐 `ai-permissions.ts` 的 `savePermissions`）。
    ///
    /// 磁盘按调用方给的原样写，**缓存则存归一后的路径**——与 `load_permissions` 保持同一形状，
    /// 否则「刚存完就查」和「重新加载后再查」会得到不同结果。
    pub fn save_permissions(&self, permissions: Vec<AiFolderPermission>) -> Result<()> {
        json2::write(
            &self.config_path,
            &AiPermissionsConfig {
                version: Some("1.0".into()),
                permissions: permissions.clone(),
            },
        )?;
        let normalized = permissions
            .into_iter()
            .map(|mut p| {
                p.path = normalize(&p.path);
                p
            })
            .collect();
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        cache.permissions = normalized;
        cache.loaded_at = Some(Instant::now());
        Ok(())
    }

    pub fn set_folder_permission(&self, folder_path: &str, level: AiPermissionLevel) -> Result<()> {
        let normalized = normalize(folder_path);
        let mut perms = self.load_permissions();
        perms.retain(|p| normalize(&p.path) != normalized);
        perms.push(AiFolderPermission {
            path: normalized,
            level: level.to_wire().into(),
            inherited: None,
        });
        self.save_permissions(perms)
    }

    pub fn remove_folder_permission(&self, folder_path: &str) -> Result<()> {
        let normalized = normalize(folder_path);
        let mut perms = self.load_permissions();
        perms.retain(|p| normalize(&p.path) != normalized);
        self.save_permissions(perms)
    }

    /// 取路径的权限级别，命中最具体（路径最长）的规则。无规则命中返回 `None`
    /// ——与 TS 一致，表示不施加任何限制。
    pub fn folder_permission_level(&self, absolute_path: &str) -> Option<AiPermissionLevel> {
        let permissions = self.load_permissions();
        if permissions.is_empty() {
            return None;
        }
        let target = normalize(absolute_path);
        permissions
            .iter()
            .filter(|p| is_within(&target, &p.path))
            .max_by_key(|p| p.path.len())
            .map(|p| AiPermissionLevel::from_wire(&p.level))
    }

    /// suggest 级别的路径不允许直接落盘，写操作必须转成待批准提案。
    pub fn path_requires_proposal(&self, absolute_path: &str) -> bool {
        self.folder_permission_level(absolute_path) == Some(AiPermissionLevel::Suggest)
    }

    /// 供搜索、列目录等批量结果过滤，隐藏设为不可见的路径。
    pub fn is_path_visible(&self, absolute_path: &str) -> bool {
        self.folder_permission_level(absolute_path) != Some(AiPermissionLevel::Invisible)
    }

    /// **工具执行前的权限门**。不通过返回 `Err`，由执行器转成工具错误回给模型。
    ///
    /// WinUI 版把这一步整个漏了——工具执行器直接调用，没有任何检查。
    pub fn assert_tool_action_allowed(
        &self,
        action: AiToolAction,
        absolute_path: Option<&str>,
    ) -> Result<(), PermissionDenied> {
        if !self.is_action_allowed(action) {
            return Err(PermissionDenied {
                message: format!(
                    "AI 权限设置中已禁用「{}」，请在设置中开启后重试",
                    action.label()
                ),
            });
        }

        let Some(path) = absolute_path else {
            return Ok(());
        };
        let Some(level) = self.folder_permission_level(path) else {
            return Ok(());
        };

        match level {
            AiPermissionLevel::Invisible => Err(PermissionDenied {
                message: format!(
                    "该路径已被设为「AI 不可见」，无法{}：{path}",
                    action.label()
                ),
            }),
            AiPermissionLevel::ReadOnly if action.is_write() => Err(PermissionDenied {
                message: format!("该路径已被设为「只读」，AI 不能{}：{path}", action.label()),
            }),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    struct TempWs(std::path::PathBuf);
    impl TempWs {
        fn new(tag: &str) -> Self {
            let n = SEQ.fetch_add(1, Ordering::Relaxed);
            let p =
                std::env::temp_dir().join(format!("mochi-perm-{}-{tag}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn svc(&self) -> AiPermissionService {
            AiPermissionService::new(&self.0)
        }
    }
    impl Drop for TempWs {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn normalize_matches_ts() {
        assert_eq!(normalize("D:\\ws\\知识库\\"), "D:/ws/知识库");
        assert_eq!(normalize("a/b//"), "a/b");
        assert_eq!(normalize("/"), "/");
        assert_eq!(normalize("D:/"), "D:/");
    }

    /// 严格分段匹配：`/kb/note-old` 不该命中 `/kb/note` 的规则。
    /// TS 的注释专门点名了裸 startsWith 的这个坑。
    #[test]
    fn sibling_prefix_paths_do_not_match() {
        assert!(is_within("D:/kb/note", "D:/kb/note"));
        assert!(is_within("D:/kb/note/a.md", "D:/kb/note"));
        assert!(
            !is_within("D:/kb/note-old/a.md", "D:/kb/note"),
            "前缀误命中"
        );
    }

    #[test]
    fn most_specific_rule_wins() {
        let ws = TempWs::new("specific");
        let svc = ws.svc();
        svc.save_permissions(vec![
            AiFolderPermission {
                path: "D:/ws".into(),
                level: "modify".into(),
                inherited: None,
            },
            AiFolderPermission {
                path: "D:/ws/私密".into(),
                level: "invisible".into(),
                inherited: None,
            },
        ])
        .unwrap();

        assert_eq!(
            svc.folder_permission_level("D:/ws/公开/a.md"),
            Some(AiPermissionLevel::Modify)
        );
        assert_eq!(
            svc.folder_permission_level("D:/ws/私密/a.md"),
            Some(AiPermissionLevel::Invisible),
            "更长的规则应覆盖父级"
        );
    }

    #[test]
    fn no_rules_means_no_restriction() {
        let ws = TempWs::new("norules");
        let svc = ws.svc();
        assert_eq!(svc.folder_permission_level("D:/ws/a.md"), None);
        assert!(svc
            .assert_tool_action_allowed(AiToolAction::WriteFile, Some("D:/ws/a.md"))
            .is_ok());
    }

    #[test]
    fn root_rule_matches_windows_and_posix_paths() {
        let ws = TempWs::new("root");
        let svc = ws.svc();
        svc.save_permissions(vec![AiFolderPermission {
            path: "/".into(),
            level: "suggest".into(),
            inherited: None,
        }])
        .unwrap();

        assert_eq!(
            svc.folder_permission_level("D:/ws/a.md"),
            Some(AiPermissionLevel::Suggest)
        );
        assert_eq!(
            svc.folder_permission_level("/ws/a.md"),
            Some(AiPermissionLevel::Suggest)
        );
    }

    #[test]
    fn invisible_blocks_everything() {
        let ws = TempWs::new("invisible");
        let svc = ws.svc();
        svc.set_folder_permission("D:/ws/私密", AiPermissionLevel::Invisible)
            .unwrap();

        assert!(!svc.is_path_visible("D:/ws/私密/a.md"));
        for action in [
            AiToolAction::ReadFile,
            AiToolAction::WriteFile,
            AiToolAction::DeleteFile,
        ] {
            let err = svc
                .assert_tool_action_allowed(action, Some("D:/ws/私密/a.md"))
                .unwrap_err();
            assert!(err.message.contains("AI 不可见"), "{}", err.message);
        }
    }

    #[test]
    fn readonly_blocks_writes_but_allows_reads() {
        let ws = TempWs::new("readonly");
        let svc = ws.svc();
        svc.set_folder_permission("D:/ws/参考", AiPermissionLevel::ReadOnly)
            .unwrap();
        let p = Some("D:/ws/参考/a.md");

        assert!(svc
            .assert_tool_action_allowed(AiToolAction::ReadFile, p)
            .is_ok());
        assert!(svc.is_path_visible("D:/ws/参考/a.md"));

        for action in [AiToolAction::WriteFile, AiToolAction::DeleteFile] {
            let err = svc.assert_tool_action_allowed(action, p).unwrap_err();
            assert!(err.message.contains("只读"), "{}", err.message);
        }
    }

    #[test]
    fn suggest_allows_but_flags_for_proposal() {
        let ws = TempWs::new("suggest");
        let svc = ws.svc();
        svc.set_folder_permission("D:/ws/草稿", AiPermissionLevel::Suggest)
            .unwrap();

        assert!(svc
            .assert_tool_action_allowed(AiToolAction::WriteFile, Some("D:/ws/草稿/a.md"))
            .is_ok());
        assert!(
            svc.path_requires_proposal("D:/ws/草稿/a.md"),
            "suggest 应要求提案"
        );
        assert!(!svc.path_requires_proposal("D:/ws/别处/a.md"));
    }

    /// C# 把动作开关写死了；这里必须真的可配置。
    #[test]
    fn action_switches_are_configurable_not_hardcoded() {
        let ws = TempWs::new("actions");
        let svc = ws.svc();

        assert!(svc.is_action_allowed(AiToolAction::WriteFile), "默认允许写");
        assert!(
            !svc.is_action_allowed(AiToolAction::ExecuteCommand),
            "执行命令默认关闭"
        );

        let mut actions = svc.action_permissions();
        actions.set(AiToolAction::WriteFile, false);
        svc.set_action_permissions(actions);

        let err = svc
            .assert_tool_action_allowed(AiToolAction::WriteFile, None)
            .unwrap_err();
        assert!(err.message.contains("写入文件"), "{}", err.message);
        assert!(svc
            .assert_tool_action_allowed(AiToolAction::ReadFile, None)
            .is_ok());
    }

    #[test]
    fn action_permissions_survive_a_settings_round_trip() {
        let mut actions = ActionPermissions::default();
        actions.set(AiToolAction::ExecuteCommand, true);
        actions.set(AiToolAction::NetworkAccess, false);

        let restored = ActionPermissions::from_setting(&actions.to_setting());
        assert_eq!(restored, actions);
        assert!(restored.is_allowed(AiToolAction::ExecuteCommand));
        assert!(!restored.is_allowed(AiToolAction::NetworkAccess));
    }

    #[test]
    fn setting_parser_tolerates_garbage() {
        let a =
            ActionPermissions::from_setting("垃圾,,write_file=0,unknown_action=1,read_file=乱写");
        assert!(!a.is_allowed(AiToolAction::WriteFile), "有效项应生效");
        assert!(a.is_allowed(AiToolAction::ReadFile), "脏值应沿用默认");
    }

    #[test]
    fn saved_config_carries_version_and_round_trips() {
        let ws = TempWs::new("save");
        let svc = ws.svc();
        svc.set_folder_permission("D:\\ws\\目录\\", AiPermissionLevel::ReadOnly)
            .unwrap();

        let raw = std::fs::read_to_string(paths::ai_permissions_file(&ws.0)).unwrap();
        assert!(
            raw.contains("\"version\": \"1.0\""),
            "保存时应写 version:\n{raw}"
        );
        assert!(
            raw.contains("\"path\": \"D:/ws/目录\""),
            "路径应归一:\n{raw}"
        );

        svc.invalidate_cache();
        assert_eq!(
            svc.folder_permission_level("D:/ws/目录/a.md"),
            Some(AiPermissionLevel::ReadOnly)
        );
    }

    #[test]
    fn remove_folder_permission() {
        let ws = TempWs::new("remove");
        let svc = ws.svc();
        svc.set_folder_permission("D:/ws/x", AiPermissionLevel::Invisible)
            .unwrap();
        assert!(svc.folder_permission_level("D:/ws/x/a.md").is_some());

        svc.remove_folder_permission("D:/ws/x").unwrap();
        assert_eq!(svc.folder_permission_level("D:/ws/x/a.md"), None);
    }

    #[test]
    fn level_wire_round_trip() {
        for level in [
            AiPermissionLevel::Invisible,
            AiPermissionLevel::ReadOnly,
            AiPermissionLevel::Suggest,
            AiPermissionLevel::Modify,
        ] {
            assert_eq!(AiPermissionLevel::from_wire(level.to_wire()), level);
        }
        assert_eq!(
            AiPermissionLevel::from_wire("乱写"),
            AiPermissionLevel::Modify
        );
    }
}

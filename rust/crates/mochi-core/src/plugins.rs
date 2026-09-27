//! 原生插件不执行 HTML/JS；UI 由宿主解释 ui.json，WASM 由 app 按权限加载。
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub manifest_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub permissions: Vec<Permission>,
    #[serde(default)]
    pub contributions: Contributions,
    #[serde(default)]
    pub module: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Permission {
    WorkspaceRead,
    WorkspaceWrite,
    PluginStorage,
    Network,
    ShellLaunch,
    AiRead,
    AiWrite,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Contributions {
    #[serde(default)]
    pub navigation: bool,
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default)]
    pub tools: Option<String>,
    #[serde(default)]
    pub ui: Option<String>,
    /// 将插件的宿主渲染界面注册到指定容器。`navigation` 是早期 v2
    /// 简写的兼容字段；新插件应使用 `slots`，以便同一插件贡献多个位置。
    #[serde(default)]
    pub slots: Vec<SlotContribution>,
}

/// 墨池明确提供给第三方插件的界面容器。插件只能向这些受控插槽贡献内容，
/// 不可注入任意 HTML、Win32 句柄或宿主内部布局。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PluginSlot {
    Navigation,
    Sidebar,
    RightSidebar,
    EditorToolbar,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotContribution {
    pub slot: PluginSlot,
    pub title: String,
    #[serde(default)]
    pub icon: String,
    /// 默认复用 `contributions.ui`；可指定该容器专用的声明式界面文件。
    #[serde(default)]
    pub ui: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RegisteredSlot {
    pub plugin: Installed,
    pub contribution: SlotContribution,
}
#[derive(Debug, Clone)]
pub struct Installed {
    pub manifest: Manifest,
    pub directory: PathBuf,
    pub enabled: bool,
}
/// 宿主渲染的最小界面描述；插件不能注入 HTML/JS。
#[derive(Debug, Clone, Deserialize)]
pub struct UiSchema {
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub actions: Vec<String>,
    #[serde(default)]
    pub sections: Vec<UiSection>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct UiSection {
    pub title: String,
    #[serde(default)]
    pub cards: Vec<String>,
    #[serde(default)]
    pub actions: Vec<String>,
}
pub struct PluginService {
    root: PathBuf,
}

impl PluginService {
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        Self {
            root: workspace.as_ref().join(".mochi").join("plugins"),
        }
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn list(&self) -> Result<Vec<Installed>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let disabled = self.disabled()?;
        let mut result = Vec::new();
        for entry in fs::read_dir(&self.root)?.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            if let Ok(manifest) = read_manifest(&entry.path()) {
                let enabled = !disabled.contains(&manifest.id);
                result.push(Installed {
                    manifest,
                    directory: entry.path(),
                    enabled,
                });
            }
        }
        result.sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
        Ok(result)
    }
    pub fn load_ui(&self, plugin: &Installed) -> Result<Option<UiSchema>> {
        self.load_ui_path(plugin, plugin.manifest.contributions.ui.as_deref())
    }
    pub fn load_slot_ui(&self, slot: &RegisteredSlot) -> Result<Option<UiSchema>> {
        self.load_ui_path(
            &slot.plugin,
            slot.contribution
                .ui
                .as_deref()
                .or(slot.plugin.manifest.contributions.ui.as_deref()),
        )
    }
    fn load_ui_path(&self, plugin: &Installed, relative: Option<&str>) -> Result<Option<UiSchema>> {
        let Some(relative) = relative else {
            return Ok(None);
        };
        let path = safe_join(&plugin.directory, relative)?;
        if !path.is_file() {
            bail!("插件 UI 文件不存在：{relative}");
        }
        Ok(Some(serde_json::from_slice(&fs::read(path)?)?))
    }
    /// 所有已启用插件的声明式容器贡献。旧 `navigation: true` 自动投影为
    /// 一个导航插槽，以保证最早的 v2 包仍能运行。
    pub fn registered_slots(&self) -> Result<Vec<RegisteredSlot>> {
        let mut slots = Vec::new();
        for plugin in self.list()?.into_iter().filter(|plugin| plugin.enabled) {
            let mut declared = plugin.manifest.contributions.slots.clone();
            if plugin.manifest.contributions.navigation
                && !declared
                    .iter()
                    .any(|slot| slot.slot == PluginSlot::Navigation)
            {
                declared.push(SlotContribution {
                    slot: PluginSlot::Navigation,
                    title: plugin.manifest.name.clone(),
                    icon: plugin.manifest.icon.clone(),
                    ui: None,
                });
            }
            slots.extend(
                declared
                    .into_iter()
                    .filter(|slot| !slot.title.trim().is_empty())
                    .map(|contribution| RegisteredSlot {
                        plugin: plugin.clone(),
                        contribution,
                    }),
            );
        }
        Ok(slots)
    }
    pub fn install_zip(&self, archive: impl AsRef<Path>) -> Result<Installed> {
        let file = fs::File::open(archive)?;
        let mut zip = zip::ZipArchive::new(file).context("无效插件 ZIP")?;
        let mut manifest_entry = None;
        for i in 0..zip.len() {
            let name = zip.by_index(i)?.name().replace('\\', "/");
            if name.ends_with("manifest.json") {
                if manifest_entry.replace(name).is_some() {
                    bail!("ZIP 中只能有一个 manifest.json");
                }
            }
        }
        let entry = manifest_entry.ok_or_else(|| anyhow::anyhow!("插件包缺少 manifest.json"))?;
        let prefix = entry.trim_end_matches("manifest.json");
        let manifest: Manifest = {
            let mut f = zip.by_name(&entry)?;
            let mut text = String::new();
            f.read_to_string(&mut text)?;
            serde_json::from_str(text.trim_start_matches('\u{feff}'))?
        };
        validate(&manifest)?;
        fs::create_dir_all(&self.root)?;
        let staging = self.root.join(format!(".install-{}", manifest.id));
        if staging.exists() {
            fs::remove_dir_all(&staging)?;
        }
        fs::create_dir(&staging)?;
        for i in 0..zip.len() {
            let mut file = zip.by_index(i)?;
            let name = file.name().replace('\\', "/");
            let Some(relative) = name.strip_prefix(prefix) else {
                continue;
            };
            if relative.is_empty() {
                continue;
            }
            let target = safe_join(&staging, relative)?;
            if file.is_dir() {
                fs::create_dir_all(target)?;
            } else {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut output = fs::File::create(target)?;
                std::io::copy(&mut file, &mut output)?;
            }
        }
        let checked = read_manifest(&staging)?;
        validate(&checked)?;
        let target = self.root.join(&manifest.id);
        if target.exists() {
            fs::remove_dir_all(&target)?;
        }
        fs::rename(&staging, &target)?;
        Ok(Installed {
            manifest: checked,
            directory: target,
            enabled: true,
        })
    }
    /// 将一个已校验的 v2 插件工程打成可导入 ZIP。
    ///
    /// 这让创作者不必安装 Node/Electron 工具链；宿主只接受同一份
    /// `manifest.json`，因此“创作 → 打包 → 导入”的校验边界一致。
    pub fn package_dir(
        &self,
        directory: impl AsRef<Path>,
        destination: impl AsRef<Path>,
    ) -> Result<()> {
        let directory = directory.as_ref();
        let manifest = read_manifest(directory)?;
        validate(&manifest)?;
        let destination = destination.as_ref();
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = fs::File::create(destination)?;
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        write_plugin_tree(&mut zip, directory, directory, options)?;
        zip.finish()?;
        Ok(())
    }
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let manifest = read_manifest(&self.root.join(id))?;
        let mut disabled = self.disabled()?;
        if enabled {
            disabled.remove(&manifest.id);
        } else {
            disabled.insert(manifest.id);
        }
        fs::create_dir_all(&self.root)?;
        fs::write(
            self.root.join(".state.json"),
            serde_json::to_vec_pretty(&State { disabled })?,
        )?;
        Ok(())
    }
    pub fn uninstall(&self, id: &str) -> Result<()> {
        validate_id(id)?;
        let path = self.root.join(id);
        if path.exists() {
            fs::remove_dir_all(path)?;
        }
        let mut disabled = self.disabled()?;
        disabled.remove(id);
        fs::write(
            self.root.join(".state.json"),
            serde_json::to_vec_pretty(&State { disabled })?,
        )?;
        Ok(())
    }
    fn disabled(&self) -> Result<BTreeSet<String>> {
        let path = self.root.join(".state.json");
        if !path.exists() {
            return Ok(BTreeSet::new());
        }
        Ok(serde_json::from_slice::<State>(&fs::read(path)?)?.disabled)
    }
}
#[derive(Serialize, Deserialize, Default)]
struct State {
    #[serde(default)]
    disabled: BTreeSet<String>,
}
fn read_manifest(dir: &Path) -> Result<Manifest> {
    let bytes = fs::read(dir.join("manifest.json"))?;
    let manifest: Manifest = serde_json::from_slice(&bytes)?;
    validate(&manifest)?;
    Ok(manifest)
}
fn validate(m: &Manifest) -> Result<()> {
    if m.manifest_version != 2 {
        bail!("仅支持原生插件 manifestVersion: 2");
    }
    validate_id(&m.id)?;
    if m.name.trim().is_empty() || m.version.trim().is_empty() {
        bail!("插件名称或版本为空");
    }
    if let Some(module) = &m.module {
        if !module.ends_with(".wasm") {
            bail!("插件模块必须是 .wasm");
        }
    }
    for slot in &m.contributions.slots {
        if slot.title.trim().is_empty() {
            bail!("插件容器贡献必须有标题");
        }
        if let Some(ui) = &slot.ui {
            validate_relative(ui)?;
        }
    }
    if let Some(ui) = &m.contributions.ui {
        validate_relative(ui)?;
    }
    Ok(())
}
fn validate_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        bail!("无效插件 ID");
    }
    Ok(())
}
fn safe_join(root: &Path, relative: &str) -> Result<PathBuf> {
    validate_relative(relative)?;
    Ok(root.join(relative))
}
fn validate_relative(relative: &str) -> Result<()> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        bail!("插件包包含越界路径");
    }
    Ok(())
}

fn write_plugin_tree(
    zip: &mut zip::ZipWriter<fs::File>,
    root: &Path,
    current: &Path,
    options: zip::write::SimpleFileOptions,
) -> Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let ty = entry.file_type()?;
        if ty.is_symlink() {
            bail!("插件工程不能包含符号链接：{}", path.display());
        }
        if ty.is_dir() {
            write_plugin_tree(zip, root, &path, options)?;
            continue;
        }
        if !ty.is_file() {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .context("插件文件不在工程目录中")?
            .to_string_lossy()
            .replace('\\', "/");
        safe_join(root, &relative)?;
        zip.start_file(relative, options)?;
        let mut input = fs::File::open(path)?;
        std::io::copy(&mut input, zip)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_a_v2_zip_and_persists_enabled_state() {
        let workspace =
            std::env::temp_dir().join(format!("mochi-plugin-{}", crate::paths::random_base36(12)));
        std::fs::create_dir_all(&workspace).unwrap();
        let archive = workspace.join("sample.zip");
        let file = std::fs::File::create(&archive).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("manifest.json", options).unwrap();
        std::io::Write::write_all(&mut zip, br#"{"manifestVersion":2,"id":"sample","name":"Sample","version":"1.0.0","permissions":["plugin-storage"],"contributions":{"navigation":true}}"#).unwrap();
        zip.finish().unwrap();
        let service = PluginService::new(&workspace);
        let installed = service.install_zip(&archive).unwrap();
        assert_eq!(installed.manifest.id, "sample");
        assert!(installed.directory.join("manifest.json").is_file());
        service.set_enabled("sample", false).unwrap();
        assert!(!service.list().unwrap()[0].enabled);
        service.uninstall("sample").unwrap();
        assert!(service.list().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn bundled_legacy_migrations_are_valid_native_plugin_packages() {
        let workspace = std::env::temp_dir().join(format!(
            "mochi-plugin-migration-{}",
            crate::paths::random_base36(12)
        ));
        let service = PluginService::new(&workspace);
        let bundles = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/dist");
        for archive in [
            "english-lab-containers-v2.zip",
            "quick-navigation-containers-v2.zip",
        ] {
            service.install_zip(bundles.join(archive)).unwrap();
        }
        let installed = service.list().unwrap();
        assert_eq!(installed.len(), 2);
        assert!(installed.iter().all(|plugin| plugin.enabled));
        assert!(installed
            .iter()
            .any(|plugin| plugin.manifest.id == "english-lab"));
        assert!(installed
            .iter()
            .any(|plugin| plugin.manifest.id == "quick-navigation"));
        assert!(service
            .registered_slots()
            .unwrap()
            .iter()
            .any(|slot| slot.contribution.slot == PluginSlot::RightSidebar));
        let english = installed
            .iter()
            .find(|plugin| plugin.manifest.id == "english-lab")
            .unwrap();
        let ui = service.load_ui(english).unwrap().unwrap();
        assert_eq!(ui.title, "English Lab");
        assert_eq!(ui.kind, "dashboard");
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn packages_a_plugin_folder_that_can_be_imported_again() {
        let workspace = std::env::temp_dir().join(format!(
            "mochi-plugin-package-{}",
            crate::paths::random_base36(12)
        ));
        let source = workspace.join("source");
        fs::create_dir_all(source.join("assets")).unwrap();
        fs::write(
            source.join("manifest.json"),
            br#"{"manifestVersion":2,"id":"roundtrip","name":"Roundtrip","version":"1.0.0"}"#,
        )
        .unwrap();
        fs::write(source.join("assets/readme.txt"), "native plugin").unwrap();
        let archive = workspace.join("roundtrip.zip");
        let service = PluginService::new(&workspace);
        service.package_dir(&source, &archive).unwrap();
        let installed = service.install_zip(&archive).unwrap();
        assert_eq!(installed.manifest.id, "roundtrip");
        assert_eq!(
            fs::read_to_string(installed.directory.join("assets/readme.txt")).unwrap(),
            "native plugin"
        );
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn exposes_only_enabled_declared_slots_and_keeps_legacy_navigation() {
        let workspace = std::env::temp_dir().join(format!(
            "mochi-plugin-slots-{}",
            crate::paths::random_base36(12)
        ));
        fs::create_dir_all(&workspace).unwrap();
        let archive = workspace.join("slots.zip");
        let file = fs::File::create(&archive).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("manifest.json", options).unwrap();
        std::io::Write::write_all(&mut zip, br#"{"manifestVersion":2,"id":"slots","name":"Slots","version":"1","contributions":{"navigation":true,"slots":[{"slot":"sidebar","title":"Sidebar"},{"slot":"right-sidebar","title":"Inspector","ui":"ui.json"},{"slot":"editor-toolbar","title":"Insert"}]}}"#).unwrap();
        zip.start_file("ui.json", options).unwrap();
        std::io::Write::write_all(&mut zip, br#"{"kind":"panel","title":"Inspector"}"#).unwrap();
        zip.finish().unwrap();
        let service = PluginService::new(&workspace);
        service.install_zip(&archive).unwrap();
        let slots = service.registered_slots().unwrap();
        assert_eq!(slots.len(), 4);
        assert!(slots
            .iter()
            .any(|slot| slot.contribution.slot == PluginSlot::Navigation));
        assert!(slots
            .iter()
            .any(|slot| slot.contribution.slot == PluginSlot::Sidebar));
        assert!(slots
            .iter()
            .any(|slot| slot.contribution.slot == PluginSlot::EditorToolbar));
        let right = slots
            .iter()
            .find(|slot| slot.contribution.slot == PluginSlot::RightSidebar)
            .unwrap();
        assert_eq!(
            service.load_slot_ui(right).unwrap().unwrap().title,
            "Inspector"
        );
        service.set_enabled("slots", false).unwrap();
        assert!(service.registered_slots().unwrap().is_empty());
        let _ = fs::remove_dir_all(workspace);
    }
}

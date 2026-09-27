//! 沿用 Electron quick-navigation/default/data.json 格式。
use anyhow::{bail, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub schema_version: u32,
    #[serde(default)]
    pub revision: u64,
    #[serde(default = "default_settings")]
    pub settings: Settings,
    #[serde(default = "default_groups")]
    pub groups: Vec<Group>,
    #[serde(default)]
    pub items: Vec<Item>,
    #[serde(default)]
    pub ui: UiState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default)]
    pub browser: Browser,
    #[serde(default = "manual_sort")]
    pub default_sort: String,
    #[serde(default)]
    pub grid: Grid,
    #[serde(default)]
    pub appearance: Appearance,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Browser {
    #[serde(default = "system_browser")]
    pub mode: String,
    #[serde(default)]
    pub executable_path: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Grid {
    #[serde(default = "default_columns")]
    pub columns: u8,
    #[serde(default = "default_rows")]
    pub rows: u8,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Appearance {
    #[serde(default)]
    pub layout: String,
    #[serde(default)]
    pub fields: Fields,
    #[serde(default)]
    pub effects: Effects,
    #[serde(default)]
    pub colors: Colors,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fields {
    #[serde(default = "yes")]
    pub icon: bool,
    #[serde(default = "yes")]
    pub name: bool,
    #[serde(default = "yes")]
    pub target: bool,
    #[serde(default)]
    pub note: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Effects {
    #[serde(default)]
    pub mac_hover: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Colors {
    #[serde(default = "sidebar_color")]
    pub sidebar: String,
    #[serde(default = "workspace_color")]
    pub workspace: String,
    #[serde(default = "tile_color")]
    pub tile: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub order: i64,
    #[serde(default)]
    pub created_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    pub name: String,
    pub target: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub arguments: Vec<String>,
    #[serde(default)]
    pub working_directory: String,
    #[serde(default)]
    pub source_path: String,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub order: i64,
    #[serde(default)]
    pub open_count: u64,
    #[serde(default)]
    pub last_opened_at: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub background_color: Option<String>,
    #[serde(default)]
    pub custom_icon: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiState {
    #[serde(default = "all_selected")]
    pub selected: String,
}

impl Default for Browser {
    fn default() -> Self {
        Self {
            mode: system_browser(),
            executable_path: String::new(),
        }
    }
}
impl Default for Grid {
    fn default() -> Self {
        Self {
            columns: default_columns(),
            rows: default_rows(),
        }
    }
}
impl Default for Fields {
    fn default() -> Self {
        Self {
            icon: true,
            name: true,
            target: true,
            note: false,
        }
    }
}
impl Default for Colors {
    fn default() -> Self {
        Self {
            sidebar: sidebar_color(),
            workspace: workspace_color(),
            tile: tile_color(),
        }
    }
}
impl Default for UiState {
    fn default() -> Self {
        Self {
            selected: all_selected(),
        }
    }
}

fn yes() -> bool {
    true
}
fn manual_sort() -> String {
    "manual".into()
}
fn system_browser() -> String {
    "system".into()
}
fn default_columns() -> u8 {
    5
}
fn default_rows() -> u8 {
    4
}
fn sidebar_color() -> String {
    "#f2f0eb".into()
}
fn workspace_color() -> String {
    "#f7f5f1".into()
}
fn tile_color() -> String {
    "#fffdfa".into()
}
fn all_selected() -> String {
    "all".into()
}
fn now() -> String {
    Utc::now().to_rfc3339()
}
fn uid(prefix: &str) -> String {
    format!(
        "{prefix}-{}-{}",
        Utc::now().timestamp_millis(),
        crate::paths::random_base36(6)
    )
}
fn default_settings() -> Settings {
    Settings {
        browser: Browser {
            mode: system_browser(),
            executable_path: String::new(),
        },
        default_sort: manual_sort(),
        grid: Grid {
            columns: 5,
            rows: 4,
        },
        appearance: Appearance {
            layout: "grid".into(),
            fields: Fields {
                icon: true,
                name: true,
                target: true,
                note: false,
            },
            effects: Effects::default(),
            colors: Colors {
                sidebar: sidebar_color(),
                workspace: workspace_color(),
                tile: tile_color(),
            },
        },
    }
}
fn default_groups() -> Vec<Group> {
    let stamp = now();
    vec![
        Group {
            id: "web".into(),
            name: "网页".into(),
            parent_id: None,
            order: 0,
            created_at: stamp.clone(),
        },
        Group {
            id: "applications".into(),
            name: "应用".into(),
            parent_id: None,
            order: 1,
            created_at: stamp,
        },
    ]
}
impl Default for Model {
    fn default() -> Self {
        Self {
            schema_version: 2,
            revision: 0,
            settings: default_settings(),
            groups: default_groups(),
            items: Vec::new(),
            ui: UiState {
                selected: all_selected(),
            },
        }
    }
}

pub struct Service {
    root: PathBuf,
}
impl Service {
    /// Electron 的 `plugins:databasePath`/`plugins:dataPath` 使用
    /// `.mochi/extensions-data/<id>/<instance>`；安装包在 `.mochi/plugins`
    /// 而数据不在那里，二者绝不能混淆。
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        Self {
            root: workspace
                .as_ref()
                .join(".mochi/extensions-data/quick-navigation/default"),
        }
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn load(&self) -> Result<Model> {
        let path = self.data_path();
        if !path.exists() {
            let model = Model::default();
            self.save(&model)?;
            return Ok(model);
        }
        let bytes =
            fs::read(&path).with_context(|| format!("读取快捷导航数据失败：{}", path.display()))?;
        let mut model: Model =
            serde_json::from_slice(&bytes).context("快捷导航 data.json 格式错误")?;
        self.normalize(&mut model);
        Ok(model)
    }
    pub fn save(&self, model: &Model) -> Result<()> {
        fs::create_dir_all(&self.root)?;
        let path = self.data_path();
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(model)?)?;
        if path.exists() {
            let backup = self.root.join("data.backup.json");
            let _ = fs::copy(&path, backup);
        }
        if path.exists() {
            fs::remove_file(&path)?;
        }
        fs::rename(temporary, path)?;
        Ok(())
    }
    pub fn add_group(
        &self,
        model: &mut Model,
        name: &str,
        parent_id: Option<&str>,
    ) -> Result<String> {
        let name = name.trim();
        if name.is_empty() {
            bail!("分组名称不能为空");
        }
        if let Some(parent) = parent_id {
            self.require_group(model, parent)?;
        }
        let parent_id = parent_id.map(str::to_owned);
        let order = model
            .groups
            .iter()
            .filter(|g| g.parent_id == parent_id)
            .count() as i64;
        let id = uid("group");
        model.groups.push(Group {
            id: id.clone(),
            name: name.into(),
            parent_id,
            order,
            created_at: now(),
        });
        self.bump_save(model)?;
        Ok(id)
    }
    pub fn rename_group(&self, model: &mut Model, id: &str, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            bail!("分组名称不能为空");
        }
        self.require_group_mut(model, id)?.name = name.into();
        self.bump_save(model)
    }
    pub fn move_group(&self, model: &mut Model, id: &str, parent_id: Option<&str>) -> Result<()> {
        if parent_id == Some(id)
            || parent_id
                .is_some_and(|parent| self.descendants(model, id).iter().any(|x| x == parent))
        {
            bail!("不能把分组移动到自己的子级");
        }
        if let Some(parent) = parent_id {
            self.require_group(model, parent)?;
        }
        let new_parent = parent_id.map(str::to_owned);
        let order = model
            .groups
            .iter()
            .filter(|g| g.parent_id == new_parent && g.id != id)
            .count() as i64;
        let group = self.require_group_mut(model, id)?;
        group.parent_id = new_parent;
        group.order = order;
        self.normalize_orders(model);
        self.bump_save(model)
    }
    pub fn delete_group(&self, model: &mut Model, id: &str) -> Result<()> {
        self.require_group(model, id)?;
        let mut ids = self.descendants(model, id);
        ids.push(id.to_owned());
        for item in &mut model.items {
            if item.group_id.as_ref().is_some_and(|x| ids.contains(x)) {
                item.group_id = None;
            }
        }
        model.groups.retain(|g| !ids.contains(&g.id));
        if ids.contains(&model.ui.selected) {
            model.ui.selected = all_selected();
        }
        self.normalize_orders(model);
        self.bump_save(model)
    }
    pub fn upsert_item(&self, model: &mut Model, mut item: Item) -> Result<String> {
        if item.target.trim().is_empty() {
            bail!("快捷方式位置不能为空");
        }
        if item.name.trim().is_empty() {
            item.name = display_name(&item.target);
        }
        if let Some(group) = item.group_id.as_deref() {
            self.require_group(model, group)?;
        }
        let stamp = now();
        if let Some(old) = model.items.iter_mut().find(|old| old.id == item.id) {
            item.created_at = old.created_at.clone();
            item.open_count = old.open_count;
            item.last_opened_at = old.last_opened_at.clone();
            item.updated_at = stamp;
            *old = item.clone();
        } else {
            if item.id.is_empty() {
                item.id = uid("item");
            }
            item.created_at = stamp.clone();
            item.updated_at = stamp;
            item.order = model
                .items
                .iter()
                .filter(|old| old.group_id == item.group_id)
                .count() as i64;
            model.items.push(item.clone());
        }
        self.bump_save(model)?;
        Ok(item.id)
    }
    pub fn remove_item(&self, model: &mut Model, id: &str) -> Result<()> {
        let before = model.items.len();
        model.items.retain(|item| item.id != id);
        if before == model.items.len() {
            bail!("快捷方式不存在");
        }
        self.bump_save(model)
    }
    pub fn move_item(&self, model: &mut Model, id: &str, group_id: Option<&str>) -> Result<()> {
        if let Some(group) = group_id {
            self.require_group(model, group)?;
        }
        let target = group_id.map(str::to_owned);
        let order = model
            .items
            .iter()
            .filter(|x| x.group_id == target && x.id != id)
            .count() as i64;
        let item = model
            .items
            .iter_mut()
            .find(|x| x.id == id)
            .context("快捷方式不存在")?;
        item.group_id = target;
        item.order = order;
        item.updated_at = now();
        self.normalize_item_orders(model);
        self.bump_save(model)
    }
    /// 在同一分组中调整条目顺序。原生 UI 的上下操作与 Electron 的拖放
    /// 使用同一份 `order` 数据，因此两个宿主切换后顺序不会丢失。
    pub fn reorder_item(&self, model: &mut Model, id: &str, direction: i32) -> Result<()> {
        let group_id = model
            .items
            .iter()
            .find(|item| item.id == id)
            .context("快捷方式不存在")?
            .group_id
            .clone();
        let mut indices: Vec<_> = model
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.group_id == group_id)
            .map(|(index, _)| index)
            .collect();
        indices.sort_by_key(|index| model.items[*index].order);
        let current = indices
            .iter()
            .position(|index| model.items[*index].id == id)
            .context("快捷方式不存在")?;
        let target = (current as i32 + direction.signum())
            .clamp(0, indices.len().saturating_sub(1) as i32) as usize;
        if target != current {
            model.items.swap(indices[current], indices[target]);
        }
        for (order, index) in indices.into_iter().enumerate() {
            model.items[index].order = order as i64;
        }
        self.bump_save(model)
    }
    /// 同级分组排序，和 Electron `reorderGroup` 使用相同的 order 语义。
    pub fn reorder_group(&self, model: &mut Model, id: &str, direction: i32) -> Result<()> {
        let parent_id = model
            .groups
            .iter()
            .find(|group| group.id == id)
            .context("分组不存在")?
            .parent_id
            .clone();
        let mut indices: Vec<_> = model
            .groups
            .iter()
            .enumerate()
            .filter(|(_, group)| group.parent_id == parent_id)
            .map(|(index, _)| index)
            .collect();
        indices.sort_by_key(|index| model.groups[*index].order);
        let current = indices
            .iter()
            .position(|index| model.groups[*index].id == id)
            .context("分组不存在")?;
        let target = (current as i32 + direction.signum())
            .clamp(0, indices.len().saturating_sub(1) as i32) as usize;
        if target != current {
            model.groups.swap(indices[current], indices[target]);
        }
        for (order, index) in indices.into_iter().enumerate() {
            model.groups[index].order = order as i64;
        }
        self.bump_save(model)
    }
    pub fn toggle_favorite(&self, model: &mut Model, id: &str) -> Result<()> {
        let item = model
            .items
            .iter_mut()
            .find(|x| x.id == id)
            .context("快捷方式不存在")?;
        item.favorite = !item.favorite;
        item.updated_at = now();
        self.bump_save(model)
    }
    pub fn record_open(&self, model: &mut Model, id: &str) -> Result<()> {
        let item = model
            .items
            .iter_mut()
            .find(|x| x.id == id)
            .context("快捷方式不存在")?;
        item.open_count += 1;
        item.last_opened_at = Some(now());
        item.updated_at = now();
        self.bump_save(model)
    }
    fn data_path(&self) -> PathBuf {
        self.root.join("data.json")
    }
    fn bump_save(&self, model: &mut Model) -> Result<()> {
        model.revision += 1;
        self.save(model)
    }
    fn require_group<'a>(&self, model: &'a Model, id: &str) -> Result<&'a Group> {
        model
            .groups
            .iter()
            .find(|g| g.id == id)
            .context("分组不存在")
    }
    fn require_group_mut<'a>(&self, model: &'a mut Model, id: &str) -> Result<&'a mut Group> {
        model
            .groups
            .iter_mut()
            .find(|g| g.id == id)
            .context("分组不存在")
    }
    fn descendants(&self, model: &Model, id: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut pending = vec![id.to_owned()];
        while let Some(parent) = pending.pop() {
            for group in model
                .groups
                .iter()
                .filter(|g| g.parent_id.as_deref() == Some(&parent))
            {
                out.push(group.id.clone());
                pending.push(group.id.clone());
            }
        }
        out
    }
    fn normalize(&self, model: &mut Model) {
        model.schema_version = 2;
        model.settings.grid.columns = model.settings.grid.columns.clamp(2, 10);
        model.settings.grid.rows = model.settings.grid.rows.clamp(2, 8);
        if model.settings.default_sort.is_empty() {
            model.settings.default_sort = manual_sort();
        }
        self.normalize_orders(model);
        self.normalize_item_orders(model);
    }
    fn normalize_orders(&self, model: &mut Model) {
        let parents: Vec<Option<String>> =
            model.groups.iter().map(|g| g.parent_id.clone()).collect();
        for parent in parents {
            let mut rows: Vec<_> = model
                .groups
                .iter_mut()
                .filter(|g| g.parent_id == parent)
                .collect();
            rows.sort_by_key(|g| g.order);
            for (order, row) in rows.into_iter().enumerate() {
                row.order = order as i64;
            }
        }
    }
    fn normalize_item_orders(&self, model: &mut Model) {
        let parents: Vec<Option<String>> = model.items.iter().map(|x| x.group_id.clone()).collect();
        for parent in parents {
            let mut rows: Vec<_> = model
                .items
                .iter_mut()
                .filter(|x| x.group_id == parent)
                .collect();
            rows.sort_by_key(|x| x.order);
            for (order, row) in rows.into_iter().enumerate() {
                row.order = order as i64;
            }
        }
    }
}
fn display_name(target: &str) -> String {
    target
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(target)
        .trim_end_matches(".lnk")
        .trim_end_matches(".url")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_electron_shape_and_keeps_nested_groups() {
        let root =
            std::env::temp_dir().join(format!("mochi-qn-{}", crate::paths::random_base36(8)));
        let service = Service::new(&root);
        let mut m = Model::default();
        let child = service
            .add_group(&mut m, "开发", Some("applications"))
            .unwrap();
        service.move_group(&mut m, &child, Some("web")).unwrap();
        let id = service
            .upsert_item(
                &mut m,
                Item {
                    id: String::new(),
                    name: "Mochi".into(),
                    target: "https://mochi.local".into(),
                    kind: "web".into(),
                    group_id: Some(child),
                    note: String::new(),
                    arguments: vec![],
                    working_directory: String::new(),
                    source_path: String::new(),
                    favorite: false,
                    order: 0,
                    open_count: 0,
                    last_opened_at: None,
                    created_at: String::new(),
                    updated_at: String::new(),
                    background_color: None,
                    custom_icon: None,
                },
            )
            .unwrap();
        service.record_open(&mut m, &id).unwrap();
        let mut second = m.items[0].clone();
        second.id.clear();
        second.name = "Docs".into();
        second.target = "https://docs.mochi.local".into();
        let second_id = service.upsert_item(&mut m, second).unwrap();
        service.reorder_item(&mut m, &second_id, -1).unwrap();
        assert_eq!(
            m.items.iter().find(|item| item.order == 0).unwrap().id,
            second_id
        );
        service.reorder_group(&mut m, "applications", -1).unwrap();
        assert_eq!(
            m.groups.iter().find(|group| group.order == 0).unwrap().id,
            "applications"
        );
        m.settings.default_sort = "usage".into();
        m.settings.browser.mode = "custom".into();
        m.settings.browser.executable_path = "C:\\Browser\\browser.exe".into();
        m.settings.appearance.layout = "list".into();
        m.settings.appearance.fields.note = true;
        service.save(&m).unwrap();
        let loaded = service.load().unwrap();
        assert_eq!(loaded.schema_version, 2);
        assert_eq!(loaded.items[0].open_count, 1);
        assert_eq!(loaded.groups.len(), 3);
        assert_eq!(loaded.settings.default_sort, "usage");
        assert_eq!(loaded.settings.browser.mode, "custom");
        assert_eq!(
            loaded.settings.browser.executable_path,
            "C:\\Browser\\browser.exe"
        );
        assert_eq!(loaded.settings.appearance.layout, "list");
        assert!(loaded.settings.appearance.fields.note);
        let _ = fs::remove_dir_all(root);
    }
}

//! 工作区模板：模板正文存于 `.mochi/templates/<分组>/`，不进入知识库文件树。
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

mod catalog;
pub use catalog::{BuiltinTemplate, BUILTINS};

pub const UNGROUPED: &str = "未分组";
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    pub name: String,
    pub group: String,
    pub path: PathBuf,
}
impl Template {
    pub fn builtin(&self) -> Option<&'static BuiltinTemplate> {
        BUILTINS.iter().find(|item| {
            item.name == self.name
                && item.group == self.group
                && self.path.extension().and_then(|s| s.to_str()) == Some(item.extension)
        })
    }

    pub fn format_label(&self) -> &'static str {
        match self.path.extension().and_then(|s| s.to_str()) {
            Some("mc") => "墨池文档",
            Some("txt") => "纯文本",
            _ => "Markdown",
        }
    }

    pub fn description(&self) -> &str {
        self.builtin()
            .map(|item| item.description)
            .unwrap_or("保存常用内容与结构，创建副本后开始填写。")
    }
}
pub struct TemplateService {
    root: PathBuf,
}

impl TemplateService {
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        Self {
            root: workspace.as_ref().join(".mochi").join("templates"),
        }
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    /// 按版本补充模板；只升级内容完全匹配的旧模板，保留用户修改与删除。
    pub fn ensure_builtins(&self) -> Result<()> {
        let marker = self.root.join(".builtins-v2");
        if marker.exists() {
            return Ok(());
        }
        let legacy_installed = self.root.join(".builtins-v1").exists();
        std::fs::create_dir_all(&self.root)?;
        for item in BUILTINS {
            let folder = self.root.join(item.group);
            let path = folder.join(format!("{}.{}", item.name, item.extension));
            let legacy = LEGACY_BUILTINS
                .iter()
                .find(|(group, name, _)| *group == item.group && *name == item.name);
            if path.exists() {
                // 被改过的模板（哪怕是故意清空的）都算用户自己的。
                if let Some((_, _, old_body)) = legacy {
                    if std::fs::read_to_string(&path)? == *old_body {
                        crate::files::FileService::new().write_file_safe(&path, item.content)?;
                    }
                }
                continue;
            }
            // 尊重 v1 里已删除的条目，以及已被转换成其他文档格式的模板。
            if (legacy_installed && legacy.is_some())
                || ["md", "mc", "txt"]
                    .iter()
                    .any(|ext| folder.join(format!("{}.{ext}", item.name)).exists())
            {
                continue;
            }
            crate::files::FileService::new().write_file_safe(&path, item.content)?;
        }
        std::fs::write(self.root.join(".builtins-v1"), "内置模板已初始化\n")?;
        std::fs::write(marker, "内置模板已初始化\n")?;
        Ok(())
    }
    pub fn groups(&self) -> Result<Vec<String>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let mut groups = std::fs::read_dir(&self.root)?
            .flatten()
            .filter_map(|entry| {
                entry
                    .path()
                    .is_dir()
                    .then(|| entry.file_name().to_string_lossy().into_owned())
            })
            .collect::<Vec<_>>();
        groups.sort_by(|a, b| compare_groups(a, b));
        Ok(groups)
    }
    pub fn list(&self) -> Result<Vec<Template>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        self.collect(&self.root, UNGROUPED, &mut out)?;
        for group in self.groups()? {
            self.collect(&self.root.join(&group), &group, &mut out)?;
        }
        out.sort_by(|a, b| {
            compare_groups(&a.group, &b.group)
                .then_with(|| crate::files::compare_names(&a.name, &b.name))
        });
        Ok(out)
    }
    fn collect(&self, dir: &Path, group: &str, out: &mut Vec<Template>) -> Result<()> {
        for entry in std::fs::read_dir(dir)?.flatten() {
            let path = entry.path();
            if path.is_file()
                && matches!(
                    path.extension().and_then(|v| v.to_str()),
                    Some("md" | "mc" | "txt")
                )
            {
                out.push(Template {
                    name: path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    group: group.to_owned(),
                    path,
                });
            }
        }
        Ok(())
    }
    pub fn create(&self, name: &str, content: &str) -> Result<Template> {
        self.create_in_group(name, content, UNGROUPED)
    }
    pub fn create_in_group(&self, name: &str, content: &str, group: &str) -> Result<Template> {
        let name = sanitize(name)?;
        let group = sanitize_group(group)?;
        let folder = if group == UNGROUPED {
            self.root.clone()
        } else {
            self.root.join(&group)
        };
        std::fs::create_dir_all(&folder)?;
        if ["md", "mc", "txt"]
            .iter()
            .any(|ext| folder.join(format!("{name}.{ext}")).exists())
        {
            bail!("同名模板已存在");
        }
        let extension = if crate::document_format::requires_mochi_format(content) {
            "mc"
        } else {
            "md"
        };
        let path = folder.join(format!("{name}.{extension}"));
        crate::files::FileService::new().write_file_safe(&path, content)?;
        Ok(Template { name, group, path })
    }
    /// 建立独立副本。只展开日期变量；主题提示保持可编辑。
    pub fn instantiate(&self, template: &Template, parent: &Path) -> Result<PathBuf> {
        let content = fill_date_variables(&self.read(template)?, chrono::Local::now().date_naive());
        let source_extension = template
            .path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("md");
        let extension = if source_extension == "md"
            && crate::document_format::requires_mochi_format(&content)
        {
            "mc"
        } else {
            source_extension
        };
        let name = sanitize(&template.name)?;
        let path = crate::files::get_unique_file_path(parent, &format!("{name}.{extension}"))?;
        crate::files::FileService::new().write_file_safe(&path, &content)?;
        Ok(path)
    }
    pub fn create_group(&self, name: &str) -> Result<()> {
        let name = sanitize(name)?;
        let path = self.root.join(name);
        if path.exists() {
            bail!("同名分组已存在");
        }
        std::fs::create_dir_all(path)?;
        Ok(())
    }
    pub fn rename_group(&self, from: &str, to: &str) -> Result<()> {
        let from = sanitize_group(from)?;
        let to = sanitize(to)?;
        if from == UNGROUPED {
            bail!("未分组不能重命名");
        }
        let a = self.root.join(from);
        let b = self.root.join(to);
        if b.exists() {
            bail!("同名分组已存在");
        }
        std::fs::rename(a, b)?;
        Ok(())
    }
    pub fn delete_group(&self, name: &str) -> Result<()> {
        let name = sanitize_group(name)?;
        if name == UNGROUPED {
            bail!("未分组不能删除");
        }
        let path = self.root.join(name);
        if std::fs::read_dir(&path)?.next().is_some() {
            bail!("分组内仍有模板，请先移动或删除模板");
        }
        std::fs::remove_dir(path)?;
        Ok(())
    }
    pub fn move_to_group(&self, t: &Template, group: &str) -> Result<Template> {
        let group = sanitize_group(group)?;
        ensure_inside(&self.root, &t.path)?;
        let target_dir = if group == UNGROUPED {
            self.root.clone()
        } else {
            self.root.join(&group)
        };
        std::fs::create_dir_all(&target_dir)?;
        let target = target_dir.join(t.path.file_name().unwrap_or_default());
        if target.exists() {
            bail!("目标分组已有同名模板");
        }
        std::fs::rename(&t.path, &target)?;
        Ok(Template {
            name: t.name.clone(),
            group,
            path: target,
        })
    }
    pub fn read(&self, t: &Template) -> Result<String> {
        ensure_inside(&self.root, &t.path)?;
        std::fs::read_to_string(&t.path)
            .with_context(|| format!("无法读取模板：{}", t.path.display()))
    }
    pub fn delete(&self, t: &Template) -> Result<()> {
        ensure_inside(&self.root, &t.path)?;
        std::fs::remove_file(&t.path).with_context(|| format!("无法删除模板：{}", t.path.display()))
    }
}
fn sanitize(raw: &str) -> Result<String> {
    let s = raw.trim();
    if s.is_empty()
        || s.len() > 120
        || s == "."
        || s == ".."
        || s.chars().any(|c| {
            c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
        })
    {
        bail!("名称无效");
    }
    Ok(s.to_owned())
}
fn sanitize_group(raw: &str) -> Result<String> {
    if raw.trim() == UNGROUPED {
        Ok(UNGROUPED.into())
    } else {
        sanitize(raw)
    }
}
fn ensure_inside(root: &Path, path: &Path) -> Result<()> {
    if !path.starts_with(root) {
        bail!("模板路径不在当前工作区内");
    }
    Ok(())
}
fn compare_groups(a: &str, b: &str) -> std::cmp::Ordering {
    let order = ["日常", "学习", "工作", "研究", "开发", "写作"];
    let rank = |group: &str| {
        order
            .iter()
            .position(|g| *g == group)
            .unwrap_or(order.len())
    };
    rank(a)
        .cmp(&rank(b))
        .then_with(|| crate::files::compare_names(a, b))
}

fn fill_date_variables(content: &str, date: chrono::NaiveDate) -> String {
    content
        .replace("{{日期}}", &date.format("%Y-%m-%d").to_string())
        .replace("{{周次}}", &date.format("%V").to_string())
        .replace("{{年份}}", &date.format("%Y").to_string())
        .replace("{{月份}}", &date.format("%m").to_string())
}

const LEGACY_BUILTINS:&[(&str,&str,&str)]=&[
 ("日常","每日复盘","# {{日期}} 每日复盘\n\n## 今天完成\n- \n\n## 收获\n- \n\n## 明日最重要的一件事\n- \n"),
 ("日常","周计划","# 第 {{周次}} 周计划\n\n## 本周目标\n- [ ] \n\n## 关键安排\n- \n\n## 周末复盘\n- \n"),
 ("工作","会议纪要","# {{会议主题}}\n\n- 时间：\n- 参会人：\n\n## 议程\n1. \n\n## 结论\n- \n\n## 行动项\n- [ ] 负责人：  截止：\n"),
 ("工作","项目计划","# {{项目名称}}\n\n## 背景与目标\n\n## 范围\n\n## 里程碑\n- [ ] \n\n## 风险与依赖\n- \n"),
 ("学习","学习笔记","# {{主题}}\n\n## 核心概念\n\n## 我的理解\n\n## 例子\n\n## 待验证问题\n- [ ] \n"),
 ("学习","读书笔记","# 《{{书名}}》\n\n## 摘要\n\n## 触动我的观点\n\n## 可实践的行动\n- [ ] \n"),
];

#[cfg(test)]
mod tests;

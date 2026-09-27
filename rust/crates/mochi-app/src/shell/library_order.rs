//! 调整资料库在侧边栏中的手动排序。
use super::*;

impl Shell {
    /// 在每种资料库类型内部保存排序，同时让当前资料库和
    /// 已打开的缓冲区继续关联原来的对象，而不是旧的索引位置。
    pub fn reorder_library(&mut self, source: &str, target: &str, after: bool) -> Result<bool> {
        let ws = self.workspace.as_ref().context("尚未打开工作区")?;
        if source == target {
            return Ok(false);
        }
        let path = mochi_core::paths::libraries_file(&ws.root);
        let mut libraries: Vec<Library> = mochi_core::json2::read_from_file(&path)?
            .context("找不到知识库配置，请重新打开工作区")?;
        let source_index = libraries
            .iter()
            .position(|l| l.id == source)
            .context("知识库已不存在")?;
        let target_index = libraries
            .iter()
            .position(|l| l.id == target)
            .context("目标知识库已不存在")?;
        anyhow::ensure!(
            libraries[source_index].kind == libraries[target_index].kind,
            "只能在同一类型下调整知识库顺序"
        );
        let selected_id = match ws.scope {
            TreeScope::Library(index) => ws.libraries.get(index).map(|l| l.id.clone()),
            TreeScope::Type(_) => None,
        };
        if let Some(id) = &selected_id {
            anyhow::ensure!(
                libraries.iter().any(|l| &l.id == id),
                "当前知识库已变更，请重新打开工作区"
            );
        }
        // 其他类型的资料库仍保留原有位置和元数据。
        let indices: Vec<usize> = libraries
            .iter()
            .enumerate()
            .filter(|(_, l)| l.kind == libraries[source_index].kind)
            .map(|(index, _)| index)
            .collect();
        let mut ordered: Vec<Library> = indices
            .iter()
            .map(|&index| libraries[index].clone())
            .collect();
        let old_order: Vec<String> = ordered.iter().map(|l| l.id.clone()).collect();
        let source_pos = ordered.iter().position(|l| l.id == source).unwrap();
        let moved = ordered.remove(source_pos);
        let target_pos = ordered.iter().position(|l| l.id == target).unwrap();
        ordered.insert(target_pos + usize::from(after), moved);
        if ordered.iter().map(|l| &l.id).eq(old_order.iter()) {
            return Ok(false);
        }
        for (index, library) in indices.into_iter().zip(ordered) {
            libraries[index] = library;
        }
        mochi_core::json2::write(&path, &libraries)?;
        let ws = self.workspace.as_mut().unwrap();
        if let Some(id) = selected_id {
            ws.scope = TreeScope::Library(libraries.iter().position(|l| l.id == id).unwrap());
        }
        ws.libraries = libraries;
        self.refresh_tree();
        Ok(true)
    }
}

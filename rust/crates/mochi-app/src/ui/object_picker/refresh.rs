//! 更新对象选择器候选项，并协调扫描结果的变化。
use std::collections::{HashMap, HashSet};

use super::{ObjectCandidate, ObjectReference, State, ROW_HEIGHT};

// 标签和摘要属于显示信息；编辑后必须更新原来的那一行。
fn identity(url: &str) -> String {
    let Some((head, query)) = url.split_once('?') else {
        return url.to_owned();
    };
    let fields = query
        .split('&')
        .filter(|pair| {
            let key = pair.split('=').next().unwrap_or_default();
            !matches!(key, "label" | "title" | "snippet" | "view")
        })
        .collect::<Vec<_>>();
    format!("{head}?{}", fields.join("&"))
}

/// 让仍存在的行保持原位并更新内容，然后追加新对象。
/// 只有完整扫描成功后才删除已不存在的对象。
pub(crate) fn reconcile_candidates(
    current: &mut Vec<ObjectCandidate>,
    fresh: Vec<ObjectCandidate>,
) {
    let mut positions = HashMap::new();
    let mut fresh = fresh.into_iter().map(Some).collect::<Vec<_>>();
    for (index, candidate) in fresh.iter().enumerate() {
        positions
            .entry(identity(&candidate.as_ref().unwrap().url))
            .or_insert(index);
    }
    let mut merged = Vec::with_capacity(fresh.len());
    let mut seen = HashSet::new();
    for candidate in current.iter() {
        let key = identity(&candidate.url);
        if let Some(&index) = positions.get(&key) {
            if let Some(candidate) = fresh[index].take() {
                seen.insert(key);
                merged.push(candidate);
            }
        }
    }
    for candidate in fresh.into_iter().flatten() {
        if seen.insert(identity(&candidate.url)) {
            merged.push(candidate);
        }
    }
    *current = merged;
}

impl State {
    pub fn update_candidates(&mut self, candidates: Vec<ObjectCandidate>) {
        let previous = self.filtered_candidates();
        let selected = previous
            .get(self.selected_row)
            .map(|(_, row)| identity(&row.url));
        let top = (self.scroll / ROW_HEIGHT).floor() as usize;
        let offset = self.scroll % ROW_HEIGHT;
        reconcile_candidates(&mut self.candidates, candidates);

        // 标题变化（以及随之变化的网址提示）时，仍保持原对象处于选中状态。
        let urls = self
            .candidates
            .iter()
            .map(|row| (identity(&row.url), &row.url))
            .collect::<HashMap<_, _>>();
        for url in &mut self.selected {
            if ObjectReference::parse(url).is_some() {
                if let Some(updated) = urls.get(&identity(url)) {
                    *url = (*updated).clone();
                }
            }
        }
        let rows = self.filtered_candidates();
        let positions = rows
            .iter()
            .enumerate()
            .map(|(index, (_, row))| (identity(&row.url), index))
            .collect::<HashMap<_, _>>();
        self.selected_row = selected
            .and_then(|key| positions.get(&key).copied())
            .unwrap_or(self.selected_row.min(rows.len().saturating_sub(1)));
        // 继续锚定在顶部的同一对象；如果它已被删除，则改用下一个仍存在的对象。
        if let Some(index) = previous
            .iter()
            .skip(top)
            .find_map(|(_, row)| positions.get(&identity(&row.url)))
        {
            self.scroll = *index as f32 * ROW_HEIGHT + offset;
        } else if rows.is_empty() {
            self.scroll = 0.0;
        }
        if previous != rows {
            self.hover = None;
        }
    }
}

#[cfg(test)]
mod tests;

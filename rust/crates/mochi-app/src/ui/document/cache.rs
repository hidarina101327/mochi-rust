//! 缓存文档排版结果，并处理表格 HTML 和换行文本。
use super::*;

/// 编辑器布局中昂贵部分的有界缓存。
///
/// 文档解析仍按变更后的缓冲区全量做一次，但改了一个块，不应让其他所有
/// 未变动的段落重新付出行内标记解析、整形和换行的代价。缓存项只包含
/// 局部的视觉行；组合文档时由调用方提供当前的 y/x/块序号/源码坐标，
/// 因此前方文本移动了字节偏移之后，后面的块依然可以复用。
///
/// 缓存刻意挂在 `DocPane` 上而不是做成全局的。这样每个编辑器的内存
/// 有上限，也不会一直攥着已关闭文档的文本。
#[derive(Default)]
pub struct LayoutCache {
    pub(super) wrapped: HashMap<u64, Vec<WrappedEntry>>,
    pub(super) order: VecDeque<(u64, u64)>,
    pub(super) entries: usize,
    pub(super) next_id: u64,
    pub(super) bytes: usize,
    pub(super) preferences: Option<crate::ui::editor_preferences::Preferences>,
    pub(super) measurement_epoch: u64,
    pub(super) generation: u64,
    pub(super) active_generation: Option<u64>,
    #[cfg(test)]
    pub(super) hits: usize,
    #[cfg(test)]
    pub(super) misses: usize,
}

pub(super) fn parse_html_table(html: &str) -> Option<(Vec<Vec<String>>, bool)> {
    let lower = html.to_ascii_lowercase();
    if !lower.contains("</table>") {
        return None;
    }
    let mut rows = Vec::new();
    let mut cursor = 0usize;
    let mut header = false;
    while let Some(rel) = lower[cursor..].find("<tr") {
        let row_start = cursor + rel;
        let Some(open_end_rel) = lower[row_start..].find('>') else {
            break;
        };
        let body_start = row_start + open_end_rel + 1;
        let Some(close_rel) = lower[body_start..].find("</tr>") else {
            break;
        };
        let body_end = body_start + close_rel;
        let row_lower = &lower[body_start..body_end];
        let row_html = &html[body_start..body_end];
        let mut cells = Vec::new();
        let mut cell_cursor = 0usize;
        while cell_cursor < row_lower.len() {
            let th = row_lower[cell_cursor..]
                .find("<th")
                .map(|n| (n, "</th>", true));
            let td = row_lower[cell_cursor..]
                .find("<td")
                .map(|n| (n, "</td>", false));
            let Some((rel, close, is_header)) = (match (th, td) {
                (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            }) else {
                break;
            };
            let tag_start = cell_cursor + rel;
            let Some(tag_end_rel) = row_lower[tag_start..].find('>') else {
                break;
            };
            let content_start = tag_start + tag_end_rel + 1;
            let Some(content_end_rel) = row_lower[content_start..].find(close) else {
                break;
            };
            let content_end = content_start + content_end_rel;
            header |= is_header;
            cells.push(strip_html_text(&row_html[content_start..content_end]));
            cell_cursor = content_end + close.len();
        }
        if !cells.is_empty() {
            rows.push(cells);
        }
        cursor = body_end + "</tr>".len();
    }
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    if cols == 0 {
        return None;
    }
    for row in &mut rows {
        row.resize(cols, String::new());
    }
    Some((rows, header))
}

fn strip_html_text(value: &str) -> String {
    let mut plain = String::new();
    let mut in_tag = false;
    for ch in value.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => plain.push(ch),
            _ => {}
        }
    }
    plain
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .trim()
        .to_owned()
}

#[derive(Clone)]
pub(super) struct WrappedEntry {
    pub(super) id: u64,
    pub(super) generation: u64,
    pub(super) text: String,
    pub(super) style: TextStyle,
    pub(super) width_bits: u32,
    pub(super) lines: Vec<WrappedLine>,
    pub(super) bytes: usize,
}

#[derive(Clone)]
pub(super) struct WrappedLine {
    pub(super) runs: Vec<Run>,
    pub(super) visible_start: usize,
    pub(super) height: f32,
}

// 独立的 wrapped() 调用沿用旧的有界 FIFO 行为。进行中的编辑器布局是
// 一个代际快照，允许超出这些上限，以便大文档在下一次编辑时继续复用；
// finish_generation 会丢弃当前快照没有触及的条目。
pub(super) const MAX_WRAPPED_ENTRIES: usize = 8192;

pub(super) const MAX_WRAPPED_BYTES: usize = 16 * 1024 * 1024;

impl LayoutCache {
    pub(crate) fn clear(&mut self) {
        self.wrapped.clear();
        self.order.clear();
        self.entries = 0;
        self.next_id = 0;
        self.bytes = 0;
        self.generation = 0;
        self.active_generation = None;
        #[cfg(test)]
        {
            self.hits = 0;
            self.misses = 0;
        }
    }

    pub(super) fn prepare(&mut self) {
        let preferences = crate::ui::editor_preferences::current();
        let epoch = crate::ui::measurement::epoch();
        if self.preferences.as_ref() != Some(&preferences) || self.measurement_epoch != epoch {
            self.clear();
            self.preferences = Some(preferences);
            self.measurement_epoch = epoch;
        }
        // 渐进式遍历可能因为编辑、缩放、模式切换或文档变更，在调用方走到
        // `finish_generation` 之前被取消。这里在开始下一个快照之前先关掉
        // 上一个；否则活跃的代际会绕过 FIFO 上限，被放弃的编辑会无限累积。
        self.finish_generation();
        self.generation = self.generation.wrapping_add(1).max(1);
        self.active_generation = Some(self.generation);
    }

    pub(super) fn finish_generation(&mut self) {
        let Some(generation) = self.active_generation.take() else {
            return;
        };
        let mut live_ids = HashSet::new();
        let mut removed_entries = 0usize;
        let mut removed_bytes = 0usize;
        for bucket in self.wrapped.values_mut() {
            bucket.retain(|entry| {
                if entry.generation == generation {
                    live_ids.insert(entry.id);
                    true
                } else {
                    removed_entries += 1;
                    removed_bytes = removed_bytes.saturating_add(entry.bytes);
                    false
                }
            });
        }
        self.wrapped.retain(|_, bucket| !bucket.is_empty());
        self.entries = self.entries.saturating_sub(removed_entries);
        self.bytes = self.bytes.saturating_sub(removed_bytes);
        self.order.retain(|(_, id)| live_ids.contains(id));
    }

    pub(super) fn wrapped(&mut self, text: &str, style: TextStyle, width: f32) -> Vec<WrappedLine> {
        let width_bits = width.to_bits();
        let key_hash = wrapped_hash(text, style, width_bits);
        let generation = self.active_generation;
        // 不在编辑器代际内时，FIFO 淘汰保证缓存再大查找也是 O(1)。代际
        // 进行中则保留条目直到本轮结束，让顺序遍历能覆盖整个活动文档，
        // 而不是在下一次编辑重访它们之前就把开头的块淘汰掉。
        let hit = self.wrapped.get_mut(&key_hash).and_then(|entries| {
            let entry = entries.iter_mut().find(|entry| {
                entry.style == style && entry.width_bits == width_bits && entry.text == text
            })?;
            if let Some(generation) = generation {
                entry.generation = generation;
            }
            Some(entry.lines.clone())
        });
        if let Some(lines) = hit {
            #[cfg(test)]
            {
                self.hits += 1;
            }
            return lines;
        }

        #[cfg(test)]
        {
            self.misses += 1;
        }
        let runs = text::parse_inline(text);
        let lines = crate::ui::math_runs::wrap_mapped(&runs, style, width)
            .into_iter()
            .map(|(runs, visible_start)| WrappedLine {
                height: text::runs_height(&runs, style),
                runs,
                visible_start,
            })
            .collect::<Vec<_>>();
        let bytes = text.len()
            + lines
                .iter()
                .map(|line| {
                    std::mem::size_of::<WrappedLine>()
                        + line
                            .runs
                            .iter()
                            .map(|run| std::mem::size_of::<Run>() + run.text.len())
                            .sum::<usize>()
                })
                .sum::<usize>();
        if bytes <= MAX_WRAPPED_BYTES {
            if generation.is_none() {
                while self.entries >= MAX_WRAPPED_ENTRIES
                    || self.bytes.saturating_add(bytes) > MAX_WRAPPED_BYTES
                {
                    let Some(old) = self.order.pop_front() else {
                        break;
                    };
                    if let Some(bucket) = self.wrapped.get_mut(&old.0) {
                        if let Some(index) = bucket.iter().position(|entry| entry.id == old.1) {
                            let removed = bucket.swap_remove(index);
                            self.bytes = self.bytes.saturating_sub(removed.bytes);
                            self.entries = self.entries.saturating_sub(1);
                        }
                        if bucket.is_empty() {
                            self.wrapped.remove(&old.0);
                        }
                    }
                }
            }
            let id = self.next_id;
            self.next_id = self.next_id.wrapping_add(1);
            self.wrapped
                .entry(key_hash)
                .or_default()
                .push(WrappedEntry {
                    id,
                    generation: generation.unwrap_or(0),
                    text: text.to_owned(),
                    style,
                    width_bits,
                    lines: lines.clone(),
                    bytes,
                });
            self.order.push_back((key_hash, id));
            self.entries += 1;
            self.bytes += bytes;
        }
        lines
    }

    #[cfg(test)]
    pub(super) fn stats(&self) -> (usize, usize, usize) {
        (self.entries, self.hits, self.misses)
    }
}

fn wrapped_hash(text: &str, style: TextStyle, width_bits: u32) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    style.hash(&mut hasher);
    width_bits.hash(&mut hasher);
    hasher.finish()
}

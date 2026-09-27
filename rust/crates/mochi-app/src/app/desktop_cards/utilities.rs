//! Desktop utilities share the card background snapshot pipeline.
use super::*;
use crate::platform::{desktop_media as media, desktop_search};
use model::{Action, Page, PageSnapshot, Row, RowMeta};

fn row(
    id: &str,
    title: impl Into<String>,
    detail: impl Into<String>,
    action: Option<Action>,
    icon: &str,
) -> Row {
    Row {
        id: id.into(),
        title: title.into(),
        detail: detail.into(),
        action,
        meta: RowMeta {
            icon: icon.into(),
            always_detail: true,
            ..Default::default()
        },
        ..Default::default()
    }
}
fn command(id: &str, title: impl Into<String>) -> Row {
    let title = title.into();
    let icon = match id {
        "media:previous" => "ArrowLeft",
        "media:next" => "ArrowRight",
        "media:toggle" => {
            if title == "暂停" {
                "Pause"
            } else {
                "Play"
            }
        }
        "configure" => {
            if title.starts_with("搜索") {
                "Search"
            } else {
                "Settings"
            }
        }
        _ if id.starts_with("source:") => "Monitor",
        _ => "file",
    };
    row(id, title, "", Some(Action::DesktopUtility(id.into())), icon)
}

pub(super) fn snapshot(root: &Path, config: &DesktopConfig) -> anyhow::Result<model::Snapshot> {
    let automatic = {
        match super::folder_actions::FILE_OPERATIONS.lock() {
            Ok(_guard) => model::folder_auto::refresh(config),
            Err(_) => vec!["文件操作队列状态异常，自动整理已跳过".into()],
        }
    };
    let mut snapshot = model::build_snapshot(root, config)?;
    for card in config.cards.iter().filter(|c| c.enabled) {
        if let Some(page) = card
            .active_page()
            .filter(|p| p.module == Module::Folder && p.folder.auto_organize)
        {
            if let Some(data) = snapshot.pages.get_mut(&model::page_key(&card.id, &page.id)) {
                data.subtitle = format!(
                    "自动整理已开启 · {}{}",
                    data.subtitle,
                    if automatic.is_empty() {
                        String::new()
                    } else {
                        format!(" · {}", automatic.join("；"))
                    }
                );
            }
        }
    }
    for card in config.cards.iter().filter(|c| c.enabled) {
        let Some(page) = card
            .active_page()
            .filter(|p| matches!(p.module, Module::Weather | Module::Music | Module::Search))
        else {
            continue;
        };
        let mut data = PageSnapshot::empty(page);
        let result = match page.module {
            Module::Weather => {
                model::weather::to_page_snapshot(page, &root.join(".mochi").join("desktop-weather"))
            }
            Module::Music => music(page),
            Module::Search => search(root, page),
            _ => unreachable!(),
        };
        match result {
            Ok(value) => data = value,
            Err(e) => {
                data.empty_message = format!("{e:#}");
                data.subtitle = format!("暂不可用：{e}");
            }
        }
        data.rows
            .retain(|row| row.action != Some(Action::DesktopUtility("configure".into())));
        data.rows.insert(
            0,
            command(
                "configure",
                match page.module {
                    Module::Weather => "设置城市…",
                    Module::Search => "搜索…",
                    _ => "音乐显示设置…",
                },
            ),
        );
        if !data.empty_message.is_empty() && data.rows.len() == 1 {
            data.rows
                .push(row("status", &data.empty_message, "", None, "file"));
        }
        snapshot
            .pages
            .insert(model::page_key(&card.id, &page.id), data);
    }
    Ok(snapshot)
}

fn music(page: &Page) -> anyhow::Result<PageSnapshot> {
    let current = media::snapshot(&page.utility.media_source)?;
    Ok(music_snapshot(page, current))
}

fn music_snapshot(page: &Page, current: media::Snapshot) -> PageSnapshot {
    let mut data = PageSnapshot::empty(page);
    let mut sources = vec![command("source:", "播放来源：跟随系统")];
    for (id, name) in &current.sources {
        sources.push(command(
            &format!("source:{id}"),
            format!("播放来源：{name}"),
        ));
    }
    if !current.available {
        data.rows.push(row(
            "media:status",
            "暂无正在播放的媒体",
            "请先播放音乐，或切换播放来源",
            None,
            "file",
        ));
        data.rows.extend(sources);
        return data;
    }
    data.subtitle = format!(
        "{} · {} / {} 秒",
        current.source, current.position_seconds, current.duration_seconds
    );
    data.rows.push(row(
        "media:title",
        if current.title.is_empty() {
            "当前媒体"
        } else {
            &current.title
        },
        &current.artist,
        Some(Action::DesktopUtility("media:toggle".into())),
        "Play",
    ));
    data.rows.extend([
        command("media:previous", "上一首"),
        command(
            "media:toggle",
            if current.playing { "暂停" } else { "播放" },
        ),
        command("media:next", "下一首"),
        row(
            "media:back",
            "后退 10 秒",
            "",
            Some(Action::DesktopUtility(format!(
                "media:seek:{}",
                current.position_seconds.saturating_sub(10)
            ))),
            "RotateCcw",
        ),
        row(
            "media:forward",
            "前进 10 秒",
            "",
            Some(Action::DesktopUtility(format!(
                "media:seek:{}",
                current
                    .position_seconds
                    .saturating_add(10)
                    .min(current.duration_seconds)
            ))),
            "RotateCw",
        ),
    ]);
    data.rows.extend(sources);
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_media_keeps_source_recovery_controls() {
        let page = Page::new(Module::Music);
        let data = music_snapshot(
            &page,
            media::Snapshot {
                sources: vec![("player.exe".into(), "播放器".into())],
                ..Default::default()
            },
        );
        assert!(data
            .rows
            .iter()
            .any(|r| r.action == Some(Action::DesktopUtility("source:".into()))));
        assert!(data
            .rows
            .iter()
            .any(|r| r.action == Some(Action::DesktopUtility("source:player.exe".into()))));
        assert!(!data.rows.iter().any(|r| r.id == "media:toggle"));
    }

    #[test]
    fn media_seek_controls_stay_within_track_and_ids_are_unique() {
        let page = Page::new(Module::Music);
        let data = music_snapshot(
            &page,
            media::Snapshot {
                available: true,
                playing: true,
                position_seconds: 3,
                duration_seconds: 8,
                ..Default::default()
            },
        );
        assert!(data
            .rows
            .iter()
            .any(|r| r.action == Some(Action::DesktopUtility("media:seek:0".into()))));
        assert!(data
            .rows
            .iter()
            .any(|r| r.action == Some(Action::DesktopUtility("media:seek:8".into()))));
        assert_eq!(
            data.rows
                .iter()
                .map(|r| &r.id)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            data.rows.len()
        );
    }
}

fn search(root: &Path, page: &Page) -> anyhow::Result<PageSnapshot> {
    let mut data = PageSnapshot::empty(page);
    let query = page.utility.query.trim();
    if query.is_empty() {
        data.empty_message = "设置关键词开始搜索电脑文件和墨池内容".into();
        return Ok(data);
    }
    let mut issues = Vec::new();
    let index = Arc::new(mochi_core::metadata_index::MetadataIndexService::new(root));
    match index.open() {
        Ok(_) => {
            let result = mochi_core::search::SearchService::new(root, index).search(
                query,
                &mochi_core::search::SearchOptions {
                    max_results: 100,
                    max_per_file: 1,
                    ..Default::default()
                },
            );
            if let Some(e) = result.error {
                issues.push(format!("墨池：{e}"));
            }
            for item in result.groups.into_iter().take(100) {
                let mut entry = row(
                    &format!("workspace:{}", item.rel_path),
                    item.title,
                    item.matches
                        .first()
                        .map(|m| m.content.clone())
                        .unwrap_or_default(),
                    Some(Action::OpenFile(item.rel_path)),
                    "file",
                );
                entry.meta.group = "墨池内容".into();
                entry.meta.path = item.path;
                data.rows.push(entry);
            }
        }
        Err(e) => issues.push(format!("墨池索引：{e}")),
    }
    match desktop_search::search(query, 100) {
        Ok(found) => {
            for item in found {
                let path = item.path.to_string_lossy().into_owned();
                if data
                    .rows
                    .iter()
                    .any(|r| r.meta.path.eq_ignore_ascii_case(&path))
                {
                    continue;
                }
                let mut entry = row(
                    &format!("file:{path}"),
                    item.name,
                    path.clone(),
                    Some(Action::DesktopUtility(format!("open:{path}"))),
                    if item.directory { "folder" } else { "file" },
                );
                entry.meta.group = "电脑文件".into();
                entry.meta.path = path;
                entry.meta.directory = item.directory;
                data.rows.push(entry);
            }
        }
        Err(e) => issues.push(format!("Everything：{e}")),
    }
    data.subtitle = format!("{} 项 · {query}", data.rows.len());
    for (i, issue) in issues.into_iter().enumerate() {
        data.rows
            .push(row(&format!("search-status:{i}"), issue, "", None, "file"));
    }
    if data.rows.is_empty() {
        data.empty_message = "未找到匹配结果".into();
    }
    Ok(data)
}

impl App {
    pub(super) fn desktop_utility_action(&mut self, page: &Page, action: &str) {
        if action == "configure" {
            let target = self
                .desktop
                .config
                .cards
                .iter()
                .find(|c| c.pages.iter().any(|p| p.id == page.id))
                .map(|c| c.id.clone());
            if let Some(card) = target {
                self.desktop_manage_page(&card, &page.id);
            }
            return;
        }
        if let Some(path) = action
            .strip_prefix("open:")
            .filter(|_| page.module == Module::Search)
        {
            self.desktop_open_target(path);
            return;
        }
        if let Some(source) = action
            .strip_prefix("source:")
            .filter(|_| page.module == Module::Music)
        {
            if let Some(p) = self
                .desktop
                .config
                .cards
                .iter_mut()
                .flat_map(|c| &mut c.pages)
                .find(|p| p.id == page.id)
            {
                p.utility.media_source = source.into();
            }
            self.desktop_config_changed();
            self.desktop_refresh();
            return;
        }
        if page.module != Module::Music || self.desktop.utility_job.is_some() {
            return;
        }
        let command = match action {
            "media:toggle" => media::Command::PlayPause,
            "media:previous" => media::Command::Previous,
            "media:next" => media::Command::Next,
            _ => {
                let Some(seconds) = action
                    .strip_prefix("media:seek:")
                    .and_then(|s| s.parse::<u64>().ok())
                else {
                    return;
                };
                media::Command::SeekSeconds(seconds)
            }
        };
        let source = page.utility.media_source.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("desktop-media-control".into())
            .spawn(move || {
                let _ = tx.send(media::control(&source, command).map_err(|e| e.to_string()));
            });
        match spawned {
            Ok(_) => self.desktop.utility_job = Some((self.desktop.epoch, rx)),
            Err(e) => self.desktop_error(e.to_string()),
        }
        self.desktop_timer_state();
    }

    pub(super) fn desktop_take_utility_result(&mut self) {
        let Some((_, rx)) = &self.desktop.utility_job else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(_) => Err("媒体操作线程停止".into()),
        };
        let (epoch, _) = self.desktop.utility_job.take().unwrap();
        if epoch != self.desktop.epoch {
            return;
        }
        if let Err(e) = result {
            self.desktop_error(e);
        }
        self.desktop_refresh();
    }
}

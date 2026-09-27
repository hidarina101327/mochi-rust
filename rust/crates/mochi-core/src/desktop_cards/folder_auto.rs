//! Opt-in organizer: establish a baseline, then move only newly observed stable files.

use super::{
    folder::{self, FolderConfig},
    folder_operations as ops, DesktopConfig, Module,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
    time::{Duration, Instant, SystemTime},
};

const STABLE_FOR: Duration = Duration::from_secs(5);
const TEMPORARY_EXTENSIONS: &[&str] = &["tmp", "part", "crdownload", "download", "partial"];

#[derive(Clone, PartialEq, Eq)]
struct Fingerprint {
    len: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
}

#[derive(Default)]
struct State {
    known: BTreeSet<PathBuf>,
    pending: BTreeMap<PathBuf, (Fingerprint, Instant)>,
    initialized: bool,
}

static STATES: LazyLock<Mutex<BTreeMap<PathBuf, State>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

pub fn refresh(config: &DesktopConfig) -> Vec<String> {
    let Ok(mut states) = STATES.lock() else {
        return vec![];
    };
    let mut active = BTreeSet::new();
    let mut messages = vec![];
    for page in config
        .cards
        .iter()
        .filter(|card| card.enabled)
        .filter_map(|card| card.active_page())
        .filter(|page| page.module == Module::Folder && page.folder.auto_organize)
    {
        let current = match folder::current_config(&page.folder) {
            Ok(config) => config,
            Err(error) => {
                messages.push(error.to_string());
                continue;
            }
        };
        let root = PathBuf::from(&current.path);
        active.insert(root.clone());
        let state = states.entry(root).or_default();
        if let Err(error) = refresh_one(&current, state) {
            messages.push(error.to_string());
        }
    }
    states.retain(|path, _| active.contains(path));
    messages
}

fn refresh_one(config: &FolderConfig, state: &mut State) -> anyhow::Result<()> {
    let plan = ops::plan_organize(config)?;
    process_plan(config, state, plan, Instant::now())
}

fn process_plan(
    config: &FolderConfig,
    state: &mut State,
    mut plan: ops::OrganizePlan,
    now: Instant,
) -> anyhow::Result<()> {
    let mut present = BTreeSet::new();
    let mut ready = BTreeSet::new();

    for item in &plan.actions {
        let path = &item.source;
        present.insert(path.clone());
        if !state.initialized {
            state.known.insert(path.clone());
            continue;
        }
        if state.known.contains(path) || is_temporary(path) {
            continue;
        }
        let Some(mark) = fingerprint(path) else {
            continue;
        };
        match state.pending.get(path) {
            Some((old, since)) if old == &mark && since.elapsed() >= STABLE_FOR => {
                ready.insert(path.clone());
            }
            Some((old, _)) if old == &mark => {}
            _ => {
                state.pending.insert(path.clone(), (mark, now));
            }
        }
    }

    state.initialized = true;
    state.known.retain(|path| present.contains(path));
    state.pending.retain(|path, _| present.contains(path));
    plan.actions.retain(|action| ready.contains(&action.source));
    // Conflicts and unsafe entries are left in place. Only report failures from an
    // actual ready operation, so a persistent plan-time conflict cannot spam refresh.
    plan.failures.clear();
    if plan.actions.is_empty() {
        return Ok(());
    }

    let results = ops::apply_organize(config, &plan);
    let mut errors = vec![];
    for result in results {
        state.pending.remove(&result.source);
        // Remember both successes and failures. A failed operation is reported once;
        // a later refresh will not hammer the same unchanged file every few seconds.
        state.known.insert(result.source.clone());
        if let Some(error) = result.error {
            errors.push(format!("{}：{error}", result.source.display()));
        }
    }
    anyhow::ensure!(errors.is_empty(), "自动整理部分失败：{}", errors.join("；"));
    Ok(())
}

fn fingerprint(path: &Path) -> Option<Fingerprint> {
    let metadata = std::fs::metadata(path).ok()?;
    Some(Fingerprint {
        len: metadata.len(),
        modified: metadata.modified().ok(),
        created: metadata.created().ok(),
    })
}

fn is_temporary(path: &Path) -> bool {
    let extension = path.extension().unwrap_or_default().to_string_lossy();
    TEMPORARY_EXTENSIONS
        .iter()
        .any(|candidate| extension.eq_ignore_ascii_case(candidate))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "mochi-folder-auto-{}-{}",
                std::process::id(),
                TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).expect("create test-owned temp folder");
            Self(path)
        }

        fn folder_config(&self) -> FolderConfig {
            FolderConfig {
                path: self.0.to_string_lossy().into_owned(),
                ..FolderConfig::default()
            }
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn first_scan_only_establishes_a_baseline() {
        let temp = TempRoot::new();
        let source = temp.0.join("photo.png");
        std::fs::write(&source, b"existing file").unwrap();
        let mut state = State::default();

        refresh_one(&temp.folder_config(), &mut state).unwrap();

        assert!(source.is_file());
        assert!(!temp.0.join("图片/photo.png").exists());
        assert!(state.initialized);
        assert!(state.known.contains(&source));
        assert!(state.pending.is_empty());
    }

    #[test]
    fn new_file_waits_for_a_stable_five_second_fingerprint() {
        let temp = TempRoot::new();
        let source = temp.0.join("new-image.png");
        std::fs::write(&source, b"new file").unwrap();
        let config = temp.folder_config();
        let mut state = State {
            initialized: true,
            ..State::default()
        };

        refresh_one(&config, &mut state).unwrap();
        assert!(
            source.is_file(),
            "the first observation only starts the timer"
        );
        assert!(state.pending.contains_key(&source));

        let (mark, _) = state.pending.get(&source).unwrap().clone();
        state.pending.insert(
            source.clone(),
            (mark, Instant::now() - Duration::from_secs(6)),
        );
        refresh_one(&config, &mut state).unwrap();

        assert!(!source.exists());
        assert!(temp.0.join("图片/new-image.png").is_file());
        assert!(state.known.contains(&source));
        assert!(!state.pending.contains_key(&source));
    }

    #[test]
    fn download_and_partial_extensions_are_never_classified() {
        let temp = TempRoot::new();
        let names = ["a.tmp", "b.part", "c.crdownload", "d.download", "e.partial"];
        for name in names {
            std::fs::write(temp.0.join(name), b"incomplete").unwrap();
        }
        let config = temp.folder_config();
        let mut state = State {
            initialized: true,
            ..State::default()
        };

        refresh_one(&config, &mut state).unwrap();

        for name in names {
            assert!(temp.0.join(name).is_file());
        }
        assert!(state.pending.is_empty());
        assert!(state.known.is_empty());
    }

    #[test]
    fn failed_ready_move_is_reported_once_and_not_retried_on_refresh() {
        let temp = TempRoot::new();
        let source = temp.0.join("photo.png");
        std::fs::write(&source, b"image").unwrap();
        let config = temp.folder_config();
        let plan = ops::plan_organize(&config).unwrap();
        assert_eq!(plan.actions.len(), 1);

        let mark = fingerprint(&source).unwrap();
        let mut state = State {
            initialized: true,
            pending: BTreeMap::from([(
                source.clone(),
                (mark, Instant::now() - Duration::from_secs(6)),
            )]),
            ..State::default()
        };
        let category = temp.0.join("图片");
        std::fs::create_dir(&category).unwrap();
        let conflicting_target = category.join("photo.png");
        std::fs::write(&conflicting_target, b"keep existing target").unwrap();

        let error = process_plan(&config, &mut state, plan, Instant::now()).unwrap_err();
        assert!(error.to_string().contains("自动整理部分失败"));
        assert!(source.is_file());
        assert_eq!(
            std::fs::read(&conflicting_target).unwrap(),
            b"keep existing target"
        );
        assert!(state.known.contains(&source));
        assert!(!state.pending.contains_key(&source));

        // The unchanged conflict is encountered again as a plan-time conflict,
        // but it is neither retried nor surfaced as the same operation failure.
        let next = refresh_one(&config, &mut state);
        assert!(next.is_ok());
        assert!(source.is_file());
        assert_eq!(
            std::fs::read(&conflicting_target).unwrap(),
            b"keep existing target"
        );
    }
}

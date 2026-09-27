//! 移动文件后的维护流程会优先检查当前打开的多维表格草稿，再读取磁盘内容。
use super::{Shell, TabKind};
use crate::ui::viewer::Content;
use anyhow::{bail, Result};
use mochi_core::base::{parse_base_document, serialize_base_document};
use mochi_core::base_reference_paths::{rename_base_document_references, BaseReferenceRename};
use mochi_core::files::FileService;
use std::path::{Path, PathBuf};

fn same_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    #[cfg(windows)]
    {
        let normalize = |path: &Path| {
            path.to_string_lossy()
                .replace('\\', "/")
                .trim_start_matches("//?/")
                .to_lowercase()
        };
        normalize(left) == normalize(right)
    }
    #[cfg(not(windows))]
    false
}

fn collect_base_files(directory: &Path, files: &mut Vec<PathBuf>, failed: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        failed.push(directory.to_owned());
        return;
    };
    for entry in entries {
        let Ok(entry) = entry else {
            failed.push(directory.to_owned());
            continue;
        };
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.')
            || [
                "node_modules",
                "dist",
                "dist-electron",
                "release",
                "build",
                "out",
                "target",
                "vendor",
                "coverage",
            ]
            .contains(&name.as_ref())
        {
            continue;
        }
        let Ok(metadata) = entry.path().symlink_metadata() else {
            failed.push(entry.path());
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                continue;
            } // 同时跳过目录联接和符号链接。
        }
        if metadata.is_dir() {
            collect_base_files(&entry.path(), files, failed);
        } else if metadata.is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("mcb"))
        {
            files.push(entry.path());
        }
    }
}

impl Shell {
    pub(super) fn update_base_references_for_rename(
        &mut self,
        from: &Path,
        to: &Path,
    ) -> Option<String> {
        let workspace = self.workspace.as_ref()?;
        let root = workspace.root.clone();
        let index = workspace.index.clone();
        let git = workspace.git.clone();
        let (root_text, old_text, new_text) = (
            root.to_string_lossy(),
            from.to_string_lossy(),
            to.to_string_lossy(),
        );
        let rename = BaseReferenceRename {
            workspace_path: &root_text,
            old_path: &old_text,
            new_path: &new_text,
        };
        let mut files = Vec::new();
        let mut failed = Vec::new();
        collect_base_files(&root, &mut files, &mut failed);
        for path in files {
            if let Some(tab_index) = self.tabs.iter().position(|tab| {
                tab.path().is_some_and(|open| same_path(open, &path))
                    && matches!(
                        tab.kind,
                        TabKind::Viewer {
                            content: Content::Base(_),
                            ..
                        }
                    )
            }) {
                let TabKind::Viewer {
                    content: Content::Base(state),
                    ..
                } = &mut self.tabs[tab_index].kind
                else {
                    unreachable!()
                };
                // 不要重新加载仍在编辑的文档：未保存字段和新记录以当前编辑器内容为准。
                if rename_base_document_references(&mut state.document, &rename) == 0 {
                    continue;
                }
                state.dirty = true;
                state.error.clear();
                if self.save_tab(tab_index) {
                    let _ = index.index_single_file(&path);
                } else {
                    failed.push(path);
                }
                continue;
            }
            let result = (|| -> Result<()> {
                let original = std::fs::read_to_string(&path)?;
                let Ok(mut document) = parse_base_document(&original) else {
                    return Ok(());
                };
                if rename_base_document_references(&mut document, &rename) == 0 {
                    return Ok(());
                }
                let updated = serialize_base_document(&document)?;
                if std::fs::read_to_string(&path)? != original {
                    bail!("引用文件同时被其它程序修改")
                }
                FileService::new().write_file_safe(&path, &updated)?;
                git.mark_write(path.clone());
                let _ = index.index_single_file(&path);
                Ok(())
            })();
            if result.is_err() {
                failed.push(path);
            }
        }
        failed.first().map(|path| {
            format!(
                "文件已移动，{} 处多维表格引用更新未能保存；本地草稿已保留：{}",
                failed.len(),
                path.display()
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mochi_core::base::*;
    use serde_json::json;
    use std::fs;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("mochi-base-rename-{}", create_id("test")));
            fs::create_dir_all(path.join("知识库/旧目录")).unwrap();
            Self(path)
        }
        fn shell(&self) -> Shell {
            let mut shell = Shell::new();
            shell.open_workspace(&self.0, || {}).unwrap();
            shell
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn document(path: &str) -> BaseDocument {
        let mut document = create_base_document();
        document
            .extra
            .insert("future".into(), json!({"preserved":true}));
        let table = &mut document.tables[0];
        table.fields[0].id = "title".into();
        let mut field = create_base_field(FieldType::Reference, "资料");
        field.id = "refs".into();
        table.fields.push(field);
        let link = format!(
            "mochi://open?path={}&label=alias&future=a%2fb",
            mochi_core::mochi_url::form_encode(path)
        );
        table.records.push(BaseRecord {
            id: "record-stable".into(),
            values: [
                ("title".into(), json!("已保存")),
                ("refs".into(), json!([link])),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        });
        table.views[0].filters.push(BaseFilter {
            field_id: "refs".into(),
            operator: FilterOperator::Equals,
            value: Some(json!([link])),
            ..Default::default()
        });
        document
    }
    fn write(path: &Path, document: &BaseDocument) {
        fs::write(path, serialize_base_document(document).unwrap()).unwrap();
    }
    fn state(shell: &mut Shell) -> &mut crate::ui::base_view::State {
        let TabKind::Viewer {
            content: Content::Base(state),
            ..
        } = &mut shell.active_mut().unwrap().kind
        else {
            panic!("base expected")
        };
        state
    }
    fn reference_path(document: &BaseDocument) -> String {
        parse_base_reference(
            document.tables[0].records[0].values["refs"][0]
                .as_str()
                .unwrap(),
        )
        .unwrap()
        .path
    }

    #[test]
    fn base_reference_rename_merges_dirty_draft_and_updates_closed_files_and_filters() {
        let fixture = Fixture::new();
        let old = fixture.0.join("知识库/旧目录");
        fs::write(old.join("笔记.md"), "note").unwrap();
        let inside = old.join("学习.mcb");
        let outside = fixture.0.join("知识库/汇总.mcb");
        let invalid = fixture.0.join("知识库/无效.mcb");
        let hidden = fixture.0.join(".ignored.mcb");
        let seed = document("知识库/旧目录/笔记.md");
        write(&inside, &seed);
        write(&outside, &seed);
        write(&hidden, &seed);
        fs::write(&invalid, "{ invalid }").unwrap();
        let hidden_raw = fs::read_to_string(&hidden).unwrap();
        let mut shell = fixture.shell();
        assert!(shell.open_file(&inside));
        let draft = state(&mut shell);
        draft.document.tables[0].records[0]
            .values
            .insert("title".into(), json!("未保存的新内容"));
        draft.document.tables[0].records.push(BaseRecord {
            id: "unsaved-record".into(),
            values: [
                ("title".into(), json!("新增草稿")),
                ("refs".into(), json!([])),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        });
        draft.dirty = true;
        let moved = shell.rename(&old, "新目录").unwrap();
        assert_eq!(
            shell.active().unwrap().path(),
            Some(moved.join("学习.mcb").as_path())
        );
        let live = state(&mut shell);
        assert!(!live.dirty);
        assert_eq!(
            live.document.tables[0].records[0].values["title"],
            json!("未保存的新内容")
        );
        assert_eq!(live.document.tables[0].records[1].id, "unsaved-record");
        assert_eq!(reference_path(&live.document), "知识库/新目录/笔记.md");
        let saved =
            parse_base_document(&fs::read_to_string(moved.join("学习.mcb")).unwrap()).unwrap();
        assert_eq!(saved, live.document);
        let closed = parse_base_document(&fs::read_to_string(outside).unwrap()).unwrap();
        assert_eq!(reference_path(&closed), "知识库/新目录/笔记.md");
        let filter = closed.tables[0].views[0].filters[0].value.as_ref().unwrap()[0]
            .as_str()
            .unwrap();
        assert_eq!(
            parse_base_reference(filter).unwrap().path,
            "知识库/新目录/笔记.md"
        );
        assert!(filter.ends_with("&label=alias&future=a%2fb"));
        assert_eq!(closed.extra, seed.extra);
        assert_eq!(fs::read_to_string(invalid).unwrap(), "{ invalid }");
        assert_eq!(fs::read_to_string(hidden).unwrap(), hidden_raw);
    }

    #[test]
    fn base_reference_rename_preserves_updated_draft_when_disk_has_conflicting_changes() {
        let fixture = Fixture::new();
        let old = fixture.0.join("知识库/旧目录/笔记.md");
        fs::write(&old, "note").unwrap();
        let base = fixture.0.join("知识库/学习.mcb");
        let seed = document("知识库/旧目录/笔记.md");
        write(&base, &seed);
        let mut shell = fixture.shell();
        assert!(shell.open_file(&base));
        state(&mut shell).document.tables[0].records[0]
            .values
            .insert("title".into(), json!("本地草稿"));
        state(&mut shell).dirty = true;
        let mut external = seed;
        external.tables[0].records[0]
            .values
            .insert("title".into(), json!("外部更新"));
        write(&base, &external);
        let disk = fs::read_to_string(&base).unwrap();
        shell.rename(&old, "新笔记.md").unwrap();
        assert_eq!(fs::read_to_string(&base).unwrap(), disk);
        assert!(state(&mut shell).dirty);
        assert_eq!(
            state(&mut shell).document.tables[0].records[0].values["title"],
            json!("本地草稿")
        );
        assert_eq!(
            reference_path(&state(&mut shell).document),
            "知识库/旧目录/新笔记.md"
        );
        assert!(shell.status.contains("引用更新未能保存"));
        assert!(shell.has_disk_conflict(&base));
    }

    #[test]
    fn base_reference_rename_runs_only_after_success_and_handles_external_moves() {
        let fixture = Fixture::new();
        let old = fixture.0.join("知识库/旧目录/笔记.md");
        let target = fixture.0.join("知识库/目标.md");
        fs::write(&old, "note").unwrap();
        fs::write(&target, "occupied").unwrap();
        let base = fixture.0.join("知识库/学习.mcb");
        write(&base, &document("知识库/旧目录/笔记.md"));
        let original = fs::read_to_string(&base).unwrap();
        let mut shell = fixture.shell();
        assert!(shell.move_path(&old, &target).is_err());
        assert_eq!(fs::read_to_string(&base).unwrap(), original);
        fs::remove_file(&target).unwrap();
        fs::rename(&old, &target).unwrap();
        shell.accept_external_rename(&old, &target);
        let saved = parse_base_document(&fs::read_to_string(base).unwrap()).unwrap();
        assert_eq!(reference_path(&saved), "知识库/目标.md");
    }

    #[cfg(windows)]
    #[test]
    fn base_reference_rename_matches_live_windows_tabs_with_different_path_case() {
        let fixture = Fixture::new();
        let old = fixture.0.join("知识库/旧目录/note.md");
        fs::write(&old, "note").unwrap();
        let base = fixture.0.join("知识库/learn.mcb");
        write(&base, &document("知识库/旧目录/note.md"));
        let mut shell = fixture.shell();
        assert!(shell.open_file(&base.with_file_name("LEARN.MCB")));
        state(&mut shell).document.tables[0].records[0]
            .values
            .insert("title".into(), json!("保留大写路径标签中的草稿"));
        state(&mut shell).dirty = true;
        shell.rename(&old, "renamed.md").unwrap();
        assert!(!state(&mut shell).dirty);
        let saved = parse_base_document(&fs::read_to_string(base).unwrap()).unwrap();
        assert_eq!(
            saved.tables[0].records[0].values["title"],
            json!("保留大写路径标签中的草稿")
        );
        assert_eq!(reference_path(&saved), "知识库/旧目录/renamed.md");
    }
}

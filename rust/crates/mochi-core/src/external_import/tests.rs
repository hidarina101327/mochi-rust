use super::*;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("mochi-import-{}", crate::paths::random_base36(12)));
        fs::create_dir_all(root.join("source/images")).unwrap();
        fs::create_dir_all(root.join("target")).unwrap();
        Self(root)
    }
    fn source(&self) -> PathBuf {
        self.0.join("source")
    }
    fn target(&self) -> PathBuf {
        self.0.join("target")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn imports_inline_reference_html_and_wiki_resources_without_changing_examples() {
    let f = Fixture::new();
    fs::write(f.source().join("images/图 (1).png"), b"image bytes").unwrap();
    fs::write(f.source().join("manual.pdf"), b"pdf bytes").unwrap();
    let markdown = concat!(
        "# 原始标题\r\n",
        "![中文](images/%E5%9B%BE%20(1).png \"标题\")\r\n",
        "![reference][pic]\r\n\r\n[pic]: <images/图 (1).png> 'Title'\r\n",
        "<img src='images/图 (1).png' width='240'>\r\n\r\n",
        "[附件](manual.pdf#page=2)\r\n![[images/图 (1).png|200]]\r\n",
        "`![example](missing.png)`\r\n```md\r\n![code](missing.png)\r\n```\r\n",
        "![远程](https://example.com/x.png) [锚点](#标题)\r\n"
    );
    let source = f.source().join("note.md");
    fs::write(&source, markdown).unwrap();
    let report = copy_paths(&[source.clone()], &f.target());
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert_eq!(report.imported.len(), 1);
    assert_eq!(report.resources, 2);
    assert_eq!(fs::read_to_string(&source).unwrap(), markdown);
    let imported = fs::read_to_string(&report.imported[0]).unwrap();
    assert!(imported.starts_with("# 原始标题\r\n"));
    assert!(imported.contains("`![example](missing.png)`"));
    assert!(imported.contains("![code](missing.png)"));
    assert!(imported.contains("https://example.com/x.png"));
    assert!(imported.contains("#page=2"));
    assert!(imported.contains("width='240'"));
    assert!(!imported.contains("images/"), "{imported}");
    // 即使整个源文件夹已删除，其中的资源仍然可以读取。
    fs::remove_dir_all(f.source()).unwrap();
    for (_, url) in references(&imported) {
        if let Some((path, _)) = local_reference(&url, &f.target()) {
            assert!(path.is_file(), "{url}: {}", path.display());
        }
    }
}

#[test]
fn nested_folders_collisions_missing_resources_and_self_copy_are_reported() {
    let f = Fixture::new();
    fs::create_dir_all(f.source().join("nested")).unwrap();
    fs::write(f.source().join("images/p.png"), b"png").unwrap();
    fs::write(
        f.source().join("nested/a.md"),
        "![ok](../images/p.png) ![missing](../lost.png)",
    )
    .unwrap();
    let first = copy_paths(&[f.source()], &f.target());
    assert_eq!(first.imported.len(), 1);
    assert_eq!(first.resources, 1);
    assert_eq!(first.warnings.len(), 1);
    assert!(first.warnings[0].contains("lost.png"));
    assert!(f.target().join("source/nested/assets").is_dir());
    let second = copy_paths(&[f.source()], &f.target());
    assert_eq!(second.imported[0].file_name().unwrap(), "source (1)");
    let rejected = copy_paths(&[f.source()], &f.source().join("nested"));
    assert!(rejected.imported.is_empty());
    assert!(rejected.warnings[0].contains("自身"));
}

#[test]
fn nested_link_images_and_escaped_paths_only_replace_destinations() {
    let f = Fixture::new();
    fs::write(f.source().join("images/a(b).png"), b"png").unwrap();
    fs::write(f.source().join("manual.pdf"), b"pdf").unwrap();
    let source = f.source().join("note.MD");
    let content = "[![same images/a(b).png](images/a\\(b\\).png)](manual.pdf)";
    fs::write(&source, content).unwrap();
    fs::write(f.target().join("note.MD"), "existing").unwrap();
    let report = copy_paths(&[source.clone(), source], &f.target());
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert_eq!(report.imported.len(), 1);
    assert_eq!(report.resources, 2);
    assert_eq!(
        fs::read_to_string(f.target().join("note.MD")).unwrap(),
        "existing"
    );
    let content = fs::read_to_string(&report.imported[0]).unwrap();
    assert!(content.starts_with("[![same images/a(b).png](./assets/"));
    assert!(content.contains(")](./assets/"));
}

#[test]
fn file_urls_absolute_paths_and_attachment_names_with_spaces_survive() {
    let f = Fixture::new();
    let asset = f.source().join("manual.pdf");
    fs::write(&asset, b"pdf").unwrap();
    let url = url::Url::from_file_path(&asset).unwrap();
    let source = f.source().join("note.md");
    fs::write(
        &source,
        format!(
            "[file]({url}) [absolute](<{}>)",
            asset.to_string_lossy().replace('\\', "/")
        ),
    )
    .unwrap();
    let report = copy_paths(&[source], &f.target());
    assert_eq!(report.resources, 1);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
}

#[test]
fn obsidian_report_imports_all_five_images_as_standalone_renderable_markdown() {
    let f = Fixture::new();
    let content = include_str!("fixtures/obsidian-images.md");
    fs::create_dir_all(f.source().join("assets")).unwrap();
    for name in [
        "ulsjvejlocb5aqql",
        "y9vz0z95tl41zoaw",
        "w2uliiza9mnu1rb7",
        "objps18zdg8i7ncv",
        "g697pgxoz4qirtzn",
    ] {
        fs::write(f.source().join(format!("assets/import-{name}.png")), name).unwrap();
    }
    let source = f.source().join("测试报告.md");
    fs::write(&source, content).unwrap();
    let report = copy_paths(&[source.clone()], &f.target());
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert_eq!(report.resources, 5);
    let imported = fs::read_to_string(&report.imported[0]).unwrap();
    assert!(!imported.contains("![["), "{imported}");
    assert!(imported.contains("修改**版本历史**中的提交作者名"));
    let images: Vec<_> = Parser::new_ext(&imported, Options::all())
        .into_offset_iter()
        .filter_map(|(event, span)| {
            if let Event::Start(Tag::Image {
                link_type: LinkType::Inline,
                dest_url,
                ..
            }) = event
            {
                Some((span, dest_url.to_string()))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(images.len(), 5);
    for (span, url) in images {
        assert!(imported[..span.start].ends_with('\n'));
        assert!(imported[span.end..].starts_with('\n'));
        let (path, _) = local_reference(&url, &f.target()).unwrap();
        assert!(path.is_file(), "{url}");
    }
    assert_eq!(fs::read_to_string(source).unwrap(), content);
}

#[test]
fn obsidian_aliases_and_escaped_filenames_share_copied_resources_and_warn_when_missing() {
    let f = Fixture::new();
    fs::write(f.source().join("images/图 (1).png"), b"png").unwrap();
    let note = f.source().join("aliases.md");
    fs::write(&note, "![[images/图 \\(1\\)\\.png|说明]]\n![[images/图 (1).png|240]]\n![[lost.png]]\n`![[code.png]]`\n").unwrap();
    let report = copy_paths(&[note], &f.target());
    assert_eq!(report.resources, 1);
    assert_eq!(report.warnings.len(), 1);
    assert!(report.warnings[0].contains("lost.png"));
    let imported = fs::read_to_string(&report.imported[0]).unwrap();
    assert!(imported.contains("![说明](<./assets/"));
    assert!(imported.contains("width=\"240\""));
    assert!(imported.contains("`![[code.png]]`"));
    let urls: Vec<_> = references(&imported)
        .into_iter()
        .map(|(_, url)| url)
        .filter(|url| url.starts_with("./assets/"))
        .collect();
    assert_eq!(urls.len(), 2);
    assert_eq!(urls[0], urls[1]);
}

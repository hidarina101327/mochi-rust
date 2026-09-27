//! 仅处理 .md / .mc，.mcb 走 base。写入前核对完整原文，拒绝覆盖外部修改。

use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{bail, Context, Result};
use mochi_blocks::model::{hash_content, BlockId, BlockKind, BlockRecord, Document, SourceSpan};
use serde::{Deserialize, Serialize};

/// 转换备份是源文件的同目录兄弟文件，后缀为 `.blocks-backup`。
/// 刻意不用核心文件服务那个临时性的 `.backup` 后缀：普通保存
/// 不能把「显式还原」还要用的转换前字节给消费掉。
pub const CONVERSION_BACKUP_SUFFIX: &str = ".blocks-backup";

/// 完全在内存里准备的源码转换结果。
///
/// `source` 是调用方给来的原始 UTF-8 文本，既充当乐观并发基准，
/// 也是 [`convert_file`] 会复制到 [`backup_path`](Self::backup_path) 的字节。
/// `converted_source` 是同样的用户内容，只是在必要处补上了持久化 ID。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversion {
    pub path: PathBuf,
    pub source: String,
    pub converted_source: String,
    pub document: Document,
    pub backup_path: PathBuf,
    pub source_hash: String,
    pub converted_hash: String,
    /// 转换后的源码是否与 `source` 逐字节不同。
    pub changed: bool,
    /// [`prepare_conversion`] 返回时为 `false`，只有 [`convert_file`]
    /// 写盘成功后才为 `true`。检查结果时就不用猜到底动没动文件系统。
    pub written: bool,
}

impl Conversion {
    /// 作为乐观并发基准的那份原始源码。
    pub fn source(&self) -> &str {
        &self.source
    }

    /// 补齐所有新分配 ID 之后的源码。
    pub fn converted(&self) -> &str {
        &self.converted_source
    }

    /// 给把产出叫「渲染结果」的调用方留的别名。
    pub fn rendered_source(&self) -> &str {
        &self.converted_source
    }

    /// 给习惯用「before」说法的调用方留的别名。
    pub fn before_source(&self) -> &str {
        &self.source
    }

    /// 给习惯用「after」说法的调用方留的别名。
    pub fn after_source(&self) -> &str {
        &self.converted_source
    }

    /// 要还原的原始字节；解析器要求 MD/MC 源码必须是合法 UTF-8，
    /// 所以这里以 UTF-8 表示。
    pub fn backup_source(&self) -> &str {
        &self.source
    }

    pub fn backup(&self) -> &Path {
        &self.backup_path
    }
}

/// 显式还原的结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Restoration {
    pub path: PathBuf,
    pub source: String,
    pub document: Document,
    pub backup_path: PathBuf,
    pub restored: bool,
}

/// 面向文档/AI 调用方的公开块 DTO。`source_span` 是相对所属
/// `Document` 源码的 UTF-8 字节区间。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockOutput {
    pub id: BlockId,
    pub kind: BlockKind,
    pub content: String,
    pub hash: String,
    pub source_span: SourceSpan,
}

/// 老调用方把这类记录叫「块摘要 / 块信息」，这两个名字都保留。
pub type BlockSummary = BlockOutput;
pub type BlockInfo = BlockOutput;

/// 返回转换与还原共用的同目录备份路径。
pub fn conversion_backup_path(path: &Path) -> PathBuf {
    append_suffix(path, CONVERSION_BACKUP_SUFFIX)
}

/// 直接解析给定的 MD/MC 源码，不碰磁盘。
pub fn document_from_source(path: &Path, source: &str) -> Result<Document> {
    ensure_supported_document(path)?;
    Document::import(document_id(path), source)
        .with_context(|| format!("parse block document {}", path.display()))
}

/// 只准备转换结果，不写任何东西。
pub fn prepare_conversion(path: &Path, source: &str) -> Result<Conversion> {
    let source_document = document_from_source(path, source)?;
    let converted_source = source_document
        .try_render_with_ids()
        .with_context(|| format!("prepare persisted block IDs for {}", path.display()))?;
    // 暴露与 `converted_source` 对应的文档。对「先在脏编辑器缓冲上准备、
    // 再拿返回的记录去保存或路由块操作」的调用方很关键：所有 ID 都已
    // 持久化，且下一次解析时保持不变。
    let document = if converted_source == source {
        source_document
    } else {
        document_from_source(path, &converted_source)
            .with_context(|| format!("reopen converted block document {}", path.display()))?
    };
    let backup_path = conversion_backup_path(path);
    Ok(Conversion {
        path: path.to_path_buf(),
        source: source.to_owned(),
        source_hash: hash_content(source),
        converted_hash: hash_content(&converted_source),
        changed: source != converted_source,
        converted_source,
        document,
        backup_path,
        written: false,
    })
}

/// 先核对当前文件的字节与 `expected` 完全一致，再执行转换。
/// 手头只有修订令牌的调用方也可以传 SHA-256 内容哈希；但优先
/// 约定仍然是传完整源码——行尾和 BOM 的变化只有它防得住。
///
/// 转换后的源码原子写入之前，原始字节会先复制到 `.blocks-backup`。
/// 已存在的备份绝不覆盖，除非里面的字节与当前内容完全相同；
/// 这样先前一次失败的转换也不会丢掉恢复点。
pub fn convert_file(path: &Path, expected: &str) -> Result<Conversion> {
    let _ = (path, expected);
    bail!("文档块转换已退役：Mochi 不会向源文件写入 mochi:block 标识")
}

/// 在当前文件仍与 `expected_current` 一致的前提下，从 `.blocks-backup`
/// 还原转换前的原始字节。用户改过、删过、文件不是合法 UTF-8、
/// 或备份缺失都算硬冲突，所有文件一律不动。
pub fn restore_file(path: &Path, expected_current: &str) -> Result<Restoration> {
    ensure_supported_document(path)?;
    let current = read_utf8(path)?;
    ensure_expected(path, &current, expected_current)?;

    let backup_path = conversion_backup_path(path);
    let original_bytes = fs::read(&backup_path)
        .with_context(|| format!("read conversion backup {}", backup_path.display()))?;
    let original_source = String::from_utf8(original_bytes.clone())
        .with_context(|| format!("conversion backup is not UTF-8: {}", backup_path.display()))?;

    // 动文件之前先校验备份。既能发现被手动替换过的 `.blocks-backup`，
    // 也保证还原不会把坏掉的块语法写进正式的 MD/MC 文档。
    let document = document_from_source(path, &original_source)
        .with_context(|| format!("validate conversion backup {}", backup_path.display()))?;

    atomic_replace(path, current.as_bytes(), &original_bytes)
        .with_context(|| format!("restore document {}", path.display()))?;
    fs::remove_file(&backup_path).with_context(|| {
        format!(
            "remove consumed conversion backup {}",
            backup_path.display()
        )
    })?;

    Ok(Restoration {
        path: path.to_path_buf(),
        source: original_source,
        document,
        backup_path,
        restored: true,
    })
}

/// 把模型记录转换成稳定的公开块输出。
pub fn block_output(block: &BlockRecord) -> BlockOutput {
    BlockOutput {
        id: block.id.clone(),
        kind: block.kind.clone(),
        content: block.content.clone(),
        hash: block.content_hash(),
        source_span: block.source_span,
    }
}

/// 按源码顺序列出全部块。
pub fn list_blocks(document: &Document) -> Vec<BlockOutput> {
    document.blocks().iter().map(block_output).collect()
}

/// 直接解析给定源码并列出块，不写盘。
pub fn list_blocks_from_source(path: &Path, source: &str) -> Result<Vec<BlockOutput>> {
    Ok(list_blocks(&document_from_source(path, source)?))
}

pub fn read_block(document: &Document, id: impl AsRef<str>) -> Result<BlockOutput> {
    let block = document.find_block(id.as_ref())?;
    Ok(block_output(block))
}

/// 面向 API/AI 调用方的别名，强调按 ID 查找。
pub fn get_block(document: &Document, id: impl AsRef<str>) -> Result<BlockOutput> {
    read_block(document, id)
}

/// 给暴露 `block_by_id` 操作的宿主环境留的显式命名别名。
pub fn block_by_id(document: &Document, id: impl AsRef<str>) -> Result<BlockOutput> {
    read_block(document, id)
}

/// 解析源码并按 ID 读取一个块，不写盘。
pub fn read_block_from_source(
    path: &Path,
    source: &str,
    id: impl AsRef<str>,
) -> Result<BlockOutput> {
    let document = document_from_source(path, source)?;
    read_block(&document, id)
}

/// 用块当前的内容哈希做乐观并发令牌，精确替换这一个块。
/// 模型层会校验 ID 唯一、拒绝标记注入、重新解析替换内容，
/// 并保留所有无关源码字节和原有行尾风格。
pub fn replace_block(
    document: &Document,
    id: impl AsRef<str>,
    before_hash: &str,
    replacement: &str,
) -> Result<Document> {
    let block = document.find_block(id.as_ref())?;
    let actual_hash = block.content_hash();
    if actual_hash != before_hash {
        bail!(
            "stale block `{}`: expected content hash {}, found {}",
            block.id,
            before_hash,
            actual_hash
        );
    }
    document.replace_block(&block.id, &block.content, replacement)
}

/// 从源码字符串出发、带哈希校验的替换。
pub fn replace_block_in_source(
    path: &Path,
    source: &str,
    id: impl AsRef<str>,
    before_hash: &str,
    replacement: &str,
) -> Result<Document> {
    let document = document_from_source(path, source)?;
    replace_block(&document, id, before_hash, replacement)
}

/// 返回基于路径的稳定文档标识。ID 本身记录在源码标记里；
/// 这个值只是诊断/索引用的归属标签。
fn document_id(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn ensure_supported_document(path: &Path) -> Result<()> {
    let extension = path.extension().and_then(OsStr::to_str).unwrap_or_default();
    if extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("mc") {
        Ok(())
    } else {
        bail!(
            "block persistence only supports .md and .mc documents: {}",
            path.display()
        )
    }
}

fn read_utf8(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("read document {}", path.display()))?;
    String::from_utf8(bytes).with_context(|| format!("document is not UTF-8: {}", path.display()))
}

fn ensure_expected(path: &Path, current: &str, expected: &str) -> Result<()> {
    if current == expected || hash_content(current) == expected {
        return Ok(());
    }
    bail!(
        "document changed on disk: {} (expected exact source or hash {})",
        path.display(),
        hash_content(expected)
    )
}

fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    PathBuf::from(format!("{}{}", path.display(), suffix))
}

fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("create parent directory {}", parent.display()))?;
    }
    Ok(())
}

fn next_sibling(path: &Path, suffix: &str) -> PathBuf {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    PathBuf::from(format!(
        "{}{}-{}-{}",
        path.display(),
        suffix,
        std::process::id(),
        sequence
    ))
}

/// 新建文件，绝不覆盖已有目标。目标以 `create_new` 打开：
/// 并发转换若抢先写好备份，本次调用会失败，而对方的恢复点
/// 一字不动地保留下来。
#[cfg(test)]
fn write_new_file(path: &Path, bytes: &[u8]) -> Result<()> {
    ensure_parent(path)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create new file {}", path.display()))?;
    let result = (|| -> std::io::Result<()> {
        file.write_all(bytes)?;
        file.sync_all()
    })();
    drop(file);
    if result.is_err() {
        // 打开时目标还不存在。尽力清理一下，避免失败的首写留下一个
        // 似是而非的「恢复点」；本来就存在的目标从未被打开或改动。
        let _ = fs::remove_file(path);
    }
    result.with_context(|| format!("write new file {}", path.display()))
}

/// 通过同目录临时文件（写完即 fsync）加上最后一次乐观并发校验来安装字节。
/// 目标文件永远不会被挪走：最后一次读到的内容若与 `expected_current`
/// 不符，或 rename 本身失败，原路径原样保留，临时文件尽力删除。
fn atomic_replace(path: &Path, expected_current: &[u8], bytes: &[u8]) -> Result<()> {
    ensure_parent(path)?;
    let temporary = next_sibling(path, ".tmp");
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);

        // 第一次读发生在解析/建备份之前。这里最后再读一次，把普通保存
        // 还残留的竞争窗口关掉，且全程不会动用户当前路径下的文件。
        let current = fs::read(path)
            .with_context(|| format!("recheck current document {}", path.display()))?;
        if current != expected_current {
            bail!(
                "document changed on disk before replacement: {}",
                path.display()
            );
        }

        // 在支持的本地文件系统（包括 Windows）上，Rust 的文件系统 rename
        // 都是原子替换目标。别先把现用目标挪走再模拟替换：那样会留下
        // 一个崩溃窗口，文档路径会短暂消失。
        fs::rename(&temporary, path)
            .with_context(|| format!("rename replacement into {}", path.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.with_context(|| format!("atomically replace {}", path.display()))
}

#[cfg(test)]
fn ensure_backup(path: &Path, backup_path: &Path, bytes: &[u8]) -> Result<()> {
    if backup_path.exists() {
        let existing = fs::read(backup_path).with_context(|| {
            format!("read existing conversion backup {}", backup_path.display())
        })?;
        if existing == bytes {
            return Ok(());
        }
        bail!(
            "conversion backup already contains different bytes: {} (source: {})",
            backup_path.display(),
            path.display()
        );
    }
    write_new_file(backup_path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering as TestOrdering};

    static TEST_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let sequence = TEST_SEQUENCE.fetch_add(1, TestOrdering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "mochi-document-blocks-{}-{sequence}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn conversion_is_pure_until_explicit_file_call() {
        let temp = TempDir::new();
        let path = temp.path("中文.md");
        let source = "# 标题🙂\r\n\r\n正文\r\n";
        let conversion = prepare_conversion(&path, source).unwrap();
        assert!(conversion.changed);
        assert!(!conversion.written);
        assert_eq!(conversion.source, source);
        assert!(conversion
            .converted_source
            .contains("<!-- mochi:block block_"));
        assert!(!path.exists());
        assert!(!conversion.backup_path.exists());
    }

    #[test]
    fn final_expected_check_rejects_stale_source_without_touching_original() {
        let temp = TempDir::new();
        let path = temp.path("stale.md");
        let original = "当前正文\r\n".as_bytes();
        fs::write(&path, original).unwrap();

        let error =
            atomic_replace(&path, "已经过时".as_bytes(), "替换正文".as_bytes()).unwrap_err();
        assert!(format!("{error:#}").contains("changed on disk"));
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    #[test]
    fn existing_conversion_backup_is_never_overwritten() {
        let temp = TempDir::new();
        let path = temp.path("backup.md");
        let backup = conversion_backup_path(&path);
        let old_backup = "更早的恢复点\r\n".as_bytes();
        fs::write(&path, "当前正文\r\n".as_bytes()).unwrap();
        fs::write(&backup, old_backup).unwrap();

        assert!(write_new_file(&backup, "不应覆盖\r\n".as_bytes()).is_err());
        assert_eq!(fs::read(&backup).unwrap(), old_backup);
        assert!(ensure_backup(&path, &backup, "新的源\r\n".as_bytes()).is_err());
        assert_eq!(fs::read(&backup).unwrap(), old_backup);
    }

    #[test]
    #[cfg(windows)]
    fn locked_document_replacement_fails_with_original_bytes_intact() {
        use std::os::windows::fs::OpenOptionsExt;

        let temp = TempDir::new();
        let path = temp.path("locked.md");
        let original = "锁定时仍应保留的正文\r\n".as_bytes();
        fs::write(&path, original).unwrap();
        // 允许读写但拒绝删除共享：Windows 上最后一次 rename 会因此失败，
        // 而重读当前内容不受影响。
        let lock = OpenOptions::new()
            .read(true)
            .share_mode(3)
            .open(&path)
            .unwrap();

        assert!(atomic_replace(&path, original, "替换正文\r\n".as_bytes()).is_err());
        drop(lock);
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    #[test]
    fn conversion_is_retired_and_leaves_source_untouched() {
        let temp = TempDir::new();
        let path = temp.path("note.md");
        let source = "# 标题\r\n\r\n中文段落\r\n\r\n中文段落\r\n";
        fs::write(&path, source.as_bytes()).unwrap();
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        assert!(convert_file(&path, source).is_err());
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        assert!(!conversion_backup_path(&path).exists());
    }

    #[test]
    fn block_output_has_hash_span_and_lookup_rejects_stale_hash() {
        let path = Path::new("sample.mc");
        let document =
            document_from_source(path, "第一块\n\n| a | b |\n|---|---|\n| 中 | 文 |\n").unwrap();
        let blocks = list_blocks(&document);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].hash, hash_content(&blocks[0].content));
        assert_eq!(blocks[0].source_span.start, 0);
        assert_eq!(
            get_block(&document, blocks[0].id.as_str()).unwrap(),
            blocks[0]
        );
        assert!(replace_block(&document, &blocks[0].id, "stale", "新块").is_err());
    }

    #[test]
    fn mc_containers_math_html_table_and_object_card_remain_whole_blocks() {
        let path = Path::new("rich.mc");
        let source = concat!(
            "<details>\n",
            "<summary>对象卡容器</summary>\n",
            "<table><tr><td>中文🙂</td></tr></table>\n",
            "</details>\n\n",
            "$$x + y$$\n\n",
            "mochi://open?path=知识库%2F目标.md&view=card\n"
        );
        let document = document_from_source(path, source).unwrap();
        assert!(document.blocks().iter().any(|block| {
            matches!(block.kind, BlockKind::Html) && block.raw_content.contains("<details>")
        }));
        assert!(document.blocks().iter().any(|block| {
            matches!(block.kind, BlockKind::Formula { display: true })
                && block.content.contains("x + y")
        }));
        assert!(document.blocks().iter().any(|block| {
            block
                .raw_content
                .contains("mochi://open?path=知识库%2F目标.md")
        }));
        let saved = document.save().unwrap();
        let reopened = document_from_source(path, &saved).unwrap();
        assert_eq!(
            document
                .blocks()
                .iter()
                .map(|block| &block.kind)
                .collect::<Vec<_>>(),
            reopened
                .blocks()
                .iter()
                .map(|block| &block.kind)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            document
                .blocks()
                .iter()
                .map(|block| &block.content)
                .collect::<Vec<_>>(),
            reopened
                .blocks()
                .iter()
                .map(|block| &block.content)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn frontmatter_and_code_metadata_stay_in_their_own_boundaries() {
        let path = Path::new("containers.mc");
        let source = concat!(
            "---\r\n",
            "title: 元数据\r\n",
            "tags: [中文]\r\n",
            "---\r\n\r\n",
            ":::mochi-highlight color=\"blue\" title=\"提示\"\r\n",
            "**正文**\r\n",
            "<details open>\r\n",
            "<summary>内层</summary>\r\n",
            "内层内容\r\n",
            "</details>\r\n",
            ":::\r\n\r\n",
            "<!-- mochi-code-block title=\"Demo\" collapsed=\"false\" -->\r\n",
            "```rs\r\n",
            "let marker = \"<!-- mochi:block block_bad -->\";\r\n",
            "```\r\n"
        );
        let document = document_from_source(path, source).unwrap();
        assert!(document
            .blocks()
            .iter()
            .all(|block| !block.content.contains("title: 元数据")));
        assert_eq!(
            document
                .blocks()
                .iter()
                .filter(|block| matches!(block.kind, BlockKind::Html))
                .count(),
            2,
            "highlight/details should each be one addressable container"
        );
        let code = document
            .blocks()
            .iter()
            .find(|block| matches!(block.kind, BlockKind::FencedCode { .. }))
            .expect("fenced code block");
        assert!(code.raw_content.contains("mochi-code-block"));
        assert!(code.raw_content.contains("collapsed=\"false\""));
        assert!(code.content.contains("block_bad"));
        assert_eq!(
            document
                .blocks()
                .iter()
                .filter(|block| block.raw_content.contains("mochi-code-block"))
                .count(),
            1,
            "code-card metadata is part of the fenced block, not a phantom HTML block"
        );
        let saved = document.save().unwrap();
        assert!(saved.contains(":::mochi-highlight"));
        assert!(saved.contains("</details>"));
        assert!(saved.contains("mochi-code-block"));
        assert!(
            saved.find("<!-- mochi:block").unwrap() < saved.find("<!-- mochi-code-block").unwrap(),
            "the block marker must precede code-card metadata so metadata stays fence-adjacent"
        );
        let reopened = document_from_source(path, &saved).unwrap();
        assert_eq!(
            document
                .blocks()
                .iter()
                .map(|block| &block.id)
                .collect::<Vec<_>>(),
            reopened
                .blocks()
                .iter()
                .map(|block| &block.id)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn invalid_duplicate_marker_and_mcb_are_safe_errors() {
        let path = Path::new("bad.md");
        let marker = "<!-- mochi:block block_00000000-0000-4000-8000-000000000001 -->";
        assert!(document_from_source(path, &format!("{marker}\n甲\n\n{marker}\n乙\n")).is_err());
        assert!(document_from_source(Path::new("base.mcb"), "{}").is_err());
    }
}

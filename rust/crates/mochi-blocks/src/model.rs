//! 导入和编辑不写磁盘；块 ID 随机生成，不由内容、位置或哈希派生。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::str::FromStr;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[path = "syntax.rs"]
mod syntax;

pub use syntax::{
    code_ranges, line_ending, line_span, line_start, mask_ranges, parse_candidates, scan_markers,
    scan_references, Candidate, CandidateKind, Marker, RawReference, BLOCK_MARKER_PREFIX,
    BLOCK_MARKER_SUFFIX,
};

/// 源字符串中的 UTF-8 字节范围，结束位置不包含在内。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceSpan {
    pub start: usize,
    pub end: usize,
}

impl SourceSpan {
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub const fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub const fn is_empty(self) -> bool {
        self.start >= self.end
    }

    pub const fn contains(self, other: SourceSpan) -> bool {
        self.start <= other.start && other.end <= self.end
    }

    pub const fn overlaps(self, other: SourceSpan) -> bool {
        self.start < other.end && other.start < self.end
    }
}

/// 持久化的块标识。文本形式特意写得明确，便于用户在 Markdown 注释中识别：`block_<UUID>`。
///
/// UUID 形式的后缀使用系统随机数生成，并设置 v4 和 RFC 4122 的对应位。
/// 输入可以包含大写十六进制字母，但会统一转为小写，避免大小写不同的重复 ID 蒙混过关。
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct BlockId(String);

impl<'de> Deserialize<'de> for BlockId {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

impl BlockId {
    pub const PREFIX: &'static str = "block_";

    /// 生成新的随机稳定 ID。没有标注的块只通过这里获取 ID，内容哈希不会用作 ID。
    pub fn generate() -> Result<Self> {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).context("generate random block ID")?;
        // 设置 UUID v4 和 RFC 4122 的对应位。
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        let hex = bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        Self::parse(format!(
            "block_{}-{}-{}-{}-{}",
            &hex[0..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..32]
        ))
    }

    /// 供习惯用 `new` 创建 ID 的调用方使用的别名。
    pub fn new() -> Result<Self> {
        Self::generate()
    }

    /// 解析 `block_<UUID>` 并统一格式。
    pub fn parse(value: impl AsRef<str>) -> Result<Self> {
        let value = value.as_ref();
        let Some(uuid) = value.strip_prefix(Self::PREFIX) else {
            bail!("invalid block ID `{value}`: expected block_<UUID>");
        };
        if !valid_uuid(uuid) {
            bail!("invalid block ID `{value}`: UUID suffix is malformed");
        }
        Ok(Self(format!(
            "{}{}",
            Self::PREFIX,
            uuid.to_ascii_lowercase()
        )))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Debug for BlockId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("BlockId").field(&self.0).finish()
    }
}

impl fmt::Display for BlockId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl AsRef<str> for BlockId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl FromStr for BlockId {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl TryFrom<&str> for BlockId {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

fn valid_uuid(value: &str) -> bool {
    if value.len() != 36 {
        return false;
    }
    value.bytes().enumerate().all(|(index, byte)| {
        if matches!(index, 8 | 13 | 18 | 23) {
            byte == b'-'
        } else {
            byte.is_ascii_hexdigit()
        }
    })
}

/// 可编辑块的语义类型。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum BlockKind {
    Heading { level: u8 },
    Paragraph,
    Todo { checked: bool },
    ListItem,
    FencedCode { language: Option<String> },
    IndentedCode,
    Table,
    Formula { display: bool },
    Html,
    ThematicBreak,
}

/// 所属块的源码范围内，对另一个块 ID 的引用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockReference {
    pub target: BlockId,
    pub span: SourceSpan,
    pub raw: String,
}

/// 可单独定位的 Markdown 块。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockRecord {
    pub id: BlockId,
    pub kind: BlockKind,
    /// 去掉持久化标记注释后的可见内容。
    pub content: String,
    /// 语义块对应的原始源码片段，包含行内或嵌套的标记注释，用于按字节准确定位问题。
    pub raw_content: String,
    pub source_span: SourceSpan,
    /// 源文件中已有的块标记。内存中新生成的 ID 在显式保存或渲染前没有标记，值为 `None`。
    pub marker_span: Option<SourceSpan>,
    pub references: Vec<BlockReference>,
}

impl BlockRecord {
    pub fn span(&self) -> SourceSpan {
        self.source_span
    }

    pub fn content_hash(&self) -> String {
        hash_content(&self.content)
    }

    pub fn is_todo(&self) -> bool {
        matches!(self.kind, BlockKind::Todo { .. })
    }
}

/// 解析后的 Markdown 文档。源码始终保留导入时或显式编辑后的原样；
/// 解析过程不会把标记注释写回用户文件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Document {
    pub document_id: String,
    pub source: String,
    pub blocks: Vec<BlockRecord>,
}

/// 公开的 `Document` serde 实现使用的数据格式。单独定义此结构，可以避免 serde 派生代码
/// 构造出源码范围、标记归属或引用与实际 Markdown 解析结果不一致的文档。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct DocumentWire {
    document_id: String,
    source: String,
    blocks: Vec<BlockRecord>,
}

impl<'de> Deserialize<'de> for Document {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = DocumentWire::deserialize(deserializer)?;
        let document = Self {
            document_id: wire.document_id,
            source: wire.source,
            blocks: wire.blocks,
        };
        validate_document_records(&document).map_err(serde::de::Error::custom)?;
        Ok(document)
    }
}

impl Document {
    /// 解析源码并分配已有的标记 ID。没有标记的块只会在内存中获得随机 ID；
    /// 调用方需要执行 `render_with_ids` 或 `save` 才能将这些 ID 持久化。
    pub fn import(document_id: impl Into<String>, source: impl Into<String>) -> Result<Self> {
        let document_id = document_id.into();
        let source = source.into();
        let candidates = syntax::parse_candidates(&source)?;
        let markers = syntax::scan_markers(&source)?;
        let marker_ids = parse_marker_ids(&markers)?;
        let assignments = assign_markers(&source, &candidates, &markers)?;
        let raw_references = syntax::scan_references(&source)?;
        let references = parse_references(&raw_references)?;

        let mut blocks = Vec::with_capacity(candidates.len());
        for (candidate_index, candidate) in candidates.iter().enumerate() {
            let marker_index = assignments[candidate_index];
            let id = match marker_index {
                Some(index) => marker_ids[index].0.clone(),
                None => BlockId::generate()?,
            };
            let raw_content = source
                .get(candidate.span.start..candidate.span.end)
                .ok_or_else(|| anyhow!("invalid UTF-8 parser span {:?}", candidate.span))?
                .to_owned();
            let content = visible_content(&source, candidate.span, &markers);
            let kind = block_kind(&candidate.kind);
            let block_references = references
                .iter()
                .filter(|reference| candidate.span.contains(reference.span))
                .filter(|reference| {
                    // 嵌套项中的引用归嵌套项所有，而非外层项；每个引用只保留在最小的候选块上。
                    smallest_candidate_for_span(&candidates, reference.span)
                        == Some(candidate_index)
                })
                .cloned()
                .collect();
            blocks.push(BlockRecord {
                id,
                kind,
                content,
                raw_content,
                source_span: candidate.span,
                marker_span: marker_index.map(|index| markers[index].span),
                references: block_references,
            });
        }

        // 候选块的顺序应当固定；在这里检查，可及时发现后续解析器改动造成的替换歧义。
        ensure_unique_block_ids(&blocks)?;
        Ok(Self {
            document_id,
            source,
            blocks,
        })
    }

    pub fn parse(document_id: impl Into<String>, source: impl Into<String>) -> Result<Self> {
        Self::import(document_id, source)
    }

    /// 根据 Markdown 源码及解析器生成的元数据校验文档。公开的 serde 解码也会调用此方法，
    /// 防止调用方伪造源码范围、块类型、标记归属或引用，构造出看似可信的 `Document`。
    pub fn validate(&self) -> Result<()> {
        validate_document_records(self)
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn blocks(&self) -> &[BlockRecord] {
        &self.blocks
    }

    pub fn block(&self, id: &BlockId) -> Option<&BlockRecord> {
        self.blocks.iter().find(|block| &block.id == id)
    }

    pub fn find_block(&self, id: impl AsRef<str>) -> Result<&BlockRecord> {
        let id = BlockId::parse(id.as_ref())?;
        self.block(&id).ok_or_else(|| {
            anyhow!(
                "block `{id}` is not present in document `{}`",
                self.document_id
            )
        })
    }

    pub fn content_hash(&self) -> String {
        hash_content(&self.source)
    }

    pub fn has_persisted_ids(&self) -> bool {
        self.blocks.iter().all(|block| block.marker_span.is_some())
    }

    /// 为导入时没有标记的块生成规范的标记行。已有源码和标记注释保持原样。
    /// 生成结果会重新导入检查；如果插入标记改变了 Markdown 结构，就返回错误，
    /// 不会悄悄重新分配 ID。
    pub fn render_with_ids(&self) -> String {
        self.try_render_with_ids()
            .expect("persisted block marker insertion changed Markdown structure")
    }

    /// [`Document::render_with_ids`] 的可返回错误版本。上面的简便方法适合已持有成功导入文档的
    /// UI 代码；存储适配器应调用这里，让持久化校验错误正常返回，而不是触发 panic。
    pub fn try_render_with_ids(&self) -> Result<String> {
        let mut insertions = Vec::new();
        for block in &self.blocks {
            if block.marker_span.is_some() {
                continue;
            }
            insertions.push(marker_insertion(&self.source, block, &block.id));
        }

        insertions.sort_by_key(|(offset, _)| *offset);
        for pair in insertions.windows(2) {
            if pair[0].0 == pair[1].0 {
                bail!(
                    "cannot persist IDs for overlapping blocks at byte {}",
                    pair[0].0
                );
            }
        }

        let mut rendered = self.source.clone();
        for (offset, marker) in insertions.into_iter().rev() {
            rendered.insert_str(offset, &marker);
        }
        let reopened = Self::import(self.document_id.clone(), rendered.clone())
            .context("validate persisted block markers")?;
        compare_id_kinds(self, &reopened)?;
        Ok(rendered)
    }

    /// 显式生成用于持久化的内容。此方法不操作文件系统；路径和写入方式由调用方决定。
    pub fn save(&self) -> Result<String> {
        self.try_render_with_ids()
    }

    /// 序列化精简的原始数据快照。快照包含带标记的源码，重新打开时不会再生成 ID。
    pub fn to_json(&self) -> Result<String> {
        let source = self.try_render_with_ids()?;
        let canonical = Self::import(self.document_id.clone(), source.clone())?;
        let snapshot = PersistedDocument {
            schema_version: 1,
            document_id: self.document_id.clone(),
            source,
            blocks: canonical.blocks,
        };
        serde_json::to_string(&snapshot).context("serialize block document")
    }

    pub fn from_json(value: &str) -> Result<Self> {
        let snapshot: PersistedDocument =
            serde_json::from_str(value).context("deserialize block document")?;
        if snapshot.schema_version != 1 {
            bail!(
                "unsupported block document schema version {}",
                snapshot.schema_version
            );
        }
        let document = Self::import(snapshot.document_id, snapshot.source)?;
        if document.blocks != snapshot.blocks {
            bail!("persisted block metadata does not match persisted source");
        }
        Ok(document)
    }

    /// 只替换一个块。替换内容必须能解析为一个语义块；若要将一个块拆成多个，
    /// 调用方应选择 [`ReplacementStrategy::Split`]。
    pub fn replace_block(
        &self,
        id: &BlockId,
        before_content: &str,
        replacement: &str,
    ) -> Result<Self> {
        self.replace_block_with_strategy(
            id,
            before_content,
            replacement,
            ReplacementStrategy::KeepId,
        )
    }

    pub fn replace_block_in_place(
        &mut self,
        id: &BlockId,
        before_content: &str,
        replacement: &str,
    ) -> Result<()> {
        *self = self.replace_block(id, before_content, replacement)?;
        Ok(())
    }

    pub fn replace_block_with_strategy(
        &self,
        id: &BlockId,
        before_content: &str,
        replacement: &str,
        strategy: ReplacementStrategy,
    ) -> Result<Self> {
        // 编辑后会重新解析全部源码。解析前须先持久化所有内存 ID；否则目标块虽保留标记，
        // 其他未标记的块却会被分配新的随机 ID。
        if !self.has_persisted_ids() {
            self.require_block_index(id)?;
            let persisted = Self::import(self.document_id.clone(), self.try_render_with_ids()?)?;
            return persisted.replace_block_with_strategy(
                id,
                before_content,
                replacement,
                strategy,
            );
        }
        let index = self.require_block_index(id)?;
        let block = &self.blocks[index];
        check_before_content(block, before_content)?;
        if matches!(strategy, ReplacementStrategy::KeepId)
            && self.blocks.iter().enumerate().any(|(other_index, other)| {
                other_index != index
                    && block.source_span.contains(other.source_span)
                    && other.source_span != block.source_span
            })
        {
            bail!("block `{id}` contains nested blocks; choose an explicit Split or merge policy");
        }
        let replacement_candidates = syntax::parse_candidates(replacement)?;
        if !syntax::scan_markers(replacement)?.is_empty() {
            bail!("replacement must not contain persisted block markers; use an explicit import first");
        }

        let (replacement, range_start) = match strategy {
            ReplacementStrategy::KeepId => {
                if replacement_candidates.len() != 1 {
                    bail!(
                        "replacement for `{id}` parses to {} blocks; choose Split explicitly",
                        replacement_candidates.len()
                    );
                }
                (
                    replacement_with_marker(self, block, replacement, id),
                    replacement_range_start(self, block, false)?,
                )
            }
            ReplacementStrategy::Split { keep_id_at } => {
                if replacement_candidates.len() < 2 {
                    bail!("split replacement for `{id}` must parse to at least two blocks");
                }
                if keep_id_at >= replacement_candidates.len() {
                    bail!(
                        "split keep_id_at {keep_id_at} is outside {} replacement blocks",
                        replacement_candidates.len()
                    );
                }
                let marked = inject_marker_at_candidate(
                    replacement,
                    &replacement_candidates,
                    keep_id_at,
                    id,
                );
                (marked, replacement_range_start(self, block, true)?)
            }
        };

        let replacement = preserve_block_ending(&self.source, block.source_span, &replacement);
        let source = splice_source(
            &self.source,
            range_start,
            block.source_span.end,
            &replacement,
        )?;
        let next = Self::import(self.document_id.clone(), source)?;
        ensure_single_id(&next, id)?;
        let removed_ids = if matches!(strategy, ReplacementStrategy::Split { .. }) {
            self.blocks
                .iter()
                .filter(|candidate| {
                    candidate.id != *id && block.source_span.contains(candidate.source_span)
                })
                .map(|candidate| candidate.id.clone())
                .collect::<HashSet<_>>()
        } else {
            HashSet::new()
        };
        validate_preserved_blocks(self, &next, block.source_span, id, &removed_ids)?;
        Ok(next)
    }

    /// 将一组按顺序排列且互不重叠的块合并为一个。合并规则明确保留第一个 ID，
    /// 其余 ID 随合并消失，无须猜测重复文本原先对应哪个块。
    pub fn merge_blocks(
        &self,
        ids: &[BlockId],
        before_contents: &[String],
        replacement: &str,
    ) -> Result<Self> {
        if !self.has_persisted_ids() {
            let persisted = Self::import(self.document_id.clone(), self.try_render_with_ids()?)?;
            return persisted.merge_blocks(ids, before_contents, replacement);
        }
        if ids.len() < 2 {
            bail!("merge requires at least two block IDs");
        }
        if ids.len() != before_contents.len() {
            bail!("merge IDs and before_contents must have equal lengths");
        }
        let mut indices = Vec::with_capacity(ids.len());
        let mut seen = HashSet::new();
        for (id, before) in ids.iter().zip(before_contents) {
            if !seen.insert(id) {
                bail!("merge contains duplicate block ID `{id}`");
            }
            let index = self.require_block_index(id)?;
            check_before_content(&self.blocks[index], before)?;
            indices.push(index);
        }
        indices.sort_by_key(|index| self.blocks[*index].source_span.start);
        for pair in indices.windows(2) {
            let previous = &self.blocks[pair[0]];
            let next = &self.blocks[pair[1]];
            if previous.source_span.overlaps(next.source_span) {
                bail!(
                    "cannot merge overlapping block IDs `{}` and `{}`",
                    previous.id,
                    next.id
                );
            }
            if self.blocks.iter().enumerate().any(|(index, candidate)| {
                !indices.contains(&index)
                    && candidate.source_span.start >= previous.source_span.start
                    && candidate.source_span.end <= next.source_span.end
            }) {
                bail!(
                    "merge range contains an unselected block between `{}` and `{}`",
                    previous.id,
                    next.id
                );
            }
        }
        if syntax::parse_candidates(replacement)?.len() != 1 {
            bail!("merge replacement must parse to exactly one block");
        }
        if !syntax::scan_markers(replacement)?.is_empty() {
            bail!("merge replacement must not contain persisted block markers");
        }

        let first = &self.blocks[indices[0]];
        let last = &self.blocks[*indices.last().expect("indices is non-empty")];
        let first_id = first.id.clone();
        let replacement = replacement_with_marker(self, first, replacement, &first_id);
        let replacement = preserve_block_ending(&self.source, first.source_span, &replacement);
        let range_start = replacement_range_start(self, first, false)?;
        let source = splice_source(
            &self.source,
            range_start,
            last.source_span.end,
            &replacement,
        )?;
        let next = Self::import(self.document_id.clone(), source)?;
        ensure_single_id(&next, &first_id)?;
        let removed_ids = indices
            .iter()
            .skip(1)
            .map(|index| self.blocks[*index].id.clone())
            .collect::<HashSet<_>>();
        let merged_span = SourceSpan::new(first.source_span.start, last.source_span.end);
        validate_preserved_blocks(self, &next, merged_span, &first_id, &removed_ids)?;
        Ok(next)
    }

    fn require_block_index(&self, id: &BlockId) -> Result<usize> {
        self.blocks
            .iter()
            .position(|block| &block.id == id)
            .ok_or_else(|| {
                anyhow!(
                    "block `{id}` is not present in document `{}`",
                    self.document_id
                )
            })
    }
}

/// 块编辑后数量变化时采用的明确规则。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ReplacementStrategy {
    KeepId,
    Split { keep_id_at: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedDocument {
    schema_version: u32,
    document_id: String,
    source: String,
    blocks: Vec<BlockRecord>,
}

/// 块的匹配结果也包含所属文档；跨文档 `[[block_id]]` 链接和重复 ID 检查都需要它。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockMatch<'a> {
    pub document_id: &'a str,
    pub block: &'a BlockRecord,
}

/// 跨文档引用的位置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceLocation {
    pub document_id: String,
    pub block_id: BlockId,
    pub reference: BlockReference,
}

/// 跨文档解析第一阶段使用的内存文档集。重复 ID 会保留下来，
/// 让 `resolve` 报告歧义，而不是悄悄选中其中一个文档。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentIndex {
    pub documents: BTreeMap<String, Document>,
}

pub type DocumentSet = DocumentIndex;

impl DocumentIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, document: Document) -> Option<Document> {
        self.documents
            .insert(document.document_id.clone(), document)
    }

    pub fn get(&self, document_id: &str) -> Option<&Document> {
        self.documents.get(document_id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Document)> {
        self.documents
            .iter()
            .map(|(id, document)| (id.as_str(), document))
    }

    pub fn resolve_all(&self, id: &BlockId) -> Vec<BlockMatch<'_>> {
        self.documents
            .values()
            .filter_map(|document| {
                document.block(id).map(|block| BlockMatch {
                    document_id: document.document_id.as_str(),
                    block,
                })
            })
            .collect()
    }

    pub fn resolve(&self, id: &BlockId) -> Result<BlockMatch<'_>> {
        let matches = self.resolve_all(id);
        match matches.as_slice() {
            [] => bail!("unresolved block reference `{id}`"),
            [single] => Ok(*single),
            _ => bail!(
                "ambiguous block reference `{id}` resolves in {} documents",
                matches.len()
            ),
        }
    }

    pub fn resolve_reference(&self, reference: &BlockReference) -> Result<BlockMatch<'_>> {
        self.resolve(&reference.target)
    }

    pub fn references_to(&self, id: &BlockId) -> Vec<ReferenceLocation> {
        self.documents
            .values()
            .flat_map(|document| {
                document.blocks.iter().flat_map(move |block| {
                    block
                        .references
                        .iter()
                        .filter(move |reference| &reference.target == id)
                        .map(move |reference| ReferenceLocation {
                            document_id: document.document_id.clone(),
                            block_id: block.id.clone(),
                            reference: reference.clone(),
                        })
                })
            })
            .collect()
    }
}

fn block_kind(kind: &CandidateKind) -> BlockKind {
    match kind {
        CandidateKind::Heading { level } => BlockKind::Heading { level: *level },
        CandidateKind::Paragraph => BlockKind::Paragraph,
        CandidateKind::ListItem {
            checked: Some(checked),
        } => BlockKind::Todo { checked: *checked },
        CandidateKind::ListItem { checked: None } => BlockKind::ListItem,
        CandidateKind::FencedCode { language } => BlockKind::FencedCode {
            language: language.clone(),
        },
        CandidateKind::IndentedCode => BlockKind::IndentedCode,
        CandidateKind::Table => BlockKind::Table,
        CandidateKind::Formula { display } => BlockKind::Formula { display: *display },
        CandidateKind::Html => BlockKind::Html,
        CandidateKind::ThematicBreak => BlockKind::ThematicBreak,
    }
}

fn validate_document_records(document: &Document) -> Result<()> {
    let candidates = syntax::parse_candidates(&document.source)?;
    let markers = syntax::scan_markers(&document.source)?;
    let marker_ids = parse_marker_ids(&markers)?;
    let assignments = assign_markers(&document.source, &candidates, &markers)?;
    let raw_references = syntax::scan_references(&document.source)?;
    let references = parse_references(&raw_references)?;

    if document.blocks.len() != candidates.len() {
        bail!(
            "document metadata contains {} blocks but source parses to {}",
            document.blocks.len(),
            candidates.len()
        );
    }
    ensure_unique_block_ids(&document.blocks)?;

    for (candidate_index, (candidate, block)) in
        candidates.iter().zip(document.blocks.iter()).enumerate()
    {
        let expected_raw = document
            .source
            .get(candidate.span.start..candidate.span.end)
            .ok_or_else(|| anyhow!("invalid UTF-8 parser span {:?}", candidate.span))?;
        let expected_content = visible_content(&document.source, candidate.span, &markers);
        let expected_kind = block_kind(&candidate.kind);
        let expected_marker = assignments[candidate_index].map(|index| markers[index].span);
        let expected_references = references
            .iter()
            .filter(|reference| candidate.span.contains(reference.span))
            .filter(|reference| {
                smallest_candidate_for_span(&candidates, reference.span) == Some(candidate_index)
            })
            .cloned()
            .collect::<Vec<_>>();

        if block.source_span != candidate.span {
            bail!(
                "block `{}` has source span {:?}, expected {:?}",
                block.id,
                block.source_span,
                candidate.span
            );
        }
        if block.raw_content != expected_raw {
            bail!("block `{}` raw content does not match source", block.id);
        }
        if block.content != expected_content {
            bail!("block `{}` visible content does not match source", block.id);
        }
        if block.kind != expected_kind {
            bail!("block `{}` kind does not match source", block.id);
        }
        if block.marker_span != expected_marker {
            bail!(
                "block `{}` marker assignment does not match source",
                block.id
            );
        }
        if block.references != expected_references {
            bail!("block `{}` references do not match source", block.id);
        }
        if let Some(marker_index) = assignments[candidate_index] {
            if block.id != marker_ids[marker_index].0 {
                bail!(
                    "block `{}` ID does not match its persisted marker",
                    block.id
                );
            }
        }
    }
    Ok(())
}

fn parse_marker_ids(markers: &[Marker]) -> Result<Vec<(BlockId, usize)>> {
    let mut seen: HashMap<BlockId, usize> = HashMap::new();
    let mut out = Vec::with_capacity(markers.len());
    for (index, marker) in markers.iter().enumerate() {
        let id = BlockId::parse(&marker.token)
            .with_context(|| format!("invalid block marker at byte {}", marker.span.start))?;
        if let Some(previous) = seen.insert(id.clone(), index) {
            bail!(
                "duplicate block ID `{id}` at bytes {} and {}",
                markers[previous].span.start,
                marker.span.start
            );
        }
        out.push((id, index));
    }
    Ok(out)
}

fn assign_markers(
    source: &str,
    candidates: &[Candidate],
    markers: &[Marker],
) -> Result<Vec<Option<usize>>> {
    let mut assignments = vec![None; candidates.len()];
    // 两个扫描器都按源码顺序返回结果。这里只保留范围尚未结束的候选块，
    // 使标记归属查找的开销随 Markdown 嵌套深度增长，而非随块总数增长。
    let mut candidate_cursor = 0usize;
    let mut active_candidates = Vec::new();
    for (marker_index, marker) in markers.iter().enumerate() {
        while candidate_cursor < candidates.len()
            && candidates[candidate_cursor].span.start <= marker.span.start
        {
            active_candidates.push(candidate_cursor);
            candidate_cursor += 1;
        }
        active_candidates
            .retain(|candidate_index| candidates[*candidate_index].span.end >= marker.span.end);
        // 父列表项中独占一行的标记，位置虽然落在父项范围内，实际却标注后面的子项。
        // 因此先检查后续候选块，再考虑外层候选块。行内注释（如 `# title
        // <!-- marker -->`）后面还有非空白内容，仍走下面的包含关系判断。
        let line_only = source
            .get(marker.line_span.start..marker.line_span.end)
            .unwrap_or_default()
            .trim_end_matches(['\r', '\n'])
            .trim()
            == marker.raw;
        let following = line_only
            .then(|| {
                candidates
                    .get(first_candidate_with_start(candidates, marker.span.end)..)
                    .unwrap_or_default()
                    .iter()
                    .enumerate()
                    .filter(|(_, candidate)| {
                        source[marker.span.end..candidate.span.start]
                            .trim()
                            .is_empty()
                    })
                    .min_by_key(|(_, candidate)| candidate.span.start)
                    .map(|(relative_index, _)| {
                        first_candidate_with_start(candidates, marker.span.end) + relative_index
                    })
            })
            .flatten();
        let containing = active_candidates
            .iter()
            .copied()
            .filter(|candidate_index| candidates[*candidate_index].span.contains(marker.span))
            .map(|candidate_index| (candidate_index, candidates[candidate_index].span.len()))
            .collect::<Vec<_>>();
        let candidate_index = if let Some(following) = following {
            following
        } else if containing.is_empty() {
            return Err(anyhow!(
                "block marker at bytes {} is not attached to a Markdown block",
                marker.span.start
            ));
        } else {
            let min_len = containing.iter().map(|(_, len)| *len).min().unwrap_or(0);
            let shortest = containing
                .iter()
                .filter(|(_, len)| *len == min_len)
                .map(|(index, _)| *index)
                .collect::<Vec<_>>();
            if shortest.len() != 1 {
                bail!(
                    "marker at bytes {} is ambiguous across {} blocks",
                    marker.span.start,
                    shortest.len()
                );
            }
            shortest[0]
        };
        if assignments[candidate_index].is_some() {
            bail!(
                "multiple block markers attach to candidate at bytes {}",
                candidates[candidate_index].span.start
            );
        }
        assignments[candidate_index] = Some(marker_index);
    }
    Ok(assignments)
}

fn parse_references(raw: &[RawReference]) -> Result<Vec<BlockReference>> {
    raw.iter()
        .map(|reference| {
            let target = BlockId::parse(&reference.token).with_context(|| {
                format!("invalid block reference at byte {}", reference.span.start)
            })?;
            Ok(BlockReference {
                target,
                span: reference.span,
                raw: reference.raw.clone(),
            })
        })
        .collect()
}

fn smallest_candidate_for_span(candidates: &[Candidate], span: SourceSpan) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| candidate.span.contains(span))
        .min_by_key(|(_, candidate)| candidate.span.len())
        .map(|(index, _)| index)
}

fn first_candidate_with_start(candidates: &[Candidate], start: usize) -> usize {
    let mut low = 0usize;
    let mut high = candidates.len();
    while low < high {
        let middle = low + (high - low) / 2;
        if candidates[middle].span.start < start {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    low
}

fn first_marker_with_start(markers: &[Marker], start: usize) -> usize {
    let mut low = 0usize;
    let mut high = markers.len();
    while low < high {
        let middle = low + (high - low) / 2;
        if markers[middle].span.start < start {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    low
}

fn visible_content(source: &str, span: SourceSpan, markers: &[Marker]) -> String {
    let first = first_marker_with_start(markers, span.start);
    let hidden = markers
        .get(first..)
        .unwrap_or_default()
        .iter()
        .take_while(|marker| marker.span.start < span.end)
        .filter(|marker| span.contains(marker.span))
        .map(|marker| {
            let line = marker.line_span;
            let line_body = source
                .get(line.start..line.end)
                .unwrap_or_default()
                .trim_end_matches(['\r', '\n']);
            let comment = source
                .get(marker.span.start..marker.span.end)
                .unwrap_or_default();
            if line_body.trim() == comment {
                line
            } else {
                marker.span
            }
        })
        .collect::<Vec<_>>();
    strip_one_line_ending(&remove_spans(source, span, &hidden))
}

fn strip_one_line_ending(value: &str) -> String {
    if let Some(value) = value.strip_suffix("\r\n") {
        value.to_owned()
    } else if let Some(value) = value.strip_suffix(['\r', '\n']) {
        value.to_owned()
    } else {
        value.to_owned()
    }
}

fn preserve_block_ending(source: &str, span: SourceSpan, replacement: &str) -> String {
    let raw = &source[span.start..span.end];
    let ending = if raw.ends_with("\r\n") {
        Some("\r\n")
    } else if raw.ends_with('\n') {
        Some("\n")
    } else if raw.ends_with('\r') {
        Some("\r")
    } else {
        None
    };
    match ending {
        Some(ending) if !replacement.ends_with(['\r', '\n']) => {
            format!("{replacement}{ending}")
        }
        _ => replacement.to_owned(),
    }
}

fn remove_spans(source: &str, outer: SourceSpan, spans: &[SourceSpan]) -> String {
    let mut spans = spans
        .iter()
        .copied()
        .filter(|span| outer.contains(*span))
        .collect::<Vec<_>>();
    spans.sort_by_key(|span| span.start);
    let mut out = String::new();
    let mut cursor = outer.start;
    for span in spans {
        if span.start > cursor {
            out.push_str(&source[cursor..span.start]);
        }
        cursor = cursor.max(span.end);
    }
    if cursor < outer.end {
        out.push_str(&source[cursor..outer.end]);
    }
    out
}

fn ensure_unique_block_ids(blocks: &[BlockRecord]) -> Result<()> {
    let mut seen = HashSet::new();
    for block in blocks {
        if !seen.insert(&block.id) {
            bail!("duplicate block ID `{}` in parsed document", block.id);
        }
    }
    Ok(())
}

fn source_indentation(source: &str, line_start: usize, block_start: usize) -> String {
    source
        .get(line_start..block_start)
        .unwrap_or_default()
        .chars()
        .take_while(|character| matches!(character, ' ' | '\t'))
        .collect()
}

fn marker_comment(id: &BlockId) -> String {
    format!("{BLOCK_MARKER_PREFIX}{id}{BLOCK_MARKER_SUFFIX}")
}

/// 紧挨着引用块之前的独立注释可能被 CommonMark 当作 HTML 块吞掉。
/// 将标记追加到引用的第一行，既能保持解析树不变，也不会出现在渲染后的 HTML 中。
fn marker_insertion(source: &str, block: &BlockRecord, id: &BlockId) -> (usize, String) {
    if matches!(block.kind, BlockKind::Html) && !is_mochi_container(source, block.source_span) {
        marker_insertion_at_end(source, block.source_span.end, id)
    } else {
        marker_insertion_at(source, block.source_span.start, id)
    }
}

fn marker_insertion_at_end(source: &str, end: usize, id: &BlockId) -> (usize, String) {
    let mut offset = end.min(source.len());
    if source[..offset].ends_with("\r\n") {
        offset -= 2;
    } else if source[..offset].ends_with(['\r', '\n']) {
        offset -= 1;
    }
    (offset, marker_comment(id))
}

fn marker_insertion_at(source: &str, block_start: usize, id: &BlockId) -> (usize, String) {
    let line_start = syntax::line_start(source, block_start);
    let line_prefix = source.get(line_start..block_start).unwrap_or_default();
    let line_body = source.get(block_start..).unwrap_or_default();
    if line_prefix.trim_start().starts_with('>')
        || line_body.trim_start_matches([' ', '\t']).starts_with('>')
    {
        let line_end = source[block_start..]
            .find(['\r', '\n'])
            .map_or(source.len(), |relative| block_start + relative);
        (line_end, marker_comment(id))
    } else {
        let indentation = source_indentation(source, line_start, block_start);
        (
            line_start,
            format!(
                "{indentation}{BLOCK_MARKER_PREFIX}{id}{BLOCK_MARKER_SUFFIX}{}",
                syntax::line_ending(source)
            ),
        )
    }
}

fn replacement_with_marker(
    document: &Document,
    block: &BlockRecord,
    replacement: &str,
    id: &BlockId,
) -> String {
    if block.marker_span.is_some_and(|marker| {
        marker.end <= block.source_span.start
            && document.source[marker.end..block.source_span.start]
                .trim()
                .is_empty()
    }) {
        replacement.to_owned()
    } else {
        let replacement_start = syntax::parse_candidates(replacement)
            .ok()
            .and_then(|candidates| candidates.first().map(|candidate| candidate.span.start))
            .unwrap_or(0);
        let candidate = syntax::parse_candidates(replacement)
            .ok()
            .and_then(|candidates| candidates.into_iter().next());
        let (offset, marker) = if let Some(candidate) = candidate {
            if matches!(candidate.kind, CandidateKind::Html)
                && !is_mochi_container(replacement, candidate.span)
            {
                marker_insertion_at_end(replacement, candidate.span.end, id)
            } else {
                marker_insertion_at(replacement, candidate.span.start, id)
            }
        } else {
            marker_insertion_at(replacement, replacement_start, id)
        };
        let mut marked = replacement.to_owned();
        marked.insert_str(offset.min(marked.len()), &marker);
        marked
    }
}

fn inject_marker_at_candidate(
    replacement: &str,
    candidates: &[Candidate],
    index: usize,
    id: &BlockId,
) -> String {
    let candidate = &candidates[index];
    let line_start = syntax::line_start(replacement, candidate.span.start);
    let line_prefix = replacement
        .get(line_start..candidate.span.start)
        .unwrap_or_default();
    let line_body = replacement.get(candidate.span.start..).unwrap_or_default();
    let (offset, marker) = if matches!(candidate.kind, CandidateKind::Html)
        && !is_mochi_container(replacement, candidate.span)
    {
        marker_insertion_at_end(replacement, candidate.span.end, id)
    } else if line_prefix.trim_start().starts_with('>')
        || line_body.trim_start_matches([' ', '\t']).starts_with('>')
    {
        let line_end = replacement[candidate.span.start..]
            .find(['\r', '\n'])
            .map_or(candidate.span.end, |relative| {
                candidate.span.start + relative
            });
        (line_end, marker_comment(id))
    } else {
        let indentation = source_indentation(replacement, line_start, candidate.span.start);
        (
            line_start,
            format!(
                "{indentation}{BLOCK_MARKER_PREFIX}{id}{BLOCK_MARKER_SUFFIX}{}",
                syntax::line_ending(replacement)
            ),
        )
    };
    let mut output = replacement.to_owned();
    output.insert_str(offset.min(output.len()), &marker);
    output
}

/// 编辑器的自定义容器语法对分隔符所在行有要求。若把标记追加到结束行的
/// `:::` 或 `</details>` 后，容器扫描器会把分隔符当作用户文本。
/// 因此这类容器的标记单独放在起始行之前。
fn is_mochi_container(source: &str, span: SourceSpan) -> bool {
    source
        .get(span.start..span.end)
        .and_then(|raw| raw.lines().next())
        .map(str::trim)
        .is_some_and(|line| {
            line == "<details>"
                || (line.starts_with("<details ") && line.ends_with('>'))
                || line.strip_prefix(":::mochi-highlight").is_some_and(|rest| {
                    rest.is_empty() || rest.chars().next().is_some_and(char::is_whitespace)
                })
        })
}

fn replacement_range_start(
    document: &Document,
    block: &BlockRecord,
    remove_marker: bool,
) -> Result<usize> {
    let Some(marker) = block.marker_span else {
        return Ok(block.source_span.start);
    };
    if block.source_span.contains(marker) {
        return Ok(block.source_span.start);
    }
    if marker.end > block.source_span.start {
        bail!("block marker for `{}` has an invalid source span", block.id);
    }
    let gap = &document.source[marker.end..block.source_span.start];
    if !gap.trim().is_empty() {
        bail!("block marker for `{}` is ambiguous", block.id);
    }
    if remove_marker {
        Ok(syntax::line_start(&document.source, marker.start))
    } else {
        Ok(block.source_span.start)
    }
}

fn splice_source(source: &str, start: usize, end: usize, replacement: &str) -> Result<String> {
    if start > end
        || end > source.len()
        || !source.is_char_boundary(start)
        || !source.is_char_boundary(end)
    {
        bail!("invalid replacement source range {start}..{end}");
    }
    let mut output = String::with_capacity(source.len() + replacement.len());
    output.push_str(&source[..start]);
    output.push_str(replacement);
    output.push_str(&source[end..]);
    Ok(output)
}

fn check_before_content(block: &BlockRecord, before_content: &str) -> Result<()> {
    if before_content == block.content || before_content == block.raw_content {
        return Ok(());
    }
    bail!(
        "stale block `{}`: before-content does not match the current block",
        block.id
    )
}

fn ensure_single_id(document: &Document, id: &BlockId) -> Result<()> {
    let matches = document
        .blocks
        .iter()
        .filter(|block| &block.id == id)
        .count();
    if matches != 1 {
        bail!(
            "replacement did not preserve exactly one occurrence of block ID `{id}` (found {matches})"
        );
    }
    Ok(())
}

fn compare_id_kinds(before: &Document, after: &Document) -> Result<()> {
    let before_kinds = before
        .blocks
        .iter()
        .map(|block| (&block.id, &block.kind))
        .collect::<HashSet<_>>();
    let after_kinds = after
        .blocks
        .iter()
        .map(|block| (&block.id, &block.kind))
        .collect::<HashSet<_>>();
    if before_kinds != after_kinds {
        bail!("persisted marker insertion changed the parsed block set");
    }
    Ok(())
}

/// 校验编辑时没有显式替换的文档部分。如果代码围栏或 HTML 块没有闭合，
/// 重新解析完整 Markdown 源码可能改变替换范围之外的块边界。
/// 即使目标 ID 仍只出现一次，这种变化也不安全，因此这里会检查所有旧 ID。
/// 编辑子块后，外层容器的可见内容可能自然变化，但其 ID 和类型必须保留。
fn validate_preserved_blocks(
    before: &Document,
    after: &Document,
    edited_span: SourceSpan,
    edited_id: &BlockId,
    explicitly_removed: &HashSet<BlockId>,
) -> Result<()> {
    for old in &before.blocks {
        if explicitly_removed.contains(&old.id) {
            continue;
        }
        let Some(current) = after.block(&old.id) else {
            bail!(
                "edit changed or removed unrelated block `{}`; Markdown boundary is ambiguous",
                old.id
            );
        };
        if old.id == *edited_id {
            continue;
        }
        let ancestor = old.source_span.contains(edited_span) && old.source_span != edited_span;
        if old.kind != current.kind {
            bail!(
                "edit changed kind of unrelated block `{}`; Markdown boundary is ambiguous",
                old.id
            );
        }
        if !ancestor && old.content != current.content {
            bail!(
                "edit changed content of unrelated block `{}`; Markdown boundary is ambiguous",
                old.id
            );
        }
        if !ancestor
            && old
                .references
                .iter()
                .map(|reference| (&reference.target, reference.raw.as_str()))
                .ne(current
                    .references
                    .iter()
                    .map(|reference| (&reference.target, reference.raw.as_str())))
        {
            bail!(
                "edit changed references of unrelated block `{}`; Markdown boundary is ambiguous",
                old.id
            );
        }
    }
    Ok(())
}

/// SHA-256 只用于乐观并发控制和差异识别。内容哈希不参与生成或选择 `BlockId`。
pub fn hash_content(content: &str) -> String {
    let digest = Sha256::digest(content.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_formula_inside_prose_remains_part_of_the_paragraph() {
        let document = Document::import("doc.md", "正文 $x + y$ 继续").unwrap();
        assert_eq!(document.blocks().len(), 1);
        assert!(matches!(document.blocks()[0].kind, BlockKind::Paragraph));
    }

    fn id(number: &str) -> BlockId {
        BlockId::parse(format!("block_00000000-0000-4000-8000-{number:0>12}")).unwrap()
    }

    #[test]
    fn id_validation_and_random_generation_are_explicit() {
        assert!(BlockId::parse("block_00000000-0000-4000-8000-000000000001").is_ok());
        assert!(BlockId::parse("block_not-an-id").is_err());
        let generated = BlockId::generate().unwrap();
        assert!(generated.as_str().starts_with("block_"));
        assert!(BlockId::parse(generated.as_str()).is_ok());
        assert!(serde_json::from_str::<BlockId>("\"block_bad\"").is_err());
    }

    #[test]
    fn direct_document_serde_round_trip_validates_source_metadata() {
        let document = Document::import("doc", "one\n\ntwo\n").unwrap();
        let encoded = serde_json::to_string(&document).unwrap();
        let restored: Document = serde_json::from_str(&encoded).unwrap();
        assert_eq!(restored, document);

        let mut forged: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        forged["blocks"][0]["content"] = serde_json::Value::String("forged".into());
        assert!(serde_json::from_value::<Document>(forged).is_err());
    }

    #[test]
    fn import_save_reopen_preserves_ids_and_unmodified_source() {
        let source = "# 标题\r\n\r\n同一段\r\n\r\n同一段\r\n";
        let document = Document::import("doc", source).unwrap();
        assert_eq!(document.source(), source);
        assert_eq!(document.blocks().len(), 3);
        let saved = document.save().unwrap();
        assert!(saved.contains("<!-- mochi:block block_"));
        assert!(saved.contains("\r\n"));
        let reopened = Document::import("doc", saved).unwrap();
        assert_eq!(
            document
                .blocks()
                .iter()
                .map(|block| block.id.clone())
                .collect::<Vec<_>>(),
            reopened
                .blocks()
                .iter()
                .map(|block| block.id.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn editing_unmarked_document_preserves_unrelated_in_memory_ids() {
        let document = Document::import("doc", "first\n\nsecond\n").unwrap();
        let first = document.blocks()[0].id.clone();
        let second = document.blocks()[1].id.clone();
        let edited = document.replace_block(&first, "first", "changed").unwrap();
        assert_eq!(edited.find_block(&first).unwrap().content, "changed");
        assert_eq!(edited.find_block(&second).unwrap().content, "second");
        assert_eq!(
            edited
                .blocks()
                .iter()
                .map(|block| &block.id)
                .collect::<Vec<_>>(),
            vec![&first, &second]
        );
        assert!(edited.has_persisted_ids());
    }

    #[test]
    fn replacement_rejects_unclosed_fence_that_would_swallow_neighbor() {
        let first = id("000000000011");
        let second = id("000000000012");
        let source =
            format!("<!-- mochi:block {first} -->\n原块\n\n<!-- mochi:block {second} -->\n邻块\n");
        let document = Document::import("doc", source).unwrap();
        let error = document
            .replace_block(&first, "原块", "```rust\n未闭合")
            .unwrap_err()
            .to_string();
        assert!(error.contains("unrelated block") || error.contains("boundary"));
        assert_eq!(document.find_block(&second).unwrap().content, "邻块");
    }

    #[test]
    fn replacement_rejects_unclosed_html_that_would_swallow_neighbor() {
        let first = id("000000000013");
        let second = id("000000000014");
        let source =
            format!("<!-- mochi:block {first} -->\n原块\n\n<!-- mochi:block {second} -->\n邻块\n");
        let document = Document::import("doc", source).unwrap();
        let error = document
            .replace_block(&first, "原块", "<script>\n未闭合")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("unrelated block")
                || error.contains("boundary")
                || error.contains("multiple block markers")
        );
        assert_eq!(document.find_block(&second).unwrap().content, "邻块");
    }

    #[test]
    fn explicit_marker_is_stable_through_move_and_edit() {
        let first = id("000000000001");
        let second = id("000000000002");
        let source = format!(
            "<!-- mochi:block {first} -->\n第一段\n\n<!-- mochi:block {second} -->\n第二段\n"
        );
        let _document = Document::import("doc", source).unwrap();
        let moved = format!(
            "<!-- mochi:block {second} -->\n第二段\n\n<!-- mochi:block {first} -->\n第一段\n"
        );
        let reopened = Document::import("doc", moved).unwrap();
        assert_eq!(reopened.blocks()[0].id, second);
        assert_eq!(reopened.blocks()[1].id, first);
        let edited = reopened
            .replace_block(&first, "第一段", "第一段（修改）")
            .unwrap();
        assert_eq!(
            edited.find_block(first.as_str()).unwrap().content,
            "第一段（修改）"
        );
    }

    #[test]
    fn duplicate_paragraphs_get_distinct_random_ids() {
        let document = Document::import("doc", "重复\n\n重复\n").unwrap();
        assert_ne!(document.blocks()[0].id, document.blocks()[1].id);
    }

    #[test]
    fn parses_todos_nested_items_fences_tables_and_formula_as_whole_blocks() {
        let source = "# H\n\n- [ ] todo\n  - [x] nested\n\n```rust\n<!-- mochi:block block_bad -->\nlet x = 1;\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n$$x + y$$\n";
        let document = Document::import("doc", source).unwrap();
        assert!(document
            .blocks()
            .iter()
            .any(|block| matches!(block.kind, BlockKind::Heading { .. })));
        assert_eq!(
            document
                .blocks()
                .iter()
                .filter(|block| matches!(block.kind, BlockKind::Todo { .. }))
                .count(),
            2
        );
        assert!(document
            .blocks()
            .iter()
            .any(|block| matches!(block.kind, BlockKind::FencedCode { .. })));
        assert!(document
            .blocks()
            .iter()
            .any(|block| matches!(block.kind, BlockKind::Table)));
        assert!(document
            .blocks()
            .iter()
            .any(|block| matches!(block.kind, BlockKind::Formula { display: true })));
        let code = document
            .blocks()
            .iter()
            .find(|block| matches!(block.kind, BlockKind::FencedCode { .. }))
            .unwrap();
        assert!(code.marker_span.is_none());
    }

    #[test]
    fn nested_list_blocks_keep_code_table_and_rule_units() {
        let source = "- item\n\n  ```rust\n  let x = 1;\n  ```\n\n  | a | b |\n  |---|---|\n  | 1 | 2 |\n\n  ---\n";
        let document = Document::import("doc", source).unwrap();
        assert!(document
            .blocks()
            .iter()
            .any(|block| matches!(block.kind, BlockKind::ListItem)));
        assert!(document
            .blocks()
            .iter()
            .any(|block| matches!(block.kind, BlockKind::FencedCode { .. })));
        assert!(document
            .blocks()
            .iter()
            .any(|block| matches!(block.kind, BlockKind::Table)));
        assert!(document
            .blocks()
            .iter()
            .any(|block| matches!(block.kind, BlockKind::ThematicBreak)));
        let saved = document.save().unwrap();
        let reopened = Document::import("doc", saved).unwrap();
        assert_eq!(document.blocks().len(), reopened.blocks().len());
        for (before, after) in document.blocks().iter().zip(reopened.blocks()) {
            assert_eq!(before.id, after.id);
            assert_eq!(before.kind, after.kind);
            assert_eq!(before.content, after.content);
        }
    }

    #[test]
    fn html_blocks_and_thematic_breaks_are_addressable_without_rewriting_source() {
        let source = "<table>\n<tr><td>中文🙂</td></tr>\n</table>\n\n---\n\n之后\n";
        let document = Document::import("doc", source).unwrap();
        assert!(document
            .blocks()
            .iter()
            .any(|block| matches!(block.kind, BlockKind::Html)));
        assert!(document
            .blocks()
            .iter()
            .any(|block| matches!(block.kind, BlockKind::ThematicBreak)));
        let saved = document.save().unwrap();
        let reopened = Document::import("doc", saved).unwrap();
        assert_eq!(document.blocks().len(), reopened.blocks().len());
        for (before, after) in document.blocks().iter().zip(reopened.blocks()) {
            assert_eq!(before.id, after.id);
            assert_eq!(before.kind, after.kind);
            assert_eq!(before.content, after.content);
        }
    }

    #[test]
    fn invalid_and_duplicate_markers_are_errors_but_code_markers_are_ignored() {
        assert!(Document::import("doc", "<!-- mochi:block block_bad -->\n# heading").is_err());
        let duplicate = "<!-- mochi:block block_00000000-0000-4000-8000-000000000001 -->\na\n\n<!-- mochi:block block_00000000-0000-4000-8000-000000000001 -->\nb";
        assert!(Document::import("doc", duplicate).is_err());
        let code = "```\n<!-- mochi:block block_bad -->\n```\n\ntext";
        assert!(Document::import("doc", code).is_ok());
    }

    #[test]
    fn replacement_requires_exact_before_and_explicit_split() {
        let document = Document::import("doc", "old\n\nother\n").unwrap();
        let first = document.blocks()[0].id.clone();
        assert!(document.replace_block(&first, "changed", "new").is_err());
        let split = document
            .replace_block_with_strategy(
                &first,
                "old",
                "one\n\ntwo",
                ReplacementStrategy::Split { keep_id_at: 1 },
            )
            .unwrap();
        assert_eq!(
            split
                .blocks()
                .iter()
                .filter(|block| block.id == first)
                .count(),
            1
        );
        assert_eq!(split.find_block(&first).unwrap().content, "two");
        assert!(document.replace_block(&first, "old", "one\n\ntwo").is_err());
    }

    #[test]
    fn merge_is_explicit_and_rejects_nested_overlap() {
        let document = Document::import("doc", "a\n\nb\n\nc\n").unwrap();
        let ids = document
            .blocks()
            .iter()
            .map(|block| block.id.clone())
            .collect::<Vec<_>>();
        let before = document
            .blocks()
            .iter()
            .take(2)
            .map(|block| block.content.clone())
            .collect::<Vec<_>>();
        let merged = document
            .merge_blocks(&ids[..2], &before, "a and b")
            .unwrap();
        assert_eq!(merged.find_block(&ids[0]).unwrap().content, "a and b");
        assert!(merged.find_block(&ids[1]).is_err());
    }

    #[test]
    fn references_are_masked_in_code_and_resolved_across_documents() {
        let target = id("000000000009");
        let source = format!("<!-- mochi:block {target} -->\ntarget\n\n```\n[[{target}]]\n```\n");
        let target_doc = Document::import("target", source).unwrap();
        let source_doc = Document::import("source", format!("text [[{target}]]\n")).unwrap();
        let mut index = DocumentIndex::new();
        index.insert(target_doc);
        index.insert(source_doc);
        let matches = index.references_to(&target);
        assert_eq!(matches.len(), 1);
        assert_eq!(index.resolve(&target).unwrap().document_id, "target");
    }

    #[test]
    fn chinese_emoji_and_crlf_ranges_are_valid_utf8_boundaries() {
        let document = Document::import("doc", "😀 中文\r\n\r\n第二段\r\n").unwrap();
        for block in document.blocks() {
            assert!(
                document.source[block.source_span.start..block.source_span.end].is_char_boundary(0)
            );
            assert!(document.source.is_char_boundary(block.source_span.start));
            assert!(document.source.is_char_boundary(block.source_span.end));
        }
        let saved = document.save().unwrap();
        assert!(saved.contains("\r\n"));
    }

    #[test]
    fn persisting_nested_items_keeps_every_item_addressable() {
        let document = Document::import("doc", "- parent\r\n  - child\r\n").unwrap();
        let item_count = document
            .blocks()
            .iter()
            .filter(|block| matches!(block.kind, BlockKind::ListItem))
            .count();
        assert_eq!(item_count, 2);
        let saved = document.try_render_with_ids();
        let saved = saved.unwrap();
        let reopened = Document::import("doc", saved).unwrap();
        assert_eq!(
            reopened
                .blocks()
                .iter()
                .filter(|block| matches!(block.kind, BlockKind::ListItem))
                .count(),
            2
        );
        assert!(reopened
            .blocks()
            .iter()
            .all(|block| block.marker_span.is_some()));
    }

    #[test]
    fn complex_markdown_save_round_trip_keeps_kind_content_and_non_marker_bytes() {
        let corpus = [
            "# 标题🙂\r\n\r\n段落一 [[block_00000000-0000-4000-8000-000000000001]]\r\n\r\n- [ ] 待办\r\n  - [x] 嵌套🙂\r\n\r\n```rust\r\n// <!-- mochi:block block_bad -->\r\nlet value = \"[[block_bad]]\";\r\n```\r\n\r\n| 列一 | 列二 |\r\n| :--- | ---: |\r\n| 中文 | emoji🙂 |\r\n\r\n$$x + y = z$$\r\n",
            "前言\n\n标题\n===\n\n> 引用段落\n> 第二行\n\nHTML 前\n\n<div>块内容</div>\n\n- alpha\n- beta\n",
            "重复段落\n\n重复段落\n\n<!-- ordinary comment -->\n末尾\n",
        ];
        for source in corpus {
            let original = Document::import("corpus", source).unwrap();
            let saved = original.save().unwrap();
            let reopened = Document::import("corpus", saved.clone()).unwrap();
            assert_eq!(
                original.blocks.len(),
                reopened.blocks.len(),
                "source: {source:?}"
            );
            for (before, after) in original.blocks.iter().zip(&reopened.blocks) {
                assert_eq!(before.id, after.id, "source: {source:?}");
                assert_eq!(before.kind, after.kind, "source: {source:?}");
                assert_eq!(before.content, after.content, "source: {source:?}");
            }
            assert_eq!(remove_persisted_marker_lines(&saved), source);
        }
    }

    fn remove_persisted_marker_lines(source: &str) -> String {
        let markers = scan_markers(source).unwrap();
        let mut spans = markers
            .iter()
            .map(|marker| {
                let line = source[marker.line_span.start..marker.line_span.end]
                    .trim_end_matches(['\r', '\n']);
                if line.trim() == marker.raw {
                    marker.line_span
                } else {
                    marker.span
                }
            })
            .collect::<Vec<_>>();
        spans.sort_by_key(|span| span.start);
        let mut output = String::new();
        let mut cursor = 0;
        for span in spans {
            output.push_str(&source[cursor..span.start]);
            cursor = span.end;
        }
        output.push_str(&source[cursor..]);
        output
    }
}

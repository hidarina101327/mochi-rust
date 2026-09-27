//! 审批按文档 ID、块 ID 和前后内容哈希匹配；重复决定幂等，过期提案不得覆盖编辑。

use std::collections::BTreeMap;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::model::{hash_content, BlockId, Document};

/// 单个块的「改前/改后」提案。`before_content` 是存在
/// [`crate::model::BlockRecord::content`] 里的确切可见块内容
///（模型层的编辑守卫也接受带行内标记的原始内容）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockProposal {
    pub document_id: String,
    pub block_id: BlockId,
    pub before_content: String,
    pub after_content: String,
    pub before_hash: String,
    pub after_hash: String,
}

impl BlockProposal {
    /// 构造提案，并给两个版本的内容各自算哈希，不动文档。
    pub fn new(
        document_id: impl Into<String>,
        block_id: BlockId,
        before_content: impl Into<String>,
        after_content: impl Into<String>,
    ) -> Self {
        let before_content = before_content.into();
        let after_content = after_content.into();
        Self {
            document_id: document_id.into(),
            block_id,
            before_hash: hash_content(&before_content),
            after_hash: hash_content(&after_content),
            before_content,
            after_content,
        }
    }

    /// 把块的当前内容记录下来，作为这次编辑的前置条件。
    ///
    /// # 错误
    ///
    /// `document` 里没有 `block_id` 时返回错误。
    pub fn from_document(
        document: &Document,
        block_id: &BlockId,
        after_content: impl Into<String>,
    ) -> Result<Self> {
        let block = document
            .block(block_id)
            .ok_or_else(|| anyhow!("block `{block_id}` is not present in proposal document"))?;
        Ok(Self::new(
            document.document_id.clone(),
            block_id.clone(),
            block.content.clone(),
            after_content,
        ))
    }

    /// 生成用于展示的差异，不应用提案。
    pub fn diff(&self) -> BlockDiff {
        BlockDiff {
            document_id: self.document_id.clone(),
            block_id: self.block_id.clone(),
            before: self.before_content.clone(),
            after: self.after_content.clone(),
            before_hash: self.before_hash.clone(),
            after_hash: self.after_hash.clone(),
        }
    }

    /// [`ApprovalBook`] 用的稳定内存键。它不是块 ID，
    /// 也不参与块 ID 的生成。
    pub fn fingerprint(&self) -> String {
        format!(
            "{}\u{1f}{}\u{1f}{}\u{1f}{}",
            self.document_id, self.block_id, self.before_hash, self.after_hash
        )
    }
}

/// 可序列化的差异视图，适合在差异界面中显示。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockDiff {
    pub document_id: String,
    pub block_id: BlockId,
    pub before: String,
    pub after: String,
    pub before_hash: String,
    pub after_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalDecision {
    Accepted,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ApprovalOutcome {
    Applied,
    AlreadyAccepted,
    Rejected,
    AlreadyRejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalEntry {
    pub document_id: String,
    pub block_id: BlockId,
    pub before_hash: String,
    pub after_hash: String,
    pub decision: ApprovalDecision,
}

impl ApprovalEntry {
    fn key(&self) -> String {
        format!(
            "{}\u{1f}{}\u{1f}{}\u{1f}{}",
            self.document_id, self.block_id, self.before_hash, self.after_hash
        )
    }
}

/// 持久化的决定记录簿。条目以提案指纹为键，
/// 可以和宿主应用的审批状态一起序列化。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalBook {
    pub entries: BTreeMap<String, ApprovalEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalResult {
    pub outcome: ApprovalOutcome,
    pub document: Document,
}

impl ApprovalBook {
    /// 建一本空的决定记录簿。
    pub fn new() -> Self {
        Self::default()
    }

    /// 按指纹顺序遍历所有决定。
    pub fn entries(&self) -> impl Iterator<Item = &ApprovalEntry> {
        self.entries.values()
    }

    /// 返回匹配的决定；条目缺失或不一致时返回 `None`。
    pub fn decision_for(&self, proposal: &BlockProposal) -> Option<ApprovalDecision> {
        let key = proposal.fingerprint();
        self.entries
            .get(&key)
            .and_then(|entry| (entry.key() == key).then_some(entry.decision))
    }

    /// 接受提案并返回结果文档，但不落盘。
    ///
    /// # 错误
    ///
    /// 返回 [`Self::apply`] 描述的校验或编辑错误。
    pub fn accept(
        &mut self,
        document: &Document,
        proposal: &BlockProposal,
    ) -> Result<ApprovalResult> {
        self.apply(document, proposal, ApprovalDecision::Accepted)
    }

    /// 记录一次拒绝，文档原样返回。
    ///
    /// # 错误
    ///
    /// 返回 [`Self::apply`] 描述的校验错误。
    pub fn reject(
        &mut self,
        document: &Document,
        proposal: &BlockProposal,
    ) -> Result<ApprovalResult> {
        self.apply(document, proposal, ApprovalDecision::Rejected)
    }

    /// 应用一条用户决定。完全相同的既有决定会返回所给文档的克隆和
    /// 幂等的结果，不重放编辑——这一点很关键：调用方可能已经
    /// 在文档的其他地方做了无关修改。
    ///
    /// # 错误
    ///
    /// 文档 ID 不匹配、决定条目损坏、决定冲突、块缺失、内容过期、
    /// 哈希不一致、或被拒绝的块替换都会返回错误。校验失败不记录决定。
    pub fn apply(
        &mut self,
        document: &Document,
        proposal: &BlockProposal,
        decision: ApprovalDecision,
    ) -> Result<ApprovalResult> {
        if document.document_id != proposal.document_id {
            bail!(
                "approval document mismatch: proposal targets `{}`, current is `{}`",
                proposal.document_id,
                document.document_id
            );
        }

        let key = proposal.fingerprint();
        if let Some(previous) = self.entries.get(&key) {
            if previous.key() != key {
                bail!("corrupt approval entry for proposal `{key}`");
            }
            if previous.decision != decision {
                bail!(
                    "conflicting approval for block `{}`: already {:?}",
                    proposal.block_id,
                    previous.decision
                );
            }
            let outcome = match decision {
                ApprovalDecision::Accepted => ApprovalOutcome::AlreadyAccepted,
                ApprovalDecision::Rejected => ApprovalOutcome::AlreadyRejected,
            };
            return Ok(ApprovalResult {
                outcome,
                document: document.clone(),
            });
        }

        let current = document.block(&proposal.block_id).ok_or_else(|| {
            anyhow!(
                "stale approval: block `{}` no longer exists",
                proposal.block_id
            )
        })?;
        if current.content != proposal.before_content
            && current.raw_content != proposal.before_content
        {
            bail!(
                "stale approval for block `{}`: current content differs from before-content",
                proposal.block_id
            );
        }
        if hash_content(&proposal.before_content) != proposal.before_hash {
            bail!("proposal before_hash does not match before_content");
        }
        if hash_content(&proposal.after_content) != proposal.after_hash {
            bail!("proposal after_hash does not match after_content");
        }

        let entry = ApprovalEntry {
            document_id: proposal.document_id.clone(),
            block_id: proposal.block_id.clone(),
            before_hash: proposal.before_hash.clone(),
            after_hash: proposal.after_hash.clone(),
            decision,
        };
        let (outcome, next_document) = match decision {
            ApprovalDecision::Accepted => {
                let next = document
                    .replace_block(
                        &proposal.block_id,
                        &proposal.before_content,
                        &proposal.after_content,
                    )
                    .context("apply accepted block proposal")?;
                (ApprovalOutcome::Applied, next)
            }
            ApprovalDecision::Rejected => (ApprovalOutcome::Rejected, document.clone()),
        };
        self.entries.insert(entry.key(), entry);
        Ok(ApprovalResult {
            outcome,
            document: next_document,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Document;

    #[test]
    fn accepting_twice_is_idempotent_and_preserves_marker() {
        let document = Document::import("doc", "old\n").unwrap();
        let id = document.blocks()[0].id.clone();
        let proposal = BlockProposal::from_document(&document, &id, "new\n").unwrap();
        let mut book = ApprovalBook::new();
        let first = book.accept(&document, &proposal).unwrap();
        assert_eq!(first.outcome, ApprovalOutcome::Applied);
        assert_eq!(first.document.find_block(&id).unwrap().content, "new");
        let second = book.accept(&first.document, &proposal).unwrap();
        assert_eq!(second.outcome, ApprovalOutcome::AlreadyAccepted);
        assert_eq!(second.document.source, first.document.source);
    }

    #[test]
    fn rejection_is_idempotent_and_does_not_write() {
        let document = Document::import("doc", "old\n").unwrap();
        let id = document.blocks()[0].id.clone();
        let proposal = BlockProposal::from_document(&document, &id, "new").unwrap();
        let mut book = ApprovalBook::new();
        let first = book.reject(&document, &proposal).unwrap();
        assert_eq!(first.outcome, ApprovalOutcome::Rejected);
        assert_eq!(first.document.source, document.source);
        let second = book.reject(&first.document, &proposal).unwrap();
        assert_eq!(second.outcome, ApprovalOutcome::AlreadyRejected);
    }

    #[test]
    fn stale_proposal_never_overwrites_user_edit() {
        let document = Document::import("doc", "old\n").unwrap();
        let id = document.blocks()[0].id.clone();
        let proposal = BlockProposal::from_document(&document, &id, "ai").unwrap();
        let user_edit = document.replace_block(&id, "old", "user").unwrap();
        let mut book = ApprovalBook::new();
        let error = book.accept(&user_edit, &proposal).unwrap_err().to_string();
        assert!(error.contains("stale"));
        assert_eq!(user_edit.find_block(&id).unwrap().content, "user");
        assert!(book.entries.is_empty());
    }

    #[test]
    fn opposite_decision_for_same_proposal_is_conflict() {
        let document = Document::import("doc", "old\n").unwrap();
        let id = document.blocks()[0].id.clone();
        let proposal = BlockProposal::from_document(&document, &id, "new").unwrap();
        let mut book = ApprovalBook::new();
        book.reject(&document, &proposal).unwrap();
        let error = book.accept(&document, &proposal).unwrap_err().to_string();
        assert!(error.contains("conflicting approval"));
    }

    #[test]
    fn proposal_and_book_round_trip_json() {
        let document = Document::import("doc", "old\n").unwrap();
        let id = document.blocks()[0].id.clone();
        let proposal = BlockProposal::from_document(&document, &id, "new").unwrap();
        let mut book = ApprovalBook::new();
        book.reject(&document, &proposal).unwrap();
        let proposal_json = serde_json::to_string(&proposal).unwrap();
        let book_json = serde_json::to_string(&book).unwrap();
        let restored: BlockProposal = serde_json::from_str(&proposal_json).unwrap();
        let restored_book: ApprovalBook = serde_json::from_str(&book_json).unwrap();
        assert_eq!(restored, proposal);
        assert_eq!(restored_book, book);
    }

    #[test]
    fn forged_entry_key_cannot_turn_into_an_idempotent_approval() {
        let document = Document::import("doc", "old\n").unwrap();
        let id = document.blocks()[0].id.clone();
        let proposal = BlockProposal::from_document(&document, &id, "new").unwrap();
        let mut entries = BTreeMap::new();
        entries.insert(
            proposal.fingerprint(),
            ApprovalEntry {
                document_id: proposal.document_id.clone(),
                block_id: BlockId::generate().unwrap(),
                before_hash: proposal.before_hash.clone(),
                after_hash: proposal.after_hash.clone(),
                decision: ApprovalDecision::Accepted,
            },
        );
        let mut book = ApprovalBook { entries };
        assert_eq!(book.decision_for(&proposal), None);
        let error = book.accept(&document, &proposal).unwrap_err().to_string();
        assert!(error.contains("corrupt approval entry"));
    }
}

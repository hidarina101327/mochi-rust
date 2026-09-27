//! 稳定的文档块与显式的编辑审批。存储由宿主应用持有；
//! 解析与提案绝不写文档文件。
//!
//! 功能
//!
//! - `sync`：基于 Git 的快照、合并与显式的工作区事务。
//! - `retrieval`：基于 SQLite 的检索与 embedding 提供方集成。
//!
//! 两个 feature 默认都关闭。sync 的仓库与事务 API 可以写文件；
//! 何时调用由宿主决定。
//!
//! # 示例
//!
//! ```
//! use mochi_blocks::approval::ApprovalBook;
//!
//! let book = ApprovalBook::new();
//! assert_eq!(book.entries().count(), 0);
//! ```

pub mod ai;
pub mod approval;
pub mod model;
#[cfg(feature = "retrieval")]
pub mod retrieval;
pub mod slash;
#[cfg(feature = "sync")]
pub mod sync;

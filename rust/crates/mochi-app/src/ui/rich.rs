//! 保源码的富文本事务。偏移永远指向 UTF-8 边界；
//! 隐藏语法绝不当键盘导航的停靠点。

mod editing;
mod mapping;
mod selection;
mod storage;

pub use editing::{apply_format, code_body, compose, delete, enter, indent, insert, insert_parsed};
#[cfg(test)]
use mapping::inline_code_end;
pub use mapping::{body_range, Mapping};
pub use selection::selected_markdown;
pub use storage::UnitList;
use storage::{
    MappingData, UnitBuilder, UnitChunk, UnitData, UnitText, Wrapper, WrapperData, WrapperList,
};
#[allow(unused_imports)]
pub use storage::{TextRef, Unit, UnitIter};

use std::ops::Range;
use std::rc::Rc;

use super::{
    document::{Block, Parsed, RangedBlock},
    editor::TextBuffer,
    text,
};

#[cfg(test)]
mod tests;

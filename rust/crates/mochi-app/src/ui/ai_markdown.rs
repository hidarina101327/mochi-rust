//! 只读的 GFM 布局，用于显示 AI 消息。不访问浏览器或文件，也不写入源码。

mod builder;
mod cache;
mod interaction;
mod models;
mod parsing;

use builder::Builder;
pub use cache::layout;
use cache::Cache;
#[cfg(test)]
use cache::{CACHE_BYTES, CACHE_ENTRIES};
use models::{
    collect_inline_parts, horizontal_distance, inline_parts, inline_runs, utf8_boundary,
    vertical_distance, CopyTarget, InlinePart, Item, Kind, LinkTarget, Node, Tone,
};
pub use models::{
    CopyKind, CopyPayload, Layout, Offsets, ScrollId, ScrollRegion, SelectableText, TextPoint,
};
use parsing::{normalize_math_delimiters, parse, table_payload};

use super::{
    draw::{Align, DrawList, TextStyle},
    highlight,
    icons::Icon,
    layout::Rect,
    text::{self, Emphasis, Run},
    theme::{self, Palette},
};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use std::{cell::RefCell, collections::VecDeque, ops::Range, rc::Rc, sync::OnceLock};

thread_local! {static CACHE:RefCell<Cache>=RefCell::new(Cache::default());}

#[cfg(test)]
mod tests;

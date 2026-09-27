//! 纯 UI 接口，供宿主提供文字排版能力。不允许 Win32 对象穿过此接口。
use super::{draw::TextStyle, text::Emphasis};
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

#[derive(Debug, Clone)]
pub struct Cluster {
    pub start: usize,
    pub end: usize,
    pub leading: f32,
    pub trailing: f32,
    pub advance: f32,
}
#[derive(Debug, Clone, Default)]
pub struct Shape {
    pub width: f32,
    pub clusters: Vec<Cluster>,
}
impl Shape {
    pub fn caret(&self, byte: usize) -> f32 {
        for c in &self.clusters {
            if byte <= c.start {
                return c.leading;
            }
            if byte < c.end {
                return c.leading;
            }
        }
        self.clusters.last().map_or(0.0, |c| c.trailing)
    }
    pub fn hit(&self, x: f32) -> usize {
        self.clusters
            .iter()
            .flat_map(|c| [(c.start, c.leading), (c.end, c.trailing)])
            .min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs()))
            .map_or(0, |c| c.0)
    }
}
pub trait Backend {
    fn shape(&self, text: &str, style: TextStyle, emphasis: Emphasis) -> Option<Rc<Shape>>;
}
thread_local! {static BACKENDS:RefCell<Vec<Weak<dyn Backend>>>=RefCell::new(Vec::new());}
thread_local! {static EPOCH:std::cell::Cell<u64>=const {std::cell::Cell::new(0)};}
/// 渲染器或格式发生变化后，布局缓存不能继续复用旧几何信息。
pub fn invalidate() {
    EPOCH.with(|e| e.set(e.get().wrapping_add(1)));
}
pub fn epoch() -> u64 {
    let _ = backend();
    EPOCH.with(|e| e.get())
}
pub fn register(backend: Rc<dyn Backend>) {
    invalidate();
    BACKENDS.with(|b| {
        let mut b = b.borrow_mut();
        b.retain(|w| w.strong_count() > 0);
        b.push(Rc::downgrade(&backend));
    });
}
fn backend() -> Option<Rc<dyn Backend>> {
    BACKENDS.with(|b| {
        let mut b = b.borrow_mut();
        let old_len = b.len();
        b.retain(|w| w.strong_count() > 0);
        if b.len() != old_len {
            invalidate();
        }
        b.last().and_then(Weak::upgrade)
    })
}
pub fn available() -> bool {
    backend().is_some()
}
pub fn shape(text: &str, style: TextStyle, emphasis: Emphasis) -> Option<Rc<Shape>> {
    backend()?.shape(text, style, emphasis.base())
}

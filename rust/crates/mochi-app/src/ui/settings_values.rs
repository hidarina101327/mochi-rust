//! 描述表到纯 UI 的只读设置快照；不在绘制时访问磁盘。
use mochi_core::app_settings::{self, AppSettings, SettingValue};
use std::{cell::RefCell, collections::HashMap};
thread_local! {static VALUES:RefCell<HashMap<String,SettingValue>>=RefCell::new(HashMap::new());}
thread_local! {static REVISION:std::cell::Cell<u64>=const { std::cell::Cell::new(0) };}
pub fn revision() -> u64 {
    REVISION.with(|value| value.get())
}
fn changed() {
    REVISION.with(|value| value.set(value.get().wrapping_add(1)));
    super::measurement::invalidate();
}
pub fn reset() {
    VALUES.with(|v| v.borrow_mut().clear());
    changed();
}
pub fn load(settings: &AppSettings) {
    VALUES.with(|v| {
        let next = app_settings::descriptors()
            .iter()
            .map(|d| (d.key.clone(), settings.read(d)))
            .collect();
        if *v.borrow() != next {
            *v.borrow_mut() = next;
            changed();
        }
    });
}
pub fn number(key: &str, fallback: f32) -> f32 {
    VALUES.with(|v| match v.borrow().get(key) {
        Some(SettingValue::Number(n)) => *n as f32,
        _ => fallback,
    })
}
pub fn boolean(key: &str, fallback: bool) -> bool {
    VALUES.with(|v| match v.borrow().get(key) {
        Some(SettingValue::Bool(b)) => *b,
        _ => fallback,
    })
}
pub fn text(key: &str, fallback: &str) -> String {
    VALUES.with(|v| {
        v.borrow()
            .get(key)
            .map(|value| value.to_storage())
            .unwrap_or_else(|| fallback.into())
    })
}
pub fn color(key: &str, fallback: u32) -> u32 {
    super::styles::color(&text(key, "")).unwrap_or(fallback)
}

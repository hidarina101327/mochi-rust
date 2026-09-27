//! 把文档区域的键盘输入转换为编辑、移动或忽略结果。
use super::*;

/// `DocPane` 会在鼠标命中、绘制选区和删除时复用同一份排版结果，因此缓存键必须
/// 区分“字节数一样、内容不同”的正文。只按长度复用会让旧行几何映射到新源码，造成
/// 用户看见的绿色选区和实际删除范围不一致。
pub(super) fn content_hash(text: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// 一次按键处理的结果。
pub struct KeyOutcome {
    /// 是否消费掉了这个按键。没消费的要交回 `DefWindowProc`。
    pub handled: bool,
    /// 是否改变了文档文本。导航和选择只需更新叠层/光标，不应使整篇
    /// 文档失效；调用方据此跳过自动保存、字数统计和全量排版。
    pub text_changed: bool,
    /// 是否保留原来的目标横坐标。上下移动时要保留——否则从长行经过短行
    /// 再回来，光标会永久停在短行的行尾。
    pub keep_desired: bool,
}

impl KeyOutcome {
    pub(super) const IGNORED: Self = KeyOutcome {
        handled: false,
        text_changed: false,
        keep_desired: false,
    };
    pub(super) const EDITED: Self = KeyOutcome {
        handled: true,
        text_changed: true,
        keep_desired: false,
    };
    pub(super) const MOVED: Self = KeyOutcome {
        handled: true,
        text_changed: false,
        keep_desired: false,
    };
    pub(super) const MOVED_VERTICALLY: Self = KeyOutcome {
        handled: true,
        text_changed: false,
        keep_desired: true,
    };
}

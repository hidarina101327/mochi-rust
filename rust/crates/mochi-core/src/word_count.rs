//! 字数口径与 shared/word-count.ts 一致：CJK 逐字、拉丁语按词元计数。
//! 假名和谚文也按字符计数，与模型 Token 估算所覆盖的中日韩文字范围不同，不要混为一谈。
pub fn count_words(content: &str) -> usize {
    let mut cjk = 0usize;
    let mut latin = 0usize;
    // 上一个字符是不是词元字符（字母/数字）——用来判断连接符后面还有没有内容
    let mut in_word = false;
    // 已经吃掉一个连接符、还在等后面的字母/数字
    let mut pending_join = false;

    for ch in content.chars() {
        if is_cjk(ch) {
            cjk += 1;
            in_word = false;
            pending_join = false;
            continue;
        }
        if ch.is_ascii_alphanumeric() {
            if !in_word && !pending_join {
                latin += 1;
            }
            in_word = true;
            pending_join = false;
            continue;
        }
        if in_word && matches!(ch, '\'' | '\u{2019}' | '-') {
            // 连接符本身不结束词元，但只有后面紧跟字母/数字才算延续
            in_word = false;
            pending_join = true;
            continue;
        }
        in_word = false;
        pending_join = false;
    }

    cjk + latin
}

/// 对齐 TS 的 `/[㐀-鿿぀-ヿ가-힯]/`。
fn is_cjk(ch: char) -> bool {
    matches!(ch,
        '\u{3400}'..='\u{9fff}'   // 中日韩统一表意文字（含扩展 A）
        | '\u{3040}'..='\u{30ff}' // 平假名 + 片假名
        | '\u{ac00}'..='\u{d7af}' // 谚文音节
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 期望值由 node 跑 `shared/word-count.ts` 的 `countWords` 得出。
    #[test]
    fn matches_the_shared_implementation() {
        assert_eq!(count_words(""), 0);
        assert_eq!(count_words("hello world"), 2);
        assert_eq!(count_words("你好世界"), 4, "CJK 逐字计");
        assert_eq!(count_words("墨池是 AI 原生笔记应用"), 10, "中英混排");
        assert_eq!(count_words("a1 b2 c3"), 3);
        assert_eq!(count_words("   \n\t  "), 0);
    }

    /// 连接符在词元内部不断词，在边缘不成词。
    #[test]
    fn latin_word_tokens_may_contain_apostrophes_and_hyphens() {
        assert_eq!(count_words("don't"), 1);
        assert_eq!(count_words("don’t"), 1, "弯引号同样算词内");
        assert_eq!(count_words("state-of-the-art"), 1);
        assert_eq!(count_words("well-known example"), 2);
        // 结尾的连接符不属于词元，也不该额外产生一个词
        assert_eq!(count_words("hello- world"), 2);
        assert_eq!(count_words("-hello"), 1);
        assert_eq!(count_words("--"), 0);
    }

    #[test]
    fn punctuation_and_symbols_are_not_words() {
        assert_eq!(count_words("!!! ??? ###"), 0);
        assert_eq!(count_words("你好，世界！"), 4, "中文标点不计入");
        assert_eq!(count_words("# 标题\n\n正文"), 4);
    }

    /// 与模型 Token 估算采用的中日韩文字范围不同，这是有意的：假名和谚文按字符计数。
    #[test]
    fn kana_and_hangul_count_as_cjk_characters_here() {
        assert_eq!(count_words("こんにちは"), 5);
        assert_eq!(count_words("안녕하세요"), 5);
        // 对照：Token 估算不会把假名和谚文归入中日韩文字范围。
        assert_eq!(crate::ai::estimate_tokens("こんにちは"), 2);
    }

    #[test]
    fn emoji_and_other_astral_characters_are_not_counted() {
        assert_eq!(count_words("🌸🌸🌸"), 0);
        assert_eq!(count_words("hi 🌸 there"), 2);
    }
}

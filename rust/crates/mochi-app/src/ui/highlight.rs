//! 高亮类别沿用 hljs，颜色对应 editor.css。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Keyword,
    BuiltIn,
    String,
    Comment,
    Number,
    Literal,
    Title,
    Variable,
    Type,
    Meta,
    Attr,
}

/// 一行里的一段着色：行内字节范围。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub kind: Token,
}

/// GitHub Light（editor.css `.ProseMirror pre .hljs-*`）与 adaptive 暗色版的颜色。
/// `auto` 保留这套现有调色板；显式颜色由代码语法高亮设置覆盖。
pub fn color(kind: Token, dark: bool) -> u32 {
    let (key, fallback) = match kind {
        Token::Keyword => (
            "code.syntax.keywordColor",
            if dark { 0xFF7B86 } else { 0xD73A49 },
        ),
        Token::BuiltIn => (
            "code.syntax.builtinColor",
            if dark { 0xFF7B86 } else { 0xD73A49 },
        ),
        Token::String => (
            "code.syntax.stringColor",
            if dark { 0xA8D279 } else { 0x032F62 },
        ),
        Token::Comment => (
            "code.syntax.commentColor",
            if dark { 0x88919F } else { 0x6A737D },
        ),
        Token::Number => (
            "code.syntax.numberColor",
            if dark { 0x79B8FF } else { 0x005CC5 },
        ),
        Token::Literal => (
            "code.syntax.literalColor",
            if dark { 0x79B8FF } else { 0x005CC5 },
        ),
        Token::Title => (
            "code.syntax.functionColor",
            if dark { 0xC9A0FF } else { 0x6F42C1 },
        ),
        Token::Variable => ("code.syntax.variableColor", 0xE36209),
        Token::Type => ("code.syntax.typeColor", 0x22863A),
        Token::Meta => (
            "code.syntax.metaColor",
            if dark { 0x88919F } else { 0x6A737D },
        ),
        Token::Attr => (
            "code.syntax.attributeColor",
            if dark { 0xA8D279 } else { 0x032F62 },
        ),
    };
    super::settings_values::color(key, fallback)
}

/// 注释是否斜体（CSS：comment/quote `font-style: italic`）。
pub fn italic(kind: Token) -> bool {
    kind == Token::Comment
}

/// 一种语言的词法特征。
struct Lang {
    line_comments: &'static [&'static str],
    block_comment: Option<(&'static str, &'static str)>,
    keywords: &'static [&'static str],
    literals: &'static [&'static str],
    builtins: &'static [&'static str],
    /// `#include` / `@Override` 这类整行或前缀式的元信息起始符。
    meta_prefixes: &'static [&'static str],
    /// 字符串引号。
    quotes: &'static [char],
    /// `$name` 视为变量（shell / php）。
    dollar_vars: bool,
    /// 大写开头的标识符视为类型（Rust / Java / C# / Kotlin / Swift / Go / Dart / TS）。
    capital_types: bool,
}

const C_LIKE_KEYWORDS: &[&str] = &[
    "if", "else", "for", "while", "do", "switch", "case", "default", "break", "continue", "return",
    "goto", "sizeof", "typedef", "struct", "union", "enum", "static", "const", "extern", "inline",
    "volatile", "register", "unsigned", "signed", "void", "int", "char", "short", "long", "float",
    "double", "bool", "auto",
];
const CPP_KEYWORDS: &[&str] = &[
    "class",
    "namespace",
    "template",
    "typename",
    "public",
    "private",
    "protected",
    "virtual",
    "override",
    "new",
    "delete",
    "this",
    "using",
    "try",
    "catch",
    "throw",
    "constexpr",
    "nullptr",
    "operator",
    "friend",
    "explicit",
    "noexcept",
    "static_cast",
    "dynamic_cast",
    "reinterpret_cast",
    "const_cast",
];
const JS_KEYWORDS: &[&str] = &[
    "const",
    "let",
    "var",
    "function",
    "return",
    "if",
    "else",
    "for",
    "while",
    "do",
    "switch",
    "case",
    "break",
    "continue",
    "new",
    "delete",
    "typeof",
    "instanceof",
    "in",
    "of",
    "class",
    "extends",
    "super",
    "this",
    "import",
    "export",
    "from",
    "default",
    "async",
    "await",
    "yield",
    "try",
    "catch",
    "finally",
    "throw",
    "void",
    "with",
    "static",
    "get",
    "set",
    "as",
];
const TS_KEYWORDS: &[&str] = &[
    "interface",
    "type",
    "enum",
    "implements",
    "declare",
    "namespace",
    "module",
    "abstract",
    "readonly",
    "keyof",
    "is",
    "public",
    "private",
    "protected",
    "never",
    "unknown",
    "any",
    "string",
    "number",
    "boolean",
    "satisfies",
];
const JS_BUILTINS: &[&str] = &[
    "console", "window", "document", "Math", "JSON", "Object", "Array", "Promise", "Map", "Set",
    "Date", "String", "Number", "Error", "require", "module",
];
const PY_KEYWORDS: &[&str] = &[
    "def", "class", "if", "elif", "else", "for", "while", "return", "import", "from", "as", "with",
    "try", "except", "finally", "raise", "lambda", "yield", "pass", "break", "continue", "in",
    "is", "not", "and", "or", "global", "nonlocal", "del", "assert", "async", "await",
];
const PY_BUILTINS: &[&str] = &[
    "print",
    "len",
    "range",
    "str",
    "int",
    "float",
    "list",
    "dict",
    "set",
    "tuple",
    "open",
    "isinstance",
    "enumerate",
    "zip",
    "map",
    "filter",
    "sorted",
    "self",
    "super",
    "type",
    "input",
];
const RUST_KEYWORDS: &[&str] = &[
    "fn",
    "let",
    "mut",
    "pub",
    "struct",
    "enum",
    "impl",
    "trait",
    "for",
    "in",
    "while",
    "loop",
    "if",
    "else",
    "match",
    "return",
    "use",
    "mod",
    "crate",
    "self",
    "Self",
    "super",
    "as",
    "where",
    "type",
    "const",
    "static",
    "ref",
    "move",
    "async",
    "await",
    "dyn",
    "unsafe",
    "extern",
    "break",
    "continue",
    "macro_rules",
];
const RUST_BUILTINS: &[&str] = &[
    "Some",
    "None",
    "Ok",
    "Err",
    "Vec",
    "String",
    "Box",
    "Option",
    "Result",
    "println",
    "vec",
    "format",
    "panic",
    "assert",
    "assert_eq",
];
const GO_KEYWORDS: &[&str] = &[
    "func",
    "package",
    "import",
    "var",
    "const",
    "type",
    "struct",
    "interface",
    "map",
    "chan",
    "go",
    "defer",
    "if",
    "else",
    "for",
    "range",
    "switch",
    "case",
    "default",
    "break",
    "continue",
    "return",
    "select",
    "fallthrough",
    "goto",
];
const GO_BUILTINS: &[&str] = &[
    "make", "len", "cap", "append", "new", "panic", "recover", "print", "println", "error",
    "string", "int", "int64", "bool", "byte", "float64", "fmt",
];
const JAVA_KEYWORDS: &[&str] = &[
    "public",
    "private",
    "protected",
    "class",
    "interface",
    "extends",
    "implements",
    "static",
    "final",
    "void",
    "new",
    "return",
    "if",
    "else",
    "for",
    "while",
    "do",
    "switch",
    "case",
    "break",
    "continue",
    "try",
    "catch",
    "finally",
    "throw",
    "throws",
    "import",
    "package",
    "this",
    "super",
    "abstract",
    "synchronized",
    "instanceof",
    "enum",
    "int",
    "long",
    "double",
    "float",
    "boolean",
    "char",
    "byte",
    "short",
    "var",
    "record",
    "default",
];
const CS_KEYWORDS: &[&str] = &[
    "using",
    "namespace",
    "class",
    "struct",
    "interface",
    "public",
    "private",
    "protected",
    "internal",
    "static",
    "readonly",
    "void",
    "var",
    "new",
    "return",
    "if",
    "else",
    "for",
    "foreach",
    "in",
    "while",
    "do",
    "switch",
    "case",
    "break",
    "continue",
    "try",
    "catch",
    "finally",
    "throw",
    "async",
    "await",
    "override",
    "virtual",
    "abstract",
    "sealed",
    "this",
    "base",
    "get",
    "set",
    "string",
    "int",
    "bool",
    "double",
    "object",
    "record",
    "is",
    "as",
    "out",
    "ref",
    "params",
    "yield",
    "default",
    "event",
    "delegate",
];
const RUBY_KEYWORDS: &[&str] = &[
    "def",
    "end",
    "class",
    "module",
    "if",
    "elsif",
    "else",
    "unless",
    "while",
    "until",
    "for",
    "in",
    "do",
    "return",
    "yield",
    "begin",
    "rescue",
    "ensure",
    "self",
    "require",
    "include",
    "attr_accessor",
    "puts",
    "then",
    "case",
    "when",
    "and",
    "or",
    "not",
    "lambda",
    "proc",
];
const PHP_KEYWORDS: &[&str] = &[
    "function",
    "class",
    "public",
    "private",
    "protected",
    "static",
    "return",
    "if",
    "else",
    "elseif",
    "foreach",
    "for",
    "while",
    "echo",
    "new",
    "use",
    "namespace",
    "extends",
    "implements",
    "try",
    "catch",
    "throw",
    "array",
    "as",
    "require",
    "include",
    "const",
    "abstract",
    "interface",
];
const SHELL_KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "in", "do", "done", "while", "case", "esac",
    "function", "return", "export", "local", "source", "alias", "exit", "set", "unset", "shift",
];
const SHELL_BUILTINS: &[&str] = &[
    "echo", "cd", "ls", "cat", "grep", "sed", "awk", "rm", "cp", "mv", "mkdir", "chmod", "curl",
    "git", "npm", "cargo", "sudo", "apt", "pip", "python", "node", "docker", "kubectl", "printf",
    "read", "test",
];
const SQL_KEYWORDS: &[&str] = &[
    "select",
    "from",
    "where",
    "insert",
    "into",
    "values",
    "update",
    "set",
    "delete",
    "create",
    "table",
    "drop",
    "alter",
    "add",
    "join",
    "inner",
    "left",
    "right",
    "outer",
    "on",
    "group",
    "by",
    "order",
    "having",
    "limit",
    "offset",
    "as",
    "and",
    "or",
    "not",
    "in",
    "exists",
    "between",
    "like",
    "is",
    "null",
    "distinct",
    "union",
    "all",
    "case",
    "when",
    "then",
    "else",
    "end",
    "primary",
    "key",
    "foreign",
    "references",
    "index",
    "view",
    "with",
    "asc",
    "desc",
    "count",
    "sum",
    "avg",
    "min",
    "max",
];
const KOTLIN_KEYWORDS: &[&str] = &[
    "fun",
    "val",
    "var",
    "class",
    "object",
    "interface",
    "data",
    "sealed",
    "open",
    "override",
    "private",
    "public",
    "internal",
    "protected",
    "if",
    "else",
    "when",
    "for",
    "while",
    "do",
    "return",
    "import",
    "package",
    "in",
    "is",
    "as",
    "try",
    "catch",
    "finally",
    "throw",
    "this",
    "super",
    "null",
    "lateinit",
    "companion",
    "suspend",
    "by",
];
const SWIFT_KEYWORDS: &[&str] = &[
    "func",
    "let",
    "var",
    "class",
    "struct",
    "enum",
    "protocol",
    "extension",
    "import",
    "if",
    "else",
    "guard",
    "for",
    "in",
    "while",
    "repeat",
    "switch",
    "case",
    "default",
    "return",
    "throw",
    "throws",
    "try",
    "catch",
    "self",
    "super",
    "init",
    "deinit",
    "static",
    "private",
    "public",
    "internal",
    "override",
    "mutating",
    "some",
    "any",
    "where",
    "async",
    "await",
];
const DART_KEYWORDS: &[&str] = &[
    "class",
    "extends",
    "implements",
    "with",
    "abstract",
    "final",
    "const",
    "var",
    "void",
    "if",
    "else",
    "for",
    "in",
    "while",
    "do",
    "switch",
    "case",
    "default",
    "return",
    "import",
    "library",
    "part",
    "new",
    "this",
    "super",
    "static",
    "async",
    "await",
    "yield",
    "late",
    "required",
    "get",
    "set",
    "is",
    "as",
    "try",
    "catch",
    "finally",
    "throw",
    "enum",
    "mixin",
    "typedef",
];
const CSS_KEYWORDS: &[&str] = &[
    "important",
    "media",
    "import",
    "keyframes",
    "font-face",
    "supports",
];
const YAML_KEYWORDS: &[&str] = &[];
const NO_WORDS: &[&str] = &[];
const BOOL_LITERALS: &[&str] = &[
    "true",
    "false",
    "null",
    "nil",
    "None",
    "True",
    "False",
    "undefined",
    "NaN",
    "Infinity",
];

fn lang_for(name: &str) -> Option<Lang> {
    let l = name.trim().to_ascii_lowercase();
    let c_like = |keywords: &'static [&'static str],
                  builtins: &'static [&'static str],
                  capital_types: bool| Lang {
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        keywords,
        literals: BOOL_LITERALS,
        builtins,
        meta_prefixes: &["#", "@"],
        quotes: &['"', '\'', '`'],
        dollar_vars: false,
        capital_types,
    };
    Some(match l.as_str() {
        "js" | "javascript" | "jsx" | "mjs" | "cjs" => c_like(JS_KEYWORDS, JS_BUILTINS, false),
        "ts" | "typescript" | "tsx" => Lang {
            keywords: TS_ALL_KEYWORDS.get_or_init(),
            ..c_like(JS_KEYWORDS, JS_BUILTINS, true)
        },
        "py" | "python" | "python3" => Lang {
            line_comments: &["#"],
            block_comment: Some(("\"\"\"", "\"\"\"")),
            keywords: PY_KEYWORDS,
            literals: BOOL_LITERALS,
            builtins: PY_BUILTINS,
            meta_prefixes: &["@"],
            quotes: &['"', '\''],
            dollar_vars: false,
            capital_types: true,
        },
        "rs" | "rust" => Lang {
            meta_prefixes: &["#[", "#!["],
            ..c_like(RUST_KEYWORDS, RUST_BUILTINS, true)
        },
        "go" | "golang" => c_like(GO_KEYWORDS, GO_BUILTINS, true),
        "java" => c_like(JAVA_KEYWORDS, NO_WORDS, true),
        "kt" | "kotlin" => c_like(KOTLIN_KEYWORDS, NO_WORDS, true),
        "swift" => c_like(SWIFT_KEYWORDS, NO_WORDS, true),
        "dart" => c_like(DART_KEYWORDS, NO_WORDS, true),
        "c" | "h" => c_like(C_LIKE_KEYWORDS, NO_WORDS, false),
        "cpp" | "c++" | "cc" | "cxx" | "hpp" => Lang {
            keywords: CPP_ALL_KEYWORDS.get_or_init(),
            ..c_like(C_LIKE_KEYWORDS, NO_WORDS, true)
        },
        "cs" | "csharp" | "c#" => c_like(CS_KEYWORDS, NO_WORDS, true),
        "php" => Lang {
            dollar_vars: true,
            ..c_like(PHP_KEYWORDS, NO_WORDS, true)
        },
        "rb" | "ruby" => Lang {
            line_comments: &["#"],
            block_comment: None,
            keywords: RUBY_KEYWORDS,
            literals: BOOL_LITERALS,
            builtins: NO_WORDS,
            meta_prefixes: &[],
            quotes: &['"', '\''],
            dollar_vars: true,
            capital_types: true,
        },
        "sh" | "bash" | "shell" | "zsh" | "console" | "powershell" | "ps1" | "cmd" => Lang {
            line_comments: &["#"],
            block_comment: None,
            keywords: SHELL_KEYWORDS,
            literals: BOOL_LITERALS,
            builtins: SHELL_BUILTINS,
            meta_prefixes: &["#!"],
            quotes: &['"', '\''],
            dollar_vars: true,
            capital_types: false,
        },
        "json" | "jsonc" | "json5" => Lang {
            line_comments: &["//"],
            block_comment: Some(("/*", "*/")),
            keywords: NO_WORDS,
            literals: BOOL_LITERALS,
            builtins: NO_WORDS,
            meta_prefixes: &[],
            quotes: &['"'],
            dollar_vars: false,
            capital_types: false,
        },
        "yaml" | "yml" | "toml" | "ini" | "conf" | "properties" | "env" => Lang {
            line_comments: &["#", ";"],
            block_comment: None,
            keywords: YAML_KEYWORDS,
            literals: BOOL_LITERALS,
            builtins: NO_WORDS,
            meta_prefixes: &["---", "["],
            quotes: &['"', '\''],
            dollar_vars: false,
            capital_types: false,
        },
        "xml" | "html" | "svg" | "vue" | "xhtml" => Lang {
            line_comments: &[],
            block_comment: Some(("<!--", "-->")),
            keywords: NO_WORDS,
            literals: NO_WORDS,
            builtins: NO_WORDS,
            meta_prefixes: &["<!", "<?"],
            quotes: &['"', '\''],
            dollar_vars: false,
            capital_types: false,
        },
        "css" | "scss" | "less" => Lang {
            line_comments: &["//"],
            block_comment: Some(("/*", "*/")),
            keywords: CSS_KEYWORDS,
            literals: NO_WORDS,
            builtins: NO_WORDS,
            meta_prefixes: &["@"],
            quotes: &['"', '\''],
            dollar_vars: true,
            capital_types: false,
        },
        "sql" | "mysql" | "postgresql" | "postgres" | "sqlite" => Lang {
            line_comments: &["--"],
            block_comment: Some(("/*", "*/")),
            keywords: SQL_KEYWORDS,
            literals: BOOL_LITERALS,
            builtins: NO_WORDS,
            meta_prefixes: &[],
            quotes: &['\'', '"', '`'],
            dollar_vars: false,
            capital_types: false,
        },
        "diff" | "patch" => Lang {
            line_comments: &[],
            block_comment: None,
            keywords: NO_WORDS,
            literals: NO_WORDS,
            builtins: NO_WORDS,
            meta_prefixes: &["@@", "diff", "index", "---", "+++"],
            quotes: &[],
            dollar_vars: false,
            capital_types: false,
        },
        "md" | "markdown" | "txt" | "text" | "plaintext" | "plain" | "" => return None,
        _ => return None,
    })
}

/// 把两张关键字表拼起来的静态缓存（TS = JS + TS 专有；C++ = C + C++ 专有）。
struct Joined(
    std::sync::OnceLock<Vec<&'static str>>,
    &'static [&'static str],
    &'static [&'static str],
);

impl Joined {
    fn get_or_init(&'static self) -> &'static [&'static str] {
        self.0
            .get_or_init(|| self.1.iter().chain(self.2.iter()).copied().collect())
    }
}

static TS_ALL_KEYWORDS: Joined = Joined(std::sync::OnceLock::new(), JS_KEYWORDS, TS_KEYWORDS);
static CPP_ALL_KEYWORDS: Joined = Joined(std::sync::OnceLock::new(), C_LIKE_KEYWORDS, CPP_KEYWORDS);

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// 逐行着色。`lang` 是围栏后的语言名（可为别名或空）。跨行的块注释靠 `in_block` 传递。
/// 不认识的语言返回每行一个空表——按 hljs 的 plaintext 处理。
pub fn highlight(lang: &str, lines: &[String]) -> Vec<Vec<Span>> {
    let Some(l) = lang_for(lang) else {
        return lines.iter().map(|_| Vec::new()).collect();
    };
    let mut in_block = false;
    lines
        .iter()
        .map(|line| highlight_line(&l, line, &mut in_block))
        .collect()
}

fn highlight_line(l: &Lang, line: &str, in_block: &mut bool) -> Vec<Span> {
    let mut spans = Vec::new();
    let bytes = line.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    // 差异标记：整行按首字符着色
    if l.block_comment.is_none() && l.quotes.is_empty() {
        if let Some(first) = line.chars().next() {
            let kind = match first {
                '+' => Some(Token::String),
                '-' => Some(Token::Keyword),
                '@' => Some(Token::Meta),
                _ => None,
            };
            if let Some(k) = kind {
                spans.push(Span {
                    start: 0,
                    end: n,
                    kind: k,
                });
            }
        }
        return spans;
    }
    // 元信息前缀：整行（`#include`、`@Override`、`#[derive]`、shebang）——行首（忽略缩进）判断
    let trimmed_start = line.len() - line.trim_start().len();
    for p in l.meta_prefixes {
        if line[trimmed_start..].starts_with(p) && !(p == &"#" && l.line_comments.contains(&"#")) {
            // `@` 装饰器只到空格/括号为止；其它到行尾
            let end = if *p == "@" {
                line[trimmed_start..]
                    .find(['(', ' '])
                    .map(|e| trimmed_start + e)
                    .unwrap_or(n)
            } else {
                n
            };
            spans.push(Span {
                start: trimmed_start,
                end,
                kind: Token::Meta,
            });
            i = end;
            break;
        }
    }
    while i < n {
        // 块注释（含跨行）
        if *in_block {
            let (_, close) = l.block_comment.unwrap();
            match line[i..].find(close) {
                Some(rel) => {
                    spans.push(Span {
                        start: i,
                        end: i + rel + close.len(),
                        kind: Token::Comment,
                    });
                    i += rel + close.len();
                    *in_block = false;
                    continue;
                }
                None => {
                    spans.push(Span {
                        start: i,
                        end: n,
                        kind: Token::Comment,
                    });
                    return spans;
                }
            }
        }
        let rest = &line[i..];
        let c = rest.chars().next().unwrap();
        if let Some((open, close)) = l.block_comment {
            if let Some(after_open) = rest.strip_prefix(open) {
                match after_open.find(close) {
                    Some(rel) => {
                        let end = i + open.len() + rel + close.len();
                        spans.push(Span {
                            start: i,
                            end,
                            kind: Token::Comment,
                        });
                        i = end;
                    }
                    None => {
                        spans.push(Span {
                            start: i,
                            end: n,
                            kind: Token::Comment,
                        });
                        *in_block = true;
                        return spans;
                    }
                }
                continue;
            }
        }
        if l.line_comments.iter().any(|lc| rest.starts_with(lc)) {
            spans.push(Span {
                start: i,
                end: n,
                kind: Token::Comment,
            });
            return spans;
        }
        if l.quotes.contains(&c) {
            // 字符串到配对引号（支持反斜杠转义）；没配对就到行尾
            let mut j = i + c.len_utf8();
            let mut closed = false;
            let mut escaped = false;
            for (k, ch) in line[j..].char_indices() {
                if escaped {
                    escaped = false;
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    continue;
                }
                if ch == c {
                    j += k + ch.len_utf8();
                    closed = true;
                    break;
                }
            }
            let end = if closed { j } else { n };
            spans.push(Span {
                start: i,
                end,
                kind: Token::String,
            });
            i = end;
            continue;
        }
        if l.dollar_vars && c == '$' {
            let end = line[i + 1..]
                .find(|ch: char| !is_ident(ch) && ch != '{' && ch != '}')
                .map(|e| i + 1 + e)
                .unwrap_or(n);
            if end > i + 1 {
                spans.push(Span {
                    start: i,
                    end,
                    kind: Token::Variable,
                });
                i = end;
                continue;
            }
        }
        if c.is_ascii_digit() {
            let end = line[i..]
                .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '.' || ch == '_'))
                .map(|e| i + e)
                .unwrap_or(n);
            // 前一个字符是标识符的一部分（如 `x1`）就不是数字
            let prev_ident = i > 0 && line[..i].chars().next_back().map(is_ident).unwrap_or(false);
            if !prev_ident {
                spans.push(Span {
                    start: i,
                    end,
                    kind: Token::Number,
                });
            }
            i = end;
            continue;
        }
        if is_ident_start(c) {
            let end = line[i..]
                .find(|ch: char| !is_ident(ch))
                .map(|e| i + e)
                .unwrap_or(n);
            let word = &line[i..end];
            let after = line[end..].trim_start();
            // SQL 关键字不分大小写，其它语言精确匹配
            let sql = std::ptr::eq(l.keywords, SQL_KEYWORDS);
            let kind = if l
                .keywords
                .iter()
                .any(|k| *k == word || (sql && k.eq_ignore_ascii_case(word)))
            {
                Some(Token::Keyword)
            } else if l.literals.contains(&word) {
                Some(Token::Literal)
            } else if l.builtins.contains(&word) {
                Some(Token::BuiltIn)
            } else if after.starts_with('(') {
                Some(Token::Title)
            } else if l.capital_types
                && word
                    .chars()
                    .next()
                    .map(|ch| ch.is_uppercase())
                    .unwrap_or(false)
            {
                Some(Token::Type)
            } else if l.keywords.is_empty() && after.starts_with(':') {
                // YAML / INI 的 `key:`
                Some(Token::Attr)
            } else {
                None
            };
            if let Some(k) = kind {
                spans.push(Span {
                    start: i,
                    end,
                    kind: k,
                });
            }
            i = end;
            continue;
        }
        i += c.len_utf8();
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(lang: &str, line: &str) -> Vec<(String, Token)> {
        highlight(lang, &[line.to_owned()])[0]
            .iter()
            .map(|s| (line[s.start..s.end].to_owned(), s.kind))
            .collect()
    }

    #[test]
    fn rust_keywords_strings_numbers_and_calls_are_classified() {
        let k = kinds("rust", r#"let x: Vec<u8> = foo("a\"b", 42); // done"#);
        assert!(k.contains(&("let".into(), Token::Keyword)));
        assert!(k.contains(&("Vec".into(), Token::BuiltIn)));
        assert!(k.contains(&("foo".into(), Token::Title)));
        assert!(k.contains(&(r#""a\"b""#.into(), Token::String)), "{k:?}");
        assert!(k.contains(&("42".into(), Token::Number)));
        assert!(k.contains(&("// done".into(), Token::Comment)));
    }

    #[test]
    fn block_comments_carry_across_lines_and_meta_lines_are_whole() {
        let out = highlight(
            "c",
            &[
                "#include <stdio.h>".into(),
                "int a; /* start".into(),
                "still comment */ int b;".into(),
            ],
        );
        assert_eq!(
            out[0],
            vec![Span {
                start: 0,
                end: 18,
                kind: Token::Meta
            }]
        );
        assert!(out[1]
            .iter()
            .any(|s| s.kind == Token::Comment && s.start == 7));
        assert_eq!(
            out[2][0],
            Span {
                start: 0,
                end: 16,
                kind: Token::Comment
            }
        );
        assert!(out[2].iter().any(|s| s.kind == Token::Keyword));
    }

    #[test]
    fn python_uses_hash_comments_and_shell_marks_dollar_variables() {
        let k = kinds("py", "def run(n): # go");
        assert!(k.contains(&("def".into(), Token::Keyword)));
        assert!(k.contains(&("run".into(), Token::Title)));
        assert!(k.contains(&("# go".into(), Token::Comment)));
        let s = kinds("bash", "echo $HOME \"hi\"");
        assert!(s.contains(&("echo".into(), Token::BuiltIn)));
        assert!(s.contains(&("$HOME".into(), Token::Variable)));
        assert!(s.contains(&("\"hi\"".into(), Token::String)));
    }

    #[test]
    fn sql_keywords_are_case_insensitive_and_unknown_languages_get_nothing() {
        let k = kinds("sql", "SELECT name FROM users WHERE id = 1");
        assert!(k.contains(&("SELECT".into(), Token::Keyword)));
        assert!(k.contains(&("FROM".into(), Token::Keyword)));
        assert!(k.contains(&("1".into(), Token::Number)));
        assert!(highlight("plaintext", &["let x = 1".into()])[0].is_empty());
        assert!(highlight("", &["let x = 1".into()])[0].is_empty());
    }

    #[test]
    fn diff_lines_color_by_their_first_character_and_colors_follow_the_css() {
        let out = highlight(
            "diff",
            &[
                "+added".into(),
                "-removed".into(),
                "@@ -1 +1 @@".into(),
                " ctx".into(),
            ],
        );
        assert_eq!(out[0][0].kind, Token::String);
        assert_eq!(out[1][0].kind, Token::Keyword);
        assert_eq!(out[2][0].kind, Token::Meta);
        assert!(out[3].is_empty());
        assert_eq!(color(Token::Keyword, false), 0xD73A49);
        assert_eq!(color(Token::String, false), 0x032F62);
        assert_eq!(color(Token::Comment, true), 0x88919F);
        assert!(italic(Token::Comment) && !italic(Token::Keyword));
    }

    #[test]
    fn syntax_color_settings_override_while_auto_keeps_light_dark_defaults() {
        use mochi_core::{
            app_settings::{self, AppSettings, SettingValue},
            settings::SettingsService,
        };
        use std::sync::Arc;

        let settings = AppSettings::new(Arc::new(SettingsService::new(None)));
        let keyword = app_settings::descriptor("code.syntax.keywordColor").unwrap();
        settings.write(keyword, &SettingValue::Text("#123456".into()));
        super::super::settings_values::load(&settings);

        assert_eq!(color(Token::Keyword, true), 0x123456);
        assert_eq!(color(Token::Comment, false), 0x6A737D);
        assert_eq!(color(Token::Comment, true), 0x88919F);
        super::super::settings_values::reset();
    }
}

//! 仅用于 RaTeX 失败或不支持的公式；无法转写的 TeX 命令原样保留。
const SYMBOLS: &[(&str, &str)] = &[
    // 希腊字母
    ("alpha", "α"),
    ("beta", "β"),
    ("gamma", "γ"),
    ("delta", "δ"),
    ("epsilon", "ε"),
    ("varepsilon", "ε"),
    ("zeta", "ζ"),
    ("eta", "η"),
    ("theta", "θ"),
    ("vartheta", "ϑ"),
    ("iota", "ι"),
    ("kappa", "κ"),
    ("lambda", "λ"),
    ("mu", "μ"),
    ("nu", "ν"),
    ("xi", "ξ"),
    ("pi", "π"),
    ("rho", "ρ"),
    ("sigma", "σ"),
    ("tau", "τ"),
    ("upsilon", "υ"),
    ("phi", "φ"),
    ("varphi", "φ"),
    ("chi", "χ"),
    ("psi", "ψ"),
    ("omega", "ω"),
    ("Gamma", "Γ"),
    ("Delta", "Δ"),
    ("Theta", "Θ"),
    ("Lambda", "Λ"),
    ("Xi", "Ξ"),
    ("Pi", "Π"),
    ("Sigma", "Σ"),
    ("Phi", "Φ"),
    ("Psi", "Ψ"),
    ("Omega", "Ω"),
    // 运算与关系
    ("times", "×"),
    ("cdot", "·"),
    ("div", "÷"),
    ("pm", "±"),
    ("mp", "∓"),
    ("leq", "≤"),
    ("le", "≤"),
    ("geq", "≥"),
    ("ge", "≥"),
    ("neq", "≠"),
    ("ne", "≠"),
    ("approx", "≈"),
    ("equiv", "≡"),
    ("sim", "∼"),
    ("simeq", "≃"),
    ("propto", "∝"),
    ("ll", "≪"),
    ("gg", "≫"),
    ("infty", "∞"),
    ("partial", "∂"),
    ("nabla", "∇"),
    ("sum", "∑"),
    ("prod", "∏"),
    ("int", "∫"),
    ("iint", "∬"),
    ("oint", "∮"),
    ("sqrt", "√"),
    ("forall", "∀"),
    ("exists", "∃"),
    ("nexists", "∄"),
    ("in", "∈"),
    ("notin", "∉"),
    ("ni", "∋"),
    ("emptyset", "∅"),
    ("varnothing", "∅"),
    ("cup", "∪"),
    ("cap", "∩"),
    ("subset", "⊂"),
    ("supset", "⊃"),
    ("subseteq", "⊆"),
    ("supseteq", "⊇"),
    ("setminus", "∖"),
    ("land", "∧"),
    ("lor", "∨"),
    ("lnot", "¬"),
    ("neg", "¬"),
    ("oplus", "⊕"),
    ("otimes", "⊗"),
    ("perp", "⊥"),
    ("parallel", "∥"),
    ("angle", "∠"),
    ("triangle", "△"),
    ("degree", "°"),
    ("circ", "∘"),
    ("star", "⋆"),
    ("ast", "∗"),
    ("bullet", "•"),
    ("therefore", "∴"),
    ("because", "∵"),
    ("vdots", "⋮"),
    ("cdots", "⋯"),
    ("ldots", "…"),
    ("dots", "…"),
    ("ddots", "⋱"),
    ("prime", "′"),
    ("hbar", "ℏ"),
    ("ell", "ℓ"),
    ("Re", "ℜ"),
    ("Im", "ℑ"),
    ("aleph", "ℵ"),
    ("wp", "℘"),
    // 箭头
    ("to", "→"),
    ("rightarrow", "→"),
    ("leftarrow", "←"),
    ("leftrightarrow", "↔"),
    ("Rightarrow", "⇒"),
    ("Leftarrow", "⇐"),
    ("Leftrightarrow", "⇔"),
    ("implies", "⟹"),
    ("iff", "⟺"),
    ("mapsto", "↦"),
    ("uparrow", "↑"),
    ("downarrow", "↓"),
    ("longrightarrow", "⟶"),
    ("longleftarrow", "⟵"),
    // 括号与空白
    ("langle", "⟨"),
    ("rangle", "⟩"),
    ("lfloor", "⌊"),
    ("rfloor", "⌋"),
    ("lceil", "⌈"),
    ("rceil", "⌉"),
    ("{", "{"),
    ("}", "}"),
    ("|", "‖"),
    ("quad", "  "),
    ("qquad", "    "),
    (",", " "),
    (";", " "),
    (" ", " "),
    ("!", ""),
    // 函数名（KaTeX 直立体）
    ("sin", "sin"),
    ("cos", "cos"),
    ("tan", "tan"),
    ("log", "log"),
    ("ln", "ln"),
    ("exp", "exp"),
    ("lim", "lim"),
    ("max", "max"),
    ("min", "min"),
    ("sup", "sup"),
    ("inf", "inf"),
    ("det", "det"),
    ("dim", "dim"),
    ("gcd", "gcd"),
    ("arg", "arg"),
    ("deg", "deg"),
];

fn symbol(name: &str) -> Option<&'static str> {
    SYMBOLS.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
}

/// 上标字符（能转的才转；转不了整段退回 `^(…)`）。
fn superscript(s: &str) -> Option<String> {
    s.chars()
        .map(|c| match c {
            '0' => Some('⁰'),
            '1' => Some('¹'),
            '2' => Some('²'),
            '3' => Some('³'),
            '4' => Some('⁴'),
            '5' => Some('⁵'),
            '6' => Some('⁶'),
            '7' => Some('⁷'),
            '8' => Some('⁸'),
            '9' => Some('⁹'),
            '+' => Some('⁺'),
            '-' => Some('⁻'),
            '=' => Some('⁼'),
            '(' => Some('⁽'),
            ')' => Some('⁾'),
            'n' => Some('ⁿ'),
            'i' => Some('ⁱ'),
            'T' => Some('ᵀ'),
            'a' => Some('ᵃ'),
            'b' => Some('ᵇ'),
            'c' => Some('ᶜ'),
            'd' => Some('ᵈ'),
            'e' => Some('ᵉ'),
            'f' => Some('ᶠ'),
            'g' => Some('ᵍ'),
            'h' => Some('ʰ'),
            'j' => Some('ʲ'),
            'k' => Some('ᵏ'),
            'l' => Some('ˡ'),
            'm' => Some('ᵐ'),
            'o' => Some('ᵒ'),
            'p' => Some('ᵖ'),
            'r' => Some('ʳ'),
            's' => Some('ˢ'),
            't' => Some('ᵗ'),
            'u' => Some('ᵘ'),
            'v' => Some('ᵛ'),
            'w' => Some('ʷ'),
            'x' => Some('ˣ'),
            'y' => Some('ʸ'),
            'z' => Some('ᶻ'),
            ' ' => Some(' '),
            '∗' | '*' => Some('*'),
            '′' => Some('′'),
            _ => None,
        })
        .collect()
}

fn subscript(s: &str) -> Option<String> {
    s.chars()
        .map(|c| match c {
            '0' => Some('₀'),
            '1' => Some('₁'),
            '2' => Some('₂'),
            '3' => Some('₃'),
            '4' => Some('₄'),
            '5' => Some('₅'),
            '6' => Some('₆'),
            '7' => Some('₇'),
            '8' => Some('₈'),
            '9' => Some('₉'),
            '+' => Some('₊'),
            '-' => Some('₋'),
            '=' => Some('₌'),
            '(' => Some('₍'),
            ')' => Some('₎'),
            'a' => Some('ₐ'),
            'e' => Some('ₑ'),
            'h' => Some('ₕ'),
            'i' => Some('ᵢ'),
            'j' => Some('ⱼ'),
            'k' => Some('ₖ'),
            'l' => Some('ₗ'),
            'm' => Some('ₘ'),
            'n' => Some('ₙ'),
            'o' => Some('ₒ'),
            'p' => Some('ₚ'),
            'r' => Some('ᵣ'),
            's' => Some('ₛ'),
            't' => Some('ₜ'),
            'u' => Some('ᵤ'),
            'v' => Some('ᵥ'),
            'x' => Some('ₓ'),
            ' ' => Some(' '),
            _ => None,
        })
        .collect()
}

/// 黑板粗体（`\mathbb`）。
fn blackboard(c: char) -> Option<char> {
    Some(match c {
        'R' => 'ℝ',
        'N' => 'ℕ',
        'Z' => 'ℤ',
        'Q' => 'ℚ',
        'C' => 'ℂ',
        'P' => 'ℙ',
        'H' => 'ℍ',
        'A' => '𝔸',
        'B' => '𝔹',
        'D' => '𝔻',
        'E' => '𝔼',
        'F' => '𝔽',
        'G' => '𝔾',
        'I' => '𝕀',
        'J' => '𝕁',
        'K' => '𝕂',
        'L' => '𝕃',
        'M' => '𝕄',
        'O' => '𝕆',
        'S' => '𝕊',
        'T' => '𝕋',
        'U' => '𝕌',
        'V' => '𝕍',
        'W' => '𝕎',
        'X' => '𝕏',
        'Y' => '𝕐',
        _ => return None,
    })
}

/// 读一个参数：`{…}`（配平花括号）或单个字符/命令。返回 (参数内容, 消耗的字符数)。
fn take_arg(chars: &[char], from: usize) -> (String, usize) {
    if from >= chars.len() {
        return (String::new(), 0);
    }
    if chars[from] == '{' {
        let mut depth = 0;
        for (k, c) in chars[from..].iter().enumerate() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        let inner: String = chars[from + 1..from + k].iter().collect();
                        return (inner, k + 1);
                    }
                }
                _ => {}
            }
        }
        // 没配平：吃到末尾
        return (chars[from + 1..].iter().collect(), chars.len() - from);
    }
    if chars[from] == '\\' {
        let mut k = from + 1;
        while k < chars.len() && chars[k].is_ascii_alphabetic() {
            k += 1;
        }
        if k == from + 1 && k < chars.len() {
            k += 1;
        }
        return (chars[from..k].iter().collect(), k - from);
    }
    (chars[from].to_string(), 1)
}

/// 把一段 TeX 转成 Unicode。
pub fn render(tex: &str) -> String {
    render_inner(tex, 0)
}

fn render_inner(tex: &str, depth: usize) -> String {
    if depth >= 32 || tex.len() > 16 * 1024 {
        return tex.chars().take(256).collect();
    }
    let chars: Vec<char> = tex.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' => {
                // 命令名：字母串，或单个符号（`\{`、`\,`）
                let mut k = i + 1;
                while k < chars.len() && chars[k].is_ascii_alphabetic() {
                    k += 1;
                }
                if k == i + 1 && k < chars.len() {
                    k += 1;
                }
                let name: String = chars[i + 1..k].iter().collect();
                i = k;
                match name.as_str() {
                    "frac" | "dfrac" | "tfrac" => {
                        let (num, n1) = take_arg(&chars, i);
                        i += n1;
                        let (den, n2) = take_arg(&chars, i);
                        i += n2;
                        let num = render_inner(&num, depth + 1);
                        let den = render_inner(&den, depth + 1);
                        let wrap = |s: &str| {
                            if s.chars().count() > 1 {
                                format!("({s})")
                            } else {
                                s.to_owned()
                            }
                        };
                        out.push_str(&format!("{}⁄{}", wrap(&num), wrap(&den)));
                    }
                    "sqrt" => {
                        let (arg, n) = take_arg(&chars, i);
                        i += n;
                        let inner = render_inner(&arg, depth + 1);
                        // 根号 + 上划线（组合字符）尽量贴近 KaTeX 的根号
                        out.push('√');
                        for ch in inner.chars() {
                            out.push(ch);
                            out.push('\u{0305}');
                        }
                    }
                    "mathbb" => {
                        let (arg, n) = take_arg(&chars, i);
                        i += n;
                        for ch in arg.chars() {
                            out.push(blackboard(ch).unwrap_or(ch));
                        }
                    }
                    "text" | "mathrm" | "textrm" | "mathbf" | "boldsymbol" | "mathit"
                    | "mathcal" | "operatorname" | "textbf" => {
                        let (arg, n) = take_arg(&chars, i);
                        i += n;
                        out.push_str(&if name == "text" || name == "textrm" || name == "textbf" {
                            arg
                        } else {
                            render_inner(&arg, depth + 1)
                        });
                    }
                    "left" | "right" | "big" | "Big" | "bigl" | "bigr" | "displaystyle"
                    | "textstyle" | "limits" | "nolimits" => {}
                    "hat" | "bar" | "vec" | "dot" | "tilde" | "overline" => {
                        let (arg, n) = take_arg(&chars, i);
                        i += n;
                        let combining = match name.as_str() {
                            "hat" => '\u{0302}',
                            "bar" | "overline" => '\u{0304}',
                            "vec" => '\u{20D7}',
                            "dot" => '\u{0307}',
                            _ => '\u{0303}',
                        };
                        let inner = render_inner(&arg, depth + 1);
                        let mut it = inner.chars();
                        if let Some(first) = it.next() {
                            out.push(first);
                            out.push(combining);
                            out.extend(it);
                        }
                    }
                    _ => match symbol(&name) {
                        Some(s) => out.push_str(s),
                        // 不认识的命令原样保留（KaTeX throwOnError:false 也显示原文）
                        None => {
                            out.push('\\');
                            out.push_str(&name);
                        }
                    },
                }
            }
            '^' | '_' => {
                let (arg, n) = take_arg(&chars, i + 1);
                i += 1 + n;
                let inner = render_inner(&arg, depth + 1);
                let converted = if c == '^' {
                    superscript(&inner)
                } else {
                    subscript(&inner)
                };
                match converted {
                    Some(s) => out.push_str(&s),
                    None => out.push_str(&format!("{c}({inner})")),
                }
            }
            '{' | '}' => i += 1,
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greek_operators_and_relations_become_unicode() {
        assert_eq!(render(r"\alpha + \beta \le \gamma"), "α + β ≤ γ");
        assert_eq!(render(r"a \times b \cdot c \neq \infty"), "a × b · c ≠ ∞");
        assert_eq!(render(r"x \in \mathbb{R}, \forall y"), "x ∈ ℝ, ∀ y");
        assert_eq!(render(r"f: A \to B"), "f: A → B");
    }

    #[test]
    fn scripts_fractions_and_roots_are_transliterated() {
        assert_eq!(render("x^2 + y^{10}"), "x² + y¹⁰");
        assert_eq!(render("a_1 + a_{ij}"), "a₁ + aᵢⱼ");
        assert_eq!(render(r"\frac{1}{2}"), "1⁄2");
        assert_eq!(render(r"\frac{a+b}{c}"), "(a+b)⁄c");
        assert!(render(r"\sqrt{x}").starts_with("√x"));
        assert_eq!(render(r"\sum_{i=1}^{n} i"), "∑ᵢ₌₁ⁿ i");
    }

    #[test]
    fn unknown_commands_and_unconvertible_scripts_fall_back_to_readable_text() {
        assert_eq!(render(r"\foo"), r"\foo");
        assert_eq!(render("x^{Q}"), "x^(Q)");
        assert_eq!(render(r"\text{if } x > 0"), "if  x > 0");
        assert_eq!(render(r"\left( a \right)"), "( a )");
        assert_eq!(render(r"\hat{x}"), "x\u{0302}");
    }
}

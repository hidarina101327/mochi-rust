//! 全局快捷键重映射；仅保存组合字符串，不改变编辑器内置快捷键。
use std::{cell::RefCell, collections::BTreeMap};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    pub key: u16,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}
impl Chord {
    pub fn parse(value: &str) -> Result<Self, String> {
        let mut out = Self {
            key: 0,
            ctrl: false,
            alt: false,
            shift: false,
        };
        for part in value.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => out.ctrl = true,
                "alt" => out.alt = true,
                "shift" => out.shift = true,
                raw => {
                    if out.key != 0 {
                        return Err("只能指定一个主键".into());
                    }
                    out.key = match raw {
                        "space" => 32,
                        "tab" => 9,
                        "enter" => 13,
                        "escape" | "esc" => 27,
                        "backspace" => 8,
                        "delete" => 46,
                        // Electron 按 DOM KeyboardEvent 中的名称保存这些快捷键，
                        // 例如 `Alt+ArrowLeft`。也接受这种写法
                        // 和面向 Win32 的简写形式，避免共享设置
                        // 悄悄丢弃导航快捷键。
                        "arrowleft" | "left" => 0x25,
                        "arrowup" | "up" => 0x26,
                        "arrowright" | "right" => 0x27,
                        "arrowdown" | "down" => 0x28,
                        "\\" => 0xdc,
                        "," => 0xbc,
                        "." | ">" => 0xbe,
                        "/" => 0xbf,
                        "-" => 0xbd,
                        "=" | "plus" => 0xbb,
                        _ if raw.starts_with('f') && raw.len() > 1 => {
                            let n = raw[1..].parse::<u16>().map_err(|_| "功能键无效")?;
                            if !(1..=24).contains(&n) {
                                return Err("功能键范围为 F1–F24".into());
                            }
                            0x6f + n
                        }
                        _ if raw.len() == 1 && raw.as_bytes()[0].is_ascii_alphanumeric() => {
                            raw.as_bytes()[0].to_ascii_uppercase() as u16
                        }
                        _ => return Err("请输入组合，例如 Ctrl+Shift+P 或 Alt+Q".into()),
                    };
                }
            }
        }
        if out.key == 0 || (!out.ctrl && !out.alt) {
            return Err("快捷键必须包含 Ctrl 或 Alt".into());
        }
        if (out.alt && (out.key == 9 || out.key == 0x73))
            || (out.ctrl && out.alt && out.key == 46)
            || (out.ctrl && out.key == 27)
        {
            return Err("该组合由 Windows 保留".into());
        }
        Ok(out)
    }
    pub fn label(self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl".into())
        }
        if self.alt {
            parts.push("Alt".into())
        }
        if self.shift {
            parts.push("Shift".into())
        }
        parts.push(match self.key {
            32 => "Space".into(),
            9 => "Tab".into(),
            13 => "Enter".into(),
            27 => "Esc".into(),
            8 => "Backspace".into(),
            46 => "Delete".into(),
            0x25 => "ArrowLeft".into(),
            0x26 => "ArrowUp".into(),
            0x27 => "ArrowRight".into(),
            0x28 => "ArrowDown".into(),
            0xdc => "\\".into(),
            0xbc => ",".into(),
            0xbe => ".".into(),
            0xbf => "/".into(),
            0xbd => "-".into(),
            0xbb => "=".into(),
            0x70..=0x87 => format!("F{}", self.key - 0x6f),
            key => (key as u8 as char).to_string(),
        });
        parts.join("+")
    }
}
thread_local! {static BINDINGS:RefCell<BTreeMap<String,String>>=const { RefCell::new(BTreeMap::new()) };}
pub fn load(raw: Option<&str>) {
    let values = raw
        .and_then(|s| serde_json::from_str::<BTreeMap<String, String>>(s).ok())
        .unwrap_or_default();
    BINDINGS.with(|b| {
        *b.borrow_mut() = values
            .into_iter()
            .filter(|(key, value)| {
                super::settings::GLOBAL_SHORTCUTS
                    .iter()
                    .any(|(d, _, _)| d == key)
                    && Chord::parse(value).is_ok()
            })
            .collect()
    });
}
pub fn binding(default: &str) -> String {
    BINDINGS.with(|b| {
        b.borrow()
            .get(default)
            .cloned()
            .unwrap_or_else(|| default.into())
    })
}
pub fn prepare(default: &str, value: &str) -> Result<String, String> {
    let chord = Chord::parse(value)?;
    for (key, description, _) in super::settings::GLOBAL_SHORTCUTS {
        if *key != default && Chord::parse(&binding(key)).ok() == Some(chord) {
            return Err(format!("与“{description}”冲突"));
        }
    }
    for (key, description) in super::settings::EDITOR_SHORTCUTS {
        if Chord::parse(key).ok() == Some(chord) {
            return Err(format!("与编辑器“{description}”冲突"));
        }
    }
    BINDINGS.with(|b| {
        let mut next = b.borrow().clone();
        if Chord::parse(default).ok() == Some(chord) {
            next.remove(default);
        } else {
            next.insert(default.into(), chord.label());
        }
        serde_json::to_string(&next).map_err(|e| e.to_string())
    })
}
pub enum Mapping {
    Original(Chord),
    Suppressed,
    Unchanged,
}
pub fn translate(input: Chord) -> Mapping {
    for (key, _, _) in super::settings::GLOBAL_SHORTCUTS {
        if Chord::parse(&binding(key)).ok() == Some(input) {
            if let Ok(original) = Chord::parse(key) {
                return Mapping::Original(original);
            }
        }
    }
    if super::settings::GLOBAL_SHORTCUTS
        .iter()
        .any(|(key, _, _)| Chord::parse(key).ok() == Some(input))
    {
        Mapping::Suppressed
    } else {
        Mapping::Unchanged
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parser_normalizes_modifiers_and_rejects_reserved_keys() {
        assert_eq!(
            Chord::parse("shift + ctrl + p").unwrap().label(),
            "Ctrl+Shift+P"
        );
        assert!(Chord::parse("Alt+F4").is_err());
        assert!(Chord::parse("P").is_err());
        assert!(Chord::parse("Ctrl+A+B").is_err());
        assert_eq!(
            Chord::parse("Alt+ArrowLeft").unwrap().label(),
            "Alt+ArrowLeft"
        );
    }
    #[test]
    fn remapping_suppresses_old_binding_and_preserves_editor_bindings() {
        load(None);
        let raw = prepare("Ctrl+P", "Alt+Q").unwrap();
        load(Some(&raw));
        assert!(matches!(
            translate(Chord::parse("Ctrl+P").unwrap()),
            Mapping::Suppressed
        ));
        assert!(
            matches!(translate(Chord::parse("Alt+Q").unwrap()),Mapping::Original(c)if c==Chord::parse("Ctrl+P").unwrap())
        );
        assert!(prepare("Ctrl+P", "Ctrl+B").is_err());
        load(None);
    }
}

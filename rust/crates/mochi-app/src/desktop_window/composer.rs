//! D2D 负责绘制输入区；Win32 EDIT 仅提供 IME、编辑和剪贴板行为。
use super::*;
use crate::ui::{
    draw::{Align, TextStyle},
    layout::Rect,
    text,
};
use windows::Win32::UI::Input::Ime::{GCS_COMPSTR, GCS_RESULTSTR, ISC_SHOWUICOMPOSITIONWINDOW};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, GetKeyState, SetFocus};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
pub fn rect(b: Rect) -> Rect {
    Rect::new(b.left, b.bottom - 80.0, b.right, b.bottom)
}
pub fn text_rect(b: Rect) -> Rect {
    Rect::new(
        b.left + 12.0,
        b.bottom - 72.0,
        b.right - 12.0,
        b.bottom - 34.0,
    )
}
pub fn send_rect(b: Rect) -> Rect {
    Rect::from_size(b.right - 40.0, b.bottom - 32.0, 28.0, 26.0)
}
pub unsafe fn install(edit: HWND) {
    unsafe {
        let _ = SetWindowSubclass(edit, Some(input_proc), 1, 0);
    }
}
unsafe extern "system" fn input_proc(
    edit: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    unsafe {
        let owner = GetParent(edit).unwrap_or_default();
        if msg == WM_IME_STARTCOMPOSITION {
            let _ = SetPropW(
                edit,
                w!("MochiComposing"),
                Some(windows::Win32::Foundation::HANDLE(1usize as *mut _)),
            );
        }
        if msg == WM_IME_ENDCOMPOSITION || msg == WM_NCDESTROY {
            let _ = RemovePropW(edit, w!("MochiComposing"));
        }
        if msg == WM_IME_SETCONTEXT {
            return DefSubclassProc(
                edit,
                msg,
                wp,
                LPARAM(lp.0 & !(ISC_SHOWUICOMPOSITIONWINDOW as isize)),
            );
        }
        if msg == WM_IME_STARTCOMPOSITION || msg == WM_IME_ENDCOMPOSITION {
            invalidate(owner);
            return LRESULT(0);
        }
        if msg == WM_IME_COMPOSITION {
            if lp.0 as u32 & GCS_RESULTSTR.0 != 0 {
                if let Some(committed) = platform::ime_string(edit, GCS_RESULTSTR.0) {
                    let text: Vec<u16> = committed.encode_utf16().chain([0]).collect();
                    SendMessageW(
                        edit,
                        0x00c2,
                        Some(WPARAM(1)),
                        Some(LPARAM(text.as_ptr() as isize)),
                    );
                }
            }
            invalidate(owner);
            return LRESULT(0);
        }
        let composing = !GetPropW(edit, w!("MochiComposing")).is_invalid();
        if msg == WM_PAINT {
            let _ = ValidateRect(Some(edit), None);
            return LRESULT(0);
        }
        if msg == WM_ERASEBKGND {
            return LRESULT(1);
        }
        if msg == WM_KEYDOWN && wp.0 == 13 && GetKeyState(0x10) >= 0 && !composing {
            super::widgets::send(owner);
            return LRESULT(0);
        }
        if msg == WM_CHAR && wp.0 == 13 && GetKeyState(0x10) >= 0 && !composing {
            return LRESULT(0);
        }
        if msg == WM_KEYDOWN && wp.0 == 65 && GetKeyState(0x11) < 0 {
            SendMessageW(edit, 0x00b1, Some(WPARAM(0)), Some(LPARAM(-1)));
            invalidate(owner);
            return LRESULT(0);
        }
        let result = DefSubclassProc(edit, msg, wp, lp);
        if matches!(
            msg,
            WM_CHAR
                | WM_KEYDOWN
                | WM_KEYUP
                | WM_SETFOCUS
                | WM_KILLFOCUS
                | WM_IME_COMPOSITION
                | WM_IME_ENDCOMPOSITION
                | WM_PASTE
                | WM_CUT
                | WM_SETTEXT
        ) {
            let _ = HideCaret(Some(edit));
            invalidate(owner);
        }
        result
    }
}
fn value(edit: HWND) -> String {
    unsafe {
        let mut buf = vec![0u16; 16001];
        let n = GetWindowTextW(edit, &mut buf);
        String::from_utf16_lossy(&buf[..n as usize])
    }
}
struct Glyph {
    text: String,
    x: f32,
    line: usize,
    index: usize,
    end: usize,
    width: f32,
}
fn glyphs(value: &str, width: f32, size: f32) -> (Vec<Glyph>, usize) {
    let mut out = vec![];
    let mut x = 0.0;
    let mut line = 0;
    let mut index = 0;
    for c in value.chars() {
        let end = index + c.len_utf16();
        if c == '\r' {
            index = end;
            continue;
        }
        if c == '\n' {
            out.push(Glyph {
                text: String::new(),
                x,
                line,
                index,
                end,
                width: 0.0,
            });
            x = 0.0;
            line += 1;
            index = end;
            continue;
        }
        let text = c.to_string();
        let w = text::measure(&text, TextStyle::Body16) * size / 16.0;
        if x + w > width && x > 0.0 {
            line += 1;
            x = 0.0;
        }
        out.push(Glyph {
            text,
            x,
            line,
            index,
            end,
            width: w,
        });
        x += w;
        index = end;
    }
    (out, line)
}
fn caret_at(glyphs: &[Glyph], position: usize) -> (f32, usize) {
    if let Some(g) = glyphs.iter().find(|g| g.index >= position) {
        return (g.x, g.line);
    }
    glyphs
        .last()
        .map(|g| {
            if g.text.is_empty() {
                (0.0, g.line + 1)
            } else {
                (g.x + g.width, g.line)
            }
        })
        .unwrap_or_default()
}
fn selection(edit: HWND) -> (usize, usize) {
    unsafe {
        let mut a = 0u32;
        let mut b = 0u32;
        SendMessageW(
            edit,
            0x00b0,
            Some(WPARAM(&mut a as *mut _ as usize)),
            Some(LPARAM(&mut b as *mut _ as isize)),
        );
        (a as usize, b as usize)
    }
}
fn with_composition(
    value: &str,
    start: usize,
    end: usize,
    preedit: &str,
    cursor: usize,
) -> (String, std::ops::Range<usize>, usize) {
    let start_byte = crate::ui::editor::byte_offset_from_utf16(value, start);
    let end_byte = crate::ui::editor::byte_offset_from_utf16(value, end);
    let text = format!("{}{}{}", &value[..start_byte], preedit, &value[end_byte..]);
    let range = start..start + preedit.encode_utf16().count();
    (
        text,
        range,
        start + preedit[..cursor].encode_utf16().count(),
    )
}
pub fn paint(edit: Option<HWND>, list: &mut DrawList, spec: &Spec, v: &View, a: Rect) {
    if !widgets::has_chat(v) {
        return;
    }
    let b = widgets::chat_body(spec, v, a);
    let r = text_rect(b);
    let (p, _) =
        painting::item_palette(&painting::palette(&widgets::chat_spec(spec, v)), v, "page");
    let size = (widgets::chat_spec(spec, v).appearance.font_size as f32).clamp(12.0, 24.0);
    let lh = size + 6.0;
    let mut value = edit.map(value).unwrap_or_default();
    let focus = edit.is_some_and(|e| unsafe { GetFocus() == e });
    let (mut start, mut end) = edit.map(selection).unwrap_or_default();
    let mut composition = None;
    if let Some(edit) = edit.filter(|e| unsafe { !GetPropW(*e, w!("MochiComposing")).is_invalid() })
    {
        if let Some(preedit) = platform::ime_string(edit, GCS_COMPSTR.0) {
            let cursor = platform::ime_cursor(edit, &preedit);
            let (display, range, caret) = with_composition(&value, start, end, &preedit, cursor);
            value = display;
            composition = Some(range);
            start = caret;
            end = caret;
        }
    }
    if value.is_empty() {
        list.text(r, "发消息…", TextStyle::Label, p.muted);
    }
    let (glyphs, _) = glyphs(&value, r.width(), size);
    let caret = caret_at(&glyphs, end);
    let scroll = ((caret.1 + 1) as f32 * lh - r.height()).max(0.0);
    list.push_clip(r);
    for g in &glyphs {
        let gr = Rect::from_size(
            r.left + g.x,
            r.top + g.line as f32 * lh - scroll,
            g.width + 1.0,
            lh,
        );
        if g.index >= start && g.index < end {
            list.rect(gr, p.border);
        }
        if !gr.intersect(&r).is_empty() && !g.text.is_empty() {
            painting::scaled(list, gr, &g.text, size, p.foreground, Align::Leading);
        }
        if composition.as_ref().is_some_and(|r| r.contains(&g.index)) {
            list.hline(gr.left, gr.right, gr.bottom - 3.0, p.muted);
        }
    }
    if focus && chrono::Local::now().timestamp_millis() % 1000 < 550 {
        let y = r.top + caret.1 as f32 * lh - scroll;
        list.rect(
            Rect::from_size(r.left + caret.0, y + 3.0, 1.0, size + 1.0),
            p.foreground,
        );
    }
    list.pop_clip();
    if focus {
        list.rounded_border(rect(b), 12.0, p.muted);
        if let Some(edit) = edit {
            platform::position_ime(
                edit,
                Rect::from_size(
                    caret.0,
                    caret.1 as f32 * lh - scroll - 16.0,
                    1.0,
                    size + 2.0,
                ),
            );
        }
    }
}
pub fn pointer(hwnd: HWND, x: f32, y: f32, dragging: bool) -> bool {
    unsafe {
        let Some(s) = state(hwnd) else {
            return false;
        };
        if !widgets::has_chat(&s.view) {
            return false;
        }
        let r = text_rect(widgets::chat_body(&s.spec, &s.view, area(hwnd)));
        if !r.contains(x, y) && !dragging {
            return false;
        }
        let x = x.clamp(r.left, r.right);
        let y = y.clamp(r.top, r.bottom);
        let Some(edit) = s.composer else {
            return false;
        };
        let size =
            (widgets::chat_spec(&s.spec, &s.view).appearance.font_size as f32).clamp(12.0, 24.0);
        let (glyphs, _) = glyphs(&value(edit), r.width(), size);
        let (_, end) = selection(edit);
        let active = caret_at(&glyphs, end).1;
        let scroll = ((active + 1) as f32 * (size + 6.0) - r.height()).max(0.0);
        let line = ((y - r.top + scroll) / (size + 6.0)).max(0.0) as usize;
        let index = glyphs
            .iter()
            .filter(|g| g.line == line)
            .find(|g| x - r.left < g.x + g.width / 2.0)
            .map(|g| g.index)
            .or_else(|| {
                glyphs
                    .iter()
                    .filter(|g| g.line == line)
                    .last()
                    .map(|g| g.end)
            })
            .unwrap_or(end);
        if !dragging {
            s.input_anchor = if GetKeyState(0x10) < 0 {
                selection(edit).0
            } else {
                index
            };
        }
        let anchor = s.input_anchor;
        let _ = SetFocus(Some(edit));
        SendMessageW(
            edit,
            0x00b1,
            Some(WPARAM(anchor)),
            Some(LPARAM(index as isize)),
        );
        let _ = HideCaret(Some(edit));
        invalidate(hwnd);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_composer_preedit_replaces_utf16_selection_without_mutating_input() {
        let original = "A😀旧文字";
        let (display, range, caret) = with_composition(original, 3, 4, "你好", 3);
        assert_eq!(display, "A😀你好文字");
        assert_eq!(range, 3..5);
        assert_eq!(caret, 4);
        let (glyphs, _) = glyphs(&display, 500.0, 16.0);
        assert_eq!(glyphs.iter().find(|g| g.text == "😀").unwrap().end, 3);
        assert_eq!(original, "A😀旧文字");
        let (lines, _) = super::glyphs("你好\r\n", 500.0, 16.0);
        assert_eq!(caret_at(&lines, 4), (0.0, 1));
    }
}

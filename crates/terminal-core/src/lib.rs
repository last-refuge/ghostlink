//! Safe ownership and immutable terminal snapshots.

use anyhow::{Result, bail, ensure};
use ghostty_sys as sys;
use std::{ffi::c_void, marker::PhantomData, ops::Range, ptr::NonNull, rc::Rc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSize {
    columns: u16,
    rows: u16,
}

impl GridSize {
    pub fn new(columns: u16, rows: u16) -> Result<Self> {
        ensure!(
            (1..=1000).contains(&columns) && (1..=500).contains(&rows),
            "Terminal size must be 1–1000 columns and 1–500 rows"
        );
        Ok(Self { columns, rows })
    }
    pub fn columns(self) -> u16 {
        self.columns
    }
    pub fn rows(self) -> u16 {
        self.rows
    }
}
impl Default for GridSize {
    fn default() -> Self {
        Self {
            columns: 100,
            rows: 30,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    pub text: String,
    pub foreground: u32,
    pub background: u32,
    /// Zero marks the trailing half of a wide character.
    pub width: u8,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Cursor {
    pub column: u16,
    pub row: u16,
    pub visible: bool,
    pub style: u8,
    pub color: u32,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub size: GridSize,
    pub cells: Vec<Cell>,
    pub cursor: Cursor,
    /// Selected cell columns in each viewport row, with an exclusive end.
    pub selection: Vec<Option<Range<usize>>>,
    /// Also true when the entire selection is outside the visible viewport.
    pub has_selection: bool,
    pub mouse_tracking: bool,
    pub alternate_screen: bool,
    pub alternate_scroll: bool,
}

#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum MouseButton {
    Left = 1,
    Right = 2,
    Middle = 3,
}
#[derive(Clone, Copy, Debug)]
pub enum MouseKind {
    Press(MouseButton),
    Release(MouseButton),
    Move(Option<MouseButton>),
    /// Positive values scroll right/down, negative values left/up.
    Scroll {
        horizontal: i16,
        vertical: i16,
    },
}
#[derive(Clone, Copy, Debug)]
pub struct MouseInput {
    pub kind: MouseKind,
    /// Fractional cell coordinates relative to the terminal text area.
    pub x: f64,
    pub y: f64,
    pub cell_width: u32,
    pub cell_height: u32,
    pub modifiers: Modifiers,
}

#[derive(Clone, Copy, Debug)]
/// Fractional viewport cell coordinates; anchors remain in native scrollback.
pub enum SelectionInput {
    Begin { x: f64, y: f64, clicks: u8 },
    Extend { x: f64, y: f64, scroll: i32 },
    End { x: f64, y: f64 },
    Clear,
}
impl Snapshot {
    pub fn cell(&self, column: u16, row: u16) -> Option<&Cell> {
        if column >= self.size.columns || row >= self.size.rows {
            return None;
        }
        self.cells
            .get(row as usize * self.size.columns as usize + column as usize)
    }
    /// A plain-text viewport for diagnostics. Use `Terminal::copy_selection` for copying.
    pub fn text(&self) -> String {
        self.cells
            .chunks(self.size.columns as usize)
            .map(|row| {
                row.iter()
                    .map(|cell| {
                        if cell.width == 0 {
                            ""
                        } else if cell.text.is_empty() {
                            " "
                        } else {
                            &cell.text
                        }
                    })
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// These identifiers belong to our bridge, never to Ghostty's evolving C enums.
#[derive(Clone, Copy, Debug)]
#[repr(u32)]
pub enum Key {
    Enter = 1,
    Tab,
    Backspace,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Delete,
    Insert,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    A = 100,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub super_key: bool,
}
impl Modifiers {
    fn bits(self) -> u32 {
        u32::from(self.shift)
            | (u32::from(self.control) << 1)
            | (u32::from(self.alt) << 2)
            | (u32::from(self.super_key) << 3)
    }
}

/// All native state stays on its creating worker. No Send/Sync or shared raw handles.
pub struct Terminal {
    raw: NonNull<sys::RawTerminal>,
    size: GridSize,
    _single_owner: PhantomData<Rc<()>>,
}
impl Terminal {
    pub fn new(size: GridSize) -> Result<Self> {
        // SAFETY: checked dimensions; the bridge returns an exclusively owned handle.
        let raw = NonNull::new(unsafe { sys::gl_terminal_new(size.columns, size.rows) })
            .ok_or_else(|| anyhow::anyhow!("libghostty-vt could not allocate a terminal"))?;
        Ok(Self {
            raw,
            size,
            _single_owner: PhantomData,
        })
    }
    /// Process every byte and return terminal-generated replies for the transport.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut replies = Vec::<u8>::new();
        unsafe extern "C" fn write(context: *mut c_void, bytes: *const u8, len: usize) {
            if len == 0 {
                return;
            }
            // SAFETY: the bridge invokes this synchronously; both slices live for the call.
            unsafe {
                (&mut *context.cast::<Vec<u8>>())
                    .extend_from_slice(std::slice::from_raw_parts(bytes, len));
            }
        }
        // SAFETY: exclusive handle, borrowed bytes/context valid until the call returns.
        unsafe {
            sys::gl_terminal_write(
                self.raw.as_ptr(),
                bytes.as_ptr(),
                bytes.len(),
                write,
                (&mut replies as *mut Vec<u8>).cast(),
            );
        }
        replies
    }
    pub fn resize(&mut self, size: GridSize) -> Result<()> {
        // SAFETY: exclusive handle and checked size.
        check(unsafe { sys::gl_terminal_resize(self.raw.as_ptr(), size.columns, size.rows) })?;
        self.size = size;
        Ok(())
    }
    pub fn snapshot(&mut self) -> Result<Snapshot> {
        let mut snapshot = Snapshot {
            size: self.size,
            cells: vec![Cell::default(); self.size.columns as usize * self.size.rows as usize],
            cursor: Cursor::default(),
            selection: vec![None; self.size.rows as usize],
            has_selection: false,
            mouse_tracking: false,
            alternate_screen: false,
            alternate_scroll: false,
        };
        let mut cursor = sys::RawCursor::default();
        let mut has_selection = false;
        let mut modes = 0;
        unsafe extern "C" fn selected(context: *mut c_void, row: u16, start: u16, end: u16) {
            // SAFETY: synchronous callback with the same owned snapshot context.
            let snapshot = unsafe { &mut *context.cast::<Snapshot>() };
            if row < snapshot.size.rows && start <= end && end <= snapshot.size.columns {
                snapshot.selection[row as usize] = Some(start as usize..end as usize);
            }
        }
        unsafe extern "C" fn cell(
            context: *mut c_void,
            column: u16,
            row: u16,
            raw: *const sys::RawCell,
        ) {
            // SAFETY: the callback's references are valid only for this synchronous invocation.
            let (snapshot, raw) = unsafe { (&mut *context.cast::<Snapshot>(), &*raw) };
            if column >= snapshot.size.columns || row >= snapshot.size.rows {
                return;
            }
            let text = if raw.len == 0 {
                String::new()
            } else {
                // SAFETY: bridge owns this initialized codepoint buffer through callback return.
                unsafe { std::slice::from_raw_parts(raw.codepoints, raw.len) }
                    .iter()
                    .map(|&c| char::from_u32(c).unwrap_or('\u{fffd}'))
                    .collect()
            };
            snapshot.cells[row as usize * snapshot.size.columns as usize + column as usize] =
                Cell {
                    text,
                    foreground: raw.foreground,
                    background: raw.background,
                    width: raw.width,
                    bold: raw.flags & 1 != 0,
                    italic: raw.flags & 2 != 0,
                    underline: raw.flags & 4 != 0,
                    strikethrough: raw.flags & 8 != 0,
                };
        }
        // SAFETY: all native pointers remain exclusive; callback data is copied before returning.
        check(unsafe {
            sys::gl_terminal_snapshot(
                self.raw.as_ptr(),
                cell,
                selected,
                (&mut snapshot as *mut Snapshot).cast(),
                &mut cursor,
                &mut has_selection,
                &mut modes,
            )
        })?;
        snapshot.cursor = Cursor {
            column: cursor.column,
            row: cursor.row,
            visible: cursor.visible != 0,
            style: cursor.style,
            color: cursor.color,
        };
        snapshot.has_selection = has_selection;
        snapshot.mouse_tracking = modes & 1 != 0;
        snapshot.alternate_screen = modes & 2 != 0;
        snapshot.alternate_scroll = modes & 4 != 0;
        Ok(snapshot)
    }
    pub fn size(&self) -> GridSize {
        self.size
    }
    pub fn encode_mouse(&mut self, event: MouseInput) -> Result<Vec<u8>> {
        ensure!(
            event.x.is_finite()
                && event.y.is_finite()
                && (1..=1000).contains(&event.cell_width)
                && (1..=1000).contains(&event.cell_height),
            "Invalid terminal mouse geometry"
        );
        let mut output = Vec::new();
        let mut encode = |action, button| -> Result<()> {
            let raw = sys::RawMouse {
                x: event.x.clamp(0., self.size.columns as f64 - 0.001),
                y: event.y.clamp(0., self.size.rows as f64 - 0.001),
                cell_width: event.cell_width,
                cell_height: event.cell_height,
                mods: event.modifiers.bits(),
                action,
                button,
            };
            let mut bytes = [0; 128];
            let mut written = 0;
            // SAFETY: checked geometry and an exclusive native encoder; fixed output capacity.
            check(unsafe {
                sys::gl_terminal_mouse(
                    self.raw.as_ptr(),
                    &raw,
                    bytes.as_mut_ptr(),
                    bytes.len(),
                    &mut written,
                )
            })?;
            ensure!(written <= bytes.len(), "Invalid mouse encoding length");
            output.extend_from_slice(&bytes[..written]);
            Ok(())
        };
        match event.kind {
            MouseKind::Press(button) => encode(0, button as u8)?,
            MouseKind::Release(button) => encode(1, button as u8)?,
            MouseKind::Move(button) => encode(2, button.map_or(0, |b| b as u8))?,
            MouseKind::Scroll {
                horizontal,
                vertical,
            } => {
                for (delta, negative, positive) in [(vertical, 4, 5), (horizontal, 6, 7)] {
                    for _ in 0..delta.unsigned_abs().min(32) {
                        encode(0, if delta < 0 { negative } else { positive })?;
                    }
                }
            }
        }
        Ok(output)
    }
    fn archive(&mut self) -> Result<Vec<u8>> {
        let mut written = 0;
        // SAFETY: synchronous size query against the exclusively owned terminal.
        check(unsafe {
            sys::gl_terminal_archive(self.raw.as_ptr(), std::ptr::null_mut(), 0, &mut written)
        })?;
        ensure!(
            written <= 64 * 1024 * 1024,
            "Terminal history exceeds the 64 MiB reconnect limit"
        );
        if written == 0 {
            return Ok(Vec::new());
        }
        let mut bytes = vec![0; written];
        // SAFETY: no mutations since the size query; output capacity matches allocation.
        check(unsafe {
            sys::gl_terminal_archive(
                self.raw.as_ptr(),
                bytes.as_mut_ptr(),
                bytes.len(),
                &mut written,
            )
        })?;
        ensure!(written <= bytes.len(), "Invalid terminal archive length");
        bytes.truncate(written);
        Ok(bytes)
    }
    /// Start a clean protocol/parser state, retaining styled output in scrollback.
    pub fn prepare_reconnect(&mut self) -> Result<()> {
        let mut fresh = Self::new(self.size)?;
        let alternate = self.snapshot()?.alternate_screen;
        let active = self.archive()?;
        if alternate {
            self.feed(b"\x1b[?1049l");
            fresh.feed(&self.archive()?);
            fresh.feed(b"\r\n\x1b[0m--- Previous remote screen ---\r\n");
        }
        fresh.feed(&active);
        fresh.feed(b"\r\n\x1b[0m--- Reconnected ---\r\n");
        // Protect previous output from the new application's initial screen clear.
        for _ in 0..self.size.rows {
            fresh.feed(b"\r\n");
        }
        fresh.feed(b"\x1b[H");
        *self = fresh;
        Ok(())
    }
    pub fn encode_key(&mut self, key: Key, modifiers: Modifiers) -> Result<Vec<u8>> {
        let mut bytes = vec![0; 256];
        let mut written = 0;
        // SAFETY: output capacity matches the allocated buffer; handle is exclusive.
        check(unsafe {
            sys::gl_terminal_key(
                self.raw.as_ptr(),
                key as u32,
                modifiers.bits(),
                bytes.as_mut_ptr(),
                bytes.len(),
                &mut written,
            )
        })?;
        ensure!(
            written <= bytes.len(),
            "Native encoder returned an invalid length"
        );
        bytes.truncate(written);
        Ok(bytes)
    }
    pub fn encode_paste(&mut self, text: &str) -> Vec<u8> {
        // Normalize Windows clipboard line endings; CR is the terminal's Enter character.
        // Strip ESC to prevent clipboard content escaping a bracketed-paste boundary.
        let text = text
            .replace("\r\n", "\n")
            .replace('\n', "\r")
            .replace('\u{1b}', "");
        // SAFETY: live, exclusively owned handle.
        if unsafe { sys::gl_terminal_bracketed_paste(self.raw.as_ptr()) } {
            [b"\x1b[200~".as_slice(), text.as_bytes(), b"\x1b[201~"].concat()
        } else {
            text.into_bytes()
        }
    }
    pub fn scroll(&mut self, delta: i32) {
        // SAFETY: live, exclusively owned handle.
        unsafe {
            sys::gl_terminal_scroll(self.raw.as_ptr(), delta);
        }
    }
    pub fn select(&mut self, input: SelectionInput) -> Result<()> {
        let (phase, clicks, x, y, scroll) = match input {
            SelectionInput::Begin { x, y, clicks } => (0, clicks.clamp(1, 3), x, y, 0),
            SelectionInput::Extend { x, y, scroll } => (1, 1, x, y, scroll),
            SelectionInput::End { x, y } => (2, 1, x, y, 0),
            SelectionInput::Clear => (3, 1, 0., 0., 0),
        };
        ensure!(
            x.is_finite() && y.is_finite(),
            "Invalid selection coordinates"
        );
        // SAFETY: the exclusive terminal owns the gesture and all tracked refs.
        check(unsafe {
            sys::gl_terminal_select(
                self.raw.as_ptr(),
                phase,
                clicks,
                x.clamp(-1., self.size.columns as f64 + 1.),
                y.clamp(-1., self.size.rows as f64 + 1.),
                scroll,
            )
        })
    }
    pub fn clear_selection(&mut self) -> Result<bool> {
        let mut changed = false;
        // SAFETY: live exclusive handle and correctly sized output.
        check(unsafe { sys::gl_terminal_selection_clear(self.raw.as_ptr(), &mut changed) })?;
        Ok(changed)
    }
    pub fn copy_selection(&mut self) -> Result<Option<String>> {
        let mut size = 0;
        let mut present = false;
        // SAFETY: null buffer is the documented size query; outputs live through the call.
        check(unsafe {
            sys::gl_terminal_selection_copy(
                self.raw.as_ptr(),
                std::ptr::null_mut(),
                0,
                &mut size,
                &mut present,
            )
        })?;
        if !present {
            return Ok(None);
        }
        ensure!(
            size <= 16 * 1024 * 1024,
            "Selection exceeds the 16 MiB clipboard limit"
        );
        if size == 0 {
            return Ok(Some(String::new()));
        }
        let mut bytes = vec![0; size];
        // SAFETY: no terminal mutations occur between query and copy; capacity matches allocation.
        check(unsafe {
            sys::gl_terminal_selection_copy(
                self.raw.as_ptr(),
                bytes.as_mut_ptr(),
                bytes.len(),
                &mut size,
                &mut present,
            )
        })?;
        ensure!(size <= bytes.len(), "Invalid selection copy length");
        bytes.truncate(size);
        Ok(Some(String::from_utf8(bytes)?))
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        // SAFETY: this is the sole owner; no callbacks or borrowed buffers outlive a call.
        unsafe {
            sys::gl_terminal_free(self.raw.as_ptr());
        }
    }
}
fn check(result: i32) -> Result<()> {
    if result == 0 {
        Ok(())
    } else {
        bail!("libghostty-vt operation failed ({result})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn terminal(columns: u16, rows: u16) -> Terminal {
        Terminal::new(GridSize::new(columns, rows).unwrap()).unwrap()
    }
    fn drag(terminal: &mut Terminal, from: (f64, f64), to: (f64, f64), clicks: u8, scroll: i32) {
        terminal
            .select(SelectionInput::Begin {
                x: from.0,
                y: from.1,
                clicks,
            })
            .unwrap();
        terminal
            .select(SelectionInput::Extend {
                x: to.0,
                y: to.1,
                scroll,
            })
            .unwrap();
        terminal
            .select(SelectionInput::End { x: to.0, y: to.1 })
            .unwrap();
    }
    #[test]
    fn selection_copy_unwraps_soft_lines_and_keeps_real_newlines() {
        let mut terminal = terminal(10, 5);
        terminal.feed(b"abcdefghijklm\r\nsecond");
        drag(&mut terminal, (0.1, 0.5), (5.9, 2.5), 1, 0);
        assert_eq!(
            terminal.copy_selection().unwrap().as_deref(),
            Some("abcdefghijklm\nsecond")
        );
        let snapshot = terminal.snapshot().unwrap();
        assert_eq!(snapshot.selection[0], Some(0..10));
        assert_eq!(snapshot.selection[2], Some(0..6));
    }
    #[test]
    fn word_and_logical_line_selection_include_wrapped_content() {
        let mut terminal = terminal(10, 5);
        terminal.feed(b"hello world\r\nnext line");
        terminal
            .select(SelectionInput::Begin {
                x: 7.5,
                y: 0.5,
                clicks: 2,
            })
            .unwrap();
        assert_eq!(terminal.copy_selection().unwrap().as_deref(), Some("world"));
        terminal
            .select(SelectionInput::Begin {
                x: 0.5,
                y: 1.5,
                clicks: 3,
            })
            .unwrap();
        assert_eq!(
            terminal.copy_selection().unwrap().as_deref(),
            Some("hello world")
        );
        drag(&mut terminal, (7.5, 0.5), (6.5, 2.5), 2, 0);
        assert_eq!(
            terminal.copy_selection().unwrap().as_deref(),
            Some("world\nnext line")
        );
        drag(&mut terminal, (0.5, 1.5), (4.5, 2.5), 3, 0);
        assert_eq!(
            terminal.copy_selection().unwrap().as_deref(),
            Some("hello world\nnext line")
        );
        terminal
            .select(SelectionInput::Begin {
                x: 0.5,
                y: 0.5,
                clicks: 1,
            })
            .unwrap();
        assert!(terminal.copy_selection().unwrap().is_none());
    }
    #[test]
    fn selection_extends_through_scrollback_and_survives_viewport_scrolls() {
        let mut terminal = terminal(12, 3);
        terminal.feed(b"lineA\r\nlineB\r\nlineC\r\nlineD\r\nlineE");
        terminal.scroll(-100);
        drag(&mut terminal, (0.1, 0.5), (4.9, 2.5), 1, 100);
        let expected = "lineA\nlineB\nlineC\nlineD\nlineE";
        assert_eq!(
            terminal.copy_selection().unwrap().as_deref(),
            Some(expected)
        );
        terminal.scroll(-100);
        assert_eq!(
            terminal.copy_selection().unwrap().as_deref(),
            Some(expected)
        );
        assert!(terminal.snapshot().unwrap().selection[0].is_some());
    }
    #[test]
    fn selected_content_tracks_output_and_resize_without_crossing_screens() {
        let mut terminal = terminal(12, 3);
        terminal.feed(b"one\r\ntwo\r\nthree");
        drag(&mut terminal, (0.1, 0.5), (2.9, 0.5), 1, 0);
        terminal.feed(b"\r\nfour");
        assert_eq!(terminal.copy_selection().unwrap().as_deref(), Some("one"));
        terminal.resize(GridSize::new(8, 4).unwrap()).unwrap();
        assert_eq!(terminal.copy_selection().unwrap().as_deref(), Some("one"));
        terminal.feed(b"\x1b[?1049hother screen");
        assert!(terminal.copy_selection().unwrap().is_none());
        assert!(
            terminal
                .snapshot()
                .unwrap()
                .selection
                .iter()
                .all(Option::is_none)
        );
    }
    #[test]
    fn selection_copy_preserves_wide_and_combining_graphemes_in_both_directions() {
        let mut terminal = terminal(20, 3);
        let text = "A界e\u{301}👻Z";
        terminal.feed(text.as_bytes());
        drag(&mut terminal, (0.1, 0.5), (6.9, 0.5), 1, 0);
        assert_eq!(terminal.copy_selection().unwrap().as_deref(), Some(text));
        drag(&mut terminal, (6.9, 0.5), (0.1, 0.5), 1, 0);
        assert_eq!(terminal.copy_selection().unwrap().as_deref(), Some(text));
        assert!(terminal.clear_selection().unwrap());
        assert!(terminal.copy_selection().unwrap().is_none());
    }
    #[test]
    fn fragmented_utf8_and_styled_wide_graphemes() {
        let mut terminal = terminal(20, 4);
        let bytes = "\x1b[31mA界e\u{301}👻\x1b[0m".as_bytes();
        for byte in bytes {
            terminal.feed(&[*byte]);
        }
        let snapshot = terminal.snapshot().unwrap();
        assert!(snapshot.text().starts_with("A界e\u{301}👻"));
        assert_eq!(snapshot.cell(1, 0).unwrap().width, 2);
        assert_eq!(snapshot.cell(2, 0).unwrap().width, 0);
        assert_ne!(snapshot.cell(0, 0).unwrap().foreground, 0xdce3df);
    }
    #[test]
    fn queries_return_to_the_transport() {
        let mut terminal = terminal(20, 4);
        assert_eq!(terminal.feed(b"abc\x1b[6n"), b"\x1b[1;4R");
    }
    #[test]
    fn cursor_keys_follow_application_mode() {
        let mut terminal = terminal(20, 4);
        assert_eq!(
            terminal.encode_key(Key::Up, Modifiers::default()).unwrap(),
            b"\x1b[A"
        );
        terminal.feed(b"\x1b[?1h");
        assert_eq!(
            terminal.encode_key(Key::Up, Modifiers::default()).unwrap(),
            b"\x1bOA"
        );
    }
    #[test]
    fn alternate_screen_resize_and_scrollback() {
        let mut terminal = terminal(20, 4);
        terminal.feed(b"main\x1b[?1049hremote");
        assert!(!terminal.snapshot().unwrap().text().contains("main"));
        terminal.resize(GridSize::new(30, 6).unwrap()).unwrap();
        terminal.feed(b"\x1b[?1049l");
        let snapshot = terminal.snapshot().unwrap();
        assert!(snapshot.text().contains("main"));
        assert_eq!(snapshot.cells.len(), 180);
        terminal.feed(b"\r\n1\r\n2\r\n3\r\n4\r\n5\r\n6\r\n7\r\n8");
        terminal.scroll(-100);
        assert!(terminal.snapshot().unwrap().text().contains("main"));
    }
    #[test]
    fn paste_respects_mode_and_cannot_close_its_own_boundary() {
        let mut terminal = terminal(20, 4);
        terminal.feed(b"\x1b[?2004h");
        assert_eq!(
            terminal.encode_paste("a\r\nb\x1b[201~"),
            b"\x1b[200~a\rb[201~\x1b[201~"
        );
    }
    #[test]
    fn rejects_invalid_dimensions() {
        assert!(GridSize::new(0, 24).is_err());
        assert!(GridSize::new(80, 0).is_err());
        assert!(GridSize::new(1001, 24).is_err());
    }
    fn mouse(kind: MouseKind) -> MouseInput {
        MouseInput {
            kind,
            x: 2.2,
            y: 3.8,
            cell_width: 9,
            cell_height: 22,
            modifiers: Modifiers::default(),
        }
    }
    #[test]
    fn mouse_protocols_track_modes_buttons_motion_modifiers_and_wheels() {
        let mut terminal = terminal(300, 40);
        assert!(
            terminal
                .encode_mouse(mouse(MouseKind::Press(MouseButton::Left)))
                .unwrap()
                .is_empty()
        );
        terminal.feed(b"\x1b[?1000h\x1b[?1006h");
        assert!(terminal.snapshot().unwrap().mouse_tracking);
        let mut press = mouse(MouseKind::Press(MouseButton::Left));
        press.modifiers.control = true;
        assert_eq!(terminal.encode_mouse(press).unwrap(), b"\x1b[<16;3;4M");
        assert_eq!(
            terminal
                .encode_mouse(mouse(MouseKind::Release(MouseButton::Left)))
                .unwrap(),
            b"\x1b[<0;3;4m"
        );
        assert!(
            terminal
                .encode_mouse(mouse(MouseKind::Move(None)))
                .unwrap()
                .is_empty()
        );
        terminal.feed(b"\x1b[?1002h");
        let moved = mouse(MouseKind::Move(Some(MouseButton::Left)));
        assert_eq!(terminal.encode_mouse(moved).unwrap(), b"\x1b[<32;3;4M");
        assert!(terminal.encode_mouse(moved).unwrap().is_empty());
        terminal.feed(b"\x1b[?1003h");
        let mut hover = mouse(MouseKind::Move(None));
        hover.x = 4.2;
        assert_eq!(terminal.encode_mouse(hover).unwrap(), b"\x1b[<35;5;4M");
        assert_eq!(
            terminal
                .encode_mouse(mouse(MouseKind::Scroll {
                    horizontal: 1,
                    vertical: -1
                }))
                .unwrap(),
            b"\x1b[<64;3;4M\x1b[<67;3;4M"
        );
        terminal.feed(b"\x1b[?1006l\x1b[?1003l\x1b[?1000h");
        assert_eq!(
            terminal
                .encode_mouse(mouse(MouseKind::Press(MouseButton::Right)))
                .unwrap(),
            b"\x1b[M\x22\x23\x24"
        );
        terminal.feed(b"\x1b[?1005h");
        let mut distant = mouse(MouseKind::Press(MouseButton::Left));
        distant.x = 260.;
        assert_eq!(
            terminal.encode_mouse(distant).unwrap(),
            "\x1b[M \u{125}$".as_bytes()
        );
        terminal.feed(b"\x1b[?1005l\x1b[?1015h");
        assert_eq!(
            terminal
                .encode_mouse(mouse(MouseKind::Press(MouseButton::Middle)))
                .unwrap(),
            b"\x1b[33;3;4M"
        );
        terminal.feed(b"\x1b[?1015l\x1b[?1016h");
        assert_eq!(
            terminal
                .encode_mouse(mouse(MouseKind::Press(MouseButton::Left)))
                .unwrap(),
            b"\x1b[<0;20;84M"
        );
    }
    #[test]
    fn alternate_screen_wheel_falls_back_to_application_cursor_keys() {
        let mut terminal = terminal(80, 24);
        terminal.feed(b"\x1b[?1049h\x1b[?1007h\x1b[?1h");
        assert!(terminal.snapshot().unwrap().alternate_screen);
        assert_eq!(
            terminal
                .encode_mouse(mouse(MouseKind::Scroll {
                    horizontal: 0,
                    vertical: -2
                }))
                .unwrap(),
            b"\x1bOA\x1bOA"
        );
        terminal.feed(b"\x1b[?1007l");
        assert!(
            terminal
                .encode_mouse(mouse(MouseKind::Scroll {
                    horizontal: 0,
                    vertical: 1
                }))
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn reconnect_retains_styled_history_and_resets_parser_and_application_modes() {
        let mut terminal = terminal(30, 5);
        terminal.feed(b"\x1b[31mOLD SHELL\x1b[0m\r\n\x1b[?1049hFULL SCREEN\x1b[?1003h\x1b[?1006h\x1b[?1h\x1b[?2004h\x1b]unfinished title");
        terminal.prepare_reconnect().unwrap();
        terminal.feed(b"\x1b[2J\x1b[HNEW SHELL");
        assert!(terminal.snapshot().unwrap().text().contains("NEW SHELL"));
        assert!(!terminal.snapshot().unwrap().mouse_tracking);
        assert_eq!(
            terminal.encode_key(Key::Up, Modifiers::default()).unwrap(),
            b"\x1b[A"
        );
        assert_eq!(terminal.encode_paste("paste"), b"paste");
        terminal.scroll(-10000);
        let old = terminal.snapshot().unwrap();
        assert!(old.text().contains("OLD SHELL"), "{}", old.text());
        assert_ne!(old.cell(0, 0).unwrap().foreground, 0xdce3df);
        let mut history = String::new();
        for _ in 0..10 {
            history.push_str(&terminal.snapshot().unwrap().text());
            terminal.scroll(1);
        }
        assert!(history.contains("FULL SCREEN"), "{history}");
    }
}

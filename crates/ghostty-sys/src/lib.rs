//! Private C ABI for the pinned libghostty-vt bridge.

use std::ffi::{c_int, c_void};

#[repr(C)]
pub struct RawTerminal {
    _private: [u8; 0],
}

#[repr(C)]
pub struct RawMouse {
    pub x: f64,
    pub y: f64,
    pub cell_width: u32,
    pub cell_height: u32,
    pub mods: u32,
    pub action: u8,
    pub button: u8,
}

#[repr(C)]
pub struct RawCell {
    pub codepoints: *const u32,
    pub len: usize,
    pub foreground: u32,
    pub background: u32,
    pub width: u8,
    pub flags: u8,
}

#[repr(C)]
#[derive(Default)]
pub struct RawCursor {
    pub column: u16,
    pub row: u16,
    pub visible: u8,
    pub style: u8,
    pub color: u32,
}

pub type WriteCallback = unsafe extern "C" fn(*mut c_void, *const u8, usize);
pub type CellCallback = unsafe extern "C" fn(*mut c_void, u16, u16, *const RawCell);
pub type SelectionCallback = unsafe extern "C" fn(*mut c_void, u16, u16, u16);

unsafe extern "C" {
    pub fn gl_terminal_new(columns: u16, rows: u16) -> *mut RawTerminal;
    pub fn gl_terminal_free(terminal: *mut RawTerminal);
    pub fn gl_terminal_write(
        terminal: *mut RawTerminal,
        bytes: *const u8,
        len: usize,
        callback: WriteCallback,
        context: *mut c_void,
    );
    pub fn gl_terminal_set_colors(
        terminal: *mut RawTerminal,
        foreground: u32,
        background: u32,
        cursor: u32,
        palette16: *const u32,
    ) -> c_int;
    pub fn gl_terminal_resize(terminal: *mut RawTerminal, columns: u16, rows: u16) -> c_int;
    pub fn gl_terminal_snapshot(
        terminal: *mut RawTerminal,
        callback: CellCallback,
        selected: SelectionCallback,
        context: *mut c_void,
        cursor: *mut RawCursor,
        has_selection: *mut bool,
        modes: *mut u8,
        background: *mut u32,
    ) -> c_int;
    pub fn gl_terminal_mouse(
        terminal: *mut RawTerminal,
        event: *const RawMouse,
        output: *mut u8,
        capacity: usize,
        written: *mut usize,
    ) -> c_int;
    pub fn gl_terminal_archive(
        terminal: *mut RawTerminal,
        output: *mut u8,
        capacity: usize,
        written: *mut usize,
    ) -> c_int;
    pub fn gl_terminal_key(
        terminal: *mut RawTerminal,
        key: u32,
        mods: u32,
        output: *mut u8,
        capacity: usize,
        written: *mut usize,
    ) -> c_int;
    pub fn gl_terminal_bracketed_paste(terminal: *mut RawTerminal) -> bool;
    pub fn gl_terminal_scroll(terminal: *mut RawTerminal, delta: i32);
    pub fn gl_terminal_select(
        terminal: *mut RawTerminal,
        phase: u8,
        clicks: u8,
        x: f64,
        y: f64,
        scroll: i32,
    ) -> c_int;
    pub fn gl_terminal_selection_clear(terminal: *mut RawTerminal, changed: *mut bool) -> c_int;
    pub fn gl_terminal_selection_copy(
        terminal: *mut RawTerminal,
        buffer: *mut u8,
        capacity: usize,
        written: *mut usize,
        present: *mut bool,
    ) -> c_int;
}

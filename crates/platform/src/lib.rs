//! Shared desktop-platform defaults. These helpers do not create directories.
use std::path::PathBuf;

pub const APP_ID: &str = "io.github.last-refuge.ghostlink";
pub const UI_FONT: &str = if cfg!(target_os = "linux") {
    "DejaVu Sans"
} else {
    ".SystemUIFont"
};
pub const TERMINAL_FONT: &str = if cfg!(windows) {
    "Cascadia Mono"
} else if cfg!(target_os = "macos") {
    "Menlo"
} else {
    "DejaVu Sans Mono"
};

/// Keep the existing Windows location; use Application Support on macOS and
/// XDG_DATA_HOME (with the standard fallback) on Linux.
pub fn data_dir() -> Option<PathBuf> {
    dirs::data_local_dir()
        .filter(|path| path.is_absolute())
        .map(|path| {
            path.join(if cfg!(target_os = "linux") {
                "ghostlink"
            } else {
                "Ghostlink"
            })
        })
}

pub fn home_dir() -> Option<PathBuf> {
    dirs::home_dir().filter(|path| path.is_absolute())
}

/// Command is the native macOS shortcut modifier. Retain Control aliases.
pub fn primary_modifier(control: bool, command: bool) -> bool {
    control || cfg!(target_os = "macos") && command
}

pub fn shortcut_hint(text: &str) -> String {
    if cfg!(target_os = "macos") {
        text.replace("Ctrl", "Cmd")
    } else {
        text.to_owned()
    }
}

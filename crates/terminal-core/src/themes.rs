//! Terminal palettes; sources and license notices: docs/theme-licenses.md.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    #[default]
    Ghostlink,
    SolarizedDark,
    SolarizedLight,
    Dracula,
    Nord,
    GruvboxDark,
    OneDark,
    TokyoNight,
    CatppuccinMocha,
}
#[derive(Clone, Copy, Debug)]
pub struct Colors {
    pub foreground: u32,
    pub background: u32,
    pub cursor: u32,
    pub palette: [u32; 16],
}
impl Theme {
    pub const ALL: &[Self] = &[
        Self::Ghostlink,
        Self::SolarizedDark,
        Self::SolarizedLight,
        Self::Dracula,
        Self::Nord,
        Self::GruvboxDark,
        Self::OneDark,
        Self::TokyoNight,
        Self::CatppuccinMocha,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Ghostlink => "Ghostlink",
            Self::SolarizedDark => "Solarized Dark",
            Self::SolarizedLight => "Solarized Light",
            Self::Dracula => "Dracula",
            Self::Nord => "Nord",
            Self::GruvboxDark => "Gruvbox Dark",
            Self::OneDark => "One Dark",
            Self::TokyoNight => "Tokyo Night",
            Self::CatppuccinMocha => "Catppuccin Mocha",
        }
    }
    pub fn colors(self) -> Colors {
        match self {
            Self::Ghostlink => Colors {
                foreground: 0xdce3df,
                background: 0x111517,
                cursor: 0xa8d8c0,
                palette: [
                    0x1d1f21, 0xcc6666, 0xb5bd68, 0xf0c674, 0x81a2be, 0xb294bb, 0x8abeb7, 0xc5c8c6,
                    0x666666, 0xd54e53, 0xb9ca4a, 0xe7c547, 0x7aa6da, 0xc397d8, 0x70c0b1, 0xeaeaea,
                ],
            },
            Self::SolarizedDark => Colors {
                foreground: 0x839496,
                background: 0x002b36,
                cursor: 0x93a1a1,
                palette: [
                    0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5,
                    0x002b36, 0xcb4b16, 0x586e75, 0x657b83, 0x839496, 0x6c71c4, 0x93a1a1, 0xfdf6e3,
                ],
            },
            Self::SolarizedLight => Colors {
                foreground: 0x657b83,
                background: 0xfdf6e3,
                cursor: 0x586e75,
                palette: [
                    0xeee8d5, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0x073642,
                    0xfdf6e3, 0xcb4b16, 0x93a1a1, 0x839496, 0x657b83, 0x6c71c4, 0x586e75, 0x002b36,
                ],
            },
            Self::Dracula => Colors {
                foreground: 0xf8f8f2,
                background: 0x282a36,
                cursor: 0xf8f8f2,
                palette: [
                    0x21222c, 0xff5555, 0x50fa7b, 0xf1fa8c, 0xbd93f9, 0xff79c6, 0x8be9fd, 0xf8f8f2,
                    0x6272a4, 0xff6e6e, 0x69ff94, 0xffffa5, 0xd6acff, 0xff92df, 0xa4ffff, 0xffffff,
                ],
            },
            Self::Nord => Colors {
                foreground: 0xd8dee9,
                background: 0x2e3440,
                cursor: 0xeceff4,
                palette: [
                    0x3b4252, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x88c0d0, 0xe5e9f0,
                    0x596377, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x8fbcbb, 0xeceff4,
                ],
            },
            Self::GruvboxDark => Colors {
                foreground: 0xebdbb2,
                background: 0x282828,
                cursor: 0xebdbb2,
                palette: [
                    0x282828, 0xcc241d, 0x98971a, 0xd79921, 0x458588, 0xb16286, 0x689d6a, 0xa89984,
                    0x928374, 0xfb4934, 0xb8bb26, 0xfabd2f, 0x83a598, 0xd3869b, 0x8ec07c, 0xebdbb2,
                ],
            },
            Self::OneDark => Colors {
                foreground: 0xabb2bf,
                background: 0x21252b,
                cursor: 0xabb2bf,
                palette: [
                    0x21252b, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xabb2bf,
                    0x767676, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xabb2bf,
                ],
            },
            Self::TokyoNight => Colors {
                foreground: 0xc0caf5,
                background: 0x1a1b26,
                cursor: 0xc0caf5,
                palette: [
                    0x15161e, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6,
                    0x414868, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5,
                ],
            },
            Self::CatppuccinMocha => Colors {
                foreground: 0xcdd6f4,
                background: 0x1e1e2e,
                cursor: 0xf5e0dc,
                palette: [
                    0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de,
                    0x585b70, 0xf7aec2, 0xc2ecbf, 0xfcd682, 0xaeccfc, 0xf398da, 0xb1eae1, 0xa6adc8,
                ],
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GridSize, Terminal};
    #[test]
    fn every_theme_sets_defaults_ansi_cursor_and_keeps_the_256_color_cube() {
        for theme in Theme::ALL {
            let colors = theme.colors();
            let mut terminal = Terminal::new(GridSize::new(20, 3).unwrap()).unwrap();
            terminal.set_theme(*theme).unwrap();
            terminal.feed(b"A\x1b[31mR\x1b[94mB\x1b[38;5;196mX");
            let screen = terminal.snapshot().unwrap();
            assert_eq!(screen.background, colors.background);
            assert_eq!(screen.cells[0].foreground, colors.foreground);
            assert_eq!(screen.cells[0].background, colors.background);
            assert_eq!(screen.cells[1].foreground, colors.palette[1]);
            assert_eq!(screen.cells[2].foreground, colors.palette[12]);
            assert_eq!(screen.cells[3].foreground, 0xff0000);
            assert_eq!(screen.cursor.color, colors.cursor);
            terminal.prepare_reconnect().unwrap();
            terminal.feed(b"\x1b[0mZ");
            let screen = terminal.snapshot().unwrap();
            assert_eq!(screen.background, colors.background);
            assert_eq!(screen.cells[0].foreground, colors.foreground);
        }
    }
    #[test]
    fn terminal_background_tracks_remote_osc_changes_and_reset() {
        let mut terminal = Terminal::new(GridSize::new(20, 3).unwrap()).unwrap();
        terminal.set_theme(Theme::SolarizedLight).unwrap();
        terminal.feed(b"\x1b]11;#123456\x07");
        assert_eq!(terminal.snapshot().unwrap().background, 0x123456);
        terminal.feed(b"\x1b]111\x07");
        assert_eq!(
            terminal.snapshot().unwrap().background,
            Theme::SolarizedLight.colors().background
        );
    }
}

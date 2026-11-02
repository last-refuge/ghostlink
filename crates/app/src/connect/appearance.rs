use super::*;
use crate::host_options::{Charset, Theme};
impl ConnectForm {
    fn charset_index(editor: &Editor) -> usize {
        let name = match &editor.options.charset {
            Charset::Utf8 => "UTF-8",
            Charset::Named(name) => name,
        };
        session::charset::NAMES
            .iter()
            .position(|candidate| *candidate == name)
            .unwrap_or(0)
    }
    fn theme_index(editor: &Editor) -> usize {
        Theme::ALL
            .iter()
            .position(|theme| *theme == editor.options.theme)
            .unwrap_or(0)
    }
    fn choose_charset(editor: &mut Editor, index: usize) {
        let name = session::charset::NAMES[index];
        editor.options.charset = if name == "UTF-8" {
            Charset::Utf8
        } else {
            Charset::Named(name.into())
        };
        editor.charset_open = false;
    }
    pub(super) fn appearance_key(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(editor) = &mut self.editor else {
            return false;
        };
        let charset = editor.charset_focus.is_focused(window);
        let theme = editor.theme_focus.is_focused(window);
        if !charset && !theme {
            return false;
        }
        if key == "tab" {
            editor.charset_open = false;
            editor.theme_open = false;
            return false;
        }
        let length = if charset {
            session::charset::NAMES.len()
        } else {
            Theme::ALL.len()
        };
        let selected = if charset {
            Self::charset_index(editor)
        } else {
            Self::theme_index(editor)
        };
        let (open, cursor, scroll) = if charset {
            (
                &mut editor.charset_open,
                &mut editor.charset_cursor,
                &editor.charset_scroll,
            )
        } else {
            (
                &mut editor.theme_open,
                &mut editor.theme_cursor,
                &editor.theme_scroll,
            )
        };
        match key {
            "enter" | "space" => {
                if *open {
                    let index = *cursor;
                    if charset {
                        Self::choose_charset(editor, index);
                    } else {
                        editor.options.theme = Theme::ALL[index];
                        editor.theme_open = false;
                    }
                } else {
                    *open = true;
                    *cursor = selected;
                    scroll.scroll_to_item(*cursor);
                }
            }
            "up" | "down" | "home" | "end" => {
                if !*open {
                    *open = true;
                    *cursor = selected;
                }
                *cursor = match key {
                    "home" => 0,
                    "end" => length - 1,
                    "up" => cursor.saturating_sub(1),
                    _ => (*cursor + 1).min(length - 1),
                };
                scroll.scroll_to_item(*cursor);
            }
            "escape" if *open => *open = false,
            _ => return false,
        }
        cx.notify();
        true
    }
    pub(super) fn render_appearance(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let Some(editor) = &self.editor else {
            return div().id("appearance-options");
        };
        let charset = match &editor.options.charset {
            Charset::Utf8 => "UTF-8",
            Charset::Named(name) => name,
        };
        div()
            .id("appearance-options")
            .flex()
            .flex_col()
            .gap_2()
            .anchor_scroll(editor.appearance_anchor.clone())
            .child(div().text_xs().child("Character set"))
            .child(
                div()
                    .id("charset-selector")
                    .debug_selector(|| "charset-selector".into())
                    .track_focus(&editor.charset_focus)
                    .px_2()
                    .py_2()
                    .rounded_sm()
                    .bg(rgb(0x171c1f))
                    .border_1()
                    .border_color(rgb(if editor.charset_focus.is_focused(window) {
                        0x71ad91
                    } else {
                        0x293438
                    }))
                    .cursor_pointer()
                    .child(format!("{charset} ▾"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        if let Some(editor) = &mut this.editor {
                            window.focus(&editor.charset_focus);
                            editor.charset_cursor = Self::charset_index(editor);
                            editor.charset_open = !editor.charset_open;
                            editor.theme_open = false;
                            editor.charset_scroll.scroll_to_item(editor.charset_cursor);
                        }
                        cx.notify();
                    })),
            )
            .when(editor.charset_open, |root| {
                root.child(
                    div()
                        .id("charset-options")
                        .track_scroll(&editor.charset_scroll)
                        .max_h(px(170.))
                        .overflow_y_scroll()
                        .children(session::charset::NAMES.iter().enumerate().map(
                            |(index, name)| {
                                let name = *name;
                                div()
                                    .id(("charset-option", index))
                                    .px_2()
                                    .py_1()
                                    .cursor_pointer()
                                    .bg(rgb(if index == editor.charset_cursor {
                                        0x293e36
                                    } else {
                                        0x171c1f
                                    }))
                                    .hover(|style| style.bg(rgb(0x293438)))
                                    .child(name)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if let Some(editor) = &mut this.editor {
                                            Self::choose_charset(editor, index);
                                            window.focus(&editor.charset_focus);
                                        }
                                        cx.notify();
                                    }))
                            },
                        )),
                )
            })
            .child(div().text_xs().child("Terminal theme"))
            .child(
                div()
                    .id("theme-selector")
                    .debug_selector(|| "theme-selector".into())
                    .track_focus(&editor.theme_focus)
                    .px_2()
                    .py_2()
                    .rounded_sm()
                    .border_1()
                    .border_color(rgb(if editor.theme_focus.is_focused(window) {
                        0x71ad91
                    } else {
                        0x293438
                    }))
                    .cursor_pointer()
                    .bg(rgb(editor.options.theme.colors().background))
                    .text_color(rgb(editor.options.theme.colors().foreground))
                    .child(format!("{} ▾", editor.options.theme.name()))
                    .on_click(cx.listener(|this, _, window, cx| {
                        if let Some(editor) = &mut this.editor {
                            window.focus(&editor.theme_focus);
                            editor.theme_cursor = Self::theme_index(editor);
                            editor.theme_open = !editor.theme_open;
                            editor.charset_open = false;
                            editor.theme_scroll.scroll_to_item(editor.theme_cursor);
                        }
                        cx.notify();
                    })),
            )
            .when(editor.theme_open, |root| {
                root.child(
                    div()
                        .id("theme-options")
                        .track_scroll(&editor.theme_scroll)
                        .max_h(px(220.))
                        .overflow_y_scroll()
                        .children(Theme::ALL.iter().enumerate().map(|(index, theme)| {
                            let theme = *theme;
                            let colors = theme.colors();
                            div()
                                .id(("theme-option", index))
                                .px_2()
                                .py_2()
                                .flex()
                                .items_center()
                                .gap_2()
                                .border_l_2()
                                .border_color(rgb(if index == editor.theme_cursor {
                                    colors.foreground
                                } else {
                                    colors.background
                                }))
                                .cursor_pointer()
                                .bg(rgb(colors.background))
                                .text_color(rgb(colors.foreground))
                                .child(div().flex_1().child(theme.name()))
                                .children(colors.palette[1..7].iter().map(|color| {
                                    div().w(px(12.)).h(px(12.)).rounded_sm().bg(rgb(*color))
                                }))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if let Some(editor) = &mut this.editor {
                                        editor.options.theme = theme;
                                        editor.theme_open = false;
                                        window.focus(&editor.theme_focus);
                                    }
                                    cx.notify();
                                }))
                        })),
                )
            })
    }
}

//! Small native-text-input field used by the connection form. Secrets are never copied.
use gpui::Focusable;
use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, ElementInputHandler, EntityInputHandler,
    FocusHandle, KeyDownEvent, MouseButton, Pixels, Point, ShapedLine, SharedString, TextRun,
    UTF16Selection, Window, canvas, div, fill, font, point, prelude::*, px, rgb, size,
};
use std::ops::Range;
use zeroize::Zeroizing;

pub struct Field {
    focus: FocusHandle,
    value: Zeroizing<String>,
    selected: Range<usize>,
    marked: Option<Range<usize>>,
    secret: bool,
    bounds: Option<Bounds<Pixels>>,
    line: Option<ShapedLine>,
    scroll: Pixels,
}
impl Field {
    pub fn new(value: &str, secret: bool, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            value: Zeroizing::new(value.into()),
            selected: value.len()..value.len(),
            marked: None,
            secret,
            bounds: None,
            line: None,
            scroll: px(0.),
        }
    }
    pub fn value(&self) -> &str {
        &self.value
    }
    pub fn set_value(&mut self, value: &str, cx: &mut Context<Self>) {
        self.value = Zeroizing::new(value.to_owned());
        self.selected = value.len()..value.len();
        self.marked = None;
        self.scroll = px(0.);
        cx.notify();
    }
    fn replace(&mut self, range: Range<usize>, text: &str) {
        self.value.replace_range(range.clone(), text);
        self.selected = range.start + text.len()..range.start + text.len();
    }
    fn display_offset(&self, offset: usize) -> usize {
        if self.secret {
            self.value[..offset].chars().count() * '•'.len_utf8()
        } else {
            offset
        }
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let mods = event.keystroke.modifiers;
        if mods.control && key == "a" {
            self.selected = 0..self.value.len();
        } else if mods.control && key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                self.replace(self.selected.clone(), &text.replace(['\r', '\n'], ""));
            }
        } else if mods.control && key == "c" && !self.secret {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.value[self.selected.clone()].to_owned(),
            ));
        } else if self.marked.is_none() {
            let previous = self.value[..self.selected.start]
                .char_indices()
                .next_back()
                .map_or(0, |(index, _)| index);
            let next = self.value[self.selected.end..]
                .chars()
                .next()
                .map_or(self.value.len(), |ch| self.selected.end + ch.len_utf8());
            match key {
                "backspace" => {
                    let range = if self.selected.is_empty() {
                        previous..self.selected.end
                    } else {
                        self.selected.clone()
                    };
                    self.replace(range, "");
                }
                "delete" => {
                    let range = if self.selected.is_empty() {
                        self.selected.start..next
                    } else {
                        self.selected.clone()
                    };
                    self.replace(range, "");
                }
                "left" => {
                    let offset = if self.selected.is_empty() {
                        previous
                    } else {
                        self.selected.start
                    };
                    self.selected = offset..offset;
                }
                "right" => {
                    let offset = if self.selected.is_empty() {
                        next
                    } else {
                        self.selected.end
                    };
                    self.selected = offset..offset;
                }
                "home" => self.selected = 0..0,
                "end" => self.selected = self.value.len()..self.value.len(),
                _ => return,
            }
        } else {
            return;
        }
        window.prevent_default();
        cx.stop_propagation();
        cx.notify();
    }
}
impl Focusable for Field {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for Field {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.focus.is_focused(window);
        let entity = cx.entity();
        let paint_entity = entity.clone();
        div()
            .id("field")
            .h(px(38.))
            .w_full()
            .border_1()
            .rounded_sm()
            .border_color(rgb(if active { 0xa8d8c0 } else { 0x3a4549 }))
            .bg(rgb(0x111517))
            .overflow_hidden()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    window.focus(&this.focus);
                    cx.notify();
                }),
            )
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let field = entity.read(cx);
                        let text: SharedString = if field.secret {
                            "•".repeat(field.value.chars().count()).into()
                        } else {
                            field.value.to_string().into()
                        };
                        let line = window.text_system().shape_line(
                            text.clone(),
                            px(14.),
                            &[TextRun {
                                len: text.len(),
                                font: font("Segoe UI"),
                                color: rgb(0xdce3df).into(),
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            }],
                            None,
                        );
                        let caret = line.x_for_index(field.display_offset(field.selected.end));
                        let selection = (
                            line.x_for_index(field.display_offset(field.selected.start)),
                            caret,
                        );
                        let scroll = (caret - bounds.size.width + px(24.)).max(px(0.));
                        (line, caret, selection, scroll)
                    },
                    move |bounds, (line, caret, selection, scroll), window, cx| {
                        let focus = paint_entity.read(cx).focus.clone();
                        window.handle_input(
                            &focus,
                            ElementInputHandler::new(bounds, paint_entity.clone()),
                            cx,
                        );
                        let origin = bounds.origin + point(px(10.) - scroll, px(7.));
                        window.with_content_mask(Some(ContentMask { bounds }), |window| {
                            if active && selection.0 != selection.1 {
                                window.paint_quad(fill(
                                    Bounds::new(
                                        origin + point(selection.0, px(0.)),
                                        size(selection.1 - selection.0, px(22.)),
                                    ),
                                    rgb(0x334c43),
                                ));
                            }
                            let _ = line.paint(origin, px(22.), window, cx);
                            if active {
                                window.paint_quad(fill(
                                    Bounds::new(
                                        origin + point(caret, px(0.)),
                                        size(px(1.), px(22.)),
                                    ),
                                    rgb(0xa8d8c0),
                                ));
                            }
                        });
                        paint_entity.update(cx, |this, _| {
                            this.bounds = Some(bounds);
                            this.line = Some(line);
                            this.scroll = scroll;
                        });
                    },
                )
                .size_full(),
            )
    }
}
fn byte_offset(text: &str, units: usize) -> usize {
    let mut count = 0;
    for (index, ch) in text.char_indices() {
        if count + ch.len_utf16() > units {
            return index;
        }
        count += ch.len_utf16();
    }
    text.len()
}
impl Field {
    fn utf16_to_bytes(&self, range: Range<usize>) -> Range<usize> {
        let start = byte_offset(&self.value, range.start);
        start..byte_offset(&self.value, range.end).max(start)
    }
    fn to_utf16(&self, range: Range<usize>) -> Range<usize> {
        self.value[..range.start].encode_utf16().count()
            ..self.value[..range.end].encode_utf16().count()
    }
}
impl EntityInputHandler for Field {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        if self.secret {
            return None;
        }
        let bytes = self.utf16_to_bytes(range);
        *actual = Some(self.to_utf16(bytes.clone()));
        Some(self.value[bytes].to_owned())
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.to_utf16(self.selected.clone()),
            reversed: false,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.clone().map(|r| self.to_utf16(r))
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .map(|r| self.utf16_to_bytes(r))
            .or(self.marked.clone())
            .unwrap_or(self.selected.clone());
        self.replace(range, &text.replace(['\r', '\n'], ""));
        self.marked = None;
        cx.notify();
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .map(|r| self.utf16_to_bytes(r))
            .or(self.marked.clone())
            .unwrap_or(self.selected.clone());
        let start = range.start;
        self.replace(range, text);
        self.marked = Some(start..start + text.len());
        if let Some(selected) = selected {
            self.selected =
                start + byte_offset(text, selected.start)..start + byte_offset(text, selected.end);
        }
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.utf16_to_bytes(range);
        let bounds = self.bounds?;
        let line = self.line.as_ref()?;
        Some(Bounds::new(
            bounds.origin
                + point(
                    px(10.) - self.scroll + line.x_for_index(self.display_offset(range.start)),
                    px(7.),
                ),
            size(px(1.), px(22.)),
        ))
    }
    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.to_utf16(self.selected.clone()).end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    #[gpui::test]
    fn terminal_authentication_prompt_receives_text_and_restores_terminal_focus(
        cx: &mut TestAppContext,
    ) {
        let window = cx.add_window(|_, cx| {
            let mut view = crate::TerminalView::from_session(
                Err(anyhow::anyhow!("test view")),
                "test".into(),
                "SSH".into(),
                true,
                cx,
            );
            view.state = session::State::AwaitingCredential(session::CredentialPrompt {
                id: 1,
                label: "Key passphrase".into(),
                detail: "Unlock identity".into(),
                secret: true,
            });
            view.credential = Some((1, cx.new(|cx| Field::new("", true, cx))));
            view.focus_credential = true;
            view
        });
        cx.simulate_keystrokes(*window, "s e c r e t");
        window
            .update(cx, |view, window, cx| {
                let field = &view.credential.as_ref().unwrap().1;
                assert_eq!(field.read(cx).value(), "secret");
                assert!(view.requires_authentication());
                assert!(field.focus_handle(cx).is_focused(window));
            })
            .unwrap();
        cx.simulate_keystrokes(*window, "enter");
        window
            .update(cx, |view, window, _| {
                assert!(view.credential.is_none());
                assert_eq!(view.state, session::State::Authenticating);
                assert!(view.focus.is_focused(window));
            })
            .unwrap();
    }
    #[gpui::test]
    fn secret_editing_and_ime_ranges_preserve_unicode_and_prevent_copy(cx: &mut TestAppContext) {
        let window = cx.add_window(|window, cx| {
            let field = Field::new("a👻b", true, cx);
            window.focus(&field.focus);
            field
        });
        window
            .update(cx, |field, window, cx| {
                field.replace_and_mark_text_in_range(Some(1..3), "界", None, window, cx);
                field.replace_text_in_range(None, "語", window, cx);
                assert_eq!(field.value(), "a語b");
                assert!(field.text_for_range(0..3, &mut None, window, cx).is_none());
                cx.write_to_clipboard(ClipboardItem::new_string("unchanged".into()));
            })
            .unwrap();
        cx.simulate_keystrokes(*window, "ctrl-a ctrl-c");
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            "unchanged"
        );
    }
}

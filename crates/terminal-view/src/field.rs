//! Native text inputs for host details, multiline startup commands, and credential prompts.
//! Secret fields suppress native text retrieval and copying.
use gpui::Focusable;
use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, ElementInputHandler, EntityInputHandler,
    FocusHandle, KeyDownEvent, MouseButton, Pixels, Point, ShapedLine, SharedString, TextRun,
    UTF16Selection, Window, canvas, div, fill, font, point, prelude::*, px, rgb, size,
};
use std::ops::Range;
use zeroize::Zeroizing;

struct FieldLine {
    source: Range<usize>,
    line: ShapedLine,
}

pub struct Field {
    focus: FocusHandle,
    value: Zeroizing<String>,
    selected: Range<usize>,
    marked: Option<Range<usize>>,
    secret: bool,
    bounds: Option<Bounds<Pixels>>,
    lines: Vec<FieldLine>,
    multiline: bool,
    scroll_y: Pixels,
    last_caret: Option<usize>,
    scroll_anchor: Option<gpui::ScrollAnchor>,
    focus_subscription: Option<gpui::Subscription>,
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
            lines: Vec::new(),
            multiline: false,
            scroll_y: px(0.),
            last_caret: None,
            scroll_anchor: None,
            focus_subscription: None,
            scroll: px(0.),
        }
    }
    pub fn new_multiline(value: &str, cx: &mut Context<Self>) -> Self {
        let mut field = Self::new("", false, cx);
        field.multiline = true;
        field.value = Zeroizing::new(field.normalize(value));
        field.selected = field.value.len()..field.value.len();
        field
    }
    pub fn set_scroll_handle(&mut self, handle: gpui::ScrollHandle) {
        if self.scroll_anchor.is_none() {
            self.scroll_anchor = Some(gpui::ScrollAnchor::for_handle(handle));
        }
    }
    pub fn is_multiline(&self) -> bool {
        self.multiline
    }
    fn normalize(&self, text: &str) -> String {
        if self.multiline {
            text.replace("\r\n", "\n").replace('\r', "\n")
        } else {
            text.replace(['\r', '\n'], "")
        }
    }
    fn line_ranges(&self) -> Vec<Range<usize>> {
        if !self.multiline {
            return std::iter::once(0..self.value.len()).collect();
        }
        let mut start = 0;
        self.value
            .split('\n')
            .map(|line| {
                let range = start..start + line.len();
                start += line.len() + 1;
                range
            })
            .collect()
    }
    fn byte_at(&self, position: Point<Pixels>) -> usize {
        let Some(bounds) = self.bounds else {
            return self.selected.end;
        };
        let row = ((f32::from(position.y - bounds.origin.y + self.scroll_y - px(7.)) / 22.)
            .floor()
            .max(0.) as usize)
            .min(self.lines.len().saturating_sub(1));
        let Some(line) = self.lines.get(row) else {
            return self.selected.end;
        };
        let offset = line
            .line
            .closest_index_for_x(position.x - bounds.origin.x + self.scroll - px(10.));
        if self.secret {
            self.value
                .char_indices()
                .nth(offset / '•'.len_utf8())
                .map_or(self.value.len(), |(index, _)| index)
        } else {
            (line.source.start + offset).min(line.source.end)
        }
    }
    pub fn value(&self) -> &str {
        &self.value
    }
    pub fn set_value(&mut self, value: &str, cx: &mut Context<Self>) {
        self.value = Zeroizing::new(self.normalize(value));
        self.selected = self.value.len()..self.value.len();
        self.last_caret = None;
        self.marked = None;
        self.scroll = px(0.);
        self.scroll_y = px(0.);
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
        let primary =
            ghostlink_platform::primary_modifier(mods.control, mods.platform) && !mods.alt;
        if primary && key == "a" {
            self.selected = 0..self.value.len();
        } else if primary && key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                self.replace(self.selected.clone(), &self.normalize(&text));
            }
        } else if primary && key == "c" && !self.secret {
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
                "enter" if self.multiline && !primary && !mods.alt => {
                    self.replace(self.selected.clone(), "\n")
                }
                "up" | "down" if self.multiline => {
                    let ranges = self.line_ranges();
                    let row = ranges
                        .iter()
                        .position(|range| self.selected.end <= range.end)
                        .unwrap_or(ranges.len() - 1);
                    let column = self.value
                        [ranges[row].start..self.selected.end.min(ranges[row].end)]
                        .chars()
                        .count();
                    let next_row = if key == "up" {
                        row.saturating_sub(1)
                    } else {
                        (row + 1).min(ranges.len() - 1)
                    };
                    let range = &ranges[next_row];
                    let offset = self.value[range.clone()]
                        .char_indices()
                        .nth(column)
                        .map_or(range.end, |(offset, _)| range.start + offset);
                    self.selected = offset..offset;
                }
                "home" => {
                    let offset = if self.multiline && !primary {
                        self.value[..self.selected.end]
                            .rfind('\n')
                            .map_or(0, |offset| offset + 1)
                    } else {
                        0
                    };
                    self.selected = offset..offset;
                }
                "end" => {
                    let offset = if self.multiline && !primary {
                        self.value[self.selected.end..]
                            .find('\n')
                            .map_or(self.value.len(), |offset| self.selected.end + offset)
                    } else {
                        self.value.len()
                    };
                    self.selected = offset..offset;
                }
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
        if self.scroll_anchor.is_some() && self.focus_subscription.is_none() {
            self.focus_subscription = Some(cx.on_focus(&self.focus, window, |this, window, cx| {
                if let Some(anchor) = &this.scroll_anchor {
                    anchor.scroll_to(window, cx);
                }
            }));
        }
        let active = self.focus.is_focused(window);
        let entity = cx.entity();
        let paint_entity = entity.clone();
        div()
            .id("field")
            .h(px(if self.multiline { 126. } else { 38. }))
            .w_full()
            .border_1()
            .rounded_sm()
            .border_color(rgb(if active { 0xa8d8c0 } else { 0x3a4549 }))
            .bg(rgb(0x111517))
            .overflow_hidden()
            .track_focus(&self.focus)
            .anchor_scroll(self.scroll_anchor.clone())
            .on_key_down(cx.listener(Self::key))
            .on_scroll_wheel(cx.listener(|this, event: &gpui::ScrollWheelEvent, _, cx| {
                if this.multiline
                    && let Some(bounds) = this.bounds
                {
                    let delta = event.delta.pixel_delta(px(22.));
                    let max =
                        (px(this.lines.len() as f32 * 22. + 14.) - bounds.size.height).max(px(0.));
                    let previous = this.scroll_y;
                    this.scroll_y = (this.scroll_y - delta.y).clamp(px(0.), max);
                    if previous != this.scroll_y {
                        cx.stop_propagation();
                        cx.notify();
                    }
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    window.focus(&this.focus);
                    let offset = this.byte_at(event.position);
                    this.selected = offset..offset;
                    cx.notify();
                }),
            )
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let field = entity.read(cx);
                        let lines = field
                            .line_ranges()
                            .into_iter()
                            .map(|source| {
                                let text: SharedString = if field.secret {
                                    "•"
                                        .repeat(field.value[source.clone()].chars().count())
                                        .into()
                                } else {
                                    field.value[source.clone()].to_owned().into()
                                };
                                let line = window.text_system().shape_line(
                                    text.clone(),
                                    px(14.),
                                    &[TextRun {
                                        len: text.len(),
                                        font: font(ghostlink_platform::UI_FONT),
                                        color: rgb(0xdce3df).into(),
                                        background_color: None,
                                        underline: None,
                                        strikethrough: None,
                                    }],
                                    None,
                                );
                                FieldLine { source, line }
                            })
                            .collect::<Vec<_>>();
                        let row = lines
                            .iter()
                            .position(|line| field.selected.end <= line.source.end)
                            .unwrap_or(lines.len() - 1);
                        let caret_x = lines[row].line.x_for_index(
                            field.display_offset(field.selected.end)
                                - field.display_offset(lines[row].source.start),
                        );
                        let caret_y = px(row as f32 * 22.);
                        let changed = field.last_caret != Some(field.selected.end);
                        let scroll = if changed {
                            if caret_x < field.scroll {
                                caret_x
                            } else {
                                field
                                    .scroll
                                    .max(caret_x - bounds.size.width + px(24.))
                                    .max(px(0.))
                            }
                        } else {
                            field.scroll
                        };
                        let scroll_y = if changed {
                            if caret_y < field.scroll_y {
                                caret_y
                            } else {
                                field
                                    .scroll_y
                                    .max(caret_y - bounds.size.height + px(34.))
                                    .max(px(0.))
                            }
                        } else {
                            field.scroll_y
                        };
                        (lines, row, caret_x, scroll, scroll_y)
                    },
                    move |bounds, (lines, caret_row, caret_x, scroll, scroll_y), window, cx| {
                        let field = paint_entity.read(cx);
                        let focus = field.focus.clone();
                        let selection = field.selected.clone();
                        let selections = lines
                            .iter()
                            .map(|line| {
                                let start =
                                    selection.start.max(line.source.start).min(line.source.end);
                                let end = selection.end.max(line.source.start).min(line.source.end);
                                (
                                    line.line.x_for_index(
                                        field.display_offset(start)
                                            - field.display_offset(line.source.start),
                                    ),
                                    line.line.x_for_index(
                                        field.display_offset(end)
                                            - field.display_offset(line.source.start),
                                    ),
                                    selection.start <= line.source.end
                                        && selection.end > line.source.end,
                                )
                            })
                            .collect::<Vec<_>>();
                        window.handle_input(
                            &focus,
                            ElementInputHandler::new(bounds, paint_entity.clone()),
                            cx,
                        );
                        let origin = bounds.origin + point(px(10.) - scroll, px(7.) - scroll_y);
                        window.with_content_mask(Some(ContentMask { bounds }), |window| {
                            for (index, line) in lines.iter().enumerate() {
                                let line_origin = origin + point(px(0.), px(index as f32 * 22.));
                                if line_origin.y + px(22.) < bounds.origin.y
                                    || line_origin.y > bounds.bottom()
                                {
                                    continue;
                                }
                                let (start, end, newline) = selections[index];
                                if active && (start != end || newline) {
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            line_origin + point(start, px(0.)),
                                            size(
                                                (end - start).max(if newline {
                                                    px(5.)
                                                } else {
                                                    px(0.)
                                                }),
                                                px(22.),
                                            ),
                                        ),
                                        rgb(0x334c43),
                                    ));
                                }
                                let _ = line.line.paint(line_origin, px(22.), window, cx);
                                if active && index == caret_row {
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            line_origin + point(caret_x, px(0.)),
                                            size(px(1.), px(22.)),
                                        ),
                                        rgb(0xa8d8c0),
                                    ));
                                }
                            }
                        });
                        paint_entity.update(cx, |this, _| {
                            this.bounds = Some(bounds);
                            this.lines = lines;
                            this.scroll = scroll;
                            this.scroll_y = scroll_y;
                            this.last_caret = Some(this.selected.end);
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
        self.replace(range, &self.normalize(text));
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
        let text = self.normalize(text);
        let start = range.start;
        self.replace(range, &text);
        self.marked = Some(start..start + text.len());
        if let Some(selected) = selected {
            self.selected = start + byte_offset(&text, selected.start)
                ..start + byte_offset(&text, selected.end);
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
        let row = self
            .lines
            .iter()
            .position(|line| range.start <= line.source.end)?;
        let line = &self.lines[row];
        Some(Bounds::new(
            bounds.origin
                + point(
                    px(10.) - self.scroll
                        + line.line.x_for_index(
                            self.display_offset(range.start)
                                - self.display_offset(line.source.start),
                        ),
                    px(7. + row as f32 * 22.) - self.scroll_y,
                ),
            size(px(1.), px(22.)),
        ))
    }
    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.value[..self.byte_at(position)].encode_utf16().count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    #[gpui::test]
    fn multiline_keyboard_clipboard_and_ime_preserve_line_breaks_and_unicode(
        cx: &mut TestAppContext,
    ) {
        let window = cx.add_window(|window, cx| {
            let field = Field::new_multiline("", cx);
            window.focus(&field.focus);
            field
        });
        cx.write_to_clipboard(ClipboardItem::new_string("alpha\r\n界👻\n".into()));
        cx.simulate_keystrokes(*window, "ctrl-v");
        cx.simulate_keystrokes(*window, "home up end enter");
        cx.simulate_input(*window, "next");
        cx.simulate_keystrokes(*window, "ctrl-a ctrl-c");
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            "alpha\n界👻\nnext\n"
        );
        window
            .update(cx, |field, window, cx| {
                assert_eq!(field.value(), "alpha\n界👻\nnext\n");
                assert_eq!(field.lines.len(), 4);
                let first = field
                    .bounds_for_range(0..0, Bounds::default(), window, cx)
                    .unwrap();
                let second = field
                    .bounds_for_range(6..6, Bounds::default(), window, cx)
                    .unwrap();
                assert_eq!(second.origin.y - first.origin.y, px(22.));
                field.replace_and_mark_text_in_range(Some(6..9), "語", Some(1..1), window, cx);
                assert_eq!(field.value(), "alpha\n語\nnext\n");
                field.replace_text_in_range(None, "!", window, cx);
                assert_eq!(field.value(), "alpha\n!\nnext\n");
            })
            .unwrap();
    }

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

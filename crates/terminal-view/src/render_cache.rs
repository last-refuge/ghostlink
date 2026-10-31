//! Retain shaped rows between snapshots; batch ASCII without moving terminal cells.
use crate::{FONT_SIZE, LINE_HEIGHT, PADDING};
use gpui::{
    App, Bounds, ContentMask, FontFeatures, Pixels, ShapedLine, StrikethroughStyle, TextRun,
    UnderlineStyle, Window, fill, font, point, px, rgb, size,
};
use std::sync::Arc;
use terminal_core::{Cell, Snapshot};

const DEFAULT_BACKGROUND: u32 = 0x111517;

#[derive(Default)]
pub(crate) struct RenderCache {
    screen: Option<Arc<Snapshot>>,
    width: Pixels,
    frame: Arc<Frame>,
}
#[derive(Default)]
pub(crate) struct Frame {
    rows: Vec<Arc<Row>>,
    columns: usize,
}
struct Row {
    backgrounds: Vec<Background>,
    text: Vec<Text>,
}
struct Background {
    column: usize,
    columns: usize,
    color: u32,
}
struct Text {
    column: usize,
    columns: usize,
    line: ShapedLine,
}

impl RenderCache {
    pub fn prepare(
        &mut self,
        screen: &Arc<Snapshot>,
        width: Pixels,
        window: &Window,
    ) -> Arc<Frame> {
        if self.width == width
            && self
                .screen
                .as_ref()
                .is_some_and(|old| Arc::ptr_eq(old, screen))
        {
            return self.frame.clone();
        }
        let columns = screen.size.columns() as usize;
        let previous = self
            .screen
            .as_ref()
            .filter(|old| self.width == width && old.size.columns() == screen.size.columns());
        let rows = screen
            .cells
            .chunks(columns)
            .enumerate()
            .map(|(index, cells)| {
                if previous.is_some_and(|old| {
                    old.cells.get(index * columns..(index + 1) * columns) == Some(cells)
                }) {
                    self.frame.rows[index].clone()
                } else {
                    Arc::new(Row::new(cells, width, window))
                }
            })
            .collect();
        self.width = width;
        self.screen = Some(screen.clone());
        self.frame = Arc::new(Frame { rows, columns });
        self.frame.clone()
    }
}

fn same_style(a: &Cell, b: &Cell) -> bool {
    a.foreground == b.foreground
        && a.bold == b.bold
        && a.italic == b.italic
        && a.underline == b.underline
        && a.strikethrough == b.strikethrough
}
fn ascii_cell(cell: &Cell) -> bool {
    cell.width == 1
        && cell.text.len() == 1
        && (cell.text.as_bytes()[0].is_ascii_graphic() || cell.text == " ")
}
impl Row {
    fn new(cells: &[Cell], width: Pixels, window: &Window) -> Self {
        let mut backgrounds: Vec<Background> = Vec::new();
        for (column, cell) in cells.iter().enumerate() {
            if cell.background == DEFAULT_BACKGROUND {
                continue;
            }
            if let Some(last) = backgrounds.last_mut()
                && last.color == cell.background
                && last.column + last.columns == column
            {
                last.columns += 1;
            } else {
                backgrounds.push(Background {
                    column,
                    columns: 1,
                    color: cell.background,
                });
            }
        }
        let mut text = Vec::new();
        let mut column = 0;
        while column < cells.len() {
            let cell = &cells[column];
            if cell.width == 0 || cell.text.is_empty() {
                column += 1;
                continue;
            }
            let mut content = cell.text.clone();
            let mut end = column + 1;
            let ascii = ascii_cell(cell);
            if ascii {
                while end < cells.len() && ascii_cell(&cells[end]) && same_style(cell, &cells[end])
                {
                    content.push_str(&cells[end].text);
                    end += 1;
                }
            }
            let mut face = font("Cascadia Mono");
            // Per-cell rendering did not form ligatures. Keep that behavior while batching.
            face.features = FontFeatures(Arc::new(vec![("calt".into(), 0), ("liga".into(), 0)]));
            if cell.bold {
                face = face.bold();
            }
            if cell.italic {
                face = face.italic();
            }
            let line = window.text_system().shape_line(
                content.clone().into(),
                px(FONT_SIZE),
                &[TextRun {
                    len: content.len(),
                    font: face,
                    color: rgb(cell.foreground).into(),
                    background_color: None,
                    underline: cell.underline.then_some(UnderlineStyle {
                        thickness: px(1.),
                        color: None,
                        wavy: false,
                    }),
                    strikethrough: cell.strikethrough.then_some(StrikethroughStyle {
                        thickness: px(1.),
                        color: None,
                    }),
                }],
                ascii.then_some(width),
            );
            // Non-ASCII graphemes retain the parser's exact one- or two-cell placement.
            text.push(Text {
                column,
                columns: if ascii {
                    end - column
                } else {
                    cell.width as usize
                },
                line,
            });
            column = end;
        }
        Self { backgrounds, text }
    }
}
impl Frame {
    pub fn paint(
        &self,
        bounds: Bounds<Pixels>,
        width: Pixels,
        selection: &[Option<std::ops::Range<usize>>],
        window: &mut Window,
        cx: &mut App,
    ) {
        for (row, cached) in self.rows.iter().enumerate() {
            let origin = bounds.origin + point(px(PADDING), px(PADDING + row as f32 * LINE_HEIGHT));
            for background in &cached.backgrounds {
                window.paint_quad(fill(
                    Bounds::new(
                        origin + point(width * background.column as f32, px(0.)),
                        size(width * background.columns as f32, px(LINE_HEIGHT)),
                    ),
                    rgb(background.color),
                ));
            }
            if let Some(Some(range)) = selection.get(row) {
                let start = range.start.min(self.columns);
                let end = range.end.min(self.columns);
                if start < end {
                    window.paint_quad(fill(
                        Bounds::new(
                            origin + point(width * start as f32, px(0.)),
                            size(width * (end - start) as f32, px(LINE_HEIGHT)),
                        ),
                        rgb(0x334c43),
                    ));
                }
            }
        }
        for (row, cached) in self.rows.iter().enumerate() {
            for text in &cached.text {
                let origin = bounds.origin
                    + point(
                        px(PADDING) + width * text.column as f32,
                        px(PADDING + row as f32 * LINE_HEIGHT),
                    );
                let clip = Bounds::new(origin, size(width * text.columns as f32, px(LINE_HEIGHT)));
                window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
                    let _ = text.line.paint(origin, px(LINE_HEIGHT), window, cx);
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use terminal_core::{GridSize, Terminal};

    fn view(cx: &mut gpui::Context<crate::TerminalView>) -> crate::TerminalView {
        crate::TerminalView::from_session(
            Err(anyhow::anyhow!("test view")),
            "test".into(),
            "".into(),
            false,
            cx,
        )
    }
    #[gpui::test]
    fn unchanged_rows_and_cursor_only_frames_reuse_layouts(cx: &mut TestAppContext) {
        let window = cx.add_window(|_, cx| view(cx));
        window
            .update(cx, |view, window, _| {
                let mut terminal = Terminal::new(GridSize::new(12, 3).unwrap()).unwrap();
                terminal.feed(b"first\r\nsecond\r\nthird");
                let first = Arc::new(terminal.snapshot().unwrap());
                let cached = view.render_cache.prepare(&first, px(9.), window);
                assert!(Arc::ptr_eq(
                    &cached,
                    &view.render_cache.prepare(&first, px(9.), window)
                ));
                terminal.feed(b"\x1b[1;1H");
                let cursor_only = Arc::new(terminal.snapshot().unwrap());
                let cursor_frame = view.render_cache.prepare(&cursor_only, px(9.), window);
                assert!(
                    cached
                        .rows
                        .iter()
                        .zip(&cursor_frame.rows)
                        .all(|(a, b)| Arc::ptr_eq(a, b))
                );
                terminal
                    .select(terminal_core::SelectionInput::Begin {
                        x: 1.5,
                        y: 0.5,
                        clicks: 3,
                    })
                    .unwrap();
                let selected = Arc::new(terminal.snapshot().unwrap());
                assert!(selected.has_selection);
                let selected_frame = view.render_cache.prepare(&selected, px(9.), window);
                assert!(
                    cursor_frame
                        .rows
                        .iter()
                        .zip(&selected_frame.rows)
                        .all(|(a, b)| Arc::ptr_eq(a, b))
                );
                terminal.feed(b"F");
                let changed = Arc::new(terminal.snapshot().unwrap());
                let changed_frame = view.render_cache.prepare(&changed, px(9.), window);
                assert!(!Arc::ptr_eq(&cached.rows[0], &changed_frame.rows[0]));
                assert!(Arc::ptr_eq(&cached.rows[1], &changed_frame.rows[1]));
                let resized = view.render_cache.prepare(&changed, px(10.), window);
                assert!(
                    resized
                        .rows
                        .iter()
                        .zip(&changed_frame.rows)
                        .all(|(a, b)| !Arc::ptr_eq(a, b))
                );
            })
            .unwrap();
    }
    #[gpui::test]
    fn batching_keeps_styles_wide_and_combining_graphemes_in_their_cells(cx: &mut TestAppContext) {
        let window = cx.add_window(|_, cx| view(cx));
        window
            .update(cx, |view, window, _| {
                let mut terminal = Terminal::new(GridSize::new(12, 2).unwrap()).unwrap();
                terminal.feed("AB界e\u{301}\x1b[1;31mZ\x1b[0m\r\n\x1b[44m    ".as_bytes());
                let screen = Arc::new(terminal.snapshot().unwrap());
                let frame = view.render_cache.prepare(&screen, px(9.), window);
                let spans = frame.rows[0]
                    .text
                    .iter()
                    .map(|text| (text.column, text.columns, text.line.text.as_ref()))
                    .collect::<Vec<_>>();
                assert_eq!(
                    spans,
                    vec![(0, 2, "AB"), (2, 2, "界"), (4, 1, "e\u{301}"), (5, 1, "Z")]
                );
                let backgrounds = &frame.rows[1].backgrounds;
                assert_eq!(backgrounds.len(), 1);
                assert_eq!((backgrounds[0].column, backgrounds[0].columns), (0, 4));
                assert_ne!(screen.cells[0].foreground, screen.cells[5].foreground);
                assert!(screen.cells[5].bold);
            })
            .unwrap();
    }
}

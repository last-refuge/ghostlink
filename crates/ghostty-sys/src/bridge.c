/* Keep Ghostty's sized structs, tagged unions, and enums on the C side.
 * The Rust ABI is deliberately small and checked against these pinned headers. */
#include "bridge.h"
#include <stdlib.h>
#include <ghostty/vt.h>

struct GlTerminal {
    GhosttyTerminal terminal;
    GhosttyRenderState render;
    GhosttyRenderStateRowIterator rows;
    GhosttyRenderStateRowCells cells;
    GhosttyKeyEncoder encoder;
    GhosttyKeyEvent event;
    GhosttyMouseEncoder mouse_encoder;
    GhosttyMouseEvent mouse_event;
    bool mouse_dirty;
    uint32_t mouse_cell_width, mouse_cell_height;
    uint16_t mouse_columns, mouse_rows;
    GhosttySelectionGesture gesture;
    GhosttySelectionGestureEvent selection_events[3];
    uint16_t columns, height;
    uint32_t *graphemes;
    size_t grapheme_capacity;
    GlWriteCallback reply;
    void *reply_context;
};

static uint32_t rgb(GhosttyColorRgb color) {
    return ((uint32_t)color.r << 16) | ((uint32_t)color.g << 8) | color.b;
}
static void write_pty(GhosttyTerminal terminal, void *userdata, const uint8_t *bytes, size_t len) {
    (void)terminal;
    GlTerminal *self = userdata;
    if (self->reply) self->reply(self->reply_context, bytes, len);
}

void gl_terminal_free(GlTerminal *self) {
    if (!self) return;
    for (size_t i = 0; i < 3; ++i) ghostty_selection_gesture_event_free(self->selection_events[i]);
    ghostty_selection_gesture_free(self->gesture, self->terminal);
    ghostty_key_event_free(self->event);
    ghostty_mouse_event_free(self->mouse_event);
    ghostty_mouse_encoder_free(self->mouse_encoder);
    ghostty_key_encoder_free(self->encoder);
    ghostty_render_state_row_cells_free(self->cells);
    ghostty_render_state_row_iterator_free(self->rows);
    ghostty_render_state_free(self->render);
    ghostty_terminal_free(self->terminal);
    free(self->graphemes);
    free(self);
}
GlTerminal *gl_terminal_new(uint16_t columns, uint16_t rows) {
    GlTerminal *self = calloc(1, sizeof(*self));
    if (!self) return NULL;
    self->columns = columns;
    self->height = rows;
    self->mouse_dirty = true;
    if (ghostty_terminal_new(NULL, &self->terminal, columns, rows) != GHOSTTY_SUCCESS ||
        ghostty_render_state_new(NULL, &self->render) != GHOSTTY_SUCCESS ||
        ghostty_render_state_row_iterator_new(NULL, &self->rows) != GHOSTTY_SUCCESS ||
        ghostty_render_state_row_cells_new(NULL, &self->cells) != GHOSTTY_SUCCESS ||
        ghostty_key_encoder_new(NULL, &self->encoder) != GHOSTTY_SUCCESS ||
        ghostty_key_event_new(NULL, &self->event) != GHOSTTY_SUCCESS ||
        ghostty_mouse_encoder_new(NULL, &self->mouse_encoder) != GHOSTTY_SUCCESS ||
        ghostty_mouse_event_new(NULL, &self->mouse_event) != GHOSTTY_SUCCESS) goto fail;
    if (ghostty_selection_gesture_new(NULL, &self->gesture) != GHOSTTY_SUCCESS ||
        ghostty_selection_gesture_event_new(NULL, &self->selection_events[0], GHOSTTY_SELECTION_GESTURE_EVENT_TYPE_PRESS) != GHOSTTY_SUCCESS ||
        ghostty_selection_gesture_event_new(NULL, &self->selection_events[1], GHOSTTY_SELECTION_GESTURE_EVENT_TYPE_DRAG) != GHOSTTY_SUCCESS ||
        ghostty_selection_gesture_event_new(NULL, &self->selection_events[2], GHOSTTY_SELECTION_GESTURE_EVENT_TYPE_RELEASE) != GHOSTTY_SUCCESS) goto fail;
    GhosttyColorRgb fg = {0xdc, 0xe3, 0xdf}, bg = {0x11, 0x15, 0x17}, cursor = {0xa8, 0xd8, 0xc0};
    if (ghostty_terminal_set(self->terminal, GHOSTTY_TERMINAL_OPT_USERDATA, self) != GHOSTTY_SUCCESS ||
        ghostty_terminal_set(self->terminal, GHOSTTY_TERMINAL_OPT_WRITE_PTY, write_pty) != GHOSTTY_SUCCESS ||
        ghostty_terminal_set(self->terminal, GHOSTTY_TERMINAL_OPT_COLOR_FOREGROUND, &fg) != GHOSTTY_SUCCESS ||
        ghostty_terminal_set(self->terminal, GHOSTTY_TERMINAL_OPT_COLOR_BACKGROUND, &bg) != GHOSTTY_SUCCESS ||
        ghostty_terminal_set(self->terminal, GHOSTTY_TERMINAL_OPT_COLOR_CURSOR, &cursor) != GHOSTTY_SUCCESS) goto fail;
    return self;
fail:
    gl_terminal_free(self);
    return NULL;
}
void gl_terminal_write(GlTerminal *self, const uint8_t *bytes, size_t len, GlWriteCallback callback, void *context) {
    self->reply = callback;
    self->reply_context = context;
    ghostty_terminal_vt_write(self->terminal, bytes, len);
    self->mouse_dirty = true;
    self->reply = NULL;
    self->reply_context = NULL;
}
int gl_terminal_resize(GlTerminal *self, uint16_t columns, uint16_t rows) {
    GhosttyResult result = ghostty_terminal_resize(self->terminal, columns, rows, 0, 0);
    if (result == GHOSTTY_SUCCESS) { self->columns = columns; self->height = rows; self->mouse_dirty = true; }
    return result;
}

#define TRY(call) do { GhosttyResult result = (call); if (result != GHOSTTY_SUCCESS) return result; } while (0)
int gl_terminal_set_colors(GlTerminal *self, uint32_t fg, uint32_t bg, uint32_t cursor, const uint32_t *palette16) {
    GhosttyColorRgb foreground = {(fg >> 16) & 255, (fg >> 8) & 255, fg & 255};
    GhosttyColorRgb background = {(bg >> 16) & 255, (bg >> 8) & 255, bg & 255};
    GhosttyColorRgb caret = {(cursor >> 16) & 255, (cursor >> 8) & 255, cursor & 255};
    GhosttyColorRgb palette[256];
    TRY(ghostty_terminal_get(self->terminal, GHOSTTY_TERMINAL_DATA_COLOR_PALETTE_DEFAULT, &palette));
    for (size_t i = 0; i < 16; ++i) {
        uint32_t value = palette16[i];
        palette[i] = (GhosttyColorRgb){(value >> 16) & 255, (value >> 8) & 255, value & 255};
    }
    TRY(ghostty_terminal_set(self->terminal, GHOSTTY_TERMINAL_OPT_COLOR_FOREGROUND, &foreground));
    TRY(ghostty_terminal_set(self->terminal, GHOSTTY_TERMINAL_OPT_COLOR_BACKGROUND, &background));
    TRY(ghostty_terminal_set(self->terminal, GHOSTTY_TERMINAL_OPT_COLOR_CURSOR, &caret));
    TRY(ghostty_terminal_set(self->terminal, GHOSTTY_TERMINAL_OPT_COLOR_PALETTE, &palette));
    return GHOSTTY_SUCCESS;
}
int gl_terminal_snapshot(GlTerminal *self, GlCellCallback callback, GlSelectionCallback selected, void *context, GlCursor *cursor, bool *has_selection, uint8_t *modes, uint32_t *background) {
    TRY(ghostty_render_state_update(self->render, self->terminal));
    GhosttyRenderStateColors colors = GHOSTTY_INIT_SIZED(GhosttyRenderStateColors);
    TRY(ghostty_render_state_colors_get(self->render, &colors));
    *background = rgb(colors.background);
    GhosttySelection active_selection = GHOSTTY_INIT_SIZED(GhosttySelection);
    GhosttyResult active_result = ghostty_terminal_get(self->terminal, GHOSTTY_TERMINAL_DATA_SELECTION, &active_selection);
    if (active_result != GHOSTTY_SUCCESS && active_result != GHOSTTY_NO_VALUE) return active_result;
    *has_selection = active_result == GHOSTTY_SUCCESS;
    bool mouse_tracking = false;
    GhosttyTerminalScreen screen;
    GhosttyTerminalModeConfig alt_scroll = { GHOSTTY_MODE_ALT_SCROLL, false };
    TRY(ghostty_terminal_get(self->terminal, GHOSTTY_TERMINAL_DATA_MOUSE_TRACKING, &mouse_tracking));
    TRY(ghostty_terminal_get(self->terminal, GHOSTTY_TERMINAL_DATA_ACTIVE_SCREEN, &screen));
    TRY(ghostty_terminal_get(self->terminal, GHOSTTY_TERMINAL_DATA_MODE, &alt_scroll));
    *modes = (uint8_t)(mouse_tracking | ((screen == GHOSTTY_TERMINAL_SCREEN_ALTERNATE) << 1) | (alt_scroll.value << 2));
    bool visible = false, in_viewport = false;
    GhosttyRenderStateCursorVisualStyle cursor_style;
    TRY(ghostty_render_state_get(self->render, GHOSTTY_RENDER_STATE_DATA_CURSOR_VISIBLE, &visible));
    TRY(ghostty_render_state_get(self->render, GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_HAS_VALUE, &in_viewport));
    TRY(ghostty_render_state_get(self->render, GHOSTTY_RENDER_STATE_DATA_CURSOR_VISUAL_STYLE, &cursor_style));
    cursor->visible = visible && in_viewport;
    cursor->style = (uint8_t)cursor_style;
    cursor->color = colors.cursor_has_value ? rgb(colors.cursor) : 0xa8d8c0;
    if (in_viewport) {
        TRY(ghostty_render_state_get(self->render, GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_X, &cursor->column));
        TRY(ghostty_render_state_get(self->render, GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_Y, &cursor->row));
    }
    TRY(ghostty_render_state_get(self->render, GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR, &self->rows));
    uint16_t row = 0;
    while (ghostty_render_state_row_iterator_next(self->rows)) {
        GhosttyRenderStateRowSelection selection = GHOSTTY_INIT_SIZED(GhosttyRenderStateRowSelection);
        GhosttyResult selection_result = ghostty_render_state_row_get(self->rows, GHOSTTY_RENDER_STATE_ROW_DATA_SELECTION, &selection);
        if (selection_result == GHOSTTY_SUCCESS) selected(context, row, selection.start_x, (uint16_t)(selection.end_x + 1));
        else if (selection_result != GHOSTTY_NO_VALUE) return selection_result;
        TRY(ghostty_render_state_row_get(self->rows, GHOSTTY_RENDER_STATE_ROW_DATA_CELLS, &self->cells));
        uint16_t column = 0;
        while (ghostty_render_state_row_cells_next(self->cells)) {
            GhosttyCell raw;
            GhosttyCellWide wide;
            GhosttyStyle style = GHOSTTY_INIT_SIZED(GhosttyStyle);
            uint32_t len = 0;
            TRY(ghostty_render_state_row_cells_get(self->cells, GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_RAW, &raw));
            TRY(ghostty_cell_get(raw, GHOSTTY_CELL_DATA_WIDE, &wide));
            TRY(ghostty_render_state_row_cells_get(self->cells, GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_STYLE, &style));
            TRY(ghostty_render_state_row_cells_get(self->cells, GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_LEN, &len));
            if (len > self->grapheme_capacity) {
                uint32_t *buffer = realloc(self->graphemes, (size_t)len * sizeof(uint32_t));
                if (!buffer) return GHOSTTY_OUT_OF_MEMORY;
                self->graphemes = buffer;
                self->grapheme_capacity = len;
            }
            if (len) TRY(ghostty_render_state_row_cells_get(self->cells, GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_BUF, self->graphemes));
            GhosttyColorRgb fg = colors.foreground, bg = colors.background;
            /* No explicit cell color is a documented non-success result. */
            ghostty_render_state_row_cells_get(self->cells, GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_FG_COLOR, &fg);
            ghostty_render_state_row_cells_get(self->cells, GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_BG_COLOR, &bg);
            if (style.inverse) { GhosttyColorRgb swap = fg; fg = bg; bg = swap; }
            if (style.invisible) fg = bg;
            if (style.faint) { fg.r = (uint8_t)(((unsigned)fg.r + bg.r) / 2); fg.g = (uint8_t)(((unsigned)fg.g + bg.g) / 2); fg.b = (uint8_t)(((unsigned)fg.b + bg.b) / 2); }
            GlCell cell = {
                self->graphemes, len, rgb(fg), rgb(bg),
                wide == GHOSTTY_CELL_WIDE_WIDE ? 2 : wide == GHOSTTY_CELL_WIDE_NARROW ? 1 : 0,
                (uint8_t)(style.bold | (style.italic << 1) | ((style.underline != 0) << 2) | (style.strikethrough << 3))
            };
            callback(context, column++, row, &cell);
        }
        row++;
    }
    return GHOSTTY_SUCCESS;
}

int gl_terminal_key(GlTerminal *self, uint32_t key, uint32_t mods, uint8_t *output, size_t capacity, size_t *written) {
    static const GhosttyKey specials[] = {
        GHOSTTY_KEY_UNIDENTIFIED, GHOSTTY_KEY_ENTER, GHOSTTY_KEY_TAB, GHOSTTY_KEY_BACKSPACE, GHOSTTY_KEY_ESCAPE,
        GHOSTTY_KEY_ARROW_UP, GHOSTTY_KEY_ARROW_DOWN, GHOSTTY_KEY_ARROW_LEFT, GHOSTTY_KEY_ARROW_RIGHT,
        GHOSTTY_KEY_HOME, GHOSTTY_KEY_END, GHOSTTY_KEY_PAGE_UP, GHOSTTY_KEY_PAGE_DOWN,
        GHOSTTY_KEY_DELETE, GHOSTTY_KEY_INSERT, GHOSTTY_KEY_F1, GHOSTTY_KEY_F2, GHOSTTY_KEY_F3,
        GHOSTTY_KEY_F4, GHOSTTY_KEY_F5, GHOSTTY_KEY_F6, GHOSTTY_KEY_F7, GHOSTTY_KEY_F8, GHOSTTY_KEY_F9,
        GHOSTTY_KEY_F10, GHOSTTY_KEY_F11, GHOSTTY_KEY_F12
    };
    static const GhosttyKey letters[] = {
        GHOSTTY_KEY_A,GHOSTTY_KEY_B,GHOSTTY_KEY_C,GHOSTTY_KEY_D,GHOSTTY_KEY_E,GHOSTTY_KEY_F,GHOSTTY_KEY_G,
        GHOSTTY_KEY_H,GHOSTTY_KEY_I,GHOSTTY_KEY_J,GHOSTTY_KEY_K,GHOSTTY_KEY_L,GHOSTTY_KEY_M,GHOSTTY_KEY_N,
        GHOSTTY_KEY_O,GHOSTTY_KEY_P,GHOSTTY_KEY_Q,GHOSTTY_KEY_R,GHOSTTY_KEY_S,GHOSTTY_KEY_T,GHOSTTY_KEY_U,
        GHOSTTY_KEY_V,GHOSTTY_KEY_W,GHOSTTY_KEY_X,GHOSTTY_KEY_Y,GHOSTTY_KEY_Z
    };
    GhosttyKey code;
    char text = 0;
    if (key >= 100 && key < 126) { code = letters[key - 100]; text = (char)('a' + key - 100); }
    else if (key < sizeof(specials) / sizeof(specials[0])) code = specials[key];
    else return GHOSTTY_INVALID_VALUE;
    GhosttyMods native_mods = 0;
    if (mods & 1) native_mods |= GHOSTTY_MODS_SHIFT;
    if (mods & 2) native_mods |= GHOSTTY_MODS_CTRL;
    if (mods & 4) native_mods |= GHOSTTY_MODS_ALT;
    if (mods & 8) native_mods |= GHOSTTY_MODS_SUPER;
    ghostty_key_event_set_action(self->event, GHOSTTY_KEY_ACTION_PRESS);
    ghostty_key_event_set_key(self->event, code);
    ghostty_key_event_set_mods(self->event, native_mods);
    ghostty_key_event_set_utf8(self->event, text ? &text : NULL, text ? 1 : 0);
    ghostty_key_event_set_unshifted_codepoint(self->event, text);
    ghostty_key_encoder_setopt_from_terminal(self->encoder, self->terminal);
    return ghostty_key_encoder_encode(self->encoder, self->event, (char *)output, capacity, written);
}
bool gl_terminal_bracketed_paste(GlTerminal *self) {
    GhosttyTerminalModeConfig config = { GHOSTTY_MODE_BRACKETED_PASTE, false };
    ghostty_terminal_get(self->terminal, GHOSTTY_TERMINAL_DATA_MODE, &config);
    return config.value;
}
void gl_terminal_scroll(GlTerminal *self, int32_t delta) {
    GhosttyTerminalScrollViewport scroll = { .tag = GHOSTTY_SCROLL_VIEWPORT_DELTA, .value = { .delta = delta } };
    ghostty_terminal_scroll_viewport(self->terminal, scroll);
}

int gl_terminal_selection_clear(GlTerminal *self, bool *changed) {
    GhosttySelection selection = GHOSTTY_INIT_SIZED(GhosttySelection);
    GhosttyResult result = ghostty_terminal_get(self->terminal, GHOSTTY_TERMINAL_DATA_SELECTION, &selection);
    if (result != GHOSTTY_SUCCESS && result != GHOSTTY_NO_VALUE) return result;
    *changed = result == GHOSTTY_SUCCESS;
    ghostty_selection_gesture_reset(self->gesture, self->terminal);
    return ghostty_terminal_set(self->terminal, GHOSTTY_TERMINAL_OPT_SELECTION, NULL);
}

int gl_terminal_select(GlTerminal *self, uint8_t phase, uint8_t clicks, double x, double y, int32_t scroll) {
    if (phase == 3) { bool changed; return gl_terminal_selection_clear(self, &changed); }
    if (phase > 2) return GHOSTTY_INVALID_VALUE;
    if (scroll) gl_terminal_scroll(self, scroll);
    GhosttyPoint point = { .tag = GHOSTTY_POINT_TAG_VIEWPORT, .value.coordinate = {
        .x = x < 0 ? 0 : x >= self->columns ? self->columns - 1 : (uint16_t)x,
        .y = y < 0 ? 0 : y >= self->height ? (uint32_t)self->height - 1u : (uint32_t)y,
    }};
    GhosttyGridRef ref;
    TRY(ghostty_terminal_grid_ref(self->terminal, point, &ref));
    GhosttySelectionGestureEvent event = self->selection_events[phase];
    TRY(ghostty_selection_gesture_event_set(event, GHOSTTY_SELECTION_GESTURE_EVENT_OPT_REF, &ref));
    if (phase != 2) {
        GhosttySurfacePosition position = { .x = x * 100, .y = y * 100 };
        TRY(ghostty_selection_gesture_event_set(event, GHOSTTY_SELECTION_GESTURE_EVENT_OPT_POSITION, &position));
    }
    if (phase == 0) {
        bool changed;
        TRY(gl_terminal_selection_clear(self, &changed));
        // Clearing is a mutation; refresh the borrowed ref before the gesture.
        TRY(ghostty_terminal_grid_ref(self->terminal, point, &ref));
        TRY(ghostty_selection_gesture_event_set(event, GHOSTTY_SELECTION_GESTURE_EVENT_OPT_REF, &ref));
        GhosttySelectionGestureBehavior behavior = clicks == 2 ? GHOSTTY_SELECTION_GESTURE_BEHAVIOR_WORD : clicks >= 3 ? GHOSTTY_SELECTION_GESTURE_BEHAVIOR_LINE : GHOSTTY_SELECTION_GESTURE_BEHAVIOR_CELL;
        GhosttySelectionGestureBehaviors behaviors = { behavior, behavior, behavior };
        TRY(ghostty_selection_gesture_event_set(event, GHOSTTY_SELECTION_GESTURE_EVENT_OPT_BEHAVIORS, &behaviors));
    } else if (phase == 1) {
        GhosttySelectionGestureGeometry geometry = { .columns = self->columns, .cell_width = 100, .padding_left = 0, .screen_height = self->height * 100 };
        TRY(ghostty_selection_gesture_event_set(event, GHOSTTY_SELECTION_GESTURE_EVENT_OPT_GEOMETRY, &geometry));
    }
    GhosttySelection selection = GHOSTTY_INIT_SIZED(GhosttySelection);
    GhosttyResult result = ghostty_selection_gesture_event(self->gesture, self->terminal, event, &selection);
    if (result == GHOSTTY_NO_VALUE) return GHOSTTY_SUCCESS;
    if (result != GHOSTTY_SUCCESS) return result;
    return ghostty_terminal_set(self->terminal, GHOSTTY_TERMINAL_OPT_SELECTION, &selection);
}

int gl_terminal_selection_copy(GlTerminal *self, uint8_t *buffer, size_t capacity, size_t *written, bool *present) {
    GhosttyTerminalSelectionFormatOptions options = GHOSTTY_INIT_SIZED(GhosttyTerminalSelectionFormatOptions);
    options.emit = GHOSTTY_FORMATTER_FORMAT_PLAIN;
    options.unwrap = true;
    options.trim = true;
    GhosttyResult result = ghostty_terminal_selection_format_buf(self->terminal, options, buffer, capacity, written);
    *present = result != GHOSTTY_NO_VALUE;
    if (!*present) { *written = 0; return GHOSTTY_SUCCESS; }
    if (!buffer && result == GHOSTTY_OUT_OF_SPACE) return GHOSTTY_SUCCESS;
    return result;
}

int gl_terminal_mouse(GlTerminal *self, const GlMouse *input, uint8_t *buffer, size_t capacity, size_t *written) {
    if (input->action > 2 || input->button > 7 || !input->cell_width || !input->cell_height) return GHOSTTY_INVALID_VALUE;
    if (input->action == 0 && (input->button == 4 || input->button == 5)) {
        bool tracking = false;
        GhosttyTerminalScreen screen;
        GhosttyTerminalModeConfig alt_scroll = { GHOSTTY_MODE_ALT_SCROLL, false };
        TRY(ghostty_terminal_get(self->terminal, GHOSTTY_TERMINAL_DATA_MOUSE_TRACKING, &tracking));
        TRY(ghostty_terminal_get(self->terminal, GHOSTTY_TERMINAL_DATA_ACTIVE_SCREEN, &screen));
        TRY(ghostty_terminal_get(self->terminal, GHOSTTY_TERMINAL_DATA_MODE, &alt_scroll));
        if (!tracking && screen == GHOSTTY_TERMINAL_SCREEN_ALTERNATE && alt_scroll.value) return gl_terminal_key(self, input->button == 4 ? 5 : 6, input->mods, buffer, capacity, written);
    }
    GhosttyMouseEncoderSize size = GHOSTTY_INIT_SIZED(GhosttyMouseEncoderSize);
    size.cell_width = input->cell_width;
    size.cell_height = input->cell_height;
    size.screen_width = self->columns * input->cell_width;
    size.screen_height = self->height * input->cell_height;
    GhosttyMods mods = 0;
    if (input->mods & 1) mods |= GHOSTTY_MODS_SHIFT;
    if (input->mods & 2) mods |= GHOSTTY_MODS_CTRL;
    if (input->mods & 4) mods |= GHOSTTY_MODS_ALT;
    if (input->mods & 8) mods |= GHOSTTY_MODS_SUPER;
    bool pressed = input->button >= 1 && input->button <= 3;
    bool deduplicate = true;
    if (self->mouse_dirty) {
        ghostty_mouse_encoder_setopt_from_terminal(self->mouse_encoder, self->terminal);
        self->mouse_dirty = false;
    }
    if (self->mouse_cell_width != input->cell_width || self->mouse_cell_height != input->cell_height || self->mouse_columns != self->columns || self->mouse_rows != self->height) {
        ghostty_mouse_encoder_setopt(self->mouse_encoder, GHOSTTY_MOUSE_ENCODER_OPT_SIZE, &size);
        self->mouse_cell_width = input->cell_width;
        self->mouse_cell_height = input->cell_height;
        self->mouse_columns = self->columns;
        self->mouse_rows = self->height;
    }
    ghostty_mouse_encoder_setopt(self->mouse_encoder, GHOSTTY_MOUSE_ENCODER_OPT_ANY_BUTTON_PRESSED, &pressed);
    ghostty_mouse_encoder_setopt(self->mouse_encoder, GHOSTTY_MOUSE_ENCODER_OPT_TRACK_LAST_CELL, &deduplicate);
    ghostty_mouse_event_set_action(self->mouse_event, (GhosttyMouseAction)input->action);
    if (input->button) ghostty_mouse_event_set_button(self->mouse_event, (GhosttyMouseButton)input->button);
    else ghostty_mouse_event_clear_button(self->mouse_event);
    ghostty_mouse_event_set_mods(self->mouse_event, mods);
    GhosttyMousePosition position = { (float)(input->x * input->cell_width), (float)(input->y * input->cell_height) };
    ghostty_mouse_event_set_position(self->mouse_event, position);
    return ghostty_mouse_encoder_encode(self->mouse_encoder, self->mouse_event, (char *)buffer, capacity, written);
}

int gl_terminal_archive(GlTerminal *self, uint8_t *buffer, size_t capacity, size_t *written) {
    GhosttySelection all = GHOSTTY_INIT_SIZED(GhosttySelection);
    GhosttyResult result = ghostty_terminal_select_all(self->terminal, &all);
    if (result == GHOSTTY_NO_VALUE) { *written = 0; return GHOSTTY_SUCCESS; }
    if (result != GHOSTTY_SUCCESS) return result;
    GhosttyTerminalSelectionFormatOptions options = GHOSTTY_INIT_SIZED(GhosttyTerminalSelectionFormatOptions);
    options.emit = GHOSTTY_FORMATTER_FORMAT_VT;
    options.unwrap = true;
    options.trim = true;
    options.selection = &all;
    result = ghostty_terminal_selection_format_buf(self->terminal, options, buffer, capacity, written);
    if (!buffer && result == GHOSTTY_OUT_OF_SPACE) return GHOSTTY_SUCCESS;
    return result;
}

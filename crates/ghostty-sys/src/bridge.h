#ifndef GHOSTLINK_BRIDGE_H
#define GHOSTLINK_BRIDGE_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

typedef struct GlTerminal GlTerminal;
typedef struct {
    double x, y;
    uint32_t cell_width, cell_height, mods;
    uint8_t action, button;
} GlMouse;
typedef struct {
    const uint32_t *codepoints;
    size_t len;
    uint32_t foreground;
    uint32_t background;
    uint8_t width;
    uint8_t flags;
} GlCell;
typedef struct {
    uint16_t column, row;
    uint8_t visible, style;
    uint32_t color;
} GlCursor;
typedef void (*GlWriteCallback)(void *, const uint8_t *, size_t);
typedef void (*GlCellCallback)(void *, uint16_t, uint16_t, const GlCell *);
typedef void (*GlSelectionCallback)(void *, uint16_t, uint16_t, uint16_t);

GlTerminal *gl_terminal_new(uint16_t columns, uint16_t rows);
void gl_terminal_free(GlTerminal *);
void gl_terminal_write(GlTerminal *, const uint8_t *, size_t, GlWriteCallback, void *);
int gl_terminal_resize(GlTerminal *, uint16_t columns, uint16_t rows);
int gl_terminal_snapshot(GlTerminal *, GlCellCallback, GlSelectionCallback, void *, GlCursor *, bool *has_selection, uint8_t *modes);
int gl_terminal_mouse(GlTerminal *, const GlMouse *, uint8_t *, size_t, size_t *);
int gl_terminal_archive(GlTerminal *, uint8_t *, size_t, size_t *);
int gl_terminal_select(GlTerminal *, uint8_t phase, uint8_t clicks, double x, double y, int32_t scroll);
int gl_terminal_selection_clear(GlTerminal *, bool *changed);
int gl_terminal_selection_copy(GlTerminal *, uint8_t *, size_t, size_t *, bool *present);
int gl_terminal_key(GlTerminal *, uint32_t key, uint32_t mods, uint8_t *, size_t, size_t *);
bool gl_terminal_bracketed_paste(GlTerminal *);
void gl_terminal_scroll(GlTerminal *, int32_t delta);
#endif

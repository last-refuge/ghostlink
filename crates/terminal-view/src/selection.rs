use super::*;
use std::time::Duration;
use terminal_core::SelectionInput;

#[derive(Default)]
struct CopySequence(u64);
impl Global for CopySequence {}

impl TerminalView {
    pub(super) fn selection_position(&self, position: Point<Pixels>) -> Option<(f64, f64)> {
        self.screen.as_ref()?;
        let bounds = self.bounds?;
        Some((
            f32::from(position.x - bounds.left() - px(PADDING)) as f64
                / f32::from(self.cell_width) as f64,
            f32::from(position.y - bounds.top() - px(PADDING)) as f64 / LINE_HEIGHT as f64,
        ))
    }
    pub(super) fn cancel_drag(&mut self) -> bool {
        let was_selecting = self.selecting;
        self.selecting = false;
        self.autoscroll = None;
        if self.mouse_capture.is_none() {
            self.drag_observers.clear();
        }
        self.drag_pointer = None;
        was_selecting
    }
    pub(super) fn selection_begin(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.requires_authentication()
            || self.remote
                && !matches!(
                    self.state,
                    State::Running | State::Exited(_) | State::Failed(_) | State::Disconnected(_)
                )
        {
            return;
        }
        let Some((x, y)) = self.selection_position(event.position) else {
            return;
        };
        self.cancel_drag();
        window.focus(&self.focus);
        self.selecting = true;
        self.drag_moved = false;
        self.drag_origin = event.position;
        self.drag_pointer = Some(event.position);
        self.send(
            Input::Select(SelectionInput::Begin {
                x,
                y,
                clicks: ((event.click_count.saturating_sub(1) % 3) + 1) as u8,
            }),
            cx,
        );
        self.observe_pointer(window, cx);
        self.autoscroll = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(40))
                    .await;
                let keep_running = this
                    .update(cx, |this, cx| {
                        if !this.selecting {
                            return false;
                        }
                        if this.drag_moved {
                            let delta = this.autoscroll_delta();
                            if delta != 0 {
                                this.selection_extend(delta, cx);
                            }
                        }
                        true
                    })
                    .unwrap_or(false);
                if !keep_running {
                    break;
                }
            }
        }));
        cx.notify();
    }
    pub(super) fn selection_move(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if !self.selecting {
            return;
        }
        self.drag_pointer = Some(position);
        if (position.x - self.drag_origin.x).abs() >= px(3.)
            || (position.y - self.drag_origin.y).abs() >= px(3.)
        {
            self.drag_moved = true;
        }
        if self.drag_moved {
            self.selection_extend(0, cx);
        }
    }
    fn selection_extend(&mut self, scroll: i32, cx: &mut Context<Self>) {
        let Some(position) = self.drag_pointer else {
            return;
        };
        if let Some((x, y)) = self.selection_position(position) {
            self.send(Input::Select(SelectionInput::Extend { x, y, scroll }), cx);
        }
    }
    pub(super) fn selection_end(&mut self, cx: &mut Context<Self>) {
        if !self.selecting {
            return;
        }
        if self.drag_moved {
            self.selection_extend(0, cx);
        }
        if let Some((x, y)) = self
            .drag_pointer
            .and_then(|position| self.selection_position(position))
        {
            self.send(Input::Select(SelectionInput::End { x, y }), cx);
        }
        self.cancel_drag();
        cx.notify();
    }
    pub(super) fn selection_scroll(&mut self, scroll: i32, cx: &mut Context<Self>) {
        if self.selecting {
            self.drag_moved = true;
            self.selection_extend(scroll, cx);
        } else {
            self.send(Input::Scroll(scroll), cx);
        }
    }
    fn autoscroll_delta(&self) -> i32 {
        let Some((_, y)) = self
            .drag_pointer
            .and_then(|position| self.selection_position(position))
        else {
            return 0;
        };
        let rows = self.screen.as_ref().map_or(0, |screen| screen.size.rows()) as f64;
        if y < 0.35 {
            -((0.35 - y).ceil().clamp(1., 10.) as i32)
        } else if y > rows - 0.35 {
            (y - (rows - 0.35)).ceil().clamp(1., 10.) as i32
        } else {
            0
        }
    }
    pub(super) fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.cancel_drag();
        self.send(Input::Select(SelectionInput::Clear), cx);
        cx.notify();
    }
    pub(super) fn copy(&mut self, cx: &mut Context<Self>) {
        if self.remote
            && !matches!(
                self.state,
                State::Running | State::Exited(_) | State::Failed(_) | State::Disconnected(_)
            )
        {
            return;
        }
        let Some(session) = &self.session else {
            return;
        };
        let response = match session.copy_selection() {
            Ok(response) => response,
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let current = cx.default_global::<CopySequence>();
        current.0 += 1;
        let sequence = current.0;
        cx.spawn(async move |this, cx| {
            let result = response.recv().await;
            let _ = this.update(cx, |this, cx| {
                if this.session.is_none() || cx.global::<CopySequence>().0 != sequence {
                    return;
                }
                match result {
                    Ok(Ok(Some(text))) if !text.is_empty() => {
                        cx.write_to_clipboard(ClipboardItem::new_string(text))
                    }
                    Ok(Err(error)) => {
                        this.error = Some(error);
                        cx.notify();
                    }
                    _ => {}
                }
            });
        })
        .detach();
    }
}

pub(super) fn install_drag_handlers(view: &Entity<TerminalView>, window: &mut Window) {
    let moved = view.downgrade();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
        if phase == DispatchPhase::Capture {
            let _ = moved.update(cx, |this, cx| {
                if this.selecting {
                    if event.pressed_button == Some(MouseButton::Left) {
                        this.selection_move(event.position, cx);
                    } else {
                        this.selection_end(cx);
                    }
                }
                this.pointer_move(event, cx);
            });
        }
    });
    let released = view.downgrade();
    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
        if phase == DispatchPhase::Capture {
            let _ = released.update(cx, |this, cx| {
                if this.selecting && event.button == MouseButton::Left {
                    this.drag_pointer = Some(event.position);
                    this.selection_end(cx);
                }
                this.pointer_up(event, cx);
            });
        }
    });
}

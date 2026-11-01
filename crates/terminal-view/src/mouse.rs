use super::*;
use terminal_core::{MouseButton as Button, MouseInput, MouseKind};

fn button(button: MouseButton) -> Option<Button> {
    match button {
        MouseButton::Left => Some(Button::Left),
        MouseButton::Right => Some(Button::Right),
        MouseButton::Middle => Some(Button::Middle),
        _ => None,
    }
}
impl TerminalView {
    pub(super) fn observe_pointer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.drag_observers.clear();
        self.drag_observers
            .push(cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.pointer_end(cx);
                }
            }));
        self.drag_observers
            .push(cx.on_blur(&self.focus, window, |this, _, cx| this.pointer_end(cx)));
    }
    pub(super) fn pointer_end(&mut self, cx: &mut Context<Self>) {
        if let Some((button, position, modifiers)) = self.mouse_capture {
            self.pointer_up(
                &MouseUpEvent {
                    button,
                    position,
                    modifiers,
                    ..Default::default()
                },
                cx,
            );
        }
        self.selection_end(cx);
    }
    fn report_mouse(
        &mut self,
        kind: MouseKind,
        position: Point<Pixels>,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) {
        if self.state != State::Running {
            return;
        }
        let Some((x, y)) = self.selection_position(position) else {
            return;
        };
        self.send(
            Input::Mouse(MouseInput {
                kind,
                x,
                y,
                cell_width: f32::from(self.cell_width).round().max(1.) as u32,
                cell_height: LINE_HEIGHT as u32,
                modifiers: TerminalModifiers {
                    shift: modifiers.shift,
                    control: modifiers.control,
                    alt: modifiers.alt,
                    super_key: modifiers.platform,
                },
            }),
            cx,
        );
    }
    pub(super) fn pointer_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pointer_end(cx);
        if !event.modifiers.shift
            && self.state == State::Running
            && self.screen.as_ref().is_some_and(|s| s.mouse_tracking)
        {
            let Some(button) = button(event.button) else {
                return;
            };
            window.focus(&self.focus);
            self.clear_selection(cx);
            self.mouse_capture = Some((event.button, event.position, event.modifiers));
            self.observe_pointer(window, cx);
            self.report_mouse(
                MouseKind::Press(button),
                event.position,
                event.modifiers,
                cx,
            );
        } else if event.button == MouseButton::Left {
            self.selection_begin(event, window, cx);
        }
    }
    pub(super) fn pointer_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if let Some((pressed, _, _)) = self.mouse_capture {
            self.mouse_capture = Some((pressed, event.position, event.modifiers));
            if event.pressed_button != Some(pressed) {
                self.pointer_end(cx);
                return;
            }
            self.report_mouse(
                MouseKind::Move(button(pressed)),
                event.position,
                event.modifiers,
                cx,
            );
        } else if !self.selecting
            && !event.modifiers.shift
            && self.bounds.is_some_and(|b| b.contains(&event.position))
            && self.screen.as_ref().is_some_and(|s| s.mouse_tracking)
        {
            self.report_mouse(MouseKind::Move(None), event.position, event.modifiers, cx);
        }
    }
    pub(super) fn pointer_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        if self
            .mouse_capture
            .is_some_and(|(button, _, _)| button == event.button)
        {
            self.mouse_capture = None;
            if let Some(button) = button(event.button) {
                self.report_mouse(
                    MouseKind::Release(button),
                    event.position,
                    event.modifiers,
                    cx,
                );
            }
            if !self.selecting {
                self.drag_observers.clear();
            }
        }
    }
    pub(super) fn pointer_scroll(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(px(LINE_HEIGHT));
        self.wheel_remainder.x -= f32::from(delta.x) / LINE_HEIGHT * 3.;
        self.wheel_remainder.y -= f32::from(delta.y) / LINE_HEIGHT * 3.;
        let horizontal = self.wheel_remainder.x.trunc().clamp(-32., 32.) as i16;
        let vertical = self.wheel_remainder.y.trunc().clamp(-32., 32.) as i16;
        self.wheel_remainder.x -= self.wheel_remainder.x.trunc();
        self.wheel_remainder.y -= self.wheel_remainder.y.trunc();
        if horizontal == 0 && vertical == 0 {
            return;
        }
        if !event.modifiers.shift
            && !self.selecting
            && self
                .screen
                .as_ref()
                .is_some_and(|s| s.mouse_tracking || s.alternate_screen && s.alternate_scroll)
            && self.state == State::Running
        {
            self.report_mouse(
                MouseKind::Scroll {
                    horizontal,
                    vertical,
                },
                event.position,
                event.modifiers,
                cx,
            );
        } else if vertical != 0 {
            self.selection_scroll(vertical as i32, cx);
        }
    }
}

//! GPUI terminal rendering and interaction.

mod field;
mod mouse;
mod perf;
mod render_cache;
mod selection;
#[cfg(all(test, windows))]
mod selection_tests;
pub use field::Field;

use gpui::{prelude::*, *};
use session::{Input, LocalProfile, Session, SshConnection, SshProfile, State};
use std::{ops::Range, sync::Arc};
use terminal_core::{GridSize, Key, Modifiers as TerminalModifiers, Snapshot};

const FONT_SIZE: f32 = 15.0;
const LINE_HEIGHT: f32 = 22.0;
const PADDING: f32 = 16.0;

pub enum TerminalEvent {
    AuthenticationRequired,
    StateChanged,
    Reconnecting,
}
impl EventEmitter<TerminalEvent> for TerminalView {}

pub struct TerminalView {
    focus: FocusHandle,
    session: Option<Session>,
    screen: Option<Arc<Snapshot>>,
    state: State,
    error: Option<String>,
    title: String,
    directory: String,
    remote: bool,
    bounds: Option<Bounds<Pixels>>,
    cell_width: Pixels,
    grid: GridSize,
    selecting: bool,
    drag_pointer: Option<Point<Pixels>>,
    drag_origin: Point<Pixels>,
    drag_moved: bool,
    autoscroll: Option<Task<()>>,
    drag_observers: Vec<Subscription>,
    mouse_capture: Option<(MouseButton, Point<Pixels>, Modifiers)>,
    wheel_remainder: Point<f32>,
    composition: String,
    composition_selection: Range<usize>,
    credential: Option<(u64, Entity<Field>)>,
    focus_credential: bool,
    render_cache: render_cache::RenderCache,
}
impl TerminalView {
    pub fn new(profile: LocalProfile, title: String, cx: &mut Context<Self>) -> Self {
        let directory = profile
            .directory
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        Self::from_session(
            Session::local(profile, GridSize::default()),
            title,
            directory,
            false,
            cx,
        )
    }
    pub fn ssh(profile: SshProfile, cx: &mut Context<Self>) -> Self {
        let title = profile.label();
        let directory = if profile.port == 0 {
            format!("SSH · {}", profile.host)
        } else {
            format!("SSH · {}:{}", profile.host, profile.port)
        };
        Self::from_session(
            Session::ssh(profile, GridSize::default()),
            title,
            directory,
            true,
            cx,
        )
    }
    pub fn ssh_in(connection: SshConnection, cx: &mut Context<Self>) -> Self {
        let title = connection.profile().label();
        let directory = format!("SSH · {}", connection.profile().host);
        Self::from_session(
            Session::ssh_in(connection, GridSize::default()),
            title,
            directory,
            true,
            cx,
        )
    }
    pub fn requires_authentication(&self) -> bool {
        matches!(
            self.state,
            State::AwaitingHostKey(_) | State::AwaitingCredential(_)
        )
    }
    /// Stop transport work immediately, even if GPUI retains an old frame's entity.
    pub fn close(&mut self) {
        self.mouse_capture = None;
        self.cancel_drag();
        self.session = None;
        self.credential = None;
    }
    pub fn is_disconnected(&self) -> bool {
        matches!(self.state, State::Disconnected(_))
    }
    pub fn is_running(&self) -> bool {
        self.state == State::Running
    }
    pub fn can_reconnect(&self) -> bool {
        self.remote
            && self.session.is_some()
            && matches!(
                self.state,
                State::Exited(_) | State::Failed(_) | State::Disconnected(_)
            )
    }
    pub fn reconnect(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.can_reconnect() {
            return false;
        }
        if let Some(session) = &self.session
            && let Err(error) = session.reconnect()
        {
            self.error = Some(error.to_string());
            cx.notify();
            return false;
        }
        self.mouse_capture = None;
        self.cancel_drag();
        self.composition.clear();
        self.composition_selection = 0..0;
        self.error = None;
        self.state = State::Connecting;
        cx.emit(TerminalEvent::StateChanged);
        cx.notify();
        true
    }
    fn request_reconnect(&mut self, cx: &mut Context<Self>) {
        if self.reconnect(cx) {
            cx.emit(TerminalEvent::Reconnecting);
        }
    }
    fn from_session(
        result: anyhow::Result<Session>,
        title: String,
        directory: String,
        remote: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let (session, state) = match result {
            Ok(session) => {
                let shutdown = session.shutdown_signal();
                App::on_app_quit(cx, move |_| shutdown.clone().shutdown()).detach();
                let updates = session.updates();
                cx.spawn(async move |this, cx| {
                    while let Ok(update) = updates.recv().await {
                        if this
                            .update(cx, |this, cx| {
                                let authentication_required = matches!(
                                    update.state,
                                    State::AwaitingHostKey(_) | State::AwaitingCredential(_)
                                ) && this.state != update.state;
                                if let State::AwaitingCredential(prompt) = &update.state {
                                    if this.credential.as_ref().map(|(id, _)| *id)
                                        != Some(prompt.id)
                                    {
                                        this.credential = Some((
                                            prompt.id,
                                            cx.new(|cx| Field::new("", prompt.secret, cx)),
                                        ));
                                        this.focus_credential = true;
                                    }
                                } else {
                                    this.credential = None;
                                    this.focus_credential = false;
                                }
                                let changed = this.state != update.state;
                                if changed && update.state != State::Running {
                                    this.pointer_end(cx);
                                }
                                this.state = update.state;
                                if changed && this.state == State::Running {
                                    this.error = None;
                                }
                                if changed {
                                    cx.emit(TerminalEvent::StateChanged);
                                }
                                if authentication_required {
                                    cx.emit(TerminalEvent::AuthenticationRequired);
                                }
                                if let Some(screen) = update.screen {
                                    this.screen = Some(screen);
                                }
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                })
                .detach();
                (Some(session), State::Starting)
            }
            Err(error) => (None, State::Failed(format!("{error:#}"))),
        };
        Self {
            focus: cx.focus_handle(),
            session,
            screen: None,
            state,
            error: None,
            title,
            directory,
            remote,
            bounds: None,
            cell_width: px(9.),
            grid: GridSize::default(),
            selecting: false,
            drag_pointer: None,
            drag_origin: Point::default(),
            drag_moved: false,
            autoscroll: None,
            drag_observers: Vec::new(),
            mouse_capture: None,
            wheel_remainder: point(0., 0.),
            composition: String::new(),
            composition_selection: 0..0,
            credential: None,
            focus_credential: false,
            render_cache: render_cache::RenderCache::default(),
        }
    }
    fn send(&mut self, input: Input, cx: &mut Context<Self>) {
        if self.remote
            && self.state != State::Running
            && matches!(input, Input::Text(_) | Input::Paste(_) | Input::Key(_, _))
        {
            self.error = Some(
                if self.can_reconnect() {
                    "Reconnect to continue working"
                } else {
                    "Wait for the SSH connection to finish before typing"
                }
                .into(),
            );
            cx.notify();
            return;
        }
        let error = self
            .session
            .as_ref()
            .and_then(|session| session.send(input).err())
            .map(|e| e.to_string());
        if self.error != error {
            self.error = error;
            cx.notify();
        }
    }
    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.state, State::AwaitingCredential(_)) {
            match event.keystroke.key.as_str() {
                "enter" => self.answer_credential(true, window, cx),
                "escape" => self.answer_credential(false, window, cx),
                _ => return,
            }
            window.prevent_default();
            cx.stop_propagation();
            return;
        }
        let keystroke = &event.keystroke;
        let mods = &keystroke.modifiers;
        let primary =
            ghostlink_platform::primary_modifier(mods.control, mods.platform) && !mods.alt;
        if primary && mods.shift && keystroke.key == "r" && self.remote {
            self.request_reconnect(cx);
            window.prevent_default();
            cx.stop_propagation();
            return;
        }
        let clipboard =
            !mods.alt && (mods.control && mods.shift || cfg!(target_os = "macos") && mods.platform);
        if clipboard && keystroke.key == "c" {
            self.copy(cx);
            window.prevent_default();
            cx.stop_propagation();
            return;
        }
        if clipboard && keystroke.key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                if self.cancel_drag() {
                    cx.notify();
                }
                self.send(Input::Paste(text), cx);
            }
            window.prevent_default();
            cx.stop_propagation();
            return;
        }
        // AltGr is delivered through Windows text input. Do not reinterpret it as Ctrl+Alt.
        let key = special_key(&keystroke.key).or_else(|| {
            if mods.control && !mods.alt || mods.alt && !mods.control {
                letter_key(&keystroke.key)
            } else {
                None
            }
        });
        if keystroke.key == "escape"
            && (self.selecting
                || self
                    .screen
                    .as_ref()
                    .is_some_and(|screen| screen.has_selection))
        {
            self.clear_selection(cx);
            window.prevent_default();
            cx.stop_propagation();
            return;
        }
        if let Some(key) = key {
            if !self.composition.is_empty() {
                return;
            }
            if self.cancel_drag() {
                cx.notify();
            }
            self.send(
                Input::Key(
                    key,
                    TerminalModifiers {
                        shift: mods.shift,
                        control: mods.control,
                        alt: mods.alt,
                        super_key: mods.platform,
                    },
                ),
                cx,
            );
            window.prevent_default();
            cx.stop_propagation();
        }
    }
    fn cursor_bounds(&self) -> Option<Bounds<Pixels>> {
        let bounds = self.bounds?;
        let cursor = self.screen.as_ref()?.cursor;
        Some(Bounds::new(
            bounds.origin
                + point(
                    px(PADDING) + self.cell_width * cursor.column as f32,
                    px(PADDING + LINE_HEIGHT * cursor.row as f32),
                ),
            size(self.cell_width, px(LINE_HEIGHT)),
        ))
    }
    fn answer_credential(&mut self, accept: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, field)) = &self.credential else {
            return;
        };
        let value = accept.then(|| field.read(cx).value().to_owned());
        if let Some(session) = &self.session {
            self.error = session
                .respond_credential(*id, value)
                .err()
                .map(|error| error.to_string());
        }
        if self.error.is_none() {
            self.credential = None;
            self.state = State::Authenticating;
            window.focus(&self.focus);
        }
        cx.notify();
    }
}
impl Focusable for TerminalView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.credential
            .as_ref()
            .map_or_else(|| self.focus.clone(), |(_, field)| field.focus_handle(cx))
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.focus_credential {
            if let Some((_, field)) = &self.credential {
                window.focus(&field.focus_handle(cx));
            }
            self.focus_credential = false;
        }
        let focused = self.focus_handle(cx).is_focused(window);
        let label = match &self.state {
            State::Starting => "Starting shell…".to_string(),
            State::Connecting => "Connecting…".into(),
            State::Authenticating => "Authenticating…".into(),
            State::AwaitingHostKey(_) => "Verify the server fingerprint to continue".into(),
            State::AwaitingCredential(_) => "Authentication required".into(),
            State::Running => "Connected".into(),
            State::Exited(code) => format!("Shell exited ({code})"),
            State::Failed(_) => "Connection failed".into(),
            State::Disconnected(_) => "Disconnected".into(),
        };
        let entity = cx.entity();
        let paint_entity = entity.clone();
        let screen = self.screen.clone();
        let prepare_screen = screen.clone();
        let composition = self.composition.clone();
        let host_prompt = if let State::AwaitingHostKey(prompt) = &self.state {
            Some(prompt.clone())
        } else {
            None
        };
        let connection_error = match &self.state {
            State::Failed(error) | State::Disconnected(error) => Some(error.clone()),
            State::Exited(code) if self.remote => Some(format!("Remote shell exited ({code})")),
            _ => None,
        };
        let credential_prompt = if let State::AwaitingCredential(prompt) = &self.state {
            self.credential
                .as_ref()
                .map(|(_, field)| (prompt.clone(), field.clone()))
        } else {
            None
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .border_1()
            .border_color(rgb(if focused { 0x40594c } else { 0x28312e }))
            .rounded(px(5.))
            .overflow_hidden()
            .bg(rgb(self
                .screen
                .as_ref()
                .map_or(0x111517, |screen| screen.background)))
            .track_focus(&self.focus)
            .key_context("Terminal")
            .on_key_down(cx.listener(Self::key_down))
            .when(!self.remote, |root| {
                root.child(
                    div()
                        .h(px(48.))
                        .flex_none()
                        .px_4()
                        .flex()
                        .items_center()
                        .gap_3()
                        .bg(rgb(0x171c1f))
                        .border_b_1()
                        .border_color(rgb(0x2b3336))
                        .child(div().text_color(rgb(0xa8d8c0)).text_lg().child(">_"))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .min_w_0()
                                .gap_1()
                                .child(
                                    div()
                                        .truncate()
                                        .text_sm()
                                        .line_height(px(18.))
                                        .child(self.title.clone()),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .line_height(px(16.))
                                        .truncate()
                                        .text_color(rgb(0x8e9b9e))
                                        .child(self.directory.clone()),
                                ),
                        )
                        .child(
                            div()
                                .ml_auto()
                                .px_2()
                                .py_1()
                                .rounded_sm()
                                .bg(rgb(0x20282b))
                                .text_xs()
                                .text_color(rgb(0xa8b6b7))
                                .child(if self.remote { "SSH" } else { "LOCAL" }),
                        ),
                )
            })
            .when_some(host_prompt, |root, prompt| {
                let challenge_id = prompt.id;
                let fingerprint = prompt.fingerprint.clone();
                let rejected_fingerprint = fingerprint.clone();
                root.child(
                    div()
                        .p_4()
                        .flex_none()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .bg(rgb(0x28251e))
                        .text_color(rgb(0xd7b77d))
                        .child(format!("First connection to {}", prompt.host))
                        .when_some(prompt.name.clone(), |root, name| {
                            root.child(format!("Host: {name}"))
                        })
                        .child("Compare this fingerprint with a trusted source before connecting.")
                        .child(format!("{} · {}", prompt.algorithm, prompt.fingerprint))
                        .child(
                            div()
                                .flex()
                                .gap_3()
                                .child(
                                    div()
                                        .id("reject-host-key")
                                        .px_3()
                                        .py_2()
                                        .rounded_sm()
                                        .bg(rgb(0x20282b))
                                        .cursor_pointer()
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if let Some(session) = &this.session {
                                                this.error = session
                                                    .verify_host(
                                                        challenge_id,
                                                        rejected_fingerprint.clone(),
                                                        false,
                                                    )
                                                    .err()
                                                    .map(|error| error.to_string());
                                            }
                                            this.state = State::Connecting;
                                            cx.notify();
                                        }))
                                        .child("Cancel"),
                                )
                                .child(
                                    div()
                                        .id("trust-host-key")
                                        .px_3()
                                        .py_2()
                                        .rounded_sm()
                                        .bg(rgb(0x334c43))
                                        .text_color(rgb(0xdce3df))
                                        .cursor_pointer()
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if let Some(session) = &this.session {
                                                this.error = session
                                                    .verify_host(
                                                        challenge_id,
                                                        fingerprint.clone(),
                                                        true,
                                                    )
                                                    .err()
                                                    .map(|error| error.to_string());
                                            }
                                            this.state = State::Connecting;
                                            cx.notify();
                                        }))
                                        .child("Trust key and connect"),
                                ),
                        ),
                )
            })
            .when_some(credential_prompt, |root, (prompt, field)| {
                root.child(
                    div()
                        .p_4()
                        .flex_none()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .bg(rgb(0x1b2023))
                        .child(prompt.detail)
                        .child(prompt.label)
                        .child(field)
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap_3()
                                .child(
                                    div()
                                        .id("cancel-authentication")
                                        .px_3()
                                        .py_2()
                                        .cursor_pointer()
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.answer_credential(false, window, cx)
                                        }))
                                        .child("Cancel"),
                                )
                                .child(
                                    div()
                                        .id("submit-authentication")
                                        .px_3()
                                        .py_2()
                                        .rounded_sm()
                                        .bg(rgb(0xa8d8c0))
                                        .text_color(rgb(0x111517))
                                        .cursor_pointer()
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.answer_credential(true, window, cx)
                                        }))
                                        .child("Continue"),
                                ),
                        ),
                )
            })
            .when_some(connection_error, |root, error| {
                root.child(
                    div()
                        .p_4()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_3()
                        .bg(rgb(0x28251e))
                        .text_color(rgb(0xd7b77d))
                        .child(div().flex_1().min_w_0().child(error))
                        .when(self.can_reconnect(), |bar| {
                            bar.child(
                                div()
                                    .id("reconnect")
                                    .debug_selector(|| "reconnect".into())
                                    .flex_none()
                                    .px_3()
                                    .py_2()
                                    .rounded_sm()
                                    .cursor_pointer()
                                    .bg(rgb(0x334c43))
                                    .text_color(rgb(0xdce3df))
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.request_reconnect(cx)),
                                    )
                                    .child(ghostlink_platform::shortcut_hint(
                                        "Reconnect · Ctrl Shift R",
                                    )),
                            )
                        }),
                )
            })
            .child(
                div()
                    .id("terminal-canvas")
                    .flex_1()
                    .min_h_0()
                    .cursor(if self.screen.as_ref().is_some_and(|s| s.mouse_tracking) {
                        CursorStyle::Arrow
                    } else {
                        CursorStyle::IBeam
                    })
                    .overflow_hidden()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, window, cx| {
                            this.pointer_down(event, window, cx);
                        }),
                    )
                    .on_mouse_down(MouseButton::Middle, cx.listener(Self::pointer_down))
                    .on_mouse_down(MouseButton::Right, cx.listener(Self::pointer_down))
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                        // Hover respects hit testing, so modal overlays don't send input
                        // to a full-screen application beneath them.
                        if this.mouse_capture.is_none() {
                            this.pointer_move(event, cx);
                        }
                    }))
                    .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                        this.pointer_scroll(event, cx)
                    }))
                    .child(
                        canvas(
                            move |bounds, window, cx| {
                                let _prepare_timer = perf::Timer::start("terminal-prepare-ms");
                                let font = font(ghostlink_platform::TERMINAL_FONT);
                                let font_id = window.text_system().resolve_font(&font);
                                let width = window
                                    .text_system()
                                    .advance(font_id, px(FONT_SIZE), 'M')
                                    .map(|s| s.width)
                                    .unwrap_or(px(9.));
                                let frame = entity.update(cx, |this, cx| {
                                    this.bounds = Some(bounds);
                                    this.cell_width = width;
                                    let columns = ((f32::from(bounds.size.width) - PADDING * 2.)
                                        / f32::from(width))
                                    .floor()
                                    .clamp(1., 1000.)
                                        as u16;
                                    let rows = ((f32::from(bounds.size.height) - PADDING * 2.)
                                        / LINE_HEIGHT)
                                        .floor()
                                        .clamp(1., 500.)
                                        as u16;
                                    let grid = GridSize::new(columns, rows).expect("clamped grid");
                                    if grid != this.grid
                                        && let Some(session) = &this.session
                                    {
                                        match session.send(Input::Resize(grid)) {
                                            Ok(()) => this.grid = grid,
                                            Err(error) => {
                                                this.error = Some(error.to_string());
                                                cx.notify();
                                            }
                                        }
                                    }
                                    prepare_screen.as_ref().map(|screen| {
                                        this.render_cache.prepare(screen, width, window)
                                    })
                                });
                                (width, frame)
                            },
                            move |bounds, (width, frame), window, cx| {
                                let _paint_timer = perf::Timer::start("terminal-paint-ms");
                                let focus = paint_entity.read(cx).focus.clone();
                                selection::install_drag_handlers(&paint_entity, window);
                                window.handle_input(
                                    &focus,
                                    ElementInputHandler::new(bounds, paint_entity.clone()),
                                    cx,
                                );
                                window.with_content_mask(Some(ContentMask { bounds }), |window| {
                                    if let Some(screen) = &screen {
                                        if let Some(frame) = &frame {
                                            frame.paint(
                                                bounds,
                                                width,
                                                &screen.selection,
                                                window,
                                                cx,
                                            );
                                        }
                                        let cursor = screen.cursor;
                                        if cursor.visible && focused {
                                            let origin = bounds.origin
                                                + point(
                                                    px(PADDING) + width * cursor.column as f32,
                                                    px(PADDING + cursor.row as f32 * LINE_HEIGHT),
                                                );
                                            let (offset, dimensions) = match cursor.style {
                                                2 => (
                                                    point(px(0.), px(LINE_HEIGHT - 2.)),
                                                    size(width, px(2.)),
                                                ),
                                                _ => (
                                                    point(px(0.), px(0.)),
                                                    size(px(2.), px(LINE_HEIGHT)),
                                                ),
                                            };
                                            window.paint_quad(fill(
                                                Bounds::new(origin + offset, dimensions),
                                                rgb(cursor.color),
                                            ));
                                            if !composition.is_empty() {
                                                let line = window.text_system().shape_line(
                                                    composition.clone().into(),
                                                    px(FONT_SIZE),
                                                    &[TextRun {
                                                        len: composition.len(),
                                                        font: font(
                                                            ghostlink_platform::TERMINAL_FONT,
                                                        ),
                                                        color: rgb(0xdce3df).into(),
                                                        background_color: Some(
                                                            rgb(0x20282b).into(),
                                                        ),
                                                        underline: Some(UnderlineStyle {
                                                            thickness: px(1.),
                                                            color: None,
                                                            wavy: false,
                                                        }),
                                                        strikethrough: None,
                                                    }],
                                                    None,
                                                );
                                                let _ =
                                                    line.paint(origin, px(LINE_HEIGHT), window, cx);
                                            }
                                        }
                                    }
                                });
                            },
                        )
                        .size_full(),
                    ),
            )
            .child(
                div()
                    .h(px(24.))
                    .flex_none()
                    .px_3()
                    .min_w_0()
                    .gap_3()
                    .flex()
                    .items_center()
                    .text_xs()
                    .text_color(rgb(
                        if matches!(self.state, State::Failed(_) | State::Disconnected(_))
                            || self.error.is_some()
                        {
                            0xd7b77d
                        } else {
                            0x8e9b9e
                        },
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(self.error.clone().unwrap_or(label)),
                    )
                    .child(div().flex_none().child(format!(
                        "{} × {}",
                        self.grid.columns(),
                        self.grid.rows()
                    ))),
            )
    }
}

fn special_key(key: &str) -> Option<Key> {
    Some(match key {
        "enter" => Key::Enter,
        "tab" => Key::Tab,
        "backspace" => Key::Backspace,
        "escape" => Key::Escape,
        "up" => Key::Up,
        "down" => Key::Down,
        "left" => Key::Left,
        "right" => Key::Right,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "delete" => Key::Delete,
        "insert" => Key::Insert,
        "f1" => Key::F1,
        "f2" => Key::F2,
        "f3" => Key::F3,
        "f4" => Key::F4,
        "f5" => Key::F5,
        "f6" => Key::F6,
        "f7" => Key::F7,
        "f8" => Key::F8,
        "f9" => Key::F9,
        "f10" => Key::F10,
        "f11" => Key::F11,
        "f12" => Key::F12,
        _ => return None,
    })
}
fn letter_key(key: &str) -> Option<Key> {
    let keys = [
        Key::A,
        Key::B,
        Key::C,
        Key::D,
        Key::E,
        Key::F,
        Key::G,
        Key::H,
        Key::I,
        Key::J,
        Key::K,
        Key::L,
        Key::M,
        Key::N,
        Key::O,
        Key::P,
        Key::Q,
        Key::R,
        Key::S,
        Key::T,
        Key::U,
        Key::V,
        Key::W,
        Key::X,
        Key::Y,
        Key::Z,
    ];
    if key.len() == 1 && key.as_bytes()[0].is_ascii_lowercase() {
        Some(keys[(key.as_bytes()[0] - b'a') as usize])
    } else {
        None
    }
}
fn utf16_range(text: &str, range: Range<usize>) -> Range<usize> {
    let byte = |target| {
        let mut count = 0;
        for (index, ch) in text.char_indices() {
            if count >= target {
                return index;
            }
            count += ch.len_utf16();
        }
        text.len()
    };
    let start = byte(range.start);
    start..byte(range.end).max(start)
}
impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let bytes = utf16_range(&self.composition, range);
        *actual = Some(
            self.composition[..bytes.start].encode_utf16().count()
                ..self.composition[..bytes.end].encode_utf16().count(),
        );
        Some(self.composition[bytes].to_owned())
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.composition_selection.clone(),
            reversed: false,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        (!self.composition.is_empty()).then(|| 0..self.composition.encode_utf16().count())
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.composition.clear();
        self.composition_selection = 0..0;
        cx.notify();
    }
    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_selecting = self.cancel_drag();
        let appearance_changed = !self.composition.is_empty() || was_selecting;
        self.composition.clear();
        self.composition_selection = 0..0;
        if !text.is_empty() {
            self.send(Input::Text(text.to_owned()), cx);
        }
        if appearance_changed {
            cx.notify();
        }
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = utf16_range(
            &self.composition,
            range.unwrap_or(0..self.composition.encode_utf16().count()),
        );
        let offset = self.composition[..range.start].encode_utf16().count();
        self.composition.replace_range(range, text);
        let selected = selected.unwrap_or_else(|| {
            let end = text.encode_utf16().count();
            end..end
        });
        self.composition_selection = offset + selected.start..offset + selected.end;
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.cursor_bounds()
    }
    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(0)
    }
}

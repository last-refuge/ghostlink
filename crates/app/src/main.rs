#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod brand;
mod chrome;
mod connect;
mod field;
mod layout;
#[cfg(test)]
mod tests;
mod ui;

use connect::{ConnectEvent, ConnectForm};
use gpui::{
    AnyElement, App, Application, Bounds, Context, CursorStyle, Div, Entity, FocusHandle,
    Focusable, FontWeight, KeyBinding, MouseButton, MouseMoveEvent, MouseUpEvent, ObjectFit,
    Pixels, ScrollHandle, Size, Stateful, TitlebarOptions, Window, WindowBounds, WindowOptions,
    actions, canvas, div, img, prelude::*, px, relative, rgb, rgba, size,
};
use layout::{Axis, Layout};
use session::{SshConnection, SshProfile};
use std::collections::HashMap;
use terminal_view::{TerminalEvent, TerminalView};

actions!(
    ghostlink,
    [
        NewTerminal,
        CloseTerminal,
        ToggleSidebar,
        ConnectSsh,
        SplitRight,
        SplitBelow,
        NextPane,
        ZoomPane,
        NextTab,
        PreviousTab,
        NextSession,
        PreviousSession
    ]
);

struct Pane {
    id: usize,
    terminal: Entity<TerminalView>,
}
struct Tab {
    id: usize,
    title: String,
    panes: Vec<Pane>,
    layout: Layout,
    active_pane: usize,
    zoom: Option<usize>,
}
struct OpenSession {
    id: usize,
    title: String,
    connection: SshConnection,
    tabs: Vec<Tab>,
    active: usize,
    next_tab: usize,
    tabs_scroll: ScrollHandle,
    recovery_owner: Option<usize>,
}
#[derive(Clone, Copy)]
struct SplitDrag {
    id: usize,
    axis: Axis,
    bounds: Bounds<Pixels>,
}
struct Workspace {
    focus: FocusHandle,
    sessions: Vec<OpenSession>,
    active: usize,
    next_id: usize,
    sidebar: bool,
    connect_form: Option<Entity<ConnectForm>>,
    split_bounds: HashMap<usize, Bounds<Pixels>>,
    drag: Option<SplitDrag>,
    viewport: Size<Pixels>,
}
impl Workspace {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        Self {
            focus,
            sessions: Vec::new(),
            active: 0,
            next_id: 0,
            sidebar: true,
            connect_form: None,
            split_bounds: HashMap::new(),
            drag: None,
            viewport: window.viewport_size(),
        }
    }
    fn id(&mut self) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
    fn active_tab(&self) -> Option<&Tab> {
        let session = self.sessions.get(self.active)?;
        session.tabs.get(session.active)
    }
    fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        let session = self.sessions.get_mut(self.active)?;
        session.tabs.get_mut(session.active)
    }
    fn sync_focus(&mut self, window: &Window, cx: &App) {
        if let Some(tab) = self.active_tab_mut()
            && let Some(pane) = tab
                .panes
                .iter()
                .find(|p| p.terminal.focus_handle(cx).is_focused(window))
        {
            tab.active_pane = pane.id;
        }
    }
    fn focus_active(&self, window: &mut Window, cx: &App) {
        let focus = self
            .active_tab()
            .and_then(|tab| tab.panes.iter().find(|p| p.id == tab.active_pane))
            .map_or_else(|| self.focus.clone(), |pane| pane.terminal.focus_handle(cx));
        window.focus(&focus);
    }
    fn reset_navigation(&mut self) {
        self.drag = None;
        self.split_bounds.clear();
        if let Some(session) = self.sessions.get(self.active) {
            session.tabs_scroll.scroll_to_item(session.active);
        }
    }
    fn open_ssh(
        &mut self,
        profile: SshProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let connection = SshConnection::new(profile)?;
        self.sync_focus(window, cx);
        let id = self.id();
        let title = connection.profile().label();
        self.sessions.push(OpenSession {
            id,
            title,
            connection,
            tabs: Vec::new(),
            active: 0,
            next_tab: 1,
            tabs_scroll: ScrollHandle::new(),
            recovery_owner: None,
        });
        self.active = self.sessions.len() - 1;
        self.open_terminal(false, Axis::Columns, window, cx);
        Ok(())
    }
    fn open_terminal(
        &mut self,
        split: bool,
        axis: Axis,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.connect_form.is_some() {
            return;
        }
        let Some(session) = self.sessions.get(self.active) else {
            self.show_connect(window, cx);
            return;
        };
        let connection = session.connection.clone();
        self.sync_focus(window, cx);
        let terminal = cx.new(|cx| TerminalView::ssh_in(connection, cx));
        let pane_id = self.id();
        let session_id = self.sessions[self.active].id;
        cx.subscribe_in(
            &terminal,
            window,
            move |this, terminal, event: &TerminalEvent, window, cx| {
                if let Some(session) = this.sessions.iter_mut().find(|s| s.id == session_id) {
                    match event {
                        TerminalEvent::Reconnecting => session.recovery_owner = Some(pane_id),
                        TerminalEvent::StateChanged
                            if session.recovery_owner == Some(pane_id)
                                && terminal.read(cx).is_running() =>
                        {
                            session.recovery_owner = None;
                            // The requested pane authenticates first, keeping prompts in place.
                            // Siblings then reuse that transport; intentionally exited shells stay closed.
                            let recover: Vec<_> = session
                                .tabs
                                .iter()
                                .flat_map(|tab| &tab.panes)
                                .filter(|p| p.terminal.read(cx).is_disconnected())
                                .map(|p| p.terminal.clone())
                                .collect();
                            for pane in recover {
                                pane.update(cx, |view, cx| view.reconnect(cx));
                            }
                        }
                        _ => {}
                    }
                }
                // A pending new tab must not hide the initial connection's prompt.
                // Keep other sessions undisturbed when a background connection asks.
                if matches!(event, TerminalEvent::AuthenticationRequired)
                    && this.connect_form.is_none()
                    && this
                        .sessions
                        .get(this.active)
                        .is_some_and(|session| session.id == session_id)
                {
                    this.reveal_pane(pane_id, window, cx);
                }
                cx.notify();
            },
        )
        .detach();
        window.focus(&terminal.focus_handle(cx));
        let pane = Pane {
            id: pane_id,
            terminal,
        };
        let layout_id = self.id();
        let session = &mut self.sessions[self.active];
        if split && !session.tabs.is_empty() {
            let tab = &mut session.tabs[session.active];
            assert!(tab.layout.split(tab.active_pane, pane_id, layout_id, axis));
            tab.panes.push(pane);
            tab.active_pane = pane_id;
            tab.zoom = None;
        } else {
            let title = format!("Terminal {}", session.next_tab);
            session.next_tab += 1;
            session.active = session.tabs.len();
            session.tabs.push(Tab {
                id: layout_id,
                title,
                panes: vec![pane],
                layout: Layout::Pane(pane_id),
                active_pane: pane_id,
                zoom: None,
            });
        }
        self.reset_navigation();
        cx.notify();
    }
    fn switch_session(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.sessions.iter().position(|s| s.id == id) else {
            return;
        };
        self.sync_focus(window, cx);
        self.active = index;
        self.reset_navigation();
        let pending = self.sessions[index]
            .tabs
            .iter()
            .flat_map(|tab| &tab.panes)
            .find(|pane| pane.terminal.read(cx).requires_authentication())
            .map(|pane| pane.id);
        if let Some(pane) = pending {
            self.reveal_pane(pane, window, cx);
        } else {
            self.focus_active(window, cx);
        }
        cx.notify();
    }
    fn reveal_pane(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_focus(window, cx);
        if let Some(session) = self.sessions.get_mut(self.active)
            && let Some((index, tab)) = session
                .tabs
                .iter_mut()
                .enumerate()
                .find(|(_, tab)| tab.panes.iter().any(|pane| pane.id == id))
        {
            session.active = index;
            tab.active_pane = id;
            if tab.zoom.is_some() {
                tab.zoom = Some(id);
            }
            self.reset_navigation();
            self.focus_active(window, cx);
            cx.notify();
        }
    }
    fn switch_tab(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_focus(window, cx);
        if let Some(session) = self.sessions.get_mut(self.active)
            && let Some(index) = session.tabs.iter().position(|tab| tab.id == id)
        {
            session.active = index;
            self.reset_navigation();
            self.focus_active(window, cx);
            cx.notify();
        }
    }
    fn cycle_tab(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(session) = self.sessions.get(self.active) {
            let index =
                (session.active as isize + step).rem_euclid(session.tabs.len() as isize) as usize;
            self.switch_tab(session.tabs[index].id, window, cx);
        }
    }
    fn cycle_session(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.sessions.is_empty() {
            let index =
                (self.active as isize + step).rem_euclid(self.sessions.len() as isize) as usize;
            self.switch_session(self.sessions[index].id, window, cx);
        }
    }
    fn close_session(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.sessions.iter().position(|s| s.id == id) else {
            return;
        };
        self.sync_focus(window, cx);
        for pane in self.sessions[index].tabs.iter().flat_map(|tab| &tab.panes) {
            pane.terminal.update(cx, |terminal, _| terminal.close());
        }
        self.sessions.remove(index);
        if index < self.active {
            self.active -= 1;
        }
        self.active = self.active.min(self.sessions.len().saturating_sub(1));
        self.reset_navigation();
        self.focus_active(window, cx);
        cx.notify();
    }
    fn close_tab(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_focus(window, cx);
        let Some(session) = self.sessions.get_mut(self.active) else {
            return;
        };
        let Some(index) = session.tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        if session.tabs.len() == 1 {
            let id = session.id;
            self.close_session(id, window, cx);
            return;
        }
        for pane in &session.tabs[index].panes {
            pane.terminal.update(cx, |terminal, _| terminal.close());
        }
        session.tabs.remove(index);
        if index < session.active {
            session.active -= 1;
        }
        session.active = session.active.min(session.tabs.len() - 1);
        self.reset_navigation();
        self.focus_active(window, cx);
        cx.notify();
    }
    fn close_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_focus(window, cx);
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        if tab.panes.len() == 1 {
            let id = tab.id;
            self.close_tab(id, window, cx);
            return;
        }
        let target = tab.active_pane;
        if let Some(pane) = tab.panes.iter().find(|pane| pane.id == target) {
            pane.terminal.update(cx, |terminal, _| terminal.close());
        }
        assert!(tab.layout.remove(target));
        tab.panes.retain(|p| p.id != target);
        tab.active_pane = tab.layout.panes()[0];
        tab.zoom = None;
        self.reset_navigation();
        self.focus_active(window, cx);
        cx.notify();
    }
    fn next_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_focus(window, cx);
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        let ids = tab.layout.panes();
        let index = ids
            .iter()
            .position(|id| *id == tab.active_pane)
            .unwrap_or(0);
        tab.active_pane = ids[(index + 1) % ids.len()];
        if tab.zoom.is_some() {
            tab.zoom = Some(tab.active_pane);
        }
        self.focus_active(window, cx);
        cx.notify();
    }
    fn zoom(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.sync_focus(window, cx);
        if let Some(tab) = self.active_tab_mut() {
            tab.zoom = if tab.zoom.is_some() {
                None
            } else {
                Some(tab.active_pane)
            };
            cx.notify();
        }
    }
    fn show_connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.connect_form.is_some() {
            return;
        }
        self.sync_focus(window, cx);
        let form = cx.new(ConnectForm::new);
        cx.subscribe_in(&form, window, |this, form, event, window, cx| {
            match event {
                ConnectEvent::Submit { profile } => {
                    // Clear the modal before opening the first tab.
                    this.connect_form = None;
                    if let Err(error) = this.open_ssh(profile.clone(), window, cx) {
                        this.connect_form = Some(form.clone());
                        form.update(cx, |form, cx| form.set_error(error.to_string(), cx));
                        form.read(cx).focus(window, cx);
                    }
                }
                ConnectEvent::Cancel => {
                    this.connect_form = None;
                    this.focus_active(window, cx);
                }
            }
            cx.notify();
        })
        .detach();
        form.read(cx).focus(window, cx);
        self.connect_form = Some(form);
        cx.notify();
    }
    fn render_layout(&self, layout: &Layout, cx: &mut Context<Self>) -> AnyElement {
        match layout {
            Layout::Pane(id) => {
                let pane = self
                    .active_tab()
                    .expect("active layout")
                    .panes
                    .iter()
                    .find(|pane| pane.id == *id)
                    .expect("pane tree and sessions agree");
                div()
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .child(pane.terminal.clone())
                    .into_any_element()
            }
            Layout::Split {
                id,
                axis,
                ratio,
                first,
                second,
            } => {
                let (id, axis, ratio) = (*id, *axis, *ratio);
                let entity = cx.entity();
                div()
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .relative()
                    .flex()
                    .when(axis == Axis::Rows, |root| root.flex_col())
                    .child(
                        canvas(
                            move |bounds, _, cx| {
                                entity.update(cx, |this, _| {
                                    this.split_bounds.insert(id, bounds);
                                });
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
                    .child(
                        div()
                            .flex_none()
                            .min_w_0()
                            .min_h_0()
                            .when(axis == Axis::Columns, |root| {
                                root.w(relative(ratio)).h_full()
                            })
                            .when(axis == Axis::Rows, |root| root.h(relative(ratio)).w_full())
                            .child(self.render_layout(first, cx)),
                    )
                    .child(
                        div()
                            .id(("divider", id))
                            .flex_none()
                            .bg(rgb(0x171c1f))
                            .hover(|s| s.bg(rgb(0x648c7b)))
                            .when(axis == Axis::Columns, |root| {
                                root.w(px(5.)).h_full().cursor(CursorStyle::ResizeLeftRight)
                            })
                            .when(axis == Axis::Rows, |root| {
                                root.h(px(5.)).w_full().cursor(CursorStyle::ResizeUpDown)
                            })
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    if let Some(bounds) = this.split_bounds.get(&id) {
                                        this.drag = Some(SplitDrag {
                                            id,
                                            axis,
                                            bounds: *bounds,
                                        });
                                    }
                                    cx.stop_propagation();
                                }),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .child(self.render_layout(second, cx)),
                    )
                    .into_any_element()
            }
        }
    }
}
fn button(id: &'static str, label: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(28.))
        .px_2()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .whitespace_nowrap()
        .rounded_sm()
        .cursor_pointer()
        .hover(|s| s.bg(rgb(0x293438)))
        .child(label)
        .tooltip(move |_, cx| {
            ui::hint(
                match id {
                    "new-tab" => "New tab · Ctrl Shift T",
                    "split-right" => "Split right · Ctrl Shift D",
                    "split-below" => "Split below · Ctrl Shift E",
                    "zoom-pane" => "Zoom / restore pane · Ctrl Shift Z",
                    "connect-ssh" => "Connect to a server · Ctrl Shift K",
                    _ => label,
                },
                cx,
            )
        })
}
impl Workspace {
    fn session_list(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .w(px(248.))
            .flex_none()
            .flex()
            .flex_col()
            .bg(rgb(0x171c1f))
            .border_r_1()
            .border_color(rgb(0x2b3336))
            .child(
                div()
                    .h(px(42.))
                    .px_4()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(0x8e9b9e))
                    .child("Sessions")
                    .child(div().ml_auto().child(self.sessions.len().to_string())),
            )
            .child(
                div()
                    .id("session-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_2()
                    .children(self.sessions.iter().enumerate().map(|(index, session)| {
                        let id = session.id;
                        let title = session.title.clone();
                        let attention = session
                            .tabs
                            .iter()
                            .flat_map(|tab| &tab.panes)
                            .any(|pane| pane.terminal.read(cx).requires_authentication());
                        let disconnected = session
                            .tabs
                            .iter()
                            .flat_map(|tab| &tab.panes)
                            .filter(|p| p.terminal.read(cx).is_disconnected())
                            .count();
                        div()
                            .id(("session", id))
                            .mb_1()
                            .px_3()
                            .py_2()
                            .rounded_sm()
                            .border_l_2()
                            .border_color(rgb(if index == self.active {
                                0x82b69c
                            } else {
                                0x171c1f
                            }))
                            .bg(rgb(if index == self.active {
                                0x24332d
                            } else {
                                0x171c1f
                            }))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(0x26312e)))
                            .tooltip(move |_, cx| ui::hint(title.clone(), cx))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.switch_session(id, window, cx)
                            }))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_sm()
                                            .line_height(px(20.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(session.title.clone()),
                                    )
                                    .child(
                                        div()
                                            .id(("close-session", id))
                                            .w(px(22.))
                                            .h(px(22.))
                                            .flex_none()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded_sm()
                                            .text_color(rgb(0x83928d))
                                            .hover(|s| {
                                                s.bg(rgb(0x39433f)).text_color(rgb(0xdce3df))
                                            })
                                            .tooltip(|_, cx| {
                                                ui::hint("Close session and its tabs", cx)
                                            })
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                cx.stop_propagation();
                                                this.close_session(id, window, cx);
                                            }))
                                            .child("×"),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .line_height(px(16.))
                                    .text_color(rgb(if attention || disconnected > 0 {
                                        0xd7b77d
                                    } else {
                                        0x8e9b9e
                                    }))
                                    .child(if attention {
                                        "Authentication needed".to_owned()
                                    } else if disconnected > 0 {
                                        format!(
                                            "Disconnected · {disconnected} {}",
                                            if disconnected == 1 { "pane" } else { "panes" }
                                        )
                                    } else {
                                        format!(
                                            "{} {}",
                                            session.tabs.len(),
                                            if session.tabs.len() == 1 {
                                                "tab"
                                            } else {
                                                "tabs"
                                            }
                                        )
                                    }),
                            )
                    }))
                    .when(self.sessions.is_empty(), |root| {
                        root.child(
                            div()
                                .px_2()
                                .py_2()
                                .text_xs()
                                .text_color(rgb(0x8e9b9e))
                                .child("No open sessions"),
                        )
                    }),
            )
            .child(
                div()
                    .px_4()
                    .py_3()
                    .text_xs()
                    .line_height(px(16.))
                    .text_color(rgb(0x788781))
                    .child("Ctrl Shift K  ·  Connect"),
            )
            .into_any_element()
    }
    fn tab_strip(&self, cx: &mut Context<Self>) -> AnyElement {
        let session = &self.sessions[self.active];
        let tab = &session.tabs[session.active];
        div()
            .id("tab-bar")
            .debug_selector(|| "tab-bar".into())
            .h(px(42.))
            .flex_none()
            .flex()
            .items_center()
            .px_2()
            .gap_1()
            .bg(rgb(0x171c1f))
            .border_b_1()
            .border_color(rgb(0x2b3336))
            .child(
                div()
                    .id("session-tabs")
                    .debug_selector(|| "tabs".into())
                    .min_w_0()
                    .h_full()
                    .overflow_x_scroll()
                    .track_scroll(&session.tabs_scroll)
                    .flex()
                    .items_end()
                    .gap_1()
                    .children(session.tabs.iter().enumerate().map(|(index, tab)| {
                        let id = tab.id;
                        div()
                            .id(("tab", id))
                            .w(px(148.))
                            .h(px(34.))
                            .px_3()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap_2()
                            .rounded_t_sm()
                            .cursor_pointer()
                            .text_color(rgb(if index == session.active {
                                0xdce3df
                            } else {
                                0x9eada5
                            }))
                            .bg(rgb(if index == session.active {
                                0x222d29
                            } else {
                                0x171c1f
                            }))
                            .border_b_2()
                            .border_color(rgb(if index == session.active {
                                0x94c4aa
                            } else {
                                0x171c1f
                            }))
                            .hover(|s| s.bg(rgb(0x26312e)))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.switch_tab(id, window, cx)
                            }))
                            .child(div().min_w_0().flex_1().truncate().child(tab.title.clone()))
                            .child(
                                div()
                                    .id(("close-tab", id))
                                    .w(px(20.))
                                    .h(px(20.))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_sm()
                                    .text_color(rgb(0x83928d))
                                    .hover(|s| s.bg(rgb(0x39433f)).text_color(rgb(0xdce3df)))
                                    .tooltip(|_, cx| ui::hint("Close tab", cx))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.close_tab(id, window, cx);
                                    }))
                                    .child("×"),
                            )
                    })),
            )
            .child(
                button("new-tab", "+")
                    .debug_selector(|| "new-tab".into())
                    .w(px(28.))
                    .text_lg()
                    .text_color(rgb(0x9eafa6))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_terminal(false, Axis::Columns, window, cx)
                    })),
            )
            .child(
                div()
                    .id("pane-actions")
                    .debug_selector(|| "pane-actions".into())
                    .ml_auto()
                    .pl_2()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_xs()
                    .text_color(rgb(0xa4b1ab))
                    .child(button("split-right", "Split right").on_click(cx.listener(
                        |this, _, window, cx| this.open_terminal(true, Axis::Columns, window, cx),
                    )))
                    .child(button("split-below", "Split below").on_click(cx.listener(
                        |this, _, window, cx| this.open_terminal(true, Axis::Rows, window, cx),
                    )))
                    .when(tab.panes.len() > 1, |root| {
                        root.child(
                            button(
                                "zoom-pane",
                                if tab.zoom.is_some() {
                                    "Restore"
                                } else {
                                    "Zoom"
                                },
                            )
                            .on_click(cx.listener(|this, _, window, cx| this.zoom(window, cx))),
                        )
                    }),
            )
            .into_any_element()
    }
    fn session_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(tab) = self.active_tab() else {
            let artwork = brand::welcome_image_size(self.viewport, self.sidebar);
            return div()
                .id("welcome-screen")
                .debug_selector(|| "welcome-screen".into())
                .flex_1()
                .min_w_0()
                .min_h_0()
                .overflow_y_scroll()
                .child(
                    div()
                        .w_full()
                        .min_h_full()
                        .p_6()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap_3()
                        .child(
                            img(brand::illustration())
                                .id("welcome-artwork")
                                .debug_selector(|| "welcome-artwork".into())
                                .w(artwork.width)
                                .h(artwork.height)
                                .flex_none()
                                .object_fit(ObjectFit::Contain),
                        )
                        .child(
                            div()
                                .text_xl()
                                .line_height(px(28.))
                                .text_center()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Connect to your server"),
                        )
                        .child(
                            div()
                                .max_w_full()
                                .text_center()
                                .text_color(rgb(0x8e9b9e))
                                .child("Your sessions live on the left. Each keeps its own tabs."),
                        )
                        .child(
                            div()
                                .mt_2()
                                .text_center()
                                .text_xs()
                                .text_color(rgb(0xa8d8c0))
                                .child("Ctrl Shift K   Connect with SSH"),
                        ),
                )
                .into_any_element();
        };
        let tree = tab.zoom.map_or_else(|| tab.layout.clone(), Layout::Pane);
        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(self.tab_strip(cx))
            .child(
                div()
                    .p_2()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_layout(&tree, cx)),
            )
            .into_any_element()
    }
}
impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.viewport != window.viewport_size() {
            self.viewport = window.viewport_size();
            if let Some(session) = self.sessions.get(self.active) {
                session.tabs_scroll.scroll_to_item(session.active);
            }
            // GPUI updates a scroll handle's viewport during prepaint. Follow
            // the selected tab once more after those new bounds are available.
            cx.defer_in(window, |workspace, _, cx| {
                if let Some(session) = workspace.sessions.get(workspace.active) {
                    session.tabs_scroll.scroll_to_item(session.active);
                    cx.notify();
                }
            });
        }
        let body = self.session_body(cx);
        let sidebar = self.session_list(cx);
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(rgb(0x111517))
            .text_color(rgb(0xdce3df))
            .font_family("Segoe UI")
            .text_sm()
            .line_height(px(20.))
            .track_focus(&self.focus)
            .key_context("Workspace")
            .on_action(cx.listener(|this, _: &NewTerminal, window, cx| {
                this.open_terminal(false, Axis::Columns, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CloseTerminal, window, cx| {
                if this.connect_form.is_none() {
                    this.close_pane(window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.sidebar = !this.sidebar;
                this.reset_navigation();
                cx.notify();
            }))
            .on_action(
                cx.listener(|this, _: &ConnectSsh, window, cx| this.show_connect(window, cx)),
            )
            .on_action(cx.listener(|this, _: &SplitRight, window, cx| {
                this.open_terminal(true, Axis::Columns, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SplitBelow, window, cx| {
                this.open_terminal(true, Axis::Rows, window, cx)
            }))
            .on_action(cx.listener(|this, _: &NextPane, window, cx| {
                if this.connect_form.is_none() {
                    this.next_pane(window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &ZoomPane, window, cx| {
                if this.connect_form.is_none() {
                    this.zoom(window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| {
                if this.connect_form.is_none() {
                    this.cycle_tab(1, window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &PreviousTab, window, cx| {
                if this.connect_form.is_none() {
                    this.cycle_tab(-1, window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &NextSession, window, cx| {
                if this.connect_form.is_none() {
                    this.cycle_session(1, window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &PreviousSession, window, cx| {
                if this.connect_form.is_none() {
                    this.cycle_session(-1, window, cx)
                }
            }))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if let Some(drag) = this.drag {
                    let ratio = match drag.axis {
                        Axis::Columns => {
                            (event.position.x - drag.bounds.left()) / drag.bounds.size.width
                        }
                        Axis::Rows => {
                            (event.position.y - drag.bounds.top()) / drag.bounds.size.height
                        }
                    };
                    if let Some(tab) = this.active_tab_mut() {
                        tab.layout.set_ratio(drag.id, ratio);
                        cx.notify();
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.drag = None),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.drag = None),
            )
            .child(
                div()
                    .h(px(44.))
                    .flex_none()
                    .px_5()
                    .flex()
                    .items_center()
                    .gap_3()
                    .bg(rgb(0x111517))
                    .border_b_1()
                    .border_color(rgb(0x2b3336))
                    .child(
                        div()
                            .id("brand-logo")
                            .debug_selector(|| "brand-logo".into())
                            .w(px(180.))
                            .h(px(40.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .overflow_hidden()
                            // Keep the asset's 3:1 ratio. Only its outer padding is clipped.
                            .child(
                                img(brand::logo())
                                    .id("brand-logo-image")
                                    .debug_selector(|| "brand-logo-image".into())
                                    .w(px(180.))
                                    .h(px(60.))
                                    .flex_none()
                                    .object_fit(ObjectFit::Contain),
                            ),
                    )
                    .when_some(
                        self.sessions
                            .get(self.active)
                            .map(|session| session.title.clone()),
                        |root, title| {
                            let hint = title.clone();
                            root.child(
                                div()
                                    .w(px(1.))
                                    .h(px(16.))
                                    .mx_1()
                                    .flex_none()
                                    .bg(rgb(0x35403b)),
                            )
                            .child(
                                div()
                                    .id("current-destination")
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(rgb(0xa4b5ac))
                                    .tooltip(move |_, cx| ui::hint(hint.clone(), cx))
                                    .child(title),
                            )
                        },
                    )
                    .child(
                        div().ml_auto().child(
                            button("connect-ssh", "Connect SSH")
                                .bg(rgb(0x293e36))
                                .text_color(rgb(0xc4e1d2))
                                .on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.show_connect(window, cx)
                                    }),
                                ),
                        ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .when(self.sidebar, |root| root.child(sidebar))
                    .child(body),
            )
            .when_some(self.connect_form.clone(), |root, form| {
                root.child(
                    div()
                        .id("ssh-modal")
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_start()
                        .justify_center()
                        .py_6()
                        .overflow_y_scroll()
                        .occlude()
                        .bg(rgba(0x000000a0))
                        .child(form),
                )
            })
    }
}
fn bind_shortcuts(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-shift-t", NewTerminal, Some("Workspace")),
        KeyBinding::new("ctrl-shift-w", CloseTerminal, Some("Workspace")),
        KeyBinding::new("ctrl-shift-b", ToggleSidebar, Some("Workspace")),
        KeyBinding::new("ctrl-shift-k", ConnectSsh, Some("Workspace")),
        KeyBinding::new("ctrl-shift-d", SplitRight, Some("Workspace")),
        KeyBinding::new("ctrl-shift-e", SplitBelow, Some("Workspace")),
        KeyBinding::new("ctrl-shift-j", NextPane, Some("Workspace")),
        KeyBinding::new("ctrl-shift-z", ZoomPane, Some("Workspace")),
        KeyBinding::new("ctrl-tab", NextTab, Some("Workspace")),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, Some("Workspace")),
        KeyBinding::new("ctrl-shift-pagedown", NextSession, Some("Workspace")),
        KeyBinding::new("ctrl-shift-pageup", PreviousSession, Some("Workspace")),
    ]);
}
fn main() {
    Application::new().run(|cx| {
        bind_shortcuts(cx);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let bounds = Bounds::centered(None, size(px(1280.), px(820.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(700.), px(520.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Ghostlink".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                chrome::apply(window);
                cx.new(|cx| Workspace::new(window, cx))
            },
        )
        .expect("Unable to open Ghostlink window");
        cx.activate(true);
    });
}

mod appearance;
mod credentials;
mod editor;
mod list;
mod routing;
use crate::host_options::Authentication;
use crate::{
    connections::{ConnectionStore, SavedConnection},
    field::Field,
};
use editor::Editor;
use ghostlink_platform::secrets::{self, SecretKind};
use gpui::{
    App, ClickEvent, Context, Div, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    KeyDownEvent, Stateful, Subscription, Window, div, prelude::*, px, rgb,
};
use session::SshProfile;
use std::collections::HashSet;
use std::sync::Arc;
use zeroize::Zeroizing;

pub enum ConnectEvent {
    Submit { profile: SshProfile, name: String },
    Cancel,
}
#[derive(Clone, Copy)]
enum Pending {
    Browse,
    New,
    Select(u64),
    Connect(u64),
    Close,
    Delete(u64),
}
pub struct ConnectForm {
    store: ConnectionStore,
    focus: FocusHandle,
    search: Entity<Field>,
    entries: Vec<SavedConnection>,
    selected: Option<u64>,
    editor: Option<Editor>,
    can_cancel: bool,
    error: Option<String>,
    load_failed: bool,
    undo: Option<SavedConnection>,
    collapsed: HashSet<String>,
    pending: Option<Pending>,
    compact_list: bool,
    connecting: bool,
    _search_subscription: Subscription,
    editor_subscriptions: Vec<Subscription>,
    content_scroll: gpui::ScrollHandle,
}
impl EventEmitter<ConnectEvent> for ConnectForm {}
impl ConnectForm {
    pub fn new(store: ConnectionStore, can_cancel: bool, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| Field::new("", false, cx));
        let subscription = cx.observe(&search, |this, _, cx| {
            this.normalize_selection(cx);
            cx.notify();
        });
        let mut this = Self {
            store,
            focus: cx.focus_handle(),
            search,
            entries: Vec::new(),
            selected: None,
            editor: None,
            can_cancel,
            error: None,
            load_failed: false,
            undo: None,
            collapsed: HashSet::new(),
            pending: None,
            compact_list: false,
            connecting: false,
            _search_subscription: subscription,
            editor_subscriptions: Vec::new(),
            content_scroll: gpui::ScrollHandle::new(),
        };
        this.reload(cx);
        if !this.load_failed {
            if this.entries.is_empty() {
                this.begin_add(cx);
            } else {
                this.begin_edit(cx);
            }
        }
        this
    }
    pub fn focus(&self, window: &mut Window, cx: &App) {
        window.focus(&if self.compact_list {
            self.search.focus_handle(cx)
        } else if let Some(editor) = self.editor.as_ref().filter(|editor| editor.id.is_none()) {
            editor.address.focus_handle(cx)
        } else if f32::from(window.viewport_size().width) < 760. && !self.compact_list {
            self.focus.clone()
        } else {
            self.search.focus_handle(cx)
        });
    }
    fn focus_editor(&self, window: &mut Window, cx: &App) {
        if let Some(editor) = &self.editor {
            window.focus(&editor.address.focus_handle(cx));
        }
    }
    pub fn set_error(&mut self, error: String, cx: &mut Context<Self>) {
        self.error = Some(error);
        cx.notify();
    }
    fn visible(&self, cx: &App) -> Vec<&SavedConnection> {
        let query = self.search.read(cx).value().trim().to_lowercase();
        self.entries
            .iter()
            .filter(|host| {
                [
                    &host.label,
                    &host.address,
                    &host.username,
                    &host.options.group,
                ]
                .iter()
                .any(|value| value.to_lowercase().contains(&query))
                    || host
                        .options
                        .tags
                        .iter()
                        .any(|tag| tag.to_lowercase().contains(&query))
            })
            .collect()
    }
    fn normalize_selection(&mut self, cx: &App) {
        let visible = self.visible(cx);
        if !visible.iter().any(|host| Some(host.id) == self.selected) {
            self.selected = visible.first().map(|host| host.id);
        }
    }
    fn reload(&mut self, cx: &App) {
        match self.store.load() {
            Ok(entries) => {
                self.entries = entries;
                self.load_failed = false;
                self.error = None;
                self.normalize_selection(cx);
            }
            Err(error) => {
                self.load_failed = true;
                self.error = Some(format!("{error:#}"));
            }
        }
    }
    fn begin_add(&mut self, cx: &mut Context<Self>) {
        self.compact_list = false;
        self.content_scroll.set_offset(Default::default());
        self.editor = Some(Editor::new(None, "", cx));
        self.watch_editor(cx);
        self.error = None;
    }
    fn begin_edit(&mut self, cx: &mut Context<Self>) {
        self.compact_list = false;
        let Some(host) = self
            .entries
            .iter()
            .find(|host| Some(host.id) == self.selected)
        else {
            return;
        };
        self.content_scroll.set_offset(Default::default());
        self.editor = Some(Editor::new(Some(host), "", cx));
        self.watch_editor(cx);
        self.error = None;
    }
    fn watch_editor(&mut self, cx: &mut Context<Self>) {
        self.editor_subscriptions.clear();
        if let Some(editor) = &mut self.editor {
            editor
                .appearance_anchor
                .get_or_insert_with(|| gpui::ScrollAnchor::for_handle(self.content_scroll.clone()));
            for field in editor.all_fields() {
                field.update(cx, |field, _| {
                    field.set_scroll_handle(self.content_scroll.clone())
                });
            }
            self.editor_subscriptions = editor
                .all_fields()
                .into_iter()
                .map(|field| cx.observe(field, |_, _, cx| cx.notify()))
                .collect();
        }
    }
    fn request(&mut self, action: Pending, window: &mut Window, cx: &mut Context<Self>) {
        if self.connecting {
            if matches!(action, Pending::Close) {
                self.connecting = false;
                cx.emit(ConnectEvent::Cancel);
            }
            return;
        }
        if self.editor.as_ref().is_some_and(|editor| editor.dirty(cx)) {
            self.pending = Some(action);
        } else {
            self.apply(action, window, cx);
        }
        cx.notify();
    }
    fn apply(&mut self, action: Pending, window: &mut Window, cx: &mut Context<Self>) {
        self.pending = None;
        match action {
            Pending::Browse => {
                self.begin_edit(cx);
                self.compact_list = true;
                window.focus(&self.search.focus_handle(cx));
            }
            Pending::New => {
                self.begin_add(cx);
                self.focus_editor(window, cx);
            }
            Pending::Select(id) | Pending::Connect(id) => {
                self.selected = Some(id);
                self.begin_edit(cx);
                if matches!(action, Pending::Connect(_)) {
                    self.connect(cx);
                } else {
                    self.focus_editor(window, cx);
                }
            }
            Pending::Close => {
                if self.can_cancel {
                    cx.emit(ConnectEvent::Cancel);
                } else {
                    if self.entries.is_empty() {
                        self.begin_add(cx);
                    } else {
                        self.begin_edit(cx);
                    }
                    self.focus(window, cx);
                }
            }
            Pending::Delete(id) => {
                self.selected = Some(id);
                self.delete(cx);
            }
        }
    }
    fn save_host(&mut self, cx: &mut Context<Self>) -> Option<SavedConnection> {
        let editor = self.editor.as_ref()?;
        let password = Zeroizing::new(editor.password.read(cx).value().to_owned());
        let proxy_password = Zeroizing::new(editor.proxy_password.read(cx).value().to_owned());
        let result = editor.host(cx).and_then(|host| self.store.save_host(host));
        match result {
            Ok(host) => {
                if let Some(previous) = self.entries.iter().find(|entry| entry.id == host.id) {
                    for kind in [SecretKind::Password, SecretKind::Proxy] {
                        if credentials::remembers(previous, kind)
                            && (!credentials::remembers(&host, kind)
                                || credentials::context(previous, kind)
                                    != credentials::context(&host, kind))
                        {
                            let _ = secrets::delete(host.id, kind);
                        }
                    }
                }
                self.selected = Some(host.id);
                self.undo = None;
                self.search.update(cx, |field, cx| field.set_value("", cx));
                self.reload(cx);
                self.begin_edit(cx);
                if let Some(editor) = &mut self.editor {
                    editor
                        .password
                        .update(cx, |field, cx| field.set_value(password.as_str(), cx));
                    editor.initial_password = password;
                    editor
                        .proxy_password
                        .update(cx, |field, cx| field.set_value(proxy_password.as_str(), cx));
                    editor.initial_proxy_password = proxy_password;
                }
                Some(host)
            }
            Err(error) => {
                self.error = Some(format!("{error:#}"));
                None
            }
        }
    }
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.save_host(cx).is_some() {
            self.focus(window, cx);
        }
        cx.notify();
    }
    fn connect(&mut self, cx: &mut Context<Self>) {
        if self.connecting {
            return;
        }
        if self.compact_list {
            self.begin_edit(cx);
        }
        let password = self
            .editor
            .as_ref()
            .map(|editor| Zeroizing::new(editor.password.read(cx).value().to_owned()));
        let Some(host) = self.save_host(cx) else {
            cx.notify();
            return;
        };
        let route = match self.store.route(host.id) {
            Ok(route) if route.last() == Some(&host) => route,
            _ => {
                self.set_error(
                    "Host details changed in another window. Reload and reconnect.".into(),
                    cx,
                );
                return;
            }
        };
        let proxy_password = self
            .editor
            .as_ref()
            .map(|editor| Zeroizing::new(editor.proxy_password.read(cx).value().to_owned()));
        let mut profiles = match credentials::build(&route, &self.store, password, proxy_password) {
            Ok(profiles) => profiles,
            Err(error) => {
                self.set_error(format!("{error:#}"), cx);
                return;
            }
        };
        if credentials::needs_keychain(&route, &profiles) {
            self.connecting = true;
            cx.spawn(async move |this, cx| {
                credentials::load_saved(&route, &mut profiles).await;
                let _ = this.update(cx, |this, cx| {
                    if !this.connecting {
                        return;
                    }
                    this.connecting = false;
                    if !this
                        .store
                        .route(host.id)
                        .is_ok_and(|current| current == route)
                    {
                        this.set_error(
                            "Host details changed while loading credentials. Reload and reconnect."
                                .into(),
                            cx,
                        );
                        return;
                    }
                    match credentials::assemble(profiles) {
                        Ok(profile) => cx.emit(ConnectEvent::Submit {
                            profile,
                            name: host.label,
                        }),
                        Err(error) => this.error = Some(format!("{error:#}")),
                    }
                    cx.notify();
                });
            })
            .detach();
        } else {
            match credentials::assemble(profiles) {
                Ok(profile) => cx.emit(ConnectEvent::Submit {
                    profile,
                    name: host.label,
                }),
                Err(error) => self.error = Some(format!("{error:#}")),
            }
        }
        cx.notify();
    }
    fn browse_key(&mut self, cx: &mut Context<Self>) {
        self.browse_identity(false, cx);
    }
    fn browse_certificate(&mut self, cx: &mut Context<Self>) {
        self.browse_identity(true, cx);
    }
    fn browse_identity(&mut self, certificate: bool, cx: &mut Context<Self>) {
        let Some(editor) = &self.editor else {
            return;
        };
        let field = if certificate {
            editor.certificate.downgrade()
        } else {
            editor.key_file.downgrade()
        };
        let result = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(
                if certificate {
                    "Choose an OpenSSH user certificate"
                } else {
                    "Choose an SSH private key"
                }
                .into(),
            ),
        });
        cx.spawn(async move |this, cx| {
            match result.await {
                Ok(Ok(Some(paths))) => { if let Some(path) = paths.first() { let _ = field.update(cx, |field, cx| field.set_value(&path.to_string_lossy(), cx)); } }
                Ok(Ok(None)) => {},
                _ => { let _ = this.update(cx, |this, cx| this.set_error("Could not open the file picker. Enter the key path directly, or check your desktop portal.".into(), cx)); }
            }
        }).detach();
    }
    fn delete(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected else {
            return;
        };
        match self.store.delete(id) {
            Ok(host) => {
                if matches!(
                    host.options.authentication,
                    Authentication::Password { remember: true }
                ) {
                    let _ = secrets::delete(host.id, SecretKind::Password);
                }
                if credentials::remembers(&host, SecretKind::Proxy) {
                    let _ = secrets::delete(host.id, SecretKind::Proxy);
                }
                self.undo = Some(host);
                self.selected = None;
                self.editor = None;
                self.reload(cx);
                if self.entries.is_empty() {
                    self.begin_add(cx);
                } else {
                    self.begin_edit(cx);
                }
            }
            Err(error) => {
                self.reload(cx);
                self.error = Some(format!("{error:#}"));
            }
        }
        cx.notify();
    }
    fn undo_delete(&mut self, cx: &mut Context<Self>) {
        let Some(host) = self.undo.clone() else {
            return;
        };
        match self.store.restore(host) {
            Ok(host) => {
                self.undo = None;
                self.selected = Some(host.id);
                self.reload(cx);
                self.begin_edit(cx);
            }
            Err(error) => self.error = Some(format!("{error:#}")),
        }
        cx.notify();
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.connecting {
            if event.keystroke.key == "escape" {
                self.connecting = false;
                cx.emit(ConnectEvent::Cancel);
                cx.notify();
            }
            window.prevent_default();
            cx.stop_propagation();
            return;
        }
        let key = event.keystroke.key.as_str();
        let mods = event.keystroke.modifiers;
        let primary =
            ghostlink_platform::primary_modifier(mods.control, mods.platform) && !mods.alt;
        if self.pending.is_some() {
            if key == "escape" {
                self.pending = None;
                cx.notify();
            } else {
                return;
            }
        } else {
            if !primary && !mods.alt && self.appearance_key(key, window, cx) {
                window.prevent_default();
                cx.stop_propagation();
                return;
            }
            match key {
                "n" if primary && !self.load_failed => self.request(Pending::New, window, cx),
                "s" if primary => self.save(window, cx),
                "f" if primary => {
                    if f32::from(window.viewport_size().width) < 760. && !self.compact_list {
                        self.request(Pending::Browse, window, cx);
                    } else {
                        window.focus(&self.search.focus_handle(cx));
                    }
                }
                "e" if primary => {
                    if let Some(id) = self.selected {
                        self.request(Pending::Select(id), window, cx);
                    }
                }
                "escape" => self.request(Pending::Close, window, cx),
                "enter"
                    if self.search.focus_handle(cx).is_focused(window)
                        || self.focus.is_focused(window) =>
                {
                    if let Some(id) = self.selected {
                        self.request(Pending::Connect(id), window, cx);
                    }
                }
                "enter"
                    if self.editor.as_ref().is_some_and(|editor| {
                        editor.snippet.focus_handle(cx).is_focused(window)
                    }) && !primary =>
                {
                    return;
                }
                "enter" => self.save(window, cx),
                "up" | "down"
                    if self.search.focus_handle(cx).is_focused(window)
                        || self.focus.is_focused(window) =>
                {
                    let visible = self.visible(cx);
                    if !visible.is_empty() {
                        let index = visible
                            .iter()
                            .position(|host| Some(host.id) == self.selected)
                            .unwrap_or(0) as isize;
                        self.selected = Some(
                            visible[(index + if key == "up" { -1 } else { 1 })
                                .rem_euclid(visible.len() as isize)
                                as usize]
                                .id,
                        );
                    }
                }
                "tab" if self.compact_list => window.focus(&self.search.focus_handle(cx)),
                "tab" => {
                    if let Some(editor) = &self.editor {
                        let mut handles = Vec::new();
                        if f32::from(window.viewport_size().width) >= 760. {
                            handles.push(self.search.focus_handle(cx));
                        }
                        for field in editor.fields() {
                            if field.entity_id() == editor.jump_search.entity_id() {
                                handles.extend([
                                    editor.charset_focus.clone(),
                                    editor.theme_focus.clone(),
                                ]);
                            }
                            handles.push(field.focus_handle(cx));
                        }
                        let index =
                            match handles.iter().position(|handle| handle.is_focused(window)) {
                                Some(current) => (current as isize
                                    + if mods.shift { -1 } else { 1 })
                                .rem_euclid(handles.len() as isize)
                                    as usize,
                                None if mods.shift => handles.len() - 1,
                                None => 0,
                            };
                        if (handles[index] == editor.charset_focus
                            || handles[index] == editor.theme_focus)
                            && let Some(anchor) = &editor.appearance_anchor
                        {
                            anchor.scroll_to(window, cx);
                        }
                        window.focus(&handles[index]);
                    }
                }
                _ => return,
            }
        }
        window.prevent_default();
        cx.stop_propagation();
        cx.notify();
    }
}
fn action(id: &'static str, label: &'static str, primary: bool) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .px_3()
        .py_2()
        .rounded_sm()
        .cursor_pointer()
        .whitespace_nowrap()
        .when(primary, |button| {
            button.bg(rgb(0xa8d8c0)).text_color(rgb(0x111517))
        })
        .when(!primary, |button| {
            button.hover(|style| style.bg(rgb(0x293438)))
        })
        .child(label)
}
impl Render for ConnectForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let wide = f32::from(window.viewport_size().width) >= 760.;
        if wide {
            self.compact_list = false;
        }
        let max_height = (f32::from(window.viewport_size().height) - 48.).max(160.);
        let content_height = (max_height
            - if !wide && !self.compact_list { 40. } else { 0. }
            - 140.
            - if self.pending.is_some() { 100. } else { 0. }
            - if self.error.is_some() { 60. } else { 0. })
        .max(80.);
        let list = self.render_list(cx);
        let editor = self.render_editor(window, cx);
        let content = if wide {
            div()
                .flex()
                .child(div().w(px(280.)).flex_none().child(list))
                .child(editor)
        } else if self.compact_list {
            div().child(list)
        } else {
            div().child(editor)
        };
        let footer = div()
            .flex()
            .items_center()
            .gap_2()
            .when(
                if self.compact_list {
                    self.selected.is_some()
                } else {
                    self.editor
                        .as_ref()
                        .is_some_and(|editor| editor.id.is_some())
                },
                |row| {
                    row.child(
                        action("edit-connection", "Edit", false).on_click(cx.listener(
                            |this, _, window, cx| {
                                if let Some(id) = this.selected {
                                    this.request(Pending::Select(id), window, cx);
                                }
                            },
                        )),
                    )
                    .child(
                        action("delete-connection", "Delete", false)
                            .text_color(rgb(0xd7b77d))
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Some(id) = if this.compact_list {
                                    this.selected
                                } else {
                                    this.editor.as_ref().and_then(|editor| editor.id)
                                } {
                                    this.request(Pending::Delete(id), window, cx);
                                }
                            })),
                    )
                },
            )
            .when(self.undo.is_some(), |row| {
                row.child(
                    action("undo-delete", "Undo delete", false)
                        .on_click(cx.listener(|this, _, _, cx| this.undo_delete(cx))),
                )
            })
            .when(self.connecting, |row| row.child("Opening system keychain…"))
            .child(div().flex_1())
            .when(
                self.editor.is_some()
                    && !self.load_failed
                    && !self.connecting
                    && (!self.compact_list || self.selected.is_some()),
                |row| {
                    row.child(
                        action("save-connection", "Save", false)
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    )
                    .child(
                        action("submit-connect", "Connect", true)
                            .on_click(cx.listener(|this, _, _, cx| this.connect(cx))),
                    )
                },
            );
        div()
            .id("connection-picker")
            .debug_selector(|| "connection-picker".into())
            .w(px(960.))
            .max_w_full()
            .max_h(px(max_height))
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .rounded_lg()
            .border_1()
            .border_color(rgb(0x3a4549))
            .bg(rgb(0x1b2023))
            .shadow_lg()
            .track_focus(&self.focus)
            .key_context("ConnectionPicker")
            .on_key_down(cx.listener(Self::key))
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Connections"),
                    )
                    .when(self.can_cancel, |row| {
                        row.child(
                            action("cancel-connect", "Close", false).on_click(cx.listener(
                                |this, _, window, cx| this.request(Pending::Close, window, cx),
                            )),
                        )
                    }),
            )
            .when(!wide && !self.compact_list, |root| {
                root.child(action("browse-hosts", "Browse hosts", false).on_click(
                    cx.listener(|this, _, window, cx| this.request(Pending::Browse, window, cx)),
                ))
            })
            .child(
                content
                    .id("host-content")
                    .track_scroll(&self.content_scroll)
                    .h(px(content_height))
                    .overflow_y_scroll(),
            )
            .when_some(self.error.clone(), |root, error| {
                root.child(div().text_xs().text_color(rgb(0xd7b77d)).child(error).when(
                    self.load_failed,
                    |row| {
                        row.child(action("reload-connections", "Try again", false).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.reload(cx);
                                if !this.load_failed {
                                    if this.entries.is_empty() {
                                        this.begin_add(cx);
                                    } else {
                                        this.begin_edit(cx);
                                    }
                                }
                                cx.notify();
                            }),
                        ))
                    },
                ))
            })
            .when(self.pending.is_some(), |root| {
                root.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child("Save your changes before continuing?")
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(action("save-changes", "Save changes", true).on_click(
                                    cx.listener(|this, _, window, cx| {
                                        if this.save_host(cx).is_some() {
                                            let action = this.pending.take().unwrap();
                                            this.apply(action, window, cx);
                                        }
                                        cx.notify();
                                    }),
                                ))
                                .child(action("discard-changes", "Discard", false).on_click(
                                    cx.listener(|this, _, window, cx| {
                                        let action = this.pending.take().unwrap();
                                        this.apply(action, window, cx);
                                        cx.notify();
                                    }),
                                ))
                                .child(action("keep-editing", "Keep editing", false).on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.pending = None;
                                        cx.notify();
                                    }),
                                )),
                        ),
                )
            })
            .child(footer)
    }
}

#[cfg(test)]
mod tests;

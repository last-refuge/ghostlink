use crate::{
    brand,
    connections::{ConnectionStore, SavedConnection},
    field::Field,
};
use gpui::{
    App, ClickEvent, Context, Div, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    KeyDownEvent, ObjectFit, ScrollHandle, Stateful, Subscription, Window, div, img, prelude::*,
    px, rgb,
};
use session::SshProfile;

pub enum ConnectEvent {
    Submit { profile: SshProfile, name: String },
    Cancel,
}
struct Editor {
    id: Option<u64>,
    destination: Entity<Field>,
    name: Entity<Field>,
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
    scroll: ScrollHandle,
    _search_subscription: Subscription,
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
            scroll: ScrollHandle::new(),
            _search_subscription: subscription,
        };
        this.reload(cx);
        if this.entries.is_empty() && !this.load_failed {
            this.begin_add(cx);
        }
        this
    }
    pub fn focus(&self, window: &mut Window, cx: &App) {
        let handle = if let Some(editor) = &self.editor {
            editor.name.focus_handle(cx)
        } else {
            self.search.focus_handle(cx)
        };
        window.focus(&handle);
    }
    pub fn set_error(&mut self, error: String, cx: &mut Context<Self>) {
        self.error = Some(error);
        cx.notify();
    }
    fn visible(&self, cx: &App) -> Vec<&SavedConnection> {
        let query = self.search.read(cx).value().trim().to_lowercase();
        self.entries
            .iter()
            .filter(|entry| {
                entry.name.to_lowercase().contains(&query)
                    || entry.destination.to_lowercase().contains(&query)
            })
            .collect()
    }
    fn normalize_selection(&mut self, cx: &App) {
        let visible = self.visible(cx);
        let index = visible
            .iter()
            .position(|entry| Some(entry.id) == self.selected)
            .unwrap_or(0);
        self.selected = visible.get(index).map(|entry| entry.id);
        self.scroll.scroll_to_item(index);
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
        let query = self.search.read(cx).value().trim().to_owned();
        let destination = if SshProfile::from_destination(&query).is_ok() {
            query
        } else {
            String::new()
        };
        self.editor = Some(Editor {
            id: None,
            destination: cx.new(|cx| Field::new(&destination, false, cx)),
            name: cx.new(|cx| Field::new("", false, cx)),
        });
        self.error = None;
    }
    fn begin_edit(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self
            .entries
            .iter()
            .find(|entry| Some(entry.id) == self.selected)
            .cloned()
        else {
            return;
        };
        self.editor = Some(Editor {
            id: Some(entry.id),
            destination: cx.new(|cx| Field::new(&entry.destination, false, cx)),
            name: cx.new(|cx| Field::new(&entry.name, false, cx)),
        });
        self.error = None;
    }
    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.can_cancel && self.entries.is_empty() {
            cx.emit(ConnectEvent::Cancel);
            return;
        }
        self.editor = None;
        self.reload(cx);
        self.focus(window, cx);
        cx.notify();
    }
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = &self.editor else {
            return;
        };
        match self.store.save(
            editor.id,
            editor.name.read(cx).value(),
            editor.destination.read(cx).value(),
        ) {
            Ok(entry) => {
                self.editor = None;
                self.search.update(cx, |field, cx| field.set_value("", cx));
                self.selected = Some(entry.id);
                self.undo = None;
                self.reload(cx);
                self.focus(window, cx);
            }
            Err(error) => self.error = Some(format!("{error:#}")),
        }
        cx.notify();
    }
    fn connect(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self
            .entries
            .iter()
            .find(|entry| Some(entry.id) == self.selected)
        else {
            return;
        };
        match SshProfile::from_destination(&entry.destination) {
            Ok(profile) => cx.emit(ConnectEvent::Submit {
                profile,
                name: entry.name.clone(),
            }),
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
            }
        }
    }
    fn delete(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected else {
            return;
        };
        match self.store.delete(id) {
            Ok(entry) => {
                self.undo = Some(entry);
                self.selected = None;
                self.reload(cx);
            }
            Err(error) => {
                self.reload(cx);
                self.error = Some(format!("{error:#}"));
            }
        }
        cx.notify();
    }
    fn undo_delete(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self.undo.clone() else {
            return;
        };
        match self.store.restore(entry) {
            Ok(entry) => {
                self.undo = None;
                self.selected = Some(entry.id);
                self.search.update(cx, |field, cx| field.set_value("", cx));
                self.reload(cx);
            }
            Err(error) => self.error = Some(format!("{error:#}")),
        }
        cx.notify();
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let mods = event.keystroke.modifiers;
        if let Some(editor) = &self.editor {
            match key {
                "escape" => self.back(window, cx),
                "enter" => self.save(window, cx),
                "tab" => {
                    let next = if editor.name.focus_handle(cx).is_focused(window) {
                        editor.destination.focus_handle(cx)
                    } else {
                        editor.name.focus_handle(cx)
                    };
                    window.focus(&next);
                }
                _ => return,
            }
        } else {
            match key {
                "escape" if self.can_cancel => cx.emit(ConnectEvent::Cancel),
                "escape" => self.search.update(cx, |field, cx| field.set_value("", cx)),
                "enter" => self.connect(cx),
                "up" | "down" => {
                    let entries = self.visible(cx);
                    if !entries.is_empty() {
                        let index = entries
                            .iter()
                            .position(|e| Some(e.id) == self.selected)
                            .unwrap_or(0);
                        let index = (index as isize + if key == "up" { -1 } else { 1 })
                            .rem_euclid(entries.len() as isize)
                            as usize;
                        self.selected = Some(entries[index].id);
                        self.scroll.scroll_to_item(index);
                        cx.notify();
                    }
                }
                "n" if mods.control && !mods.alt && !self.load_failed => {
                    self.begin_add(cx);
                    self.focus(window, cx);
                    cx.notify();
                }
                "e" if mods.control && !mods.alt => {
                    self.begin_edit(cx);
                    self.focus(window, cx);
                    cx.notify();
                }
                "f" if mods.control && !mods.alt => self.focus(window, cx),
                "delete" if self.focus.is_focused(window) => self.delete(cx),
                "tab" => self.focus(window, cx),
                _ => return,
            }
        }
        window.prevent_default();
        cx.stop_propagation();
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
        let title = match &self.editor {
            Some(editor) if editor.id.is_some() => "Edit connection",
            Some(_) => "Add a connection",
            None => "Connections",
        };
        let max_height = (f32::from(window.viewport_size().height) - 48.).max(160.);
        let list_height = (f32::from(window.viewport_size().height)
            - 384.
            - if self.undo.is_some() { 48. } else { 0. })
        .clamp(64., 360.);
        div()
            .id("connection-picker")
            .debug_selector(|| "connection-picker".into())
            .w(px(640.))
            .max_w_full()
            .max_h(px(max_height))
            .overflow_y_scroll()
            .p_5()
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
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .when(self.editor.is_none() && !self.load_failed, |row| {
                        row.child(action("add-connection", "Add connection", false).on_click(
                            cx.listener(|this, _, window, cx| {
                                this.begin_add(cx);
                                this.focus(window, cx);
                                cx.notify();
                            }),
                        ))
                    }),
            )
            .child(div().text_color(rgb(0x8e9b9e)).child(if self.editor.is_some() {
                "Use a hostname, user@host:port, or an SSH config alias."
            } else {
                "Choose a connection to open a new session."
            }))
            .when(self.editor.is_none(), |root| {
                root.when(!self.entries.is_empty(), |root| {
                    root.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .line_height(px(16.))
                                    .text_color(rgb(0x8e9b9e))
                                    .child("Find a connection"),
                            )
                            .child(self.search.clone()),
                    )
                })
                .child(
                    div()
                        .id("saved-connections")
                        .debug_selector(|| "saved-connections".into())
                        .max_h(px(list_height))
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll)
                        .children(self.visible(cx).into_iter().map(|entry| {
                            let id = entry.id;
                            div()
                                .id(("connection", id as usize))
                                .px_3()
                                .py_3()
                                .mb_1()
                                .rounded_sm()
                                .cursor_pointer()
                                .bg(rgb(if self.selected == Some(id) {
                                    0x293e36
                                } else {
                                    0x171c1f
                                }))
                                .hover(|style| style.bg(rgb(0x30463c)))
                                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                                    this.selected = Some(id);
                                    this.error = None;
                                    window.focus(&this.focus);
                                    cx.notify();
                                    if event.click_count() >= 2 {
                                        this.connect(cx);
                                    }
                                }))
                                .child(
                                    div()
                                        .truncate()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(entry.name.clone()),
                                )
                                .child(
                                    div()
                                        .truncate()
                                        .text_xs()
                                        .text_color(rgb(0x9aada4))
                                        .child(entry.destination.clone()),
                                )
                        })),
                )
                .when(self.visible(cx).is_empty() && !self.load_failed, |root| {
                    root.child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_2()
                            .py_3()
                            .when(self.entries.is_empty(), |root| {
                                root.child(
                                    img(brand::illustration())
                                        .w(px(224.))
                                        .h(px(126.))
                                        .object_fit(ObjectFit::Contain),
                                )
                            })
                            .child(if self.entries.is_empty() {
                                "Add your first SSH connection to get started."
                            } else {
                                "No connections match your search."
                            }),
                    )
                })
            })
            .when_some(self.editor.as_ref(), |root, editor| {
                root.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_5()
                        .when(self.entries.is_empty() && editor.id.is_none(), |row| {
                            row.child(
                                img(brand::illustration())
                                    .w(px(176.))
                                    .h(px(128.))
                                    .flex_none()
                                    .object_fit(ObjectFit::Contain),
                            )
                        })
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(
                                    div()
                                        .text_xs()
                                        .line_height(px(16.))
                                        .text_color(rgb(0x8e9b9e))
                                        .child("Connection name (optional)"),
                                )
                                .child(
                                    div()
                                        .id("connection-name")
                                        .debug_selector(|| "connection-name".into())
                                        .child(editor.name.clone()),
                                )
                                .child(
                                    div()
                                        .mt_2()
                                        .text_xs()
                                        .line_height(px(16.))
                                        .text_color(rgb(0x8e9b9e))
                                        .child("Host or SSH alias"),
                                )
                                .child(
                                    div()
                                        .id("connection-destination")
                                        .debug_selector(|| "connection-destination".into())
                                        .child(editor.destination.clone()),
                                ),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .line_height(px(18.))
                        .text_color(rgb(0xa8b6b7))
                        .child("Uses your SSH config and existing keys. Authentication is requested when you connect."),
                )
            })
            .when_some(self.error.clone(), |root, error| {
                root.child(div().text_color(rgb(0xd7b77d)).child(error).when(
                    self.load_failed,
                    |root| {
                        root.child(action("reload-connections", "Try again", false).on_click(
                            cx.listener(|this, _, window, cx| {
                                this.reload(cx);
                                if this.entries.is_empty() && !this.load_failed {
                                    this.begin_add(cx);
                                }
                                this.focus(window, cx);
                                cx.notify();
                            }),
                        ))
                    },
                ))
            })
            .when_some(
                self.undo.as_ref().filter(|_| self.editor.is_none()),
                |root, entry| {
                    root.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .truncate()
                                    .text_xs()
                                    .text_color(rgb(0x9aada4))
                                    .child(format!("Removed {}", entry.name)),
                            )
                            .child(action("undo-delete", "Undo", false).on_click(
                                cx.listener(|this, _, _, cx| this.undo_delete(cx)),
                            )),
                    )
                },
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(self.editor.is_none() && self.selected.is_some(), |row| {
                        row.child(action("edit-connection", "Edit", false).on_click(cx.listener(
                            |this, _, window, cx| {
                                this.begin_edit(cx);
                                this.focus(window, cx);
                                cx.notify();
                            },
                        )))
                        .child(
                            action("delete-connection", "Delete", false)
                                .text_color(rgb(0xd7b77d))
                                .on_click(cx.listener(|this, _, _, cx| this.delete(cx))),
                        )
                    })
                    .child(div().flex_1())
                    .when(self.editor.is_some(), |row| {
                        row.child(action("cancel-editor", "Back", false).on_click(
                            cx.listener(|this, _, window, cx| this.back(window, cx)),
                        ))
                        .child(action("save-connection", "Save", true).on_click(
                            cx.listener(|this, _, window, cx| this.save(window, cx)),
                        ))
                    })
                    .when(self.editor.is_none(), |row| {
                        row.when(self.can_cancel, |row| {
                            row.child(action("cancel-connect", "Cancel", false).on_click(
                                cx.listener(|_, _, _, cx| cx.emit(ConnectEvent::Cancel)),
                            ))
                        })
                        .when(self.selected.is_some(), |row| {
                            row.child(action("submit-connect", "Connect", true).on_click(
                                cx.listener(|this, _, _, cx| this.connect(cx)),
                            ))
                        })
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use session::SshAuth;
    #[gpui::test]
    fn first_connection_is_saved_then_connects_with_automatic_authentication(
        cx: &mut TestAppContext,
    ) {
        use std::{cell::RefCell, rc::Rc};
        let store = ConnectionStore::local();
        let retained = store.clone();
        let window = cx.add_window(|window, cx| {
            let form = ConnectForm::new(store, false, cx);
            form.focus(window, cx);
            form
        });
        let submitted = Rc::new(RefCell::new(None));
        let captured = submitted.clone();
        let form = window.root(cx).unwrap();
        let _subscription = cx.update(|cx| {
            cx.subscribe(&form, move |_, event, _| {
                if let ConnectEvent::Submit { profile, name } = event {
                    *captured.borrow_mut() = Some((profile.clone(), name.clone()));
                }
            })
        });
        cx.simulate_input(*window, "Production");
        cx.simulate_keystrokes(*window, "tab");
        cx.simulate_input(*window, "deploy@example.org:2222");
        cx.simulate_keystrokes(*window, "enter");
        assert_eq!(
            retained.load().unwrap()[0].destination,
            "deploy@example.org:2222"
        );
        assert_eq!(retained.load().unwrap()[0].name, "Production");
        assert!(submitted.borrow().is_none());
        cx.simulate_keystrokes(*window, "enter");
        let saved = submitted.borrow();
        let (profile, name) = saved.as_ref().unwrap();
        assert_eq!(name, "Production");
        assert_eq!(profile.host, "example.org");
        assert_eq!(profile.username, "deploy");
        assert_eq!(profile.port, 2222);
        assert!(matches!(profile.auth, SshAuth::Automatic { .. }));
    }
    #[gpui::test]
    fn catalog_supports_search_edit_delete_undo_and_persists_across_dialogs(
        cx: &mut TestAppContext,
    ) {
        let store = ConnectionStore::local();
        store.save(None, "Production", "prod").unwrap();
        store.save(None, "Staging", "stage").unwrap();
        let retained = store.clone();
        let window = cx.add_window(|window, cx| {
            let form = ConnectForm::new(store, true, cx);
            form.focus(window, cx);
            form
        });
        cx.simulate_keystrokes(*window, "s t a");
        window
            .update(cx, |form, _, cx| {
                assert_eq!(form.visible(cx).len(), 1);
                assert_eq!(form.visible(cx)[0].name, "Staging");
            })
            .unwrap();
        cx.simulate_keystrokes(*window, "ctrl-e");
        cx.simulate_keystrokes(*window, "ctrl-a");
        cx.simulate_input(*window, "Test environment");
        cx.simulate_keystrokes(*window, "enter");
        window
            .update(cx, |form, _, cx| {
                form.delete(cx);
                assert_eq!(form.entries.len(), 1);
                form.undo_delete(cx);
                assert_eq!(form.entries.len(), 2);
            })
            .unwrap();
        assert!(
            retained
                .load()
                .unwrap()
                .iter()
                .any(|e| e.name == "Test environment")
        );
        let reopened = cx.add_window(|_, cx| ConnectForm::new(retained, true, cx));
        reopened
            .update(cx, |form, _, _| assert_eq!(form.entries.len(), 2))
            .unwrap();
    }
    #[gpui::test]
    fn invalid_destination_stays_in_editor_and_tab_cycles_fields(cx: &mut TestAppContext) {
        let window = cx.add_window(|window, cx| {
            let form = ConnectForm::new(ConnectionStore::local(), false, cx);
            form.focus(window, cx);
            form
        });
        cx.simulate_keystrokes(*window, "enter");
        window
            .update(cx, |form, _, _| assert!(form.error.is_some()))
            .unwrap();
        cx.simulate_input(*window, "Production");
        cx.simulate_keystrokes(*window, "tab");
        cx.simulate_input(*window, "prod");
        window
            .update(cx, |form, window, cx| {
                let editor = form.editor.as_ref().unwrap();
                assert_eq!(editor.name.read(cx).value(), "Production");
                assert_eq!(editor.destination.read(cx).value(), "prod");
                assert!(editor.destination.focus_handle(cx).is_focused(window));
            })
            .unwrap();
        cx.simulate_keystrokes(*window, "shift-tab");
        window
            .update(cx, |form, window, cx| {
                assert!(
                    form.editor
                        .as_ref()
                        .unwrap()
                        .name
                        .focus_handle(cx)
                        .is_focused(window)
                );
            })
            .unwrap();
    }
}

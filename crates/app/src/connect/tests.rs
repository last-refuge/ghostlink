use super::*;
use gpui::TestAppContext;
use session::SshAuth;

#[gpui::test]
fn explicit_remote_user_and_port_survive_keyboard_save_and_connect(cx: &mut TestAppContext) {
    use std::{cell::RefCell, rc::Rc};
    let store = ConnectionStore::local();
    let retained = store.clone();
    let window = cx.add_window(|window, cx| {
        let form = ConnectForm::new(store, false, cx);
        form.focus(window, cx);
        form
    });
    let captured = Rc::new(RefCell::new(None));
    let output = captured.clone();
    let form = window.root(cx).unwrap();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&form, move |_, event, _| {
            if let ConnectEvent::Submit { profile, name } = event {
                *output.borrow_mut() = Some((profile.clone(), name.clone()));
            }
        })
    });
    cx.simulate_input(*window, "example.org");
    cx.simulate_keystrokes(*window, "tab");
    cx.simulate_input(*window, "Production");
    cx.simulate_keystrokes(*window, "tab tab tab");
    cx.simulate_input(*window, "deploy");
    cx.simulate_keystrokes(*window, "tab");
    cx.simulate_input(*window, "2222");
    cx.simulate_keystrokes(*window, "ctrl-s");
    let saved = retained.load().unwrap();
    assert_eq!(saved[0].destination(), "deploy@example.org:2222");
    assert_eq!(saved[0].label, "Production");
    assert!(captured.borrow().is_none());
    cx.simulate_keystrokes(*window, "enter");
    let result = captured.borrow();
    let (profile, name) = result.as_ref().unwrap();
    assert_eq!(name, "Production");
    assert_eq!(profile.host, "example.org");
    assert_eq!(profile.username, "deploy");
    assert_eq!(profile.port, 2222);
    assert!(matches!(profile.auth, SshAuth::Automatic { .. }));
}

#[gpui::test]
fn switching_hosts_requires_saving_or_discarding_unsaved_changes(cx: &mut TestAppContext) {
    let store = ConnectionStore::local();
    let first = store.save(None, "First", "first").unwrap();
    let second = store.save(None, "Second", "second").unwrap();
    let retained = store.clone();
    let window = cx.add_window(|_, cx| ConnectForm::new(store, true, cx));
    window
        .update(cx, |form, window, cx| {
            form.editor
                .as_ref()
                .unwrap()
                .name
                .update(cx, |field, cx| field.set_value("Unsaved name", cx));
            form.request(Pending::Select(second.id), window, cx);
            assert!(form.pending.is_some());
            assert_eq!(form.editor.as_ref().unwrap().id, Some(first.id));
            assert_eq!(retained.load().unwrap()[0].label, "First");
            form.save_host(cx).unwrap();
            let pending = form.pending.take().unwrap();
            form.apply(pending, window, cx);
            assert_eq!(form.editor.as_ref().unwrap().id, Some(second.id));
            assert!(
                retained
                    .load()
                    .unwrap()
                    .iter()
                    .any(|host| host.label == "Unsaved name")
            );
            form.editor
                .as_ref()
                .unwrap()
                .name
                .update(cx, |field, cx| field.set_value("Discard me", cx));
            form.request(Pending::Select(first.id), window, cx);
            let pending = form.pending.take().unwrap();
            form.apply(pending, window, cx);
            assert_eq!(form.editor.as_ref().unwrap().id, Some(first.id));
            assert!(
                !retained
                    .load()
                    .unwrap()
                    .iter()
                    .any(|host| host.label == "Discard me")
            );
        })
        .unwrap();
}

#[gpui::test]
fn search_matches_group_tags_and_username_and_delete_can_be_undone(cx: &mut TestAppContext) {
    let store = ConnectionStore::local();
    let mut production = store.save(None, "Production", "deploy@prod").unwrap();
    production.options.group = "Databases".into();
    production.options.tags = vec!["Postgres".into()];
    store.save_host(production.clone()).unwrap();
    store.save(None, "Staging", "stage").unwrap();
    let retained = store.clone();
    let window = cx.add_window(|_, cx| ConnectForm::new(store, true, cx));
    window
        .update(cx, |form, _, cx| {
            for query in ["postgres", "DATABASES", "deploy"] {
                form.search
                    .update(cx, |field, cx| field.set_value(query, cx));
                assert_eq!(form.visible(cx).len(), 1);
                assert_eq!(form.visible(cx)[0].id, production.id);
            }
            form.selected = Some(production.id);
            form.delete(cx);
            assert_eq!(form.entries.len(), 1);
            form.undo_delete(cx);
            assert_eq!(form.entries.len(), 2);
            assert_eq!(
                retained
                    .load()
                    .unwrap()
                    .iter()
                    .find(|host| host.id == production.id)
                    .unwrap()
                    .options,
                production.options
            );
        })
        .unwrap();
}

#[gpui::test]
fn invalid_address_and_port_remain_editable_and_tab_order_is_reversible(cx: &mut TestAppContext) {
    let store = ConnectionStore::local();
    let retained = store.clone();
    let window = cx.add_window(|window, cx| {
        let form = ConnectForm::new(store, false, cx);
        form.focus(window, cx);
        form
    });
    cx.simulate_keystrokes(*window, "enter");
    window
        .update(cx, |form, _, _| assert!(form.error.is_some()))
        .unwrap();
    cx.simulate_input(*window, "prod");
    cx.simulate_keystrokes(*window, "tab");
    cx.simulate_input(*window, "Production");
    cx.simulate_keystrokes(*window, "shift-tab");
    window
        .update(cx, |form, window, cx| {
            let editor = form.editor.as_ref().unwrap();
            assert!(editor.address.focus_handle(cx).is_focused(window));
            editor.port.update(cx, |field, cx| field.set_value("0", cx));
            form.save(window, cx);
            assert!(form.error.as_ref().unwrap().contains("port"));
            assert!(retained.load().unwrap().is_empty());
        })
        .unwrap();
}

#[gpui::test]
fn password_stays_out_of_catalog_and_is_passed_only_to_password_authentication(
    cx: &mut TestAppContext,
) {
    use std::{cell::RefCell, rc::Rc};
    let store = ConnectionStore::local();
    let retained = store.clone();
    let window = cx.add_window(|_, cx| ConnectForm::new(store, false, cx));
    let captured = Rc::new(RefCell::new(None));
    let output = captured.clone();
    let form = window.root(cx).unwrap();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&form, move |_, event, _| {
            if let ConnectEvent::Submit { profile, .. } = event {
                *output.borrow_mut() = Some(profile.clone());
            }
        })
    });
    window
        .update(cx, |form, window, cx| {
            let editor = form.editor.as_mut().unwrap();
            editor
                .address
                .update(cx, |field, cx| field.set_value("example.org", cx));
            editor
                .username
                .update(cx, |field, cx| field.set_value("deploy", cx));
            editor.options.authentication = Authentication::Password { remember: false };
            editor.password.update(cx, |field, cx| {
                field.set_value("fixture secret password", cx)
            });
            assert!(
                editor
                    .fields()
                    .iter()
                    .any(|field| field.entity_id() == editor.password.entity_id())
            );
            assert!(
                !editor
                    .fields()
                    .iter()
                    .any(|field| field.entity_id() == editor.key_file.entity_id())
            );
            form.save(window, cx);
            assert!(!form.editor.as_ref().unwrap().dirty(cx));
            form.connect(cx);
        })
        .unwrap();
    let result = captured.borrow();
    let profile = result.as_ref().unwrap();
    assert!(profile.options.password_only);
    assert_eq!(
        profile.options.password.as_deref().map(String::as_str),
        Some("fixture secret password")
    );
    assert!(profile.options.on_password_accepted.is_none());
    let encoded = serde_json::to_string(&retained.load().unwrap()).unwrap();
    assert!(!encoded.contains("fixture secret password"));
    assert!(!format!("{profile:?}").contains("fixture secret password"));
}

#[gpui::test]
fn compact_browser_connects_the_selected_host_instead_of_the_previous_editor(
    cx: &mut TestAppContext,
) {
    use std::{cell::RefCell, rc::Rc};
    let store = ConnectionStore::local();
    let first = store.save(None, "First", "one@first").unwrap();
    let second = store.save(None, "Second", "two@second").unwrap();
    let window = cx.add_window(|_, cx| ConnectForm::new(store, true, cx));
    let captured = Rc::new(RefCell::new(None));
    let output = captured.clone();
    let form = window.root(cx).unwrap();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&form, move |_, event, _| {
            if let ConnectEvent::Submit { profile, .. } = event {
                *output.borrow_mut() = Some(profile.clone());
            }
        })
    });
    window
        .update(cx, |form, window, cx| {
            assert_eq!(form.editor.as_ref().unwrap().id, Some(first.id));
            form.request(Pending::Browse, window, cx);
            form.selected = Some(second.id);
            form.connect(cx);
        })
        .unwrap();
    let result = captured.borrow();
    let profile = result.as_ref().unwrap();
    assert_eq!(profile.host, "second");
    assert_eq!(profile.username, "two");
}

#[gpui::test]
fn advanced_editor_preserves_multiline_startup_environment_and_forwarding(cx: &mut TestAppContext) {
    let store = ConnectionStore::local();
    let host = store.save(None, "Advanced", "deploy@host").unwrap();
    let retained = store.clone();
    let window = cx.add_window(|_, cx| ConnectForm::new(store, true, cx));
    window
        .update(cx, |form, window, cx| {
            let editor = form.editor.as_mut().unwrap();
            editor.snippet.update(cx, |field, cx| {
                field.set_value("cd /srv/app\necho ready", cx)
            });
            editor.options.agent_forwarding = true;
            editor.environment.push((
                cx.new(|cx| Field::new("LANG", false, cx)),
                cx.new(|cx| Field::new("en_US.UTF-8", false, cx)),
            ));
            form.watch_editor(cx);
            form.save(window, cx);
            assert!(form.error.is_none());
            let editor = form.editor.as_ref().unwrap();
            assert_eq!(editor.snippet.read(cx).value(), "cd /srv/app\necho ready");
            assert!(editor.snippet.read(cx).is_multiline());
            assert_eq!(editor.environment.len(), 1);
        })
        .unwrap();
    let hosts = retained.load().unwrap();
    let saved = hosts.iter().find(|entry| entry.id == host.id).unwrap();
    let profile = saved.to_profile().unwrap();
    assert!(profile.options.agent_forwarding);
    assert_eq!(profile.options.startup_snippet, "cd /srv/app\necho ready");
    assert_eq!(
        profile.options.environment,
        vec![("LANG".into(), "en_US.UTF-8".into())]
    );
}

#[gpui::test]
fn routing_and_appearance_editor_submit_options_without_serializing_secrets(
    cx: &mut TestAppContext,
) {
    use std::{cell::RefCell, rc::Rc};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("connections.json");
    let store = ConnectionStore::at(path.clone());
    let gateway = store.save(None, "Gateway", "jump@gateway").unwrap();
    let root = store.save(None, "Application", "deploy@app").unwrap();
    let window = cx.add_window(|_, cx| ConnectForm::new(store, true, cx));
    let form = window.root(cx).unwrap();
    let captured = Rc::new(RefCell::new(None));
    let output = captured.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&form, move |_, event, _| {
            if let ConnectEvent::Submit { profile, .. } = event {
                *output.borrow_mut() = Some(profile.clone());
            }
        })
    });
    window
        .update(cx, |form, window, cx| {
            form.apply(Pending::Select(root.id), window, cx);
            let editor = form.editor.as_mut().unwrap();
            editor.options.jump_hosts = vec![gateway.id];
            editor.options.proxy = Some(crate::host_options::Proxy {
                kind: crate::host_options::ProxyKind::Http,
                host: String::new(),
                port: 8080,
                username: String::new(),
                remember: false,
            });
            editor
                .proxy_host
                .update(cx, |field, cx| field.set_value("proxy.example", cx));
            editor
                .proxy_port
                .update(cx, |field, cx| field.set_value("8080", cx));
            editor
                .proxy_username
                .update(cx, |field, cx| field.set_value("proxy-user", cx));
            editor.proxy_password.update(cx, |field, cx| {
                field.set_value("test-only-proxy-secret", cx)
            });
            editor.options.charset = crate::host_options::Charset::Named("Shift_JIS".into());
            editor.options.theme = crate::host_options::Theme::Nord;
            form.connect(cx);
        })
        .unwrap();
    let result = captured.borrow();
    let profile = result.as_ref().unwrap();
    assert_eq!(profile.options.jump_hosts.len(), 1);
    assert_eq!(profile.options.jump_hosts[0].username, "jump");
    assert_eq!(profile.options.jump_hosts[0].host, "gateway");
    assert_eq!(profile.options.charset.as_deref(), Some("Shift_JIS"));
    assert_eq!(profile.options.theme, crate::host_options::Theme::Nord);
    let proxy = profile.options.proxy.as_ref().unwrap();
    assert_eq!(proxy.host, "proxy.example");
    assert_eq!(
        proxy.password.as_deref().map(String::as_str),
        Some("test-only-proxy-secret")
    );
    let catalog = std::fs::read_to_string(path).unwrap();
    assert!(!catalog.contains("test-only-proxy-secret"));
    assert!(catalog.contains("Shift_JIS"));
    assert!(catalog.contains("Nord"));
}

#[gpui::test]
fn appearance_selectors_support_tab_arrows_accept_cancel_and_save(cx: &mut TestAppContext) {
    use crate::host_options::{Charset, Theme};
    let store = ConnectionStore::local();
    let host = store.save(None, "Editor keyboard", "example.org").unwrap();
    let retained = store.clone();
    let window = cx.add_window(|window, cx| {
        let form = ConnectForm::new(store, true, cx);
        let editor = form.editor.as_ref().unwrap();
        window.focus(&editor.port.focus_handle(cx));
        form
    });
    cx.simulate_keystrokes(*window, "tab");
    window
        .update(cx, |form, window, _| {
            assert!(
                form.editor
                    .as_ref()
                    .unwrap()
                    .charset_focus
                    .is_focused(window)
            );
        })
        .unwrap();
    cx.simulate_keystrokes(*window, "enter down enter");
    window
        .update(cx, |form, _, _| {
            let editor = form.editor.as_ref().unwrap();
            assert_eq!(editor.options.charset, Charset::Named("ISO-8859-1".into()));
            assert!(!editor.charset_open);
        })
        .unwrap();
    // Enter chose the menu value; it did not save the host.
    assert_eq!(retained.load().unwrap()[0].options.charset, Charset::Utf8);
    cx.simulate_keystrokes(*window, "tab enter end escape");
    window
        .update(cx, |form, window, _| {
            let editor = form.editor.as_ref().unwrap();
            assert!(editor.theme_focus.is_focused(window));
            assert_eq!(editor.options.theme, Theme::Ghostlink);
            assert!(!editor.theme_open);
            assert!(form.pending.is_none());
        })
        .unwrap();
    cx.simulate_keystrokes(*window, "enter down enter ctrl-s");
    assert_eq!(retained.load().unwrap()[0].id, host.id);
    assert_eq!(
        retained.load().unwrap()[0].options.theme,
        Theme::SolarizedDark
    );
    assert_eq!(
        retained.load().unwrap()[0].options.charset,
        Charset::Named("ISO-8859-1".into())
    );
}

#[gpui::test]
fn narrow_editor_tab_stays_visible_and_find_opens_the_host_list(cx: &mut TestAppContext) {
    let store = ConnectionStore::local();
    store.save(None, "Host", "example.org").unwrap();
    let (form, cx) = cx.add_window_view(|_, cx| ConnectForm::new(store, true, cx));
    cx.simulate_resize(gpui::size(px(700.), px(700.)));
    cx.update(|window, cx| {
        let handle = form
            .read(cx)
            .editor
            .as_ref()
            .unwrap()
            .keepalive
            .focus_handle(cx);
        window.focus(&handle);
    });
    cx.simulate_keystrokes("tab");
    cx.update(|window, cx| {
        let editor = form.read(cx).editor.as_ref().unwrap();
        assert!(editor.address.focus_handle(cx).is_focused(window));
    });
    cx.simulate_keystrokes("ctrl-f");
    cx.update(|_, cx| {
        assert!(form.read(cx).compact_list);
        assert!(form.read(cx).pending.is_none());
    });
    // Returning to the wide split layout must release the list-only Tab trap.
    cx.simulate_resize(gpui::size(px(1200.), px(820.)));
    cx.update(|_, cx| assert!(!form.read(cx).compact_list));
    cx.simulate_keystrokes("tab");
    cx.update(|window, cx| {
        let editor = form.read(cx).editor.as_ref().unwrap();
        assert!(editor.address.focus_handle(cx).is_focused(window));
    });
}

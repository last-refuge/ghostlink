use super::*;
use gpui::TestAppContext;
use session::SshAuth;

fn profile(user: &str) -> SshProfile {
    // UI tests use only a closed loopback port, without user config or keys.
    SshProfile {
        host: "127.0.0.1".into(),
        port: 9,
        username: user.into(),
        auth: SshAuth::Agent,
        known_hosts: "unused-test-known-hosts".into(),
    }
}
fn setup(cx: &mut TestAppContext) {
    let executor = cx.executor();
    cx.on_quit(move || executor.allow_parking());
    cx.update(bind_shortcuts);
}

#[gpui::test]
fn compact_tab_bar_keeps_controls_separate_at_small_and_large_sizes(cx: &mut TestAppContext) {
    setup(cx);
    let (workspace, cx) = cx.add_window_view(Workspace::new);
    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace
                .open_ssh(profile("long-user-name-for-overflow-check"), window, cx)
                .unwrap();
            workspace.open_terminal(false, Axis::Columns, window, cx);
            workspace.open_terminal(true, Axis::Columns, window, cx);
        })
    });
    for width in [700., 1280., 1947.] {
        cx.simulate_resize(size(px(width), px(820.)));
        let tabs = cx.debug_bounds("tabs").unwrap();
        let plus = cx.debug_bounds("new-tab").unwrap();
        let actions = cx.debug_bounds("pane-actions").unwrap();
        let bar = cx.debug_bounds("tab-bar").unwrap();
        assert!(
            tabs.right() <= plus.left(),
            "Tabs overlap new-tab control at {width}: {tabs:?} {plus:?}"
        );
        assert!(
            plus.right() <= actions.left(),
            "New-tab control overlaps pane actions at {width}: {plus:?} {actions:?}"
        );
        assert!(
            actions.right() <= bar.right(),
            "Pane actions overflow the tab bar at {width}"
        );
        assert_eq!(bar.size.height, px(42.));
        assert!(
            plus.left() - tabs.right() <= px(8.),
            "New-tab control drifted away from the tabs"
        );
    }
    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            for _ in 0..10 {
                workspace.open_terminal(false, Axis::Columns, window, cx);
            }
        })
    });
    cx.simulate_resize(size(px(700.), px(820.)));
    let tabs = cx.debug_bounds("tabs").unwrap();
    cx.update(|_, cx| {
        workspace.update(cx, |workspace, _| {
            let session = &workspace.sessions[workspace.active];
            let active = session.tabs_scroll.bounds_for_item(session.active).unwrap();
            let left = active.left() + session.tabs_scroll.offset().x;
            let right = active.right() + session.tabs_scroll.offset().x;
            assert!(
                left >= tabs.left() && right <= tabs.right(),
                "Active tab was clipped after resize: {left:?} {right:?} {tabs:?}"
            );
        })
    });
}
#[gpui::test]
fn empty_workspace_connect_shortcut_and_last_tab_return_to_welcome(cx: &mut TestAppContext) {
    setup(cx);
    let window = cx.add_window(Workspace::new);
    window
        .update(cx, |workspace, window, _| {
            assert!(workspace.sessions.is_empty());
            assert!(workspace.sidebar);
            assert!(workspace.focus.is_focused(window));
        })
        .unwrap();
    cx.simulate_keystrokes(*window, "ctrl-shift-t escape");
    window
        .update(cx, |workspace, window, cx| {
            assert!(workspace.connect_form.is_none());
            assert!(workspace.sessions.is_empty());
            workspace.open_ssh(profile("alpha"), window, cx).unwrap();
        })
        .unwrap();
    cx.simulate_keystrokes(*window, "ctrl-shift-w");
    window
        .update(cx, |workspace, window, _| {
            assert!(workspace.sessions.is_empty());
            assert!(workspace.focus.is_focused(window));
        })
        .unwrap();
    cx.simulate_keystrokes(*window, "ctrl-shift-k");
    window
        .update(cx, |workspace, _, _| {
            assert!(workspace.connect_form.is_some())
        })
        .unwrap();
}

#[gpui::test]
fn sessions_restore_their_tabs_layout_zoom_and_focused_pane(cx: &mut TestAppContext) {
    setup(cx);
    let window = cx.add_window(Workspace::new);
    let alpha = window
        .update(cx, |workspace, window, cx| {
            workspace.open_ssh(profile("alpha"), window, cx).unwrap();
            workspace.sessions[0].id
        })
        .unwrap();
    cx.simulate_keystrokes(
        *window,
        "ctrl-shift-t ctrl-shift-d ctrl-shift-e ctrl-shift-z",
    );
    let (alpha_tab, alpha_pane, layout) = window
        .update(cx, |workspace, window, cx| {
            assert_eq!(workspace.sessions.len(), 1);
            assert_eq!(workspace.sessions[0].tabs.len(), 2);
            let tab = workspace.active_tab().unwrap();
            assert_eq!(tab.panes.len(), 3);
            assert_eq!(tab.zoom, Some(tab.active_pane));
            let saved = (tab.id, tab.active_pane, tab.layout.clone());
            workspace.open_ssh(profile("beta"), window, cx).unwrap();
            saved
        })
        .unwrap();
    cx.simulate_keystrokes(*window, "ctrl-shift-t");
    let beta_tab =
        window
            .update(cx, |workspace, window, cx| {
                assert_eq!(workspace.sessions[1].tabs.len(), 2);
                let beta_tab = workspace.active_tab().unwrap().id;
                workspace.switch_session(alpha, window, cx);
                let tab = workspace.active_tab().unwrap();
                assert_eq!((tab.id, tab.active_pane), (alpha_tab, alpha_pane));
                assert_eq!(tab.layout, layout);
                assert_eq!(tab.zoom, Some(alpha_pane));
                assert!(tab.panes.iter().any(|pane| pane.id == alpha_pane
                    && pane.terminal.focus_handle(cx).is_focused(window)));
                beta_tab
            })
            .unwrap();
    cx.simulate_keystrokes(*window, "ctrl-shift-pagedown");
    window
        .update(cx, |workspace, _, _| {
            assert_eq!(workspace.active_tab().unwrap().id, beta_tab)
        })
        .unwrap();
    cx.simulate_keystrokes(*window, "ctrl-tab");
    window
        .update(cx, |workspace, _, _| {
            assert_eq!(workspace.sessions[1].active, 0)
        })
        .unwrap();
    cx.simulate_keystrokes(*window, "ctrl-shift-tab");
    window
        .update(cx, |workspace, _, _| {
            assert_eq!(workspace.active_tab().unwrap().id, beta_tab)
        })
        .unwrap();
}

#[gpui::test]
fn authentication_prompts_reveal_their_tab_without_stealing_another_session(
    cx: &mut TestAppContext,
) {
    setup(cx);
    let window = cx.add_window(Workspace::new);
    let (alpha, first_tab, terminal) = window
        .update(cx, |workspace, window, cx| {
            workspace.open_ssh(profile("alpha"), window, cx).unwrap();
            workspace.open_terminal(false, Axis::Columns, window, cx);
            let alpha = workspace.sessions[0].id;
            let first_tab = workspace.sessions[0].tabs[0].id;
            let terminal = workspace.sessions[0].tabs[0].panes[0].terminal.clone();
            workspace.open_ssh(profile("beta"), window, cx).unwrap();
            (alpha, first_tab, terminal)
        })
        .unwrap();
    cx.update(|cx| terminal.update(cx, |_, cx| cx.emit(TerminalEvent::AuthenticationRequired)));
    window
        .update(cx, |workspace, window, cx| {
            assert_eq!(workspace.active, 1);
            workspace.switch_session(alpha, window, cx);
        })
        .unwrap();
    cx.update(|cx| terminal.update(cx, |_, cx| cx.emit(TerminalEvent::AuthenticationRequired)));
    window
        .update(cx, |workspace, window, cx| {
            assert_eq!(workspace.active_tab().unwrap().id, first_tab);
            assert!(terminal.focus_handle(cx).is_focused(window));
        })
        .unwrap();
}
#[gpui::test]
fn closing_tabs_and_sessions_preserves_other_groups(cx: &mut TestAppContext) {
    setup(cx);
    let window = cx.add_window(Workspace::new);
    window
        .update(cx, |workspace, window, cx| {
            for user in ["alpha", "beta", "gamma"] {
                workspace.open_ssh(profile(user), window, cx).unwrap();
            }
            let alpha = workspace.sessions[0].id;
            let beta = workspace.sessions[1].id;
            let gamma = workspace.sessions[2].id;
            workspace.open_terminal(false, Axis::Columns, window, cx);
            let tab = workspace.active_tab().unwrap().id;
            workspace.close_session(alpha, window, cx);
            assert_eq!(workspace.sessions[workspace.active].id, gamma);
            assert_eq!(workspace.active_tab().unwrap().id, tab);
            workspace.close_tab(tab, window, cx);
            assert_eq!(workspace.sessions[workspace.active].tabs.len(), 1);
            workspace.close_pane(window, cx);
            assert_eq!(workspace.sessions.len(), 1);
            assert_eq!(workspace.sessions[0].id, beta);
            assert_eq!(workspace.sessions[0].tabs.len(), 1);
            assert!(
                workspace.active_tab().unwrap().panes[0]
                    .terminal
                    .focus_handle(cx)
                    .is_focused(window)
            );
        })
        .unwrap();
}

#[gpui::test]
fn keyboard_splits_zoom_and_close_keep_focus_and_session_layout_consistent(
    cx: &mut TestAppContext,
) {
    setup(cx);
    let window = cx.add_window(Workspace::new);
    window
        .update(cx, |workspace, window, cx| {
            workspace.open_ssh(profile("alpha"), window, cx).unwrap()
        })
        .unwrap();
    cx.simulate_keystrokes(*window, "ctrl-shift-d ctrl-shift-e");
    window
        .update(cx, |workspace, window, cx| {
            let tab = workspace.active_tab().unwrap();
            assert_eq!(tab.panes.len(), 3);
            assert_eq!(tab.layout.panes().len(), 3);
            assert!(
                tab.panes
                    .iter()
                    .any(|p| p.id == tab.active_pane
                        && p.terminal.focus_handle(cx).is_focused(window))
            );
            assert_eq!(workspace.split_bounds.len(), 2);
        })
        .unwrap();
    cx.simulate_keystrokes(*window, "ctrl-shift-z ctrl-shift-j ctrl-shift-w");
    window
        .update(cx, |workspace, window, cx| {
            let tab = workspace.active_tab().unwrap();
            assert_eq!(tab.panes.len(), 2);
            assert!(tab.zoom.is_none());
            assert!(
                tab.panes
                    .iter()
                    .any(|p| p.id == tab.active_pane
                        && p.terminal.focus_handle(cx).is_focused(window))
            );
        })
        .unwrap();
    cx.simulate_keystrokes(*window, "ctrl-shift-k ctrl-shift-d escape");
    window
        .update(cx, |workspace, _, _| {
            assert!(workspace.connect_form.is_none());
            assert_eq!(workspace.active_tab().unwrap().panes.len(), 2);
        })
        .unwrap();
}

#[gpui::test]
fn dragging_divider_updates_the_nested_layout(cx: &mut TestAppContext) {
    setup(cx);
    let (workspace, cx) = cx.add_window_view(Workspace::new);
    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.open_ssh(profile("alpha"), window, cx).unwrap()
        })
    });
    cx.simulate_keystrokes("ctrl-shift-d");
    let (split_id, bounds) = cx.update(|_, cx| {
        workspace.update(cx, |workspace, _| {
            let Layout::Split { id, .. } = workspace.active_tab().unwrap().layout else {
                panic!("Expected split")
            };
            (id, workspace.split_bounds[&id])
        })
    });
    let start = gpui::point(
        bounds.left() + bounds.size.width * 0.5 + px(2.),
        bounds.top() + bounds.size.height * 0.5,
    );
    let end = gpui::point(bounds.left() + bounds.size.width * 0.7, start.y);
    cx.simulate_mouse_down(start, MouseButton::Left, Default::default());
    cx.simulate_mouse_move(end, MouseButton::Left, Default::default());
    cx.simulate_mouse_up(end, MouseButton::Left, Default::default());
    cx.update(|_, cx| {
        workspace.update(cx, |workspace, _| {
            let Layout::Split { id, ratio, .. } = workspace.active_tab().unwrap().layout else {
                panic!("Expected split")
            };
            assert_eq!(id, split_id);
            assert!((ratio - 0.7).abs() < 0.01);
            assert!(workspace.drag.is_none());
        })
    });
}

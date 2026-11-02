use crate::{LINE_HEIGHT, PADDING, TerminalView};
use gpui::{
    ClipboardItem, Entity, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Point, TestAppContext, VisualTestContext, point, px, size,
};
use session::{LocalProfile, Session, State};
use std::{
    thread,
    time::{Duration, Instant},
};
use terminal_core::GridSize;

#[gpui::test]
fn application_mouse_capture_and_shift_selection_do_not_conflict(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| {
        let mut view = TerminalView::from_session(
            Err(anyhow::anyhow!("synthetic application viewport")),
            "mouse fixture".into(),
            String::new(),
            true,
            cx,
        );
        let mut terminal = terminal_core::Terminal::new(GridSize::default()).unwrap();
        terminal.feed(b"\x1b[?1002h\x1b[?1006halpha beta");
        view.screen = Some(std::sync::Arc::new(terminal.snapshot().unwrap()));
        view.state = State::Running;
        view
    });
    cx.update(|window, _| window.activate_window());
    cx.simulate_resize(size(px(680.), px(390.)));
    let start = position(&view, 0.1, 0.5, cx);
    let end = position(&view, 4.9, 0.5, cx);
    cx.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: start,
        click_count: 1,
        ..Default::default()
    });
    cx.update(|_, cx| {
        let v = view.read(cx);
        assert!(v.mouse_capture.is_some());
        assert!(!v.selecting);
    });
    cx.simulate_event(MouseUpEvent {
        button: MouseButton::Left,
        position: end,
        ..Default::default()
    });
    cx.update(|_, cx| assert!(view.read(cx).mouse_capture.is_none()));
    let shift = gpui::Modifiers {
        shift: true,
        ..Default::default()
    };
    cx.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: start,
        click_count: 1,
        modifiers: shift,
        ..Default::default()
    });
    cx.update(|_, cx| {
        let view = view.read(cx);
        assert!(view.selecting);
        assert!(view.mouse_capture.is_none());
    });
    cx.simulate_event(MouseMoveEvent {
        position: end,
        pressed_button: Some(MouseButton::Left),
        modifiers: shift,
    });
    cx.simulate_event(MouseUpEvent {
        button: MouseButton::Left,
        position: end,
        modifiers: shift,
        ..Default::default()
    });
    cx.update(|_, cx| assert!(!view.read(cx).selecting));
    cx.simulate_event(MouseDownEvent {
        button: MouseButton::Right,
        position: start,
        click_count: 1,
        ..Default::default()
    });
    cx.update(|_, cx| assert!(view.read(cx).mouse_capture.is_some()));
    cx.deactivate_window();
    wait(cx, |cx| {
        cx.update(|_, cx| view.read(cx).mouse_capture.is_none())
    });
}

#[gpui::test]
fn failed_ssh_connection_exposes_reconnect_and_the_shortcut_retries_in_place(
    cx: &mut TestAppContext,
) {
    use std::{cell::Cell, rc::Rc};
    let executor = cx.executor();
    cx.on_quit(move || executor.allow_parking());
    let (view, cx) = cx.add_window_view(|window, cx| {
        let view = TerminalView::ssh(
            session::SshProfile {
                host: "127.0.0.1".into(),
                port: 9,
                username: "fixture".into(),
                auth: session::SshAuth::Agent,
                options: Default::default(),
                known_hosts: "unused-test-known-hosts".into(),
            },
            cx,
        );
        window.focus(&view.focus);
        view
    });
    wait(cx, |cx| {
        cx.update(|_, cx| matches!(view.read(cx).state, State::Failed(_)))
    });
    assert!(cx.debug_bounds("reconnect").is_some());
    cx.simulate_keystrokes("x");
    cx.update(|_, cx| {
        assert!(
            view.read(cx)
                .error
                .as_ref()
                .is_some_and(|error| error.contains("Reconnect"))
        )
    });
    let requests = Rc::new(Cell::new(0));
    let count = requests.clone();
    let _subscription = cx.update(|_, cx| {
        cx.subscribe(&view, move |_, event, _| {
            if matches!(event, crate::TerminalEvent::Reconnecting) {
                count.set(count.get() + 1)
            }
        })
    });
    cx.simulate_keystrokes("ctrl-shift-r");
    assert_eq!(requests.get(), 1);
    wait(cx, |cx| {
        cx.update(|_, cx| matches!(view.read(cx).state, State::Failed(_)))
    });
    assert!(cx.debug_bounds("reconnect").is_some());
}

fn fixture<'a>(
    cx: &'a mut TestAppContext,
    command: &str,
) -> (Entity<TerminalView>, &'a mut VisualTestContext) {
    let executor = cx.executor();
    cx.on_quit(move || executor.allow_parking());
    let profile = LocalProfile {
        args: vec![
            "-NoLogo".into(),
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-Command".into(),
            format!("{command}; exit 0"),
        ],
        ..LocalProfile::default()
    };
    // A local deterministic producer exercises the same parser and UI as SSH,
    // without accessing the user's SSH configuration, keys, or external hosts.
    let (view, cx) = cx.add_window_view(|_, cx| {
        TerminalView::from_session(
            Session::local(profile, GridSize::default()),
            "selection fixture".into(),
            String::new(),
            true,
            cx,
        )
    });
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    cx.simulate_resize(size(px(680.), px(390.)));
    wait(cx, |cx| {
        cx.update(|_, cx| {
            let view = view.read(cx);
            assert!(!matches!(view.state, State::Failed(_)), "{:?}", view.state);
            view.state == State::Exited(0)
                && view.screen.as_ref().is_some_and(|s| s.size == view.grid)
        })
    });
    (view, cx)
}

fn wait(cx: &mut VisualTestContext, mut predicate: impl FnMut(&mut VisualTestContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        cx.run_until_parked();
        cx.refresh().unwrap();
        cx.run_until_parked();
        if predicate(cx) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Timed out waiting for terminal UI"
        );
        thread::sleep(Duration::from_millis(2));
    }
}

fn position(
    view: &Entity<TerminalView>,
    x: f32,
    y: f32,
    cx: &mut VisualTestContext,
) -> Point<Pixels> {
    cx.update(|_, cx| {
        let view = view.read(cx);
        view.bounds.unwrap().origin
            + point(
                px(PADDING) + view.cell_width * x,
                px(PADDING + LINE_HEIGHT * y),
            )
    })
}

fn click(cx: &mut VisualTestContext, position: Point<Pixels>, clicks: usize) {
    cx.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position,
        click_count: clicks,
        ..Default::default()
    });
    cx.simulate_event(MouseUpEvent {
        button: MouseButton::Left,
        position,
        click_count: clicks,
        ..Default::default()
    });
}

fn clipboard(cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()))
}

#[gpui::test]
fn word_line_copy_and_escape_use_the_terminal_selection(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx, "[Console]::Write(\"alpha beta`r`nsecond`r`n\")");
    let row = cx.update(|_, cx| {
        view.read(cx)
            .screen
            .as_ref()
            .unwrap()
            .text()
            .lines()
            .position(|line| line == "alpha beta")
            .unwrap()
    }) as f32;
    let word = position(&view, 1.5, row + 0.5, cx);
    click(cx, word, 2);
    cx.simulate_keystrokes("ctrl-shift-c");
    wait(cx, |cx| clipboard(cx).as_deref() == Some("alpha"));
    let line = position(&view, 7.5, row + 0.5, cx);
    click(cx, line, 3);
    cx.simulate_keystrokes("ctrl-shift-c");
    wait(cx, |cx| clipboard(cx).as_deref() == Some("alpha beta"));
    cx.simulate_keystrokes("escape");
    wait(cx, |cx| {
        cx.update(|_, cx| !view.read(cx).screen.as_ref().unwrap().has_selection)
    });
    cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("keep clipboard".into())));
    cx.simulate_keystrokes("ctrl-shift-c");
    // A second native request is a barrier after the keyboard copy request.
    let barrier = cx.update(|_, cx| {
        view.read(cx)
            .session
            .as_ref()
            .unwrap()
            .copy_selection()
            .unwrap()
    });
    wait(cx, |_| barrier.try_recv().is_ok());
    cx.run_until_parked();
    assert_eq!(clipboard(cx).as_deref(), Some("keep clipboard"));
}

#[gpui::test]
fn drag_autoscroll_crosses_history_and_release_outside_the_pane_stops_it(cx: &mut TestAppContext) {
    let (view, cx) = fixture(
        cx,
        "0..99 | ForEach-Object { [Console]::WriteLine(('line-{0:D3}' -f $_)) }",
    );
    let anchor = cx.update(|_, cx| {
        view.read(cx)
            .screen
            .as_ref()
            .unwrap()
            .text()
            .lines()
            .nth(3)
            .unwrap()
            .to_owned()
    });
    let origin = position(&view, 7.9, 3.5, cx);
    let outside = position(&view, 0.1, -4., cx);
    cx.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: origin,
        click_count: 1,
        ..Default::default()
    });
    cx.simulate_event(MouseMoveEvent {
        position: outside,
        pressed_button: Some(MouseButton::Left),
        ..Default::default()
    });
    for _ in 0..20 {
        cx.executor().advance_clock(Duration::from_millis(40));
        cx.run_until_parked();
        thread::sleep(Duration::from_millis(2));
    }
    cx.simulate_event(MouseUpEvent {
        button: MouseButton::Left,
        position: outside,
        click_count: 1,
        ..Default::default()
    });
    cx.update(|_, cx| {
        let view = view.read(cx);
        assert!(!view.selecting);
        assert!(view.autoscroll.is_none());
        assert!(view.drag_observers.is_empty());
    });
    cx.simulate_keystrokes("ctrl-shift-c");
    wait(cx, |cx| clipboard(cx).is_some());
    let text = clipboard(cx).unwrap();
    assert!(text.contains(&anchor), "Missing anchor {anchor}: {text}");
    assert!(
        text.lines().count() > 40,
        "Auto-scroll did not extend through history: {text}"
    );
    let stopped = cx.update(|_, cx| view.read(cx).screen.as_ref().unwrap().text());
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_, cx| view.read(cx).screen.as_ref().unwrap().text()),
        stopped
    );
    // Moving focus to another pane/tab cancels a live drag as well.
    cx.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: origin,
        click_count: 1,
        ..Default::default()
    });
    cx.update(|window, _| window.blur());
    wait(cx, |cx| cx.update(|_, cx| !view.read(cx).selecting));
}

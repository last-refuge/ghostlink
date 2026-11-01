//! Exercise real Unix PTYs independently of the SSH loopback server.
use super::*;

fn wait(session: &Session, predicate: impl Fn(&Update) -> bool) -> Update {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(update) = session.updates.try_recv() {
            assert!(!matches!(update.state, State::Failed(_)), "{update:?}");
            if predicate(&update) {
                return update;
            }
        }
        assert!(Instant::now() < deadline, "Timed out waiting for Unix PTY");
        thread::sleep(Duration::from_millis(5));
    }
}

fn shell(script: &str, size: GridSize) -> Session {
    Session::local(
        LocalProfile {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            directory: None,
        },
        size,
    )
    .unwrap()
}

#[test]
fn unix_pty_input_unicode_resize_exit_and_retained_copy() {
    let session = shell(
        "printf 'READY\\r\\n'; IFS= read -r line; printf 'ECHO:%s\\r\\n' \"$line\"; stty size; exit 7",
        GridSize::default(),
    );
    wait(&session, |u| {
        u.screen
            .as_ref()
            .is_some_and(|s| s.text().contains("READY"))
    });
    let size = GridSize::new(92, 28).unwrap();
    session.send(Input::Resize(size)).unwrap();
    session.send(Input::Text("héllo 远端\r".into())).unwrap();
    let update = wait(&session, |u| matches!(u.state, State::Exited(_)));
    assert_eq!(update.state, State::Exited(7));
    let screen = update.screen.unwrap();
    assert_eq!(screen.size, size);
    let text = screen.text();
    assert!(text.contains("ECHO:héllo 远端"), "{text}");
    assert!(text.contains("28 92"), "{text}");
    let row = text
        .lines()
        .position(|line| line.starts_with("ECHO:"))
        .unwrap() as f64;
    session
        .send(Input::Select(SelectionInput::Begin {
            x: 0.5,
            y: row + 0.5,
            clicks: 2,
        }))
        .unwrap();
    session
        .send(Input::Select(SelectionInput::End {
            x: 0.5,
            y: row + 0.5,
        }))
        .unwrap();
    assert_eq!(copied_selection(&session).as_deref(), Some("ECHO"));
    let signal = session.shutdown_signal();
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::timeout(Duration::from_secs(3), signal.shutdown())
                .await
                .unwrap();
        });
    assert!(session.finished.is_closed());
}

#[test]
fn unix_pty_drains_large_output_before_exit_and_stops_when_dropped() {
    let session = shell(
        "i=0; while [ \"$i\" -lt 5000 ]; do printf 'line-%s\\r\\n' \"$i\"; i=$((i+1)); done; printf 'GHOSTLINK_DONE\\r\\n'; exit 9",
        GridSize::default(),
    );
    let update = wait(&session, |u| matches!(u.state, State::Exited(_)));
    assert_eq!(update.state, State::Exited(9));
    let text = update.screen.unwrap().text();
    assert!(
        text.contains("line-4999") && text.contains("GHOSTLINK_DONE"),
        "{text}"
    );
    let updates = session.updates();
    drop(session);
    let deadline = Instant::now() + Duration::from_secs(3);
    while !updates.is_closed() {
        let _ = updates.try_recv();
        assert!(Instant::now() < deadline, "Unix PTY worker did not stop");
        thread::sleep(Duration::from_millis(5));
    }
}

//! Manual real-application gate; regular CI remains independent of WSL packages.
use super::*;
use terminal_core::{MouseButton, MouseInput, MouseKind};

fn wait(session: &Session, predicate: impl Fn(&Update) -> bool) -> Update {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut last = None;
    loop {
        if let Ok(update) = session.updates.try_recv() {
            if predicate(&update) {
                return update;
            }
            let diagnostic = (
                update.state.clone(),
                update.screen.as_ref().map(|s| s.text()),
            );
            assert!(
                !matches!(
                    update.state,
                    State::Failed(_) | State::Disconnected(_) | State::Exited(_)
                ),
                "Real application stopped early: {diagnostic:?}"
            );
            last = Some(diagnostic);
        }
        assert!(
            Instant::now() < deadline,
            "Real application timed out: {last:?}"
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
struct TmuxCleanup {
    program: PathBuf,
    socket: Option<String>,
}
impl Drop for TmuxCleanup {
    fn drop(&mut self) {
        use std::os::windows::process::CommandExt;
        if let Some(socket) = &self.socket {
            let _ = std::process::Command::new(&self.program)
                .args([
                    "-d",
                    "Ubuntu",
                    "--exec",
                    "tmux",
                    "-L",
                    socket,
                    "kill-server",
                ])
                .creation_flags(0x08000000)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }
}

#[test]
#[ignore = "manual real Neovim/tmux gate; requires WSL Ubuntu and GHOSTLINK_NVIM pointing to its Linux Neovim executable"]
fn real_neovim_and_tmux_keyboard_mouse_scroll_resize_and_alternate_screen() {
    let nvim = std::env::var("GHOSTLINK_NVIM")
        .expect("Set GHOSTLINK_NVIM to a Neovim executable inside Ubuntu");
    let lua = "lua local l={};for i=1,200 do l[i]=string.format('probe-%03d',i) end;vim.api.nvim_buf_set_lines(0,0,-1,false,l);vim.o.mouse='a'";
    let editor = format!("{} --clean -n -i NONE -c {}", quoted(&nvim), quoted(lua));
    for multiplexed in [false, true] {
        let socket = format!(
            "ghostlink-compat-{}-{}",
            std::process::id(),
            rand::random::<u32>()
        );
        let command = if multiplexed {
            format!(
                "tmux -L {socket} -f /dev/null new-session -s probe {}",
                quoted(&editor)
            )
        } else {
            editor.clone()
        };
        // All editor state is temporary. The private tmux socket keeps normal sessions untouched.
        let script = format!(
            "export TERM=xterm-256color; printf 'BEFORE APP\\r\\n'; {command}; printf 'GHOSTLINK_COMPAT_DONE\\r\\n'"
        );
        let program =
            PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/wsl.exe");
        let _cleanup = TmuxCleanup {
            program: program.clone(),
            socket: multiplexed.then_some(socket),
        };
        let profile = LocalProfile {
            program: program.to_string_lossy().into_owned(),
            args: vec![
                "-d".into(),
                "Ubuntu".into(),
                "--exec".into(),
                "sh".into(),
                "-lc".into(),
                script,
            ],
            directory: None,
        };
        let session = Session::local(profile, GridSize::new(80, 24).unwrap()).unwrap();
        wait(&session, |u| {
            u.screen
                .as_ref()
                .is_some_and(|s| s.mouse_tracking && s.text().contains("probe-001"))
        });
        session
            .send(Input::Key(Key::Down, Modifiers::default()))
            .unwrap();
        wait(&session, |u| {
            u.screen.as_ref().is_some_and(|s| s.cursor.row == 1)
        });
        let mouse = MouseInput {
            kind: MouseKind::Press(MouseButton::Left),
            x: 3.5,
            y: 4.5,
            cell_width: 9,
            cell_height: 22,
            modifiers: Modifiers::default(),
        };
        session.send(Input::Mouse(mouse)).unwrap();
        session
            .send(Input::Mouse(MouseInput {
                kind: MouseKind::Release(MouseButton::Left),
                ..mouse
            }))
            .unwrap();
        wait(&session, |u| {
            u.screen.as_ref().is_some_and(|s| s.cursor.row == 4)
        });
        session
            .send(Input::Resize(GridSize::new(100, 34).unwrap()))
            .unwrap();
        wait(&session, |u| {
            u.screen.as_ref().is_some_and(|s| {
                s.size == GridSize::new(100, 34).unwrap() && s.text().contains("probe-030")
            })
        });
        session
            .send(Input::Mouse(MouseInput {
                kind: MouseKind::Scroll {
                    horizontal: 0,
                    vertical: 3,
                },
                ..mouse
            }))
            .unwrap();
        wait(&session, |u| {
            u.screen
                .as_ref()
                .is_some_and(|s| !s.text().contains("probe-001"))
        });
        session.send(Input::Text("i".into())).unwrap();
        session
            .send(Input::Paste("GHOSTLINK 界👻 ".into()))
            .unwrap();
        session
            .send(Input::Key(Key::Escape, Modifiers::default()))
            .unwrap();
        wait(&session, |u| {
            u.screen
                .as_ref()
                .is_some_and(|s| s.text().contains("GHOSTLINK 界👻"))
        });
        if multiplexed {
            session
                .send(Input::Key(
                    Key::B,
                    Modifiers {
                        control: true,
                        ..Default::default()
                    },
                ))
                .unwrap();
            session
                .send(Input::Text(
                    ":display-message GHOSTLINK_TMUX_READY\r".into(),
                ))
                .unwrap();
            wait(&session, |u| {
                u.screen
                    .as_ref()
                    .is_some_and(|s| s.text().contains("GHOSTLINK_TMUX_READY"))
            });
        }
        session.send(Input::Text(":qa!\r".into())).unwrap();
        let exited = wait(&session, |u| matches!(u.state, State::Exited(_)));
        assert_eq!(exited.state, State::Exited(0));
        let screen = exited.screen.unwrap();
        assert!(screen.text().contains("GHOSTLINK_COMPAT_DONE"));
        assert!(
            !screen.mouse_tracking,
            "Application mouse mode leaked after exit"
        );
        assert!(!screen.alternate_screen, "Primary screen was not restored");
        eprintln!(
            "Real Neovim{}: keyboard, mouse, scrolling, resize, Unicode paste, and exit passed",
            if multiplexed { " inside tmux" } else { "" }
        );
    }
}

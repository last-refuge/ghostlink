use super::*;
use russh::{
    Channel, ChannelId, Pty,
    server::{self, Server as _},
};
use std::sync::atomic::AtomicUsize;

#[derive(Debug)]
enum Observed {
    Closed,
    Pty(u32, u32),
    Resize(u32, u32),
    Bytes(Vec<u8>),
}
#[derive(Clone)]
struct TestServer {
    events: mpsc::Sender<Observed>,
    authentications: Arc<AtomicUsize>,
    authorized_key: PublicKey,
    interactive: bool,
    startup: StartupBehavior,
}
#[derive(Clone, Copy, Default)]
enum StartupBehavior {
    #[default]
    Normal,
    Interleaved,
    Refused,
    Closed,
}
impl server::Server for TestServer {
    type Handler = Self;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        self.clone()
    }
}
impl server::Handler for TestServer {
    type Error = russh::Error;
    async fn channel_close(
        &mut self,
        _: ChannelId,
        _: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let _ = self.events.send(Observed::Closed);
        Ok(())
    }
    async fn auth_publickey_offered(
        &mut self,
        user: &str,
        key: &PublicKey,
    ) -> Result<server::Auth, Self::Error> {
        Ok(
            if user == "tester" && key.key_data() == self.authorized_key.key_data() {
                server::Auth::Accept
            } else {
                server::Auth::reject()
            },
        )
    }
    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        user: &str,
        _: &str,
        response: Option<server::Response<'a>>,
    ) -> Result<server::Auth, Self::Error> {
        if !self.interactive || user != "tester" {
            return Ok(server::Auth::reject());
        }
        if let Some(mut response) = response {
            return Ok(
                if response
                    .next()
                    .is_some_and(|value| value.as_ref() == b"123456")
                {
                    server::Auth::Accept
                } else {
                    server::Auth::reject()
                },
            );
        }
        Ok(server::Auth::Partial {
            name: "Verification".into(),
            instructions: "".into(),
            prompts: vec![("Verification code".into(), false)].into(),
        })
    }
    async fn auth_password(
        &mut self,
        user: &str,
        password: &str,
    ) -> Result<server::Auth, Self::Error> {
        self.authentications.fetch_add(1, Ordering::SeqCst);
        Ok(if user == "tester" && password == "test password" {
            server::Auth::Accept
        } else {
            server::Auth::reject()
        })
    }
    async fn auth_publickey(
        &mut self,
        user: &str,
        key: &PublicKey,
    ) -> Result<server::Auth, Self::Error> {
        self.authentications.fetch_add(1, Ordering::SeqCst);
        Ok(
            if user == "tester" && key.key_data() == self.authorized_key.key_data() {
                server::Auth::Accept
            } else {
                server::Auth::reject()
            },
        )
    }
    async fn channel_open_session(
        &mut self,
        _: Channel<server::Msg>,
        reply: server::ChannelOpenHandle,
        _: &mut server::Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    async fn pty_request(
        &mut self,
        channel: ChannelId,
        term: &str,
        columns: u32,
        rows: u32,
        _: u32,
        _: u32,
        _: &[(Pty, u32)],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        assert_eq!(term, "xterm-256color");
        let _ = self.events.send(Observed::Pty(columns, rows));
        session.channel_success(channel)?;
        if matches!(self.startup, StartupBehavior::Interleaved) {
            session.xon_xoff_request(channel, false)?;
            session.data(channel, b"PTY STARTUP\r\n\x1b[6n".as_slice())?;
        }
        Ok(())
    }
    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        match self.startup {
            StartupBehavior::Interleaved => {
                session.xon_xoff_request(channel, true)?;
                session.data(channel, b"EARLY STDOUT\r\n".as_slice())?;
                session.extended_data(channel, 1, b"EARLY STDERR\r\n".as_slice())?;
            }
            StartupBehavior::Refused => {
                session.extended_data(channel, 1, b"SHELL DISABLED\r\n".as_slice())?;
                session.channel_failure(channel)?;
                return Ok(());
            }
            StartupBehavior::Closed => {
                session.extended_data(channel, 1, b"SESSION CLOSED\r\n".as_slice())?;
                session.eof(channel)?;
                session.close(channel)?;
                return Ok(());
            }
            StartupBehavior::Normal => {}
        }
        session.channel_success(channel)?;
        let greeting = if matches!(self.startup, StartupBehavior::Interleaved) {
            b"REMOTE READY\r\n\x1b[6n".as_slice()
        } else {
            b"\x1b[?1049h\x1b[?1h\x1b[2J\x1b[HREMOTE READY\r\n\x1b[6n".as_slice()
        };
        session.data(channel, greeting)?;
        session.extended_data(channel, 1, b"STDERR READY\r\n".as_slice())?;
        Ok(())
    }
    async fn window_change_request(
        &mut self,
        _: ChannelId,
        columns: u32,
        rows: u32,
        _: u32,
        _: u32,
        _: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let _ = self.events.send(Observed::Resize(columns, rows));
        Ok(())
    }
    async fn data(
        &mut self,
        channel: ChannelId,
        bytes: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let _ = self.events.send(Observed::Bytes(bytes.to_vec()));
        if bytes.starts_with(b"latency-") {
            session.data(channel, bytes.to_vec())?;
        } else if bytes == b"disconnect\r" {
            session.data(channel, b"BEFORE DISCONNECT\r\n".as_slice())?;
            session.disconnect(russh::Disconnect::ByApplication, "fixture disconnect", "")?;
        } else if bytes == b"mouse\r" {
            session.data(channel, b"\x1b[?1002h\x1b[?1006hMOUSE READY\r\n".as_slice())?;
        } else if bytes == b"finish\r" {
            // Closing immediately after sending bytes must not lose the final frame.
            session.data(channel, b"FINAL OUTPUT\r\n".as_slice())?;
            session.exit_status_request(channel, 23)?;
            session.eof(channel)?;
            session.close(channel)?;
        } else if bytes == b"bulk\r" {
            let text = (0..5000)
                .map(|i| format!("remote-line-{i}\r\n"))
                .collect::<String>();
            session.data(channel, text.into_bytes())?;
        } else if bytes == b"roundtrip\r" {
            session.data(channel, b"ECHO ROUNDTRIP\r\n".as_slice())?;
        }
        Ok(())
    }
}
#[test]
fn mouse_events_reach_the_remote_application_without_scrolling_the_viewport() {
    let fixture = Fixture::new();
    fixture.trust();
    let session = Session::ssh(fixture.profile(), GridSize::default()).unwrap();
    wait(&session, |u| u.state == State::Running);
    session.send(Input::Text("mouse\r".into())).unwrap();
    wait(&session, |u| {
        u.screen.as_ref().is_some_and(|s| s.mouse_tracking)
    });
    let event = terminal_core::MouseInput {
        kind: terminal_core::MouseKind::Press(terminal_core::MouseButton::Left),
        x: 2.5,
        y: 3.5,
        cell_width: 9,
        cell_height: 22,
        modifiers: Modifiers::default(),
    };
    session.send(Input::Mouse(event)).unwrap();
    fixture.observed(|e| matches!(e,Observed::Bytes(b) if b==b"\x1b[<0;3;4M"));
    session
        .send(Input::Mouse(terminal_core::MouseInput {
            kind: terminal_core::MouseKind::Scroll {
                horizontal: 0,
                vertical: -2,
            },
            ..event
        }))
        .unwrap();
    fixture.observed(|e| matches!(e,Observed::Bytes(b) if b==b"\x1b[<64;3;4M\x1b[<64;3;4M"));
}
#[test]
fn reconnect_replaces_a_dead_shared_transport_once_and_keeps_history_and_size() {
    let fixture = Fixture::with_startup(
        keys::Algorithm::Ed25519,
        false,
        StartupBehavior::Interleaved,
    );
    fixture.trust();
    let shared = SshConnection::new(fixture.profile()).unwrap();
    let first = Session::ssh_in(shared.clone(), GridSize::default()).unwrap();
    let second = Session::ssh_in(shared, GridSize::default()).unwrap();
    wait(&first, |u| u.state == State::Running);
    wait(&second, |u| u.state == State::Running);
    first.send(Input::Text("roundtrip\r".into())).unwrap();
    wait(&first, |u| {
        u.screen
            .as_ref()
            .is_some_and(|s| s.text().contains("ECHO ROUNDTRIP"))
    });
    first.send(Input::Text("disconnect\r".into())).unwrap();
    wait(&first, |u| matches!(u.state, State::Disconnected(_)));
    wait(&second, |u| matches!(u.state, State::Disconnected(_)));
    first
        .send(Input::Resize(GridSize::new(92, 28).unwrap()))
        .unwrap();
    first.reconnect().unwrap();
    second.reconnect().unwrap();
    let recovered = wait(&first, |u| u.state == State::Running);
    wait(&second, |u| u.state == State::Running);
    assert_eq!(fixture.authentications.load(Ordering::SeqCst), 2);
    assert_eq!(
        recovered.screen.unwrap().size,
        GridSize::new(92, 28).unwrap()
    );
    first.send(Input::Scroll(-10000)).unwrap();
    wait(&first, |u| {
        u.screen
            .as_ref()
            .is_some_and(|s| s.text().contains("ECHO ROUNDTRIP"))
    });
    second.send(Input::Text("roundtrip\r".into())).unwrap();
    wait(&second, |u| {
        u.screen
            .as_ref()
            .is_some_and(|s| s.text().contains("ECHO ROUNDTRIP"))
    });
}
#[test]
fn reconnect_after_shell_exit_reuses_authentication_and_changed_keys_still_block() {
    let fixture = Fixture::with_startup(
        keys::Algorithm::Ed25519,
        false,
        StartupBehavior::Interleaved,
    );
    fixture.trust();
    let profile = fixture.profile();
    let session = Session::ssh(profile.clone(), GridSize::default()).unwrap();
    wait(&session, |u| u.state == State::Running);
    session.send(Input::Text("finish\r".into())).unwrap();
    wait(&session, |u| u.state == State::Exited(23));
    session.reconnect().unwrap();
    wait(&session, |u| u.state == State::Running);
    assert_eq!(fixture.authentications.load(Ordering::SeqCst), 1);
    session.send(Input::Text("disconnect\r".into())).unwrap();
    wait(&session, |u| matches!(u.state, State::Disconnected(_)));
    fs::write(
        &profile.known_hosts,
        format!(
            "{} {}\n",
            endpoint(&profile.host, profile.port),
            fixture.user_key.public_key().to_openssh().unwrap()
        ),
    )
    .unwrap();
    session.reconnect().unwrap();
    wait(
        &session,
        |u| matches!(&u.state,State::Failed(error) if error.contains("HOST KEY CHANGED")),
    );
    assert_eq!(fixture.authentications.load(Ordering::SeqCst), 1);
    session.send(Input::Scroll(-10000)).unwrap();
    wait(&session, |u| {
        u.screen
            .as_ref()
            .is_some_and(|s| s.text().contains("FINAL OUTPUT"))
    });
}
struct Fixture {
    port: u16,
    key: keys::PrivateKey,
    user_key: keys::PrivateKey,
    events: mpsc::Receiver<Observed>,
    authentications: Arc<AtomicUsize>,
    directory: tempfile::TempDir,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Fixture {
    fn new() -> Self {
        Self::with_options(keys::Algorithm::Ed25519, false)
    }
    fn with_options(algorithm: keys::Algorithm, interactive: bool) -> Self {
        Self::with_startup(algorithm, interactive, StartupBehavior::Normal)
    }
    fn with_startup(
        algorithm: keys::Algorithm,
        interactive: bool,
        startup: StartupBehavior,
    ) -> Self {
        let key = keys::PrivateKey::random(&mut rand::rng(), keys::Algorithm::Ed25519).unwrap();
        let user_key = keys::PrivateKey::random(&mut rand::rng(), algorithm).unwrap();
        let config = Arc::new(server::Config {
            keys: vec![key.clone()],
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            ..Default::default()
        });
        let (events_tx, events) = mpsc::channel();
        let (port_tx, port_rx) = mpsc::channel();
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let authentications = Arc::new(AtomicUsize::new(0));
        let mut server = TestServer {
            events: events_tx,
            authentications: authentications.clone(),
            authorized_key: user_key.public_key().clone(),
            interactive,
            startup,
        };
        let thread = thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async move {
                let socket=tokio::net::TcpListener::bind(("127.0.0.1",0)).await.unwrap();
                port_tx.send(socket.local_addr().unwrap().port()).unwrap();
                tokio::select! { result=server.run_on_socket(config,&socket)=>{result.unwrap();}, _=stopped=>{} }
            });
        });
        Self {
            port: port_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            key,
            user_key,
            events,
            authentications,
            directory: tempfile::tempdir().unwrap(),
            stop: Some(stop),
            thread: Some(thread),
        }
    }
    fn profile(&self) -> SshProfile {
        SshProfile {
            host: "127.0.0.1".into(),
            port: self.port,
            username: "tester".into(),
            auth: SshAuth::Password(Zeroizing::new("test password".into())),
            known_hosts: self.directory.path().join("known_hosts"),
        }
    }
    fn trust(&self) {
        let profile = self.profile();
        remember_host(
            &profile.known_hosts,
            &profile.host,
            profile.port,
            self.key.public_key(),
        )
        .unwrap();
    }
    fn observed(&self, predicate: impl Fn(&Observed) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let value = self
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("Missing server event");
            if predicate(&value) {
                return;
            }
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.stop.take().unwrap().send(());
        self.thread.take().unwrap().join().unwrap();
    }
}
fn wait(session: &Session, predicate: impl Fn(&Update) -> bool) -> Update {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(update) = session.updates.try_recv() {
            if predicate(&update) {
                return update;
            }
            if let State::Failed(error) = update.state {
                panic!("Unexpected SSH failure: {error}");
            }
            if let State::AwaitingCredential(prompt) = update.state {
                panic!("Unexpected credential prompt: {}", prompt.label);
            }
        }
        assert!(
            Instant::now() < deadline,
            "Timed out waiting for SSH session"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

impl Fixture {
    fn automatic(&self, extra: &str) -> SshProfile {
        let home = self.directory.path();
        fs::create_dir_all(home.join(".ssh")).unwrap();
        fs::write(home.join(".ssh/config"),format!("Host fixture\n HostName 127.0.0.1\n User tester\n Port {}\n IdentityAgent none\n{extra}",self.port)).unwrap();
        SshProfile {
            host: "fixture".into(),
            port: 0,
            username: String::new(),
            auth: SshAuth::Automatic {
                home: home.to_owned(),
            },
            known_hosts: home.join("known_hosts"),
        }
    }
    fn write_identity(&self, name: &str, passphrase: Option<&str>) {
        let key = if let Some(passphrase) = passphrase {
            self.user_key.encrypt(&mut rand::rng(), passphrase).unwrap()
        } else {
            self.user_key.clone()
        };
        fs::write(
            self.directory.path().join(".ssh").join(name),
            key.to_openssh(keys::ssh_key::LineEnding::LF)
                .unwrap()
                .as_bytes(),
        )
        .unwrap();
    }
}
#[test]
fn grouped_tabs_authenticate_once_and_keep_sibling_channels_alive() {
    let fixture = Fixture::new();
    fixture.trust();
    let connection = SshConnection::new(fixture.automatic("")).unwrap();
    let first = Session::ssh_in(connection.clone(), GridSize::default()).unwrap();
    let update = wait(&first, |u| matches!(u.state, State::AwaitingCredential(_)));
    let State::AwaitingCredential(prompt) = update.state else {
        unreachable!()
    };
    first
        .respond_credential(prompt.id, Some("test password".into()))
        .unwrap();
    wait(&first, |u| u.state == State::Running);
    let second = Session::ssh_in(connection.clone(), GridSize::default()).unwrap();
    wait(&second, |u| u.state == State::Running);
    assert_eq!(fixture.authentications.load(Ordering::SeqCst), 1);
    first.send(Input::Text("finish\r".into())).unwrap();
    wait(&first, |u| u.state == State::Exited(23));
    drop(first);
    let third = Session::ssh_in(connection.clone(), GridSize::default()).unwrap();
    wait(&third, |u| u.state == State::Running);
    drop(second);
    fixture.observed(|e| matches!(e, Observed::Closed));
    third.send(Input::Text("roundtrip\r".into())).unwrap();
    wait(&third, |u| {
        u.screen
            .as_ref()
            .is_some_and(|s| s.text().contains("ECHO ROUNDTRIP"))
    });
    assert_eq!(fixture.authentications.load(Ordering::SeqCst), 1);
}
#[test]
fn simultaneous_tabs_share_authentication_and_cancelled_authentication_can_move_to_another_tab() {
    let fixture = Fixture::new();
    let profile = fixture.automatic("");
    fixture.write_identity("id_ed25519", None);
    let connection = SshConnection::new(profile).unwrap();
    let first = Session::ssh_in(connection.clone(), GridSize::default()).unwrap();
    wait(&first, |u| matches!(u.state, State::AwaitingHostKey(_)));
    let second = Session::ssh_in(connection.clone(), GridSize::default()).unwrap();
    drop(first);
    let update = wait(&second, |u| matches!(u.state, State::AwaitingHostKey(_)));
    let State::AwaitingHostKey(prompt) = update.state else {
        unreachable!()
    };
    assert_eq!(fixture.authentications.load(Ordering::SeqCst), 0);
    second.verify_host(prompt.fingerprint, true).unwrap();
    wait(&second, |u| u.state == State::Running);
    let third = Session::ssh_in(connection, GridSize::default()).unwrap();
    wait(&third, |u| u.state == State::Running);
    assert_eq!(fixture.authentications.load(Ordering::SeqCst), 1);
}
#[test]
#[ignore = "manual loopback SSH input-to-snapshot latency probe"]
fn ssh_input_latency_probe() {
    let fixture = Fixture::new();
    fixture.trust();
    let session = Session::ssh(fixture.profile(), GridSize::default()).unwrap();
    wait(&session, |u| u.state == State::Running);
    crate::probe::measure(
        "Loopback SSH",
        &session,
        (0..32).map(|index| {
            let marker = format!("latency-{index:02}");
            (Input::Text(format!("{marker}\r\n")), marker)
        }),
    );
}
#[test]
fn startup_messages_do_not_masquerade_as_request_rejections() {
    let fixture = Fixture::with_startup(
        keys::Algorithm::Ed25519,
        false,
        StartupBehavior::Interleaved,
    );
    fixture.trust();
    let profile = fixture.automatic("");
    let session = Session::ssh(profile, GridSize::default()).unwrap();
    let update = wait(&session, |u| {
        matches!(u.state, State::AwaitingCredential(_))
    });
    let State::AwaitingCredential(prompt) = update.state else {
        unreachable!()
    };
    session
        .respond_credential(prompt.id, Some("test password".into()))
        .unwrap();
    let update = wait(&session, |u| {
        u.screen
            .as_ref()
            .is_some_and(|s| s.text().contains("REMOTE READY"))
    });
    assert_eq!(update.state, State::Running);
    let text = update.screen.unwrap().text();
    for expected in ["PTY STARTUP", "EARLY STDOUT", "EARLY STDERR"] {
        assert!(text.contains(expected), "Missing {expected}: {text}");
    }
    fixture.observed(|e| matches!(e, Observed::Bytes(bytes) if bytes == b"\x1b[2;1R"));
    session.send(Input::Text("roundtrip\r".into())).unwrap();
    wait(&session, |u| {
        u.screen
            .as_ref()
            .is_some_and(|s| s.text().contains("ECHO ROUNDTRIP"))
    });
}
#[test]
fn actual_shell_rejection_and_channel_closure_keep_server_diagnostics() {
    for (behavior, error, diagnostic) in [
        (
            StartupBehavior::Refused,
            "Server refused the shell request",
            "SHELL DISABLED",
        ),
        (
            StartupBehavior::Closed,
            "closed before the shell request completed",
            "SESSION CLOSED",
        ),
    ] {
        let fixture = Fixture::with_startup(keys::Algorithm::Ed25519, false, behavior);
        fixture.trust();
        let session = Session::ssh(fixture.profile(), GridSize::default()).unwrap();
        let update = wait(
            &session,
            |u| matches!(&u.state, State::Failed(message) if message.contains(error)),
        );
        let text = update.screen.expect("Lost server diagnostic").text();
        assert!(text.contains(diagnostic), "{text}");
    }
}
#[test]
fn automatic_discovers_standard_key_and_resolves_alias_without_a_prompt() {
    let fixture = Fixture::new();
    fixture.trust();
    let profile = fixture.automatic("");
    fixture.write_identity("id_ed25519", None);
    let session = Session::ssh(profile, GridSize::default()).unwrap();
    wait(&session, |u| u.state == State::Running);
}
#[test]
fn automatic_uses_a_configured_identity_and_missing_agent_is_nonfatal() {
    let fixture = Fixture::new();
    fixture.trust();
    let profile = fixture.automatic(" IdentityFile \"~/.ssh/custom key\"\n");
    fixture.write_identity("custom key", None);
    // Replace the explicit disabled agent with a guaranteed absent endpoint.
    let config = fixture.directory.path().join(".ssh/config");
    let absent = if cfg!(windows) {
        format!(r"\\.\pipe\ghostlink-absent-{}", rand::random::<u64>())
    } else {
        fixture
            .directory
            .path()
            .join("absent-agent.sock")
            .to_string_lossy()
            .into_owned()
    };
    let text = fs::read_to_string(&config)
        .unwrap()
        .replace("IdentityAgent none", &format!("IdentityAgent \"{absent}\""));
    fs::write(config, text).unwrap();
    let session = Session::ssh(profile, GridSize::default()).unwrap();
    wait(&session, |u| u.state == State::Running);
}
#[cfg(unix)]
#[test]
fn automatic_authenticates_with_a_private_unix_agent_without_key_files() {
    use std::{
        os::unix::fs::PermissionsExt,
        process::{Child, Command, Stdio},
    };
    struct Agent(Child);
    impl Drop for Agent {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    // Keep the socket path short enough for macOS's Unix-domain socket limit.
    let directory = tempfile::Builder::new()
        .prefix("gl-agent-")
        .tempdir_in("/tmp")
        .unwrap();
    let socket = directory.path().join("agent.sock");
    let mut agent = Agent(
        Command::new("ssh-agent")
            .args(["-D", "-a"])
            .arg(&socket)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("Install OpenSSH client tools for Unix agent tests"),
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    while !socket.exists() {
        assert!(
            agent.0.try_wait().unwrap().is_none(),
            "Test SSH agent exited"
        );
        assert!(Instant::now() < deadline, "Test SSH agent did not start");
        thread::sleep(Duration::from_millis(5));
    }
    let fixture = Fixture::new();
    fixture.trust();
    let profile = fixture.automatic("");
    fixture.write_identity("id_ed25519", None);
    let key = fixture.directory.path().join(".ssh/id_ed25519");
    fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
    let result = Command::new("ssh-add")
        .arg(&key)
        .env("SSH_AUTH_SOCK", &socket)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    fs::remove_file(key).unwrap();
    let config = fixture.directory.path().join(".ssh/config");
    let text = fs::read_to_string(&config).unwrap().replace(
        "IdentityAgent none",
        &format!("IdentityAgent \"{}\"", socket.display()),
    );
    fs::write(config, text).unwrap();
    let session = Session::ssh(profile, GridSize::default()).unwrap();
    wait(&session, |u| u.state == State::Running);
    assert_eq!(fixture.authentications.load(Ordering::SeqCst), 1);
}
#[test]
fn automatic_unlocks_only_a_server_accepted_encrypted_identity() {
    let fixture = Fixture::new();
    fixture.trust();
    let profile = fixture.automatic("");
    fixture.write_identity("id_ed25519", Some("unlock me"));
    let session = Session::ssh(profile, GridSize::default()).unwrap();
    let update = wait(&session, |u| {
        matches!(u.state, State::AwaitingCredential(_))
    });
    let State::AwaitingCredential(prompt) = update.state else {
        unreachable!()
    };
    assert_eq!(prompt.label, "Key passphrase");
    assert!(prompt.secret);
    session
        .respond_credential(prompt.id, Some("unlock me".into()))
        .unwrap();
    wait(&session, |u| u.state == State::Running);
}
#[test]
fn automatic_does_not_prompt_for_unrelated_encrypted_keys() {
    let fixture = Fixture::new();
    fixture.trust();
    let profile = fixture.automatic("");
    fixture.write_identity("id_rsa", None);
    let unrelated = keys::PrivateKey::random(&mut rand::rng(), keys::Algorithm::Ed25519)
        .unwrap()
        .encrypt(&mut rand::rng(), "never ask")
        .unwrap();
    fs::write(
        fixture.directory.path().join(".ssh/id_ed25519"),
        unrelated
            .to_openssh(keys::ssh_key::LineEnding::LF)
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    let session = Session::ssh(profile, GridSize::default()).unwrap();
    wait(&session, |u| u.state == State::Running);
}
#[test]
fn automatic_rsa_identity_uses_the_negotiated_hash() {
    let fixture = Fixture::with_options(keys::Algorithm::Rsa { hash: None }, false);
    fixture.trust();
    let profile = fixture.automatic("");
    fixture.write_identity("id_rsa", None);
    let session = Session::ssh(profile, GridSize::default()).unwrap();
    wait(&session, |u| u.state == State::Running);
}
#[test]
fn automatic_password_fallback_and_cancellation() {
    let fixture = Fixture::new();
    fixture.trust();
    let session = Session::ssh(fixture.automatic(""), GridSize::default()).unwrap();
    let update = wait(&session, |u| {
        matches!(u.state, State::AwaitingCredential(_))
    });
    let State::AwaitingCredential(prompt) = update.state else {
        unreachable!()
    };
    assert_eq!(prompt.label, "Password");
    session
        .respond_credential(prompt.id, Some("test password".into()))
        .unwrap();
    wait(&session, |u| u.state == State::Running);
    let another = Session::ssh(fixture.automatic(""), GridSize::default()).unwrap();
    let update = wait(&another, |u| {
        matches!(u.state, State::AwaitingCredential(_))
    });
    let State::AwaitingCredential(prompt) = update.state else {
        unreachable!()
    };
    another.respond_credential(prompt.id, None).unwrap();
    wait(
        &another,
        |u| matches!(&u.state,State::Failed(error)if error.contains("Authentication cancelled")),
    );
}

#[test]
fn automatic_respects_batch_mode_and_authentication_preferences() {
    let fixture = Fixture::new();
    fixture.trust();
    for config in [" BatchMode yes\n", " PreferredAuthentications publickey\n"] {
        let session = Session::ssh(fixture.automatic(config), GridSize::default()).unwrap();
        wait(
            &session,
            |u| matches!(&u.state,State::Failed(error)if error.contains("No available identity was accepted")),
        );
    }
}
#[test]
fn automatic_responds_to_keyboard_interactive_without_exposing_an_auth_selector() {
    let fixture = Fixture::with_options(keys::Algorithm::Ed25519, true);
    fixture.trust();
    let session = Session::ssh(fixture.automatic(""), GridSize::default()).unwrap();
    let update = wait(&session, |u| {
        matches!(u.state, State::AwaitingCredential(_))
    });
    let State::AwaitingCredential(prompt) = update.state else {
        unreachable!()
    };
    assert_eq!(prompt.label, "Verification code");
    session
        .respond_credential(prompt.id, Some("123456".into()))
        .unwrap();
    wait(&session, |u| u.state == State::Running);
}
#[test]
fn destination_accepts_aliases_users_and_ipv6_and_rejects_bad_ports() {
    for (destination, host, user, port) in [
        ("prod", "prod", "", 0),
        ("deploy@host:2222", "host", "deploy", 2222),
        ("ssh://user@[::1]:2200", "::1", "user", 2200),
        ("2001:db8::0", "2001:db8::0", "", 0),
    ] {
        let profile = SshProfile::from_destination(destination).unwrap();
        assert_eq!(profile.host, host);
        assert_eq!(profile.username, user);
        assert_eq!(profile.port, port);
    }
    for invalid in [
        "host:0",
        "host:00",
        "host:65536",
        "[::1]:0",
        "host:abc",
        "host\nsecond",
    ] {
        assert!(SshProfile::from_destination(invalid).is_err(), "{invalid}");
    }
}
#[test]
fn trust_precedes_authentication_and_remote_io_uses_terminal_modes() {
    let fixture = Fixture::new();
    let profile = fixture.profile();
    let store = profile.known_hosts.clone();
    let session = Session::ssh(profile, GridSize::default()).unwrap();
    let update = wait(&session, |u| matches!(u.state, State::AwaitingHostKey(_)));
    assert_eq!(fixture.authentications.load(Ordering::SeqCst), 0);
    assert!(!store.exists());
    let State::AwaitingHostKey(prompt) = update.state else {
        unreachable!()
    };
    assert_eq!(
        prompt.fingerprint,
        fixture
            .key
            .public_key()
            .fingerprint(HashAlg::Sha256)
            .to_string()
    );
    session.verify_host(prompt.fingerprint, true).unwrap();
    wait(&session, |u| {
        u.screen
            .as_ref()
            .is_some_and(|s| s.text().contains("STDERR READY"))
    });
    assert!(store.exists());
    fixture.observed(|e| matches!(e, Observed::Pty(100, 30)));
    fixture.observed(|e| matches!(e,Observed::Bytes(b) if b==b"\x1b[2;1R"));
    session
        .send(Input::Resize(GridSize::new(92, 28).unwrap()))
        .unwrap();
    fixture.observed(|e| matches!(e, Observed::Resize(92, 28)));
    session
        .send(Input::Key(Key::Up, Modifiers::default()))
        .unwrap();
    fixture.observed(|e| matches!(e,Observed::Bytes(b) if b==b"\x1bOA"));
    session.send(Input::Text("roundtrip\r".into())).unwrap();
    wait(&session, |u| {
        u.screen
            .as_ref()
            .is_some_and(|s| s.text().contains("ECHO ROUNDTRIP"))
    });
    session.send(Input::Text("bulk\r".into())).unwrap();
    wait(&session, |u| {
        u.screen
            .as_ref()
            .is_some_and(|s| s.text().contains("remote-line-4999"))
    });
    session.send(Input::Text("finish\r".into())).unwrap();
    let update = wait(&session, |u| u.state == State::Exited(23));
    let screen = update.screen.unwrap();
    assert_eq!(screen.size, GridSize::new(92, 28).unwrap());
    assert!(screen.text().contains("FINAL OUTPUT"));
    let row = screen
        .text()
        .lines()
        .position(|line| line == "FINAL OUTPUT")
        .unwrap() as f64;
    session
        .send(Input::Select(SelectionInput::Begin {
            x: 1.5,
            y: row + 0.5,
            clicks: 3,
        }))
        .unwrap();
    session
        .send(Input::Select(SelectionInput::End {
            x: 1.5,
            y: row + 0.5,
        }))
        .unwrap();
    assert_eq!(
        crate::copied_selection(&session).as_deref(),
        Some("FINAL OUTPUT")
    );
    let selected = wait(&session, |u| {
        u.screen.as_ref().is_some_and(|s| s.has_selection)
    });
    assert_eq!(selected.state, State::Exited(23));
}
#[test]
fn rejected_unknown_key_never_authenticates_or_writes_trust() {
    let fixture = Fixture::new();
    let profile = fixture.profile();
    let store = profile.known_hosts.clone();
    let session = Session::ssh(profile, GridSize::default()).unwrap();
    let update = wait(&session, |u| matches!(u.state, State::AwaitingHostKey(_)));
    let State::AwaitingHostKey(prompt) = update.state else {
        unreachable!()
    };
    session.verify_host(prompt.fingerprint, false).unwrap();
    wait(
        &session,
        |u| matches!(&u.state,State::Failed(error) if error.contains("not trusted")),
    );
    assert!(!store.exists());
    assert_eq!(fixture.authentications.load(Ordering::SeqCst), 0);
}
#[test]
fn changed_host_key_blocks_before_authentication() {
    let fixture = Fixture::new();
    let profile = fixture.profile();
    remember_host(
        &profile.known_hosts,
        &profile.host,
        profile.port,
        fixture.user_key.public_key(),
    )
    .unwrap();
    let session = Session::ssh(profile, GridSize::default()).unwrap();
    wait(
        &session,
        |u| matches!(&u.state,State::Failed(error) if error.contains("HOST KEY CHANGED")),
    );
    assert_eq!(fixture.authentications.load(Ordering::SeqCst), 0);
}
#[test]
fn known_key_and_encrypted_private_key_authenticate() {
    let fixture = Fixture::new();
    fixture.trust();
    let mut profile = fixture.profile();
    let path = fixture.directory.path().join("id_ed25519");
    let encrypted = fixture
        .user_key
        .encrypt(&mut rand::rng(), "key passphrase")
        .unwrap();
    fs::write(
        &path,
        encrypted
            .to_openssh(keys::ssh_key::LineEnding::LF)
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    profile.auth = SshAuth::PrivateKey {
        path,
        passphrase: Zeroizing::new("key passphrase".into()),
    };
    let session = Session::ssh(profile, GridSize::default()).unwrap();
    wait(&session, |u| u.state == State::Running);
    assert_eq!(fixture.authentications.load(Ordering::SeqCst), 1);
}
#[test]
fn failed_authentication_is_explicit_and_debug_redacts_secrets() {
    let fixture = Fixture::new();
    fixture.trust();
    let mut profile = fixture.profile();
    profile.auth = SshAuth::Password(Zeroizing::new("wrong secret".into()));
    assert!(!format!("{profile:?}").contains("wrong secret"));
    let session = Session::ssh(profile, GridSize::default()).unwrap();
    wait(
        &session,
        |u| matches!(&u.state,State::Failed(error) if error.contains("authentication rejected")),
    );
}
#[test]
fn cancellation_interrupts_a_pending_trust_prompt() {
    let fixture = Fixture::new();
    let session = Session::ssh(fixture.profile(), GridSize::default()).unwrap();
    wait(&session, |u| matches!(u.state, State::AwaitingHostKey(_)));
    let finished = session.finished.clone();
    drop(session);
    let deadline = Instant::now() + Duration::from_secs(3);
    while !finished.is_closed() {
        assert!(Instant::now() < deadline, "SSH did not cancel");
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(fixture.authentications.load(Ordering::SeqCst), 0);
}
#[test]
fn malformed_or_unreadable_host_store_fails_closed_and_profiles_reject_injection() {
    let fixture = Fixture::new();
    let mut profile = fixture.profile();
    fs::write(&profile.known_hosts, "invalid entry\n").unwrap();
    assert!(
        check_host(
            &profile.known_hosts,
            &profile.host,
            profile.port,
            fixture.key.public_key()
        )
        .is_err()
    );
    profile.host = "example.org\nother-host".into();
    assert!(profile.validate().is_err());
    profile.host = "https://example.org".into();
    assert!(profile.validate().is_err());
}

#[cfg(windows)]
#[tokio::test(flavor = "current_thread")]
async fn missing_windows_agent_explains_automatic_identity_discovery() {
    let pipe = format!(
        r"\\.\pipe\ghostlink-missing-agent-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    );
    let Err(error) = connect_windows_agent(std::ffi::OsStr::new(&pipe)).await else {
        panic!("Test pipe unexpectedly exists");
    };
    let message = format!("{error:#}");
    assert!(message.contains("agent is unavailable"), "{message}");
    assert!(
        message.contains("Automatic identity discovery"),
        "{message}"
    );
    assert!(message.contains("without an agent"), "{message}");
}

#[test]
fn rejected_keys_explain_the_implicit_local_username_before_password_fallback() {
    let fixture = Fixture::new();
    fixture.trust();
    let profile = fixture.automatic("");
    fixture.write_identity("id_ed25519", None);
    // No User line: the server only knows "tester", so the local login is rejected.
    fs::write(
        fixture.directory.path().join(".ssh/config"),
        format!(
            "Host fixture\n HostName 127.0.0.1\n Port {}\n IdentityAgent none\n",
            fixture.port
        ),
    )
    .unwrap();
    let session = Session::ssh(profile, GridSize::default()).unwrap();
    let update = wait(&session, |u| {
        matches!(u.state, State::AwaitingCredential(_))
    });
    let State::AwaitingCredential(prompt) = update.state else {
        unreachable!()
    };
    assert_eq!(prompt.label, "Password");
    assert!(
        prompt
            .detail
            .contains("did not accept your SSH key (id_ed25519)"),
        "{}",
        prompt.detail
    );
    assert!(
        prompt.detail.contains("signed in as your local username"),
        "{}",
        prompt.detail
    );
    assert!(
        prompt.detail.contains("user@127.0.0.1"),
        "{}",
        prompt.detail
    );
    session
        .respond_credential(prompt.id, Some("wrong".into()))
        .unwrap();
    // Whether the server allows another try or ends authentication, the account
    // name and the reason stay visible.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        assert!(
            Instant::now() < deadline,
            "Timed out waiting for SSH session"
        );
        let Ok(update) = session.updates.try_recv() else {
            thread::sleep(Duration::from_millis(5));
            continue;
        };
        match update.state {
            State::AwaitingCredential(retry) => {
                assert!(
                    retry.detail.contains("was not accepted for"),
                    "{}",
                    retry.detail
                );
                assert!(retry.detail.contains("local username"), "{}", retry.detail);
                session.respond_credential(retry.id, None).unwrap();
                break;
            }
            State::Failed(error) => {
                assert!(error.contains("local username"), "{error}");
                break;
            }
            _ => {}
        }
    }
}
#[test]
fn explicit_user_with_rejected_key_does_not_blame_the_username() {
    let fixture = Fixture::new();
    fixture.trust();
    let profile = fixture.automatic("");
    let session = Session::ssh(profile, GridSize::default()).unwrap();
    let update = wait(&session, |u| {
        matches!(u.state, State::AwaitingCredential(_))
    });
    let State::AwaitingCredential(prompt) = update.state else {
        unreachable!()
    };
    assert!(
        !prompt.detail.contains("local username"),
        "{}",
        prompt.detail
    );
    session.respond_credential(prompt.id, None).unwrap();
}

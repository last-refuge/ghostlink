//! Session workers and bounded transport queues.

#[cfg(all(test, windows))]
mod full_screen;
#[cfg(test)]
mod probe;
mod ssh;
pub use ssh::{CredentialPrompt, HostKeyPrompt, SshAuth, SshConnection, SshProfile};

use anyhow::{Context as _, Result, bail, ensure};
use async_channel::{Receiver, Sender};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::{
    collections::VecDeque,
    io::{Read, Write},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use terminal_core::{GridSize, Key, Modifiers, MouseInput, SelectionInput, Snapshot, Terminal};

#[derive(Clone, Debug)]
pub struct LocalProfile {
    pub program: String,
    pub args: Vec<String>,
    pub directory: Option<PathBuf>,
}
impl Default for LocalProfile {
    fn default() -> Self {
        // Windows PowerShell is present on supported Windows installations.
        // Use the absolute system path rather than searching an untrusted working directory.
        let system = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:/Windows"));
        Self {
            program: system
                .join("System32/WindowsPowerShell/v1.0/powershell.exe")
                .to_string_lossy()
                .into_owned(),
            args: vec!["-NoLogo".into()],
            directory: std::env::current_dir().ok(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Input {
    Text(String),
    Paste(String),
    Key(Key, Modifiers),
    Resize(GridSize),
    Scroll(i32),
    Mouse(MouseInput),
    Reconnect,
    Select(SelectionInput),
    Copy(Sender<std::result::Result<Option<String>, String>>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Starting,
    Connecting,
    Authenticating,
    AwaitingHostKey(HostKeyPrompt),
    AwaitingCredential(CredentialPrompt),
    Running,
    Exited(u32),
    Disconnected(String),
    Failed(String),
}
#[derive(Clone, Debug)]
pub struct Update {
    pub screen: Option<Arc<Snapshot>>,
    pub state: State,
}

pub struct Session {
    input: Sender<Input>,
    wake: Option<mpsc::SyncSender<Output>>,
    updates: Receiver<Update>,
    cancelled: Arc<AtomicBool>,
    finished: Receiver<()>,
    host_decisions: Option<Sender<ssh::HostKeyDecision>>,
    credentials: Option<Sender<ssh::CredentialReply>>,
}

#[derive(Clone)]
pub struct ShutdownSignal {
    cancelled: Arc<AtomicBool>,
    finished: Receiver<()>,
    input: Sender<Input>,
}
impl ShutdownSignal {
    /// Cooperate with GPUI's asynchronous quit hook so workers can terminate
    /// their shells before the application's process exits.
    pub async fn shutdown(self) {
        self.cancelled.store(true, Ordering::Release);
        self.input.close();
        let _ = self.finished.recv().await;
    }
}
impl Session {
    pub fn local(profile: LocalProfile, size: GridSize) -> Result<Self> {
        let (input, input_rx) = async_channel::bounded(64);
        let (output_tx, output) = mpsc::sync_channel(32);
        let events = LocalEvents {
            input: input_rx,
            output_tx: output_tx.clone(),
            output,
        };
        let readonly_input = events.input.clone();
        let (updates_tx, updates) = async_channel::bounded(1);
        let (finished_tx, finished) = async_channel::bounded::<()>(1);
        let stale_updates = updates.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        thread::Builder::new()
            .name("terminal-session".into())
            .spawn(move || {
                let mut transport = Some((profile, events));
                terminal_worker(
                    size,
                    readonly_input,
                    &updates_tx,
                    &stale_updates,
                    worker_cancelled.clone(),
                    false,
                    |terminal, last_screen| {
                        let (profile, events) =
                            transport.take().expect("local transport starts once");
                        run_local(
                            profile,
                            size,
                            events,
                            &updates_tx,
                            &stale_updates,
                            worker_cancelled.clone(),
                            last_screen,
                            terminal,
                        )
                    },
                );
                drop(finished_tx);
            })
            .context("Starting terminal worker")?;
        Ok(Self {
            input,
            wake: Some(output_tx),
            updates,
            cancelled,
            finished,
            host_decisions: None,
            credentials: None,
        })
    }
    pub fn updates(&self) -> Receiver<Update> {
        self.updates.clone()
    }
    pub fn ssh(profile: SshProfile, size: GridSize) -> Result<Self> {
        Self::ssh_in(SshConnection::new(profile)?, size)
    }
    /// Open an independent PTY on an existing authenticated SSH connection.
    pub fn ssh_in(connection: SshConnection, size: GridSize) -> Result<Self> {
        let (input, input_rx) = async_channel::bounded(64);
        let readonly_input = input_rx.clone();
        let (updates_tx, updates) = async_channel::bounded(1);
        let (finished_tx, finished) = async_channel::bounded::<()>(1);
        let (host_decisions, decisions) = async_channel::bounded(1);
        let (credentials, replies) = async_channel::bounded(1);
        let stale = updates.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        thread::Builder::new()
            .name("ssh-session".into())
            .spawn(move || {
                let mut reconnecting = false;
                terminal_worker(
                    size,
                    readonly_input,
                    &updates_tx,
                    &stale,
                    worker_cancelled.clone(),
                    true,
                    |terminal, last_screen| {
                        let reconnect = reconnecting;
                        reconnecting = true;
                        if reconnect {
                            while decisions.try_recv().is_ok() {}
                            while replies.try_recv().is_ok() {}
                        }
                        ssh::run(
                            connection.clone(),
                            terminal.size(),
                            input_rx.clone(),
                            &updates_tx,
                            &stale,
                            decisions.clone(),
                            replies.clone(),
                            worker_cancelled.clone(),
                            last_screen,
                            terminal,
                            reconnect,
                        )
                    },
                );
                drop(finished_tx);
            })
            .context("Starting SSH worker")?;
        Ok(Self {
            input,
            wake: None,
            updates,
            cancelled,
            finished,
            host_decisions: Some(host_decisions),
            credentials: Some(credentials),
        })
    }
    pub fn verify_host(&self, fingerprint: String, accept: bool) -> Result<()> {
        self.host_decisions
            .as_ref()
            .context("This is not an SSH session")?
            .try_send(ssh::HostKeyDecision {
                fingerprint,
                accept,
            })
            .map_err(|_| anyhow::anyhow!("Host-key decision was not accepted"))
    }
    pub fn respond_credential(&self, id: u64, value: Option<String>) -> Result<()> {
        self.credentials
            .as_ref()
            .context("This is not an SSH session")?
            .try_send(ssh::CredentialReply {
                id,
                value: value.map(zeroize::Zeroizing::new),
            })
            .map_err(|_| anyhow::anyhow!("Authentication response was not accepted"))
    }
    pub fn shutdown_signal(&self) -> ShutdownSignal {
        ShutdownSignal {
            cancelled: self.cancelled.clone(),
            finished: self.finished.clone(),
            input: self.input.clone(),
        }
    }
    pub fn copy_selection(&self) -> Result<Receiver<std::result::Result<Option<String>, String>>> {
        let (tx, rx) = async_channel::bounded(1);
        self.send(Input::Copy(tx))?;
        Ok(rx)
    }
    pub fn reconnect(&self) -> Result<()> {
        ensure!(
            self.host_decisions.is_some(),
            "Reconnect is only available for SSH sessions"
        );
        self.send(Input::Reconnect)
    }
    pub fn send(&self, input: Input) -> Result<()> {
        if let Input::Text(text) | Input::Paste(text) = &input {
            ensure!(text.len() <= 1024 * 1024, "Input exceeds the 1 MiB limit");
        }
        self.input
            .try_send(input)
            .map_err(|error| anyhow::anyhow!("Terminal input was not accepted: {error}"))?;
        if let Some(wake) = &self.wake {
            // A full queue already has work ready for the actor. Coalesce wakeups;
            // the actual input remains in its separate, lossless bounded queue.
            let _ = wake.try_send(Output::Wake);
        }
        Ok(())
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        // Never join a blocking ConPTY operation on the UI thread.
        self.cancelled.store(true, Ordering::Release);
        self.input.close();
        self.updates.close();
        if let Some(wake) = &self.wake {
            let _ = wake.try_send(Output::Wake);
        }
    }
}

fn publish(tx: &Sender<Update>, stale: &Receiver<Update>, mut update: Update) {
    // Only render snapshots may be superseded. The byte stream is lossless.
    loop {
        match tx.try_send(update) {
            Ok(()) | Err(async_channel::TrySendError::Closed(_)) => return,
            Err(async_channel::TrySendError::Full(value)) => {
                update = value;
                let _ = stale.try_recv();
            }
        }
    }
}
fn pty_size(size: GridSize) -> PtySize {
    PtySize {
        cols: size.columns(),
        rows: size.rows(),
        pixel_width: 0,
        pixel_height: 0,
    }
}

enum Output {
    Wake,
    Bytes(Vec<u8>),
    Eof,
    Failed(std::io::Error),
}

struct LocalEvents {
    input: Receiver<Input>,
    output_tx: mpsc::SyncSender<Output>,
    output: mpsc::Receiver<Output>,
}

/// Ensures failure paths after spawn also terminate the shell.
struct ChildGuard(Box<dyn portable_pty::Child + Send + Sync>);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

struct MasterGuard(Option<Box<dyn portable_pty::MasterPty + Send>>);
impl MasterGuard {
    fn close(&mut self) {
        if let Some(master) = self.0.take() {
            thread::spawn(move || drop(master));
        }
    }
}
impl Drop for MasterGuard {
    fn drop(&mut self) {
        self.close();
    }
}

#[allow(clippy::too_many_arguments)]
fn run_local(
    profile: LocalProfile,
    mut size: GridSize,
    events: LocalEvents,
    updates: &Sender<Update>,
    stale: &Receiver<Update>,
    cancelled: Arc<AtomicBool>,
    last_screen: &mut Option<Arc<Snapshot>>,
    terminal: &mut Terminal,
) -> Result<State> {
    let LocalEvents {
        input,
        output_tx,
        output,
    } = events;
    publish(
        updates,
        stale,
        Update {
            screen: None,
            state: State::Starting,
        },
    );
    let pair = native_pty_system()
        .openpty(pty_size(size))
        .context("Opening ConPTY")?;
    let mut command = CommandBuilder::new(&profile.program);
    command.args(&profile.args);
    if let Some(directory) = &profile.directory {
        command.cwd(directory);
    }
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    let mut child = ChildGuard(
        pair.slave
            .spawn_command(command)
            .context("Launching local shell")?,
    );
    drop(pair.slave);
    let mut reader = pair
        .master
        .try_clone_reader()
        .context("Opening ConPTY output")?;
    let mut writer = pair.master.take_writer().context("Opening ConPTY input")?;
    let (write_tx, write_rx) = async_channel::bounded::<Vec<u8>>(64);
    let (write_error_tx, write_error) = mpsc::channel();
    thread::Builder::new()
        .name("conpty-read".into())
        .spawn(move || {
            let mut bytes = [0; 16 * 1024];
            loop {
                let event = match reader.read(&mut bytes) {
                    Ok(0) => Output::Eof,
                    Ok(count) => Output::Bytes(bytes[..count].to_vec()),
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => Output::Failed(error),
                };
                let done = !matches!(event, Output::Bytes(_));
                // If the session closes, continue draining until ClosePseudoConsole completes.
                // Stopping the output reader before closing ConPTY can deadlock its shutdown.
                let _ = output_tx.send(event);
                if done {
                    break;
                }
            }
        })?;
    thread::Builder::new()
        .name("conpty-write".into())
        .spawn(move || {
            while let Ok(bytes) = write_rx.recv_blocking() {
                if let Err(error) = writer.write_all(&bytes).and_then(|_| writer.flush()) {
                    let _ = write_error_tx.send(error);
                    break;
                }
            }
        })?;
    let mut master = MasterGuard(Some(pair.master));
    let mut pending = VecDeque::<Vec<u8>>::new();
    let mut pending_bytes = 0;
    let mut dirty = true;
    let mut viewport_scrolled = false;
    let mut last_frame = Instant::now() - Duration::from_millis(17);
    let mut ended = false;
    while !cancelled.load(Ordering::Acquire) {
        if let Ok(error) = write_error.try_recv() {
            bail!("Writing to ConPTY: {error}");
        }
        for _ in 0..64 {
            if !pending.is_empty() {
                break;
            }
            let Ok(event) = input.try_recv() else {
                break;
            };
            viewport_scrolled |=
                matches!(&event,Input::Select(SelectionInput::Extend{scroll,..}) if *scroll!=0);
            if let Some(changed) = handle_view_input(&event, terminal)? {
                dirty |= changed;
                continue;
            }
            if matches!(&event, Input::Text(_) | Input::Paste(_) | Input::Key(_, _)) {
                dirty |= terminal.clear_selection()?;
                terminal.scroll(i32::MAX);
                dirty |= viewport_scrolled;
                viewport_scrolled = false;
            }
            let bytes = match event {
                Input::Text(text) => text.into_bytes(),
                Input::Paste(text) => terminal.encode_paste(&text),
                Input::Key(key, modifiers) => terminal.encode_key(key, modifiers)?,
                Input::Mouse(event) => terminal.encode_mouse(event)?,
                Input::Resize(new_size) => {
                    if new_size != size {
                        // Resize the parser first so output from the newly sized PTY is interpreted correctly.
                        terminal.resize(new_size)?;
                        master
                            .0
                            .as_ref()
                            .context("Session has exited")?
                            .resize(pty_size(new_size))?;
                        size = new_size;
                        dirty = true;
                    }
                    continue;
                }
                Input::Scroll(delta) => {
                    terminal.scroll(delta);
                    viewport_scrolled = true;
                    dirty = true;
                    continue;
                }
                Input::Select(_) | Input::Copy(_) | Input::Reconnect => continue,
            };
            if !bytes.is_empty() {
                pending_bytes += bytes.len();
                pending.push_back(bytes);
            }
        }
        // Keep parsing output even when the child's input pipe is backpressured.
        // Blocking this actor on writes can deadlock a child that is also writing.
        while let Some(bytes) = pending.pop_front() {
            let len = bytes.len();
            match write_tx.try_send(bytes) {
                Ok(()) => pending_bytes -= len,
                Err(async_channel::TrySendError::Full(bytes)) => {
                    pending.push_front(bytes);
                    break;
                }
                Err(async_channel::TrySendError::Closed(_)) => bail!("ConPTY writer stopped"),
            }
        }
        let timeout = if !input.is_empty() && pending.is_empty() {
            // Wakeups may be coalesced when their queue is full. Drain remaining
            // accepted input before blocking, even when the child does not echo.
            Duration::ZERO
        } else if dirty {
            Duration::from_millis(16).saturating_sub(last_frame.elapsed())
        } else {
            Duration::from_millis(50)
        };
        match output.recv_timeout(timeout) {
            Ok(Output::Wake) => {}
            Ok(Output::Bytes(bytes)) => {
                let replies = terminal.feed(&bytes);
                if !replies.is_empty() {
                    pending_bytes += replies.len();
                    pending.push_back(replies);
                }
                dirty = true;
            }
            Ok(Output::Eof) => ended = true,
            Ok(Output::Failed(error)) => {
                if child.0.try_wait()?.is_some() {
                    ended = true;
                } else {
                    return Err(error).context("Reading ConPTY output");
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => ended = true,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        ensure!(
            pending_bytes <= 2 * 1024 * 1024,
            "ConPTY input backlog exceeded 2 MiB; the child is not reading input"
        );
        if dirty && (last_frame.elapsed() >= Duration::from_millis(16) || ended) {
            let screen = Arc::new(terminal.snapshot()?);
            *last_screen = Some(screen.clone());
            publish(
                updates,
                stale,
                Update {
                    screen: Some(screen),
                    state: State::Running,
                },
            );
            last_frame = Instant::now();
            dirty = false;
        }
        if ended {
            break;
        }
        // Closing ConPTY may block until output drains. Close it on another worker
        // while this worker continues consuming all remaining output, through EOF.
        if child.0.try_wait()?.is_some() {
            master.close();
        }
    }
    let status = child
        .0
        .try_wait()?
        .map(|status| status.exit_code())
        .unwrap_or(0);
    let _ = child.0.kill();
    // Unblock a producer before dropping the pseudoconsole. The read worker stays alive
    // and drains the remaining pipe bytes, even after its receiver has been dropped.
    drop(output);
    drop(write_tx);
    drop(master);
    Ok(State::Exited(status))
}

fn handle_view_input(input: &Input, terminal: &mut Terminal) -> Result<Option<bool>> {
    match input {
        Input::Select(event) => {
            terminal.select(*event)?;
            Ok(Some(true))
        }
        Input::Copy(reply) => {
            let _ = reply.try_send(terminal.copy_selection().map_err(|error| error.to_string()));
            Ok(Some(false))
        }
        Input::Reconnect => Ok(Some(false)),
        _ => Ok(None),
    }
}

fn terminal_worker(
    size: GridSize,
    input: Receiver<Input>,
    updates: &Sender<Update>,
    stale: &Receiver<Update>,
    cancelled: Arc<AtomicBool>,
    reconnectable: bool,
    mut run: impl FnMut(&mut Terminal, &mut Option<Arc<Snapshot>>) -> Result<State>,
) {
    let mut terminal = match Terminal::new(size) {
        Ok(terminal) => terminal,
        Err(error) => {
            publish(
                updates,
                stale,
                Update {
                    screen: None,
                    state: State::Failed(format!("{error:#}")),
                },
            );
            return;
        }
    };
    let mut last_screen = None;
    'connection: loop {
        let mut state = match run(&mut terminal, &mut last_screen) {
            Ok(state) => state,
            Err(error) => State::Failed(format!("{error:#}")),
        };
        if let Ok(snapshot) = terminal.snapshot() {
            last_screen = Some(Arc::new(snapshot));
        }
        publish(
            updates,
            stale,
            Update {
                screen: last_screen.clone(),
                state: state.clone(),
            },
        );
        // Keep the owning parser alive for selection, scrolling, and copying after
        // exit/disconnection. Closing the input channel wakes this blocking receive.
        while !cancelled.load(Ordering::Acquire) {
            let Ok(event) = input.recv_blocking() else {
                break;
            };
            if matches!(event, Input::Reconnect)
                && reconnectable
                && !cancelled.load(Ordering::Acquire)
            {
                continue 'connection;
            }
            let changed = match &event {
                Input::Scroll(delta) => {
                    terminal.scroll(*delta);
                    Ok(true)
                }
                Input::Resize(size) => terminal.resize(*size).map(|_| true),
                _ => {
                    handle_view_input(&event, &mut terminal).map(|changed| changed.unwrap_or(false))
                }
            };
            match changed {
                Ok(false) => continue,
                Ok(true) => {}
                Err(error) => state = State::Failed(format!("{error:#}")),
            }
            if let Ok(snapshot) = terminal.snapshot() {
                last_screen = Some(Arc::new(snapshot));
            }
            publish(
                updates,
                stale,
                Update {
                    screen: last_screen.clone(),
                    state: state.clone(),
                },
            );
        }
        break;
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    fn wait_for(session: &Session, mut predicate: impl FnMut(&Update) -> bool) -> Update {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok(update) = session.updates.try_recv() {
                if let State::Failed(error) = &update.state {
                    panic!("Session failed: {error}");
                }
                if predicate(&update) {
                    return update;
                }
            }
            assert!(Instant::now() < deadline, "Timed out waiting for ConPTY");
            thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    #[ignore = "manual ConPTY input-to-snapshot latency probe"]
    fn local_input_latency_probe() {
        let profile = LocalProfile {
            args: vec!["-NoLogo".into(), "-NoProfile".into(), "-Command".into(),
                "[Console]::Write('LATENCY READY'); 1..32 | ForEach-Object { $key=[Console]::ReadKey($true); [Console]::Write(\"`r`nACK $_\") }; Start-Sleep -Seconds 1".into()],
            ..LocalProfile::default()
        };
        let session = Session::local(profile, GridSize::default()).unwrap();
        wait_for(&session, |u| {
            u.screen
                .as_ref()
                .is_some_and(|s| s.text().contains("LATENCY READY"))
        });
        crate::probe::measure(
            "ConPTY",
            &session,
            (1..=32).map(|index| (Input::Text("x".into()), format!("ACK {index}"))),
        );
    }
    #[test]
    fn powershell_output_drains_before_exit() {
        let profile = LocalProfile { args: vec!["-NoLogo".into(), "-NoProfile".into(), "-NonInteractive".into(), "-Command".into(),
            "0..4999 | ForEach-Object { Write-Output ('line-' + $_) }; Write-Output 'GHOSTLINK_DONE'; exit 7".into()], ..LocalProfile::default() };
        let session = Session::local(profile, GridSize::new(80, 24).unwrap()).unwrap();
        let update = wait_for(&session, |update| matches!(update.state, State::Exited(_)));
        assert_eq!(
            update.state,
            State::Exited(7),
            "{}",
            update
                .screen
                .as_ref()
                .map_or_else(String::new, |screen| screen.text())
        );
        let text = update.screen.unwrap().text();
        assert!(text.contains("line-4999"), "{text}");
        assert!(text.contains("GHOSTLINK_DONE"), "{text}");
    }
    #[test]
    fn powershell_accepts_input_and_resize() {
        let profile = LocalProfile {
            args: vec!["-NoLogo".into(), "-NoProfile".into()],
            ..LocalProfile::default()
        };
        let session = Session::local(profile, GridSize::new(80, 24).unwrap()).unwrap();
        wait_for(&session, |update| {
            update
                .screen
                .as_ref()
                .is_some_and(|screen| screen.text().contains("PS "))
        });
        session
            .send(Input::Resize(GridSize::new(92, 28).unwrap()))
            .unwrap();
        // Construct the marker so it cannot be matched just from the echoed command.
        session
            .send(Input::Text(
                "Write-Output ('ROUND' + 'TRIP'); exit 0\r".into(),
            ))
            .unwrap();
        let update = wait_for(&session, |update| matches!(update.state, State::Exited(_)));
        let screen = update.screen.unwrap();
        assert_eq!(screen.size, GridSize::new(92, 28).unwrap());
        assert!(screen.text().contains("ROUNDTRIP"), "{}", screen.text());
    }
    #[test]
    fn selection_and_copy_survive_exit_and_shutdown_wakes_the_retained_worker() {
        let profile = LocalProfile {
            args: vec![
                "-NoLogo".into(),
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                "[Console]::Write(\"alpha beta`r`nsecond`r`n\"); exit 0".into(),
            ],
            ..LocalProfile::default()
        };
        let session = Session::local(profile, GridSize::new(40, 6).unwrap()).unwrap();
        let update = wait_for(&session, |u| u.state == State::Exited(0));
        let row = update
            .screen
            .unwrap()
            .text()
            .lines()
            .position(|line| line == "alpha beta")
            .unwrap() as f64;
        assert_eq!(copied_selection(&session), None);
        session
            .send(Input::Select(SelectionInput::Begin {
                x: 1.5,
                y: row + 0.5,
                clicks: 2,
            }))
            .unwrap();
        session
            .send(Input::Select(SelectionInput::End {
                x: 1.5,
                y: row + 0.5,
            }))
            .unwrap();
        assert_eq!(copied_selection(&session).as_deref(), Some("alpha"));
        let selected = wait_for(&session, |u| {
            u.screen.as_ref().is_some_and(|s| s.has_selection)
        });
        assert_eq!(selected.state, State::Exited(0));
        assert_eq!(selected.screen.unwrap().selection[row as usize], Some(0..5));
        session.send(Input::Select(SelectionInput::Clear)).unwrap();
        assert_eq!(copied_selection(&session), None);
        let shutdown = session.shutdown_signal();
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(async {
                tokio::time::timeout(Duration::from_secs(3), shutdown.shutdown())
                    .await
                    .expect("Retained parser did not stop on app quit");
            });
        assert!(session.finished.is_closed());
    }
    #[test]
    fn spawn_failure_is_reported() {
        let profile = LocalProfile {
            program: "C:/ghostlink-missing-shell/does-not-exist.exe".into(),
            ..LocalProfile::default()
        };
        let session = Session::local(profile, GridSize::default()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(update) = session.updates.try_recv()
                && matches!(update.state, State::Failed(_))
            {
                break;
            }
            assert!(Instant::now() < deadline, "Spawn error was not reported");
            thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    fn dropping_session_closes_update_channel() {
        let profile = LocalProfile {
            args: vec!["-NoLogo".into(), "-NoProfile".into()],
            ..LocalProfile::default()
        };
        let session = Session::local(profile, GridSize::default()).unwrap();
        wait_for(&session, |update| update.state == State::Running);
        let updates = session.updates();
        drop(session);
        let deadline = Instant::now() + Duration::from_secs(10);
        while !updates.is_closed() {
            let _ = updates.try_recv();
            assert!(
                Instant::now() < deadline,
                "Session worker did not shut down"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
fn copied_selection(session: &Session) -> Option<String> {
    let response = session.copy_selection().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Ok(result) = response.try_recv() {
            return result.unwrap();
        }
        assert!(
            Instant::now() < deadline,
            "Timed out copying terminal selection"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

use super::*;
use russh::{
    ChannelMsg, client,
    keys::{self, HashAlg, PublicKey, PublicKeyOrCertificate},
};
use std::{
    fmt,
    fs::{self, OpenOptions},
    io::Write as _,
    path::Path,
    sync::Mutex,
};
use zeroize::Zeroizing;

mod automatic;
mod config;

/// One authenticated transport, shared by a sidebar session's tabs and panes.
#[derive(Clone)]
pub struct SshConnection(Arc<ConnectionInner>);
struct ConnectionInner {
    profile: SshProfile,
    runtime: Option<tokio::runtime::Runtime>,
    authenticated: tokio::sync::OnceCell<Arc<client::Handle<Verifier>>>,
}
impl SshConnection {
    pub fn new(profile: SshProfile) -> Result<Self> {
        profile.validate()?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("ssh-transport")
            .enable_all()
            .build()
            .context("Starting SSH transport")?;
        Ok(Self(Arc::new(ConnectionInner {
            profile,
            runtime: Some(runtime),
            authenticated: tokio::sync::OnceCell::new(),
        })))
    }
    pub fn profile(&self) -> &SshProfile {
        &self.0.profile
    }
}
impl Drop for ConnectionInner {
    fn drop(&mut self) {
        // Dropping the last tab/session must never join transport tasks on GPUI.
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

struct CloseChannel(Arc<russh::ChannelWriteHalf<client::Msg>>);
impl Drop for CloseChannel {
    fn drop(&mut self) {
        let writer = self.0.clone();
        tokio::spawn(async move {
            let _ = tokio::time::timeout(Duration::from_secs(2), writer.close()).await;
        });
    }
}
struct AbortWriter(tokio::task::JoinHandle<std::result::Result<(), russh::Error>>);
impl Drop for AbortWriter {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Clone)]
pub enum SshAuth {
    Automatic {
        home: PathBuf,
    },
    Password(Zeroizing<String>),
    PrivateKey {
        path: PathBuf,
        passphrase: Zeroizing<String>,
    },
    Agent,
}
impl fmt::Debug for SshAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Automatic { .. } => f.write_str("Automatic"),
            Self::Password(_) => f.write_str("Password([redacted])"),
            Self::PrivateKey { path, .. } => f
                .debug_struct("PrivateKey")
                .field("path", path)
                .finish_non_exhaustive(),
            Self::Agent => f.write_str("Agent"),
        }
    }
}
#[derive(Clone, Debug)]
pub struct SshProfile {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: SshAuth,
    pub known_hosts: PathBuf,
}
impl SshProfile {
    pub fn from_destination(destination: &str) -> Result<Self> {
        let destination = destination
            .trim()
            .strip_prefix("ssh://")
            .unwrap_or(destination.trim());
        let (username, address) = destination
            .rsplit_once('@')
            .map_or(("", destination), |(user, address)| (user, address));
        let parse_port = |value: &str| -> Result<u16> {
            let port = value
                .parse::<u16>()
                .context("Enter an SSH port between 1 and 65535")?;
            ensure!(port > 0, "SSH port must be between 1 and 65535");
            Ok(port)
        };
        let (host, port) = if let Some(address) = address.strip_prefix('[') {
            let (host, tail) = address
                .split_once(']')
                .context("Close the IPv6 address with ]")?;
            let port = if tail.is_empty() {
                0
            } else {
                parse_port(
                    tail.strip_prefix(':')
                        .context("Use host:port for a custom SSH port")?,
                )?
            };
            (host, port)
        } else if address.matches(':').count() == 1 {
            let (host, port) = address.split_once(':').unwrap();
            (host, parse_port(port)?)
        } else {
            (address, 0)
        };
        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .context("Could not find your home directory for SSH key discovery")?;
        Self::new(
            host.into(),
            port,
            username.into(),
            SshAuth::Automatic {
                home: PathBuf::from(home),
            },
        )
    }
    pub fn new(host: String, port: u16, username: String, auth: SshAuth) -> Result<Self> {
        let directory = std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is not set")?;
        let profile = Self {
            host,
            port,
            username,
            auth,
            known_hosts: PathBuf::from(directory).join("Ghostlink/known_hosts"),
        };
        profile.validate()?;
        Ok(profile)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.host.is_empty()
                && self.host.len() <= 253
                && self
                    .host
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-_:%".contains(&b)),
            "Enter a hostname or IP address without a scheme or port"
        );
        let automatic = matches!(self.auth, SshAuth::Automatic { .. });
        ensure!(
            self.port != 0 || automatic,
            "SSH port must be between 1 and 65535"
        );
        ensure!(
            (!self.username.is_empty() || automatic)
                && self.username.len() <= 256
                && !self
                    .username
                    .chars()
                    .any(|c| c.is_whitespace() || c.is_control()),
            "Enter an SSH username without spaces"
        );
        ensure!(
            !self.known_hosts.as_os_str().is_empty(),
            "A host-key store is required"
        );
        if let SshAuth::PrivateKey { path, .. } = &self.auth {
            ensure!(!path.as_os_str().is_empty(), "Choose a private-key file");
            ensure!(path.is_absolute(), "Enter an absolute private-key path");
        }
        Ok(())
    }
    pub fn label(&self) -> String {
        let user = if self.username.is_empty() {
            String::new()
        } else {
            format!("{}@", self.username)
        };
        let port = if self.port == 0 || self.port == 22 {
            String::new()
        } else {
            format!(":{}", self.port)
        };
        format!("{user}{}{port}", self.host)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostKeyPrompt {
    pub host: String,
    pub algorithm: String,
    pub fingerprint: String,
}
pub(crate) struct HostKeyDecision {
    pub fingerprint: String,
    pub accept: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialPrompt {
    pub id: u64,
    pub label: String,
    pub detail: String,
    pub secret: bool,
}
pub(crate) struct CredentialReply {
    pub id: u64,
    pub value: Option<Zeroizing<String>>,
}

static HOST_STORE_LOCK: Mutex<()> = Mutex::new(());

fn endpoint(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_ascii_lowercase()
    } else {
        format!("[{}]:{port}", host.to_ascii_lowercase())
    }
}
fn check_host(path: &Path, host: &str, port: u16, key: &PublicKey) -> Result<bool> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error).context("Reading Ghostlink's host-key store"),
    };
    let expected = endpoint(host, port);
    let mut found = false;
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields = line.split_whitespace().collect::<Vec<_>>();
        ensure!(
            fields.len() >= 3,
            "Invalid host-key store entry on line {}",
            index + 1
        );
        let recorded = PublicKey::from_openssh(&format!("{} {}", fields[1], fields[2]))
            .with_context(|| format!("Invalid host key on line {}", index + 1))?;
        if fields[0] == expected {
            ensure!(
                recorded.key_data() == key.key_data(),
                "HOST KEY CHANGED for {expected}. Connection blocked. Verify the new key with the server administrator before editing {}",
                path.display()
            );
            found = true;
        }
    }
    Ok(found)
}
fn remember_host(path: &Path, host: &str, port: u16, key: &PublicKey) -> Result<()> {
    let _guard = HOST_STORE_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("Host-key store lock failed"))?;
    // Recheck under the lock: another pane may have trusted a key while this prompt was open.
    if check_host(path, host, port, key)? {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("Creating host-key store directory")?;
    }
    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .context("Opening host-key store for writing")?;
    writeln!(file, "\n{} {}", endpoint(host, port), key.to_openssh()?)?;
    file.sync_all().context("Saving trusted host key")?;
    Ok(())
}

struct Verifier {
    host: String,
    port: u16,
    known_hosts: PathBuf,
    updates: Sender<Update>,
    stale: Receiver<Update>,
    decisions: Receiver<HostKeyDecision>,
}
impl client::Handler for Verifier {
    type Error = anyhow::Error;
    async fn check_server_key(&mut self, presented: &PublicKeyOrCertificate) -> Result<bool> {
        ensure!(
            presented.certificate().is_none(),
            "Host certificates are not supported yet; use a server with a verifiable plain host key"
        );
        let key = presented.public_key();
        let known = {
            let _guard = HOST_STORE_LOCK
                .lock()
                .map_err(|_| anyhow::anyhow!("Host-key store lock failed"))?;
            check_host(&self.known_hosts, &self.host, self.port, &key)?
        };
        if known {
            return Ok(true);
        }
        let prompt = HostKeyPrompt {
            host: endpoint(&self.host, self.port),
            algorithm: key.algorithm().to_string(),
            fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
        };
        publish(
            &self.updates,
            &self.stale,
            Update {
                screen: None,
                state: State::AwaitingHostKey(prompt.clone()),
            },
        );
        let decision = self
            .decisions
            .recv()
            .await
            .context("Host-key verification cancelled")?;
        ensure!(
            decision.fingerprint == prompt.fingerprint && decision.accept,
            "Host key was not trusted; connection cancelled"
        );
        remember_host(&self.known_hosts, &self.host, self.port, &key)?;
        Ok(true)
    }
}

enum Outgoing {
    Bytes(Vec<u8>),
    Resize(GridSize),
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
    connection: SshConnection,
    size: GridSize,
    input: Receiver<Input>,
    updates: &Sender<Update>,
    stale: &Receiver<Update>,
    decisions: Receiver<HostKeyDecision>,
    credentials: Receiver<CredentialReply>,
    cancelled: Arc<AtomicBool>,
    last_screen: &mut Option<Arc<Snapshot>>,
    terminal: &mut Terminal,
) -> Result<State> {
    let runtime = connection.0.runtime.as_ref().expect("live SSH runtime");
    runtime.block_on(async {
        let cancellation = async {
            while !cancelled.load(Ordering::Acquire) { tokio::time::sleep(Duration::from_millis(10)).await; }
        };
        tokio::select! {
            result = connect(&connection, size, input, updates, stale, decisions, credentials, last_screen,terminal) => result,
            _ = cancellation => Ok(State::Exited(0)),
        }
    })
}

async fn authenticate(
    session: &mut client::Handle<Verifier>,
    username: &str,
    auth: SshAuth,
) -> Result<()> {
    let success = match auth {
        SshAuth::Automatic { .. } => bail!("Automatic credentials were not resolved"),
        SshAuth::Password(password) => session
            .authenticate_password(username, password.as_str())
            .await?
            .success(),
        SshAuth::PrivateKey { path, passphrase } => {
            let key = keys::load_secret_key(
                path,
                if passphrase.is_empty() {
                    None
                } else {
                    Some(passphrase.as_str())
                },
            )
            .context("Loading private key")?;
            let hash = session.best_supported_rsa_hash().await?.flatten();
            session
                .authenticate_publickey(
                    username,
                    keys::PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                )
                .await?
                .success()
        }
        SshAuth::Agent => {
            #[cfg(windows)]
            let mut agent =
                connect_windows_agent(std::ffi::OsStr::new(r"\\.\pipe\openssh-ssh-agent")).await?;
            #[cfg(unix)]
            let mut agent = keys::agent::client::AgentClient::connect_env()
                .await
                .context("Connecting to SSH_AUTH_SOCK")?;
            let identities = agent
                .request_identities()
                .await
                .context("Reading agent identities")?;
            ensure!(
                identities.iter().any(|identity| matches!(
                    identity,
                    keys::agent::AgentIdentity::PublicKey { .. }
                )),
                "The SSH agent has no supported keys. Load a key into the agent or use automatic identity discovery"
            );
            let hash = session.best_supported_rsa_hash().await?.flatten();
            let mut accepted = false;
            for identity in identities {
                if let keys::agent::AgentIdentity::PublicKey { key, .. } = identity
                    && session
                        .authenticate_publickey_with(username, key, hash, &mut agent)
                        .await?
                        .success()
                {
                    accepted = true;
                    break;
                }
            }
            accepted
        }
    };
    ensure!(
        success,
        "SSH authentication rejected. Check the username and selected credentials"
    );
    Ok(())
}

#[cfg(windows)]
async fn connect_windows_agent(
    path: &std::ffi::OsStr,
) -> Result<keys::agent::client::AgentClient<tokio::net::windows::named_pipe::NamedPipeClient>> {
    keys::agent::client::AgentClient::connect_named_pipe(path).await.map_err(|error| {
        if matches!(&error, keys::Error::IO(io) if io.kind() == std::io::ErrorKind::NotFound) {
            anyhow::anyhow!("The Windows OpenSSH agent is unavailable. Automatic identity discovery can use your key files without an agent")
        } else {
            anyhow::Error::new(error).context("Cannot access the Windows OpenSSH agent. Automatic identity discovery can use your key files without an agent")
        }
    })
}

#[derive(Default)]
struct StartupMessages {
    events: VecDeque<ChannelMsg>,
    bytes: usize,
}

async fn request_accepted(
    channel: &mut russh::ChannelReadHalf,
    name: &str,
    startup: &mut StartupMessages,
) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match channel.wait().await {
                Some(ChannelMsg::Success) => return Ok(()),
                Some(ChannelMsg::Failure) => bail!("Server refused the {name} request"),
                Some(ChannelMsg::Close) | None => {
                    bail!("SSH channel closed before the {name} request completed")
                }
                // russh has already applied the window adjustment. Neither this
                // notification nor flow-control advice is a request reply.
                Some(ChannelMsg::WindowAdjusted { .. } | ChannelMsg::XonXoff { .. }) => {}
                Some(message) => {
                    startup.bytes += match &message {
                        ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
                            data.len()
                        }
                        _ => 0,
                    };
                    ensure!(
                        startup.bytes <= 2 * 1024 * 1024 && startup.events.len() < 4096,
                        "SSH startup output exceeded its buffer limit before the {name} reply"
                    );
                    // Preserve early output and exit events for the normal read loop.
                    startup.events.push_back(message);
                }
            }
        }
    })
    .await
    .with_context(|| format!("Server did not answer the {name} request"))?
}

async fn authenticate_connection(
    mut profile: SshProfile,
    updates: &Sender<Update>,
    stale: &Receiver<Update>,
    decisions: Receiver<HostKeyDecision>,
    credentials: Receiver<CredentialReply>,
) -> Result<Arc<client::Handle<Verifier>>> {
    publish(
        updates,
        stale,
        Update {
            screen: None,
            state: State::Connecting,
        },
    );
    let automatic = if let SshAuth::Automatic { home } = &profile.auth {
        let resolved = config::resolve(&profile, home)?;
        profile.host = resolved.host.clone();
        profile.username = resolved.user.clone();
        profile.port = resolved.port;
        profile.validate()?;
        Some(resolved)
    } else {
        None
    };
    let stream = tokio::time::timeout(
        Duration::from_secs(15),
        tokio::net::TcpStream::connect((profile.host.as_str(), profile.port)),
    )
    .await
    .context("SSH connection timed out")?
    .context("Connecting to SSH server")?;
    // connect_stream accepts a pre-opened socket, so russh's Config::nodelay
    // setting does not apply here. Interactive keystrokes must leave immediately.
    stream
        .set_nodelay(true)
        .context("Configuring interactive SSH socket")?;
    let config = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        ..Default::default()
    });
    let verifier = Verifier {
        host: profile.host,
        port: profile.port,
        known_hosts: profile.known_hosts,
        updates: updates.clone(),
        stale: stale.clone(),
        decisions,
    };
    // This includes interactive first-use trust. Cancellation remains available throughout.
    let mut connection = tokio::time::timeout(
        Duration::from_secs(300),
        client::connect_stream(config, stream, verifier),
    )
    .await
    .context("SSH handshake or host-key verification timed out")?
    .context("SSH handshake failed")?;
    publish(
        updates,
        stale,
        Update {
            screen: None,
            state: State::Authenticating,
        },
    );
    if let Some(plan) = automatic {
        automatic::authenticate(
            &mut connection,
            &profile.username,
            plan,
            automatic::Prompter::new(updates, stale, credentials),
        )
        .await?;
    } else {
        tokio::time::timeout(
            Duration::from_secs(30),
            authenticate(&mut connection, &profile.username, profile.auth),
        )
        .await
        .context("SSH authentication timed out")??;
    }
    Ok(Arc::new(connection))
}

#[allow(clippy::too_many_arguments)]
async fn connect(
    shared: &SshConnection,
    mut size: GridSize,
    input: Receiver<Input>,
    updates: &Sender<Update>,
    stale: &Receiver<Update>,
    decisions: Receiver<HostKeyDecision>,
    credentials: Receiver<CredentialReply>,
    last_screen: &mut Option<Arc<Snapshot>>,
    terminal: &mut Terminal,
) -> Result<State> {
    publish(
        updates,
        stale,
        Update {
            screen: None,
            state: State::Connecting,
        },
    );
    let connection = shared
        .0
        .authenticated
        .get_or_try_init(|| {
            authenticate_connection(
                shared.0.profile.clone(),
                updates,
                stale,
                decisions,
                credentials,
            )
        })
        .await?;
    ensure!(
        !connection.is_closed(),
        "SSH connection has closed. Connect to this server again."
    );
    let channel = tokio::time::timeout(Duration::from_secs(15), connection.channel_open_session())
        .await
        .context("SSH channel open timed out")??;
    let (mut reader, writer) = channel.split();
    let writer = Arc::new(writer);
    let _close_channel = CloseChannel(writer.clone());
    let mut startup = StartupMessages::default();
    let opened = async {
        writer
            .request_pty(
                true,
                "xterm-256color",
                size.columns() as u32,
                size.rows() as u32,
                0,
                0,
                &[],
            )
            .await?;
        request_accepted(&mut reader, "terminal", &mut startup).await?;
        writer.request_shell(true).await?;
        request_accepted(&mut reader, "shell", &mut startup).await
    }
    .await;
    if let Err(error) = opened {
        // Keep any server explanation visible alongside the connection error.
        for message in startup.events {
            if let ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } = message {
                terminal.feed(&data);
            }
        }
        *last_screen = Some(Arc::new(terminal.snapshot()?));
        return Err(error);
    }
    let (write_tx, write_rx) = async_channel::bounded::<Outgoing>(64);
    let mut write_task = AbortWriter(tokio::spawn(async move {
        while let Ok(command) = write_rx.recv().await {
            match command {
                Outgoing::Bytes(bytes) => writer.data_bytes(bytes).await?,
                Outgoing::Resize(size) => {
                    writer
                        .window_change(size.columns() as u32, size.rows() as u32, 0, 0)
                        .await?
                }
            }
        }
        Ok::<_, russh::Error>(())
    }));
    let mut pending = VecDeque::<Outgoing>::new();
    let mut pending_bytes = 0usize;
    let mut dirty = true;
    let mut viewport_scrolled = false;
    let mut exit_status = None;
    let mut render_tick = tokio::time::interval(Duration::from_millis(16));
    render_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        while let Some(command) = pending.pop_front() {
            let count = match &command {
                Outgoing::Bytes(bytes) => bytes.len(),
                Outgoing::Resize(_) => 0,
            };
            match write_tx.try_send(command) {
                Ok(()) => pending_bytes -= count,
                Err(async_channel::TrySendError::Full(command)) => {
                    pending.push_front(command);
                    break;
                }
                Err(async_channel::TrySendError::Closed(_)) => bail!("SSH channel writer stopped"),
            }
        }
        ensure!(
            pending_bytes <= 2 * 1024 * 1024,
            "SSH input backlog exceeded 2 MiB; the server is not reading input"
        );
        tokio::select! {
            event = input.recv(), if pending.is_empty() && startup.events.is_empty() => {
                let Ok(event) = event else { break; };
                viewport_scrolled |= matches!(&event,Input::Select(SelectionInput::Extend{scroll,..}) if *scroll!=0);
                if let Some(changed)=handle_view_input(&event,terminal)? { dirty |= changed; continue; }
                if matches!(&event, Input::Text(_) | Input::Paste(_) | Input::Key(_, _)) {
                    dirty |= terminal.clear_selection()?;
                    terminal.scroll(i32::MAX);
                    dirty |= viewport_scrolled;
                    viewport_scrolled = false;
                }
                let bytes = match event {
                    Input::Text(text) => text.into_bytes(),
                    Input::Paste(text) => terminal.encode_paste(&text),
                    Input::Key(key, mods) => terminal.encode_key(key, mods)?,
                    Input::Scroll(delta) => { terminal.scroll(delta); viewport_scrolled = true; dirty = true; continue; },
                    Input::Resize(new_size) => {
                        if size != new_size { terminal.resize(new_size)?; size = new_size; pending.push_back(Outgoing::Resize(size)); dirty = true; }
                        continue;
                    },
                    Input::Select(_) | Input::Copy(_) => continue,
                };
                if !bytes.is_empty() { pending_bytes += bytes.len(); pending.push_back(Outgoing::Bytes(bytes)); }
            },
            event = async {
                if let Some(message) = startup.events.pop_front() {
                    Some(message)
                } else {
                    reader.wait().await
                }
            } => match event {
                Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                    let replies = terminal.feed(&data);
                    if !replies.is_empty() { pending_bytes += replies.len(); pending.push_back(Outgoing::Bytes(replies)); }
                    dirty = true;
                },
                Some(ChannelMsg::ExitStatus { exit_status: code }) => exit_status = Some(code),
                Some(ChannelMsg::ExitSignal { signal_name, .. }) => bail!("Remote shell terminated by {signal_name:?}"),
                Some(ChannelMsg::Close) | None => break,
                _ => {},
            },
            _ = render_tick.tick(), if startup.events.is_empty() => {
                if dirty {
                    let screen = Arc::new(terminal.snapshot()?);
                    *last_screen = Some(screen.clone());
                    publish(updates, stale, Update { screen: Some(screen), state: State::Running });
                    dirty = false;
                }
            },
            result = &mut write_task.0 => { result.context("SSH writer failed")??; bail!("SSH writer closed unexpectedly"); },
        }
    }
    // Keep the final bytes even when ExitStatus and Close arrive before the next frame.
    *last_screen = Some(Arc::new(terminal.snapshot()?));
    write_task.0.abort();
    let status = exit_status.context("SSH connection closed without an exit status")?;
    Ok(State::Exited(status))
}

#[cfg(test)]
mod tests;

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
mod proxy;
pub use proxy::{ProxyKind, ProxyOptions};

/// One authenticated transport, shared by a sidebar session's tabs and panes.
#[derive(Clone)]
pub struct SshConnection(Arc<ConnectionInner>);
struct ConnectionInner {
    profile: SshProfile,
    runtime: Option<tokio::runtime::Runtime>,
    authenticated: tokio::sync::Mutex<Option<Arc<AuthenticatedRoute>>>,
}
struct AuthenticatedRoute {
    connection: Arc<client::Handle<Verifier>>,
    jumps: Vec<Arc<client::Handle<Verifier>>>,
    agent_access: Arc<AtomicBool>,
}
impl AuthenticatedRoute {
    fn is_closed(&self) -> bool {
        self.connection.is_closed() || self.jumps.iter().any(|hop| hop.is_closed())
    }
}
static NEXT_PROMPT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
fn next_prompt_id() -> u64 {
    NEXT_PROMPT_ID.fetch_add(1, Ordering::Relaxed)
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
            authenticated: tokio::sync::Mutex::new(None),
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
/// Per-host overrides; secrets are held only in zeroizing memory.
#[derive(Clone, Default)]
pub struct SshOptions {
    pub identity_file: Option<PathBuf>,
    pub proxy: Option<ProxyOptions>,
    pub jump_hosts: Vec<SshProfile>,
    pub display_name: Option<String>,
    pub certificate: Option<PathBuf>,
    pub agent_forwarding: bool,
    pub agent_socket: Option<PathBuf>,
    pub environment: Vec<(String, String)>,
    pub startup_snippet: String,
    pub keepalive_secs: Option<u64>,
    pub control_h_backspace: bool,
    pub charset: Option<String>,
    pub theme: terminal_core::themes::Theme,
    pub password: Option<Zeroizing<String>>,
    pub password_only: bool,
    pub credential_notice: Option<String>,
    pub on_password_accepted: Option<Arc<dyn Fn(Zeroizing<String>) + Send + Sync>>,
}
impl fmt::Debug for SshOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SshOptions")
            .field("identity_file", &self.identity_file)
            .field("password", &self.password.as_ref().map(|_| "[redacted]"))
            .field("password_only", &self.password_only)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub struct SshProfile {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: SshAuth,
    pub known_hosts: PathBuf,
    pub options: Box<SshOptions>,
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
        let home = ghostlink_platform::home_dir()
            .context("Could not find your home directory for SSH key discovery")?;
        Self::new(
            host.into(),
            port,
            username.into(),
            SshAuth::Automatic { home },
        )
    }
    pub fn new(host: String, port: u16, username: String, auth: SshAuth) -> Result<Self> {
        let directory = ghostlink_platform::data_dir()
            .context("Could not find your application data directory for SSH host keys")?;
        let profile = Self {
            host,
            port,
            username,
            auth,
            known_hosts: directory.join("known_hosts"),
            options: Box::default(),
        };
        profile.validate()?;
        Ok(profile)
    }
    pub fn validate(&self) -> Result<()> {
        fn visit(profile: &SshProfile, count: &mut usize) -> Result<()> {
            *count += 1;
            ensure!(*count <= 9, "SSH route exceeds 8 jump hosts");
            profile.validate_local()?;
            for jump in &profile.options.jump_hosts {
                visit(jump, count)?;
            }
            Ok(())
        }
        visit(self, &mut 0)
    }
    fn validate_local(&self) -> Result<()> {
        if let Some(name) = &self.options.charset {
            crate::charset::validate(name)?;
        }
        if let Some(proxy) = &self.options.proxy {
            proxy.validate()?;
        }

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
        ensure!(
            self.options
                .keepalive_secs
                .is_none_or(|seconds| seconds <= 86400),
            "Keep-alive must be between 0 and 86400 seconds"
        );
        ensure!(
            self.options.startup_snippet.len() <= 64 * 1024
                && !self.options.startup_snippet.contains('\0'),
            "Invalid startup snippet"
        );
        ensure!(
            self.options.environment.len() <= 128,
            "Too many environment variables"
        );
        for (name, value) in &self.options.environment {
            ensure!(
                !name.is_empty()
                    && name.len() <= 256
                    && name
                        .bytes()
                        .enumerate()
                        .all(|(index, byte)| byte.is_ascii_alphabetic()
                            || byte == b'_'
                            || index > 0 && byte.is_ascii_digit()),
                "Invalid environment variable name"
            );
            ensure!(
                value.len() <= 8192 && !value.contains('\0'),
                "Invalid environment variable value"
            );
        }
        if self.options.certificate.is_some() {
            ensure!(
                self.options.identity_file.is_some()
                    || matches!(self.auth, SshAuth::PrivateKey { .. }),
                "Choose a private key for the SSH certificate"
            );
        }
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
    pub id: u64,
    pub name: Option<String>,
    pub host: String,
    pub algorithm: String,
    pub fingerprint: String,
}
pub(crate) struct HostKeyDecision {
    pub id: u64,
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
    name: Option<String>,
    port: u16,
    known_hosts: PathBuf,
    updates: Sender<Update>,
    stale: Receiver<Update>,
    decisions: Receiver<HostKeyDecision>,
    agent_socket: Option<PathBuf>,
    agent_access: Arc<AtomicBool>,
    agent_slots: Arc<tokio::sync::Semaphore>,
}
impl client::Handler for Verifier {
    type Error = anyhow::Error;
    async fn server_channel_open_agent_forward(
        &mut self,
        channel: russh::Channel<client::Msg>,
        reply: client::ChannelOpenHandle,
        _: &mut client::Session,
    ) -> Result<()> {
        if !self.agent_access.load(Ordering::Acquire) {
            reply
                .reject(russh::ChannelOpenFailure::AdministrativelyProhibited)
                .await;
            return Ok(());
        }
        let Some(path) = &self.agent_socket else {
            reply
                .reject(russh::ChannelOpenFailure::AdministrativelyProhibited)
                .await;
            return Ok(());
        };
        let Ok(permit) = self.agent_slots.clone().try_acquire_owned() else {
            reply
                .reject(russh::ChannelOpenFailure::ResourceShortage)
                .await;
            return Ok(());
        };
        let Ok(mut local) = ghostlink_platform::agent::connect(path).await else {
            reply.reject(russh::ChannelOpenFailure::ConnectFailed).await;
            return Ok(());
        };
        reply.accept().await;
        tokio::spawn(async move {
            let _permit = permit;
            let mut remote = channel.into_stream();
            let _ = tokio::time::timeout(
                Duration::from_secs(300),
                tokio::io::copy_bidirectional(&mut remote, &mut local),
            )
            .await;
        });
        Ok(())
    }
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
            id: next_prompt_id(),
            name: self.name.clone(),
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
        let decision = tokio::time::timeout(Duration::from_secs(300), async {
            loop {
                let decision = self
                    .decisions
                    .recv()
                    .await
                    .context("Host-key verification cancelled")?;
                if decision.id == prompt.id {
                    return Ok::<_, anyhow::Error>(decision);
                }
            }
        })
        .await
        .context("Host-key verification timed out")??;
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
    reconnecting: bool,
) -> Result<State> {
    let runtime = connection.0.runtime.as_ref().expect("live SSH runtime");
    runtime.block_on(async {
        let mut online = false;
        let cancellation = async {
            while !cancelled.load(Ordering::Acquire) { tokio::time::sleep(Duration::from_millis(10)).await; }
        };
        let result=tokio::select! {
            result = connect(&connection, size, input, updates, stale, decisions, credentials, last_screen,terminal,reconnecting,&mut online) => result,
            _ = cancellation => Ok(State::Exited(0)),
        };
        match result { Err(error) if online => Ok(State::Disconnected(format!("{error:#}"))), other => other }
    })
}

fn load_certificate(path: &Path) -> Result<keys::ssh_key::Certificate> {
    ensure!(
        fs::metadata(path)
            .with_context(|| format!("Reading SSH certificate {}", path.display()))?
            .len()
            <= 64 * 1024,
        "SSH certificate exceeds 64 KiB"
    );
    let text = fs::read_to_string(path).context("Reading SSH certificate")?;
    keys::ssh_key::Certificate::from_openssh(text.trim())
        .context("Invalid OpenSSH user certificate")
}

async fn authenticate(
    session: &mut client::Handle<Verifier>,
    username: &str,
    auth: SshAuth,
    certificate: Option<PathBuf>,
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
            if let Some(path) = certificate {
                let certificate = load_certificate(&path)?;
                ensure!(
                    certificate.public_key() == key.public_key().key_data(),
                    "The SSH certificate does not match the private key"
                );
                session
                    .authenticate_openssh_cert(username, Arc::new(key), certificate)
                    .await?
                    .success()
            } else {
                let hash = session.best_supported_rsa_hash().await?.flatten();
                session
                    .authenticate_publickey(
                        username,
                        keys::PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                    )
                    .await?
                    .success()
            }
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

async fn request_reply(
    channel: &mut russh::ChannelReadHalf,
    name: &str,
    startup: &mut StartupMessages,
) -> Result<bool> {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match channel.wait().await {
                Some(ChannelMsg::Success) => return Ok(true),
                Some(ChannelMsg::Failure) => return Ok(false),
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

async fn request_accepted(
    channel: &mut russh::ChannelReadHalf,
    name: &str,
    startup: &mut StartupMessages,
) -> Result<()> {
    ensure!(
        request_reply(channel, name, startup).await?,
        "Server refused the {name} request"
    );
    Ok(())
}

fn resolve_hop(mut profile: SshProfile) -> Result<(SshProfile, Option<config::Resolved>)> {
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
    Ok((profile, automatic))
}

#[allow(clippy::too_many_arguments)]
async fn authenticate_hop(
    profile: SshProfile,
    automatic: Option<config::Resolved>,
    stream: proxy::Stream,
    updates: &Sender<Update>,
    stale: &Receiver<Update>,
    decisions: Receiver<HostKeyDecision>,
    prompts: automatic::Prompter,
) -> Result<(Arc<client::Handle<Verifier>>, Arc<AtomicBool>)> {
    let agent_access = Arc::new(AtomicBool::new(false));
    let agent_socket = if profile.options.agent_forwarding {
        profile
            .options
            .agent_socket
            .clone()
            .or_else(|| automatic.as_ref().and_then(|plan| plan.agent.clone()))
            .or_else(|| {
                if automatic.is_none() {
                    std::env::var_os("SSH_AUTH_SOCK")
                        .map(PathBuf::from)
                        .or_else(config::default_agent)
                } else {
                    None
                }
            })
    } else {
        None
    };
    let forwarding_available = if let Some(path) = &agent_socket {
        ghostlink_platform::agent::connect(path).await.is_ok()
    } else {
        false
    };
    let config = Arc::new(client::Config {
        keepalive_interval: match profile.options.keepalive_secs {
            None => Some(Duration::from_secs(30)),
            Some(0) => None,
            Some(seconds) => Some(Duration::from_secs(seconds)),
        },
        keepalive_max: 3,
        ..Default::default()
    });
    let verifier = Verifier {
        host: profile.host,
        name: profile.options.display_name.clone(),
        port: profile.port,
        known_hosts: profile.known_hosts,
        agent_socket,
        agent_access: agent_access.clone(),
        agent_slots: Arc::new(tokio::sync::Semaphore::new(16)),
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
            prompts,
            *profile.options,
        )
        .await?;
    } else {
        tokio::time::timeout(
            Duration::from_secs(30),
            authenticate(
                &mut connection,
                &profile.username,
                profile.auth,
                profile.options.certificate,
            ),
        )
        .await
        .context("SSH authentication timed out")??;
    }
    agent_access.store(forwarding_available, Ordering::Release);
    Ok((Arc::new(connection), agent_access))
}

async fn authenticate_connection(
    profile: SshProfile,
    updates: &Sender<Update>,
    stale: &Receiver<Update>,
    decisions: Receiver<HostKeyDecision>,
    credentials: Receiver<CredentialReply>,
) -> Result<Arc<AuthenticatedRoute>> {
    fn flatten(profile: &SshProfile, hops: &mut Vec<SshProfile>) -> Result<()> {
        ensure!(hops.len() < 9, "SSH route exceeds 8 jump hosts");
        for hop in &profile.options.jump_hosts {
            flatten(hop, hops)?;
        }
        let mut hop = profile.clone();
        hop.options.jump_hosts.clear();
        ensure!(hops.len() < 9, "SSH route exceeds 8 jump hosts");
        hops.push(hop);
        Ok(())
    }
    profile.validate()?;
    let mut profiles = Vec::new();
    flatten(&profile, &mut profiles)?;
    let mut hops = Vec::<Arc<client::Handle<Verifier>>>::new();
    let mut final_agent = Arc::new(AtomicBool::new(false));
    for profile in profiles {
        let (mut profile, automatic) = resolve_hop(profile)?;
        let context = profile
            .options
            .display_name
            .clone()
            .unwrap_or_else(|| profile.label());
        let mut prompts = automatic::Prompter::new(updates, stale, credentials.clone())
            .with_context(context.clone());
        if let Some(proxy) = &mut profile.options.proxy
            && proxy.needs_password()
            && proxy.password.is_none()
        {
            proxy.password = Some(
                prompts
                    .ask(
                        "Proxy password".into(),
                        format!(
                            "Password for proxy {}@{}:{}",
                            proxy.username, proxy.host, proxy.port
                        ),
                        true,
                    )
                    .await?,
            );
        }
        let (host, port) = profile
            .options
            .proxy
            .as_ref()
            .map(|proxy| (proxy.host.as_str(), proxy.port))
            .unwrap_or((&profile.host, profile.port));
        let stream: proxy::Stream = if let Some(previous) = hops.last() {
            let channel = tokio::time::timeout(
                Duration::from_secs(15),
                previous.channel_open_direct_tcpip(host, port as u32, "127.0.0.1", 0),
            )
            .await
            .with_context(|| format!("Jump host timed out opening {context}"))?
            .with_context(|| {
                format!(
                    "Jump host could not open {context}; check forwarding policy and destination"
                )
            })?;
            Box::new(channel.into_stream())
        } else {
            let stream = tokio::time::timeout(
                Duration::from_secs(15),
                tokio::net::TcpStream::connect((host, port)),
            )
            .await
            .with_context(|| format!("Connecting to {context} timed out"))?
            .with_context(|| format!("Connecting to {context}"))?;
            stream
                .set_nodelay(true)
                .context("Configuring interactive SSH socket")?;
            Box::new(stream)
        };
        let stream = if let Some(proxy) = &profile.options.proxy {
            tokio::time::timeout(
                Duration::from_secs(15),
                proxy::tunnel(stream, proxy, &profile.host, profile.port),
            )
            .await
            .with_context(|| format!("Proxy handshake timed out for {context}"))?
            .with_context(|| format!("Opening proxy tunnel for {context}"))?
        } else {
            stream
        };
        let (connection, agent) = authenticate_hop(
            profile,
            automatic,
            stream,
            updates,
            stale,
            decisions.clone(),
            prompts,
        )
        .await
        .with_context(|| format!("SSH hop {context}"))?;
        hops.push(connection);
        final_agent = agent;
    }
    let connection = hops.pop().context("SSH route is empty")?;
    Ok(Arc::new(AuthenticatedRoute {
        connection,
        jumps: hops,
        agent_access: final_agent,
    }))
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
    reconnecting: bool,
    online: &mut bool,
) -> Result<State> {
    publish(
        updates,
        stale,
        Update {
            screen: None,
            state: State::Connecting,
        },
    );
    terminal.set_theme(shared.0.profile.options.theme)?;
    let route = {
        let mut authenticated = shared.0.authenticated.lock().await;
        if authenticated
            .as_ref()
            .is_some_and(|connection| connection.is_closed())
        {
            *authenticated = None;
        }
        if authenticated.is_none() {
            *authenticated = Some(
                authenticate_connection(
                    shared.0.profile.clone(),
                    updates,
                    stale,
                    decisions,
                    credentials,
                )
                .await?,
            );
        }
        authenticated
            .as_ref()
            .expect("authenticated transport")
            .clone()
    };
    let connection = route.connection.clone();
    let channel = tokio::time::timeout(Duration::from_secs(15), connection.channel_open_session())
        .await
        .context("SSH channel open timed out")??;
    let (mut reader, writer) = channel.split();
    let writer = Arc::new(writer);
    let _close_channel = CloseChannel(writer.clone());
    let mut startup = StartupMessages::default();
    let options = &shared.0.profile.options;
    let mut codec = crate::charset::Codec::new(options.charset.as_deref().unwrap_or("UTF-8"))?;
    let mut refused_environment = Vec::new();
    let mut forwarding_notice = false;
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
        for (name, value) in &options.environment {
            writer.set_env(true, name.clone(), value.clone()).await?;
            if !request_reply(&mut reader, "environment", &mut startup).await? {
                refused_environment.push(name.clone());
            }
        }
        if options.agent_forwarding {
            if route.agent_access.load(Ordering::Acquire) {
                writer.agent_forward(true).await?;
                forwarding_notice =
                    !request_reply(&mut reader, "agent forwarding", &mut startup).await?;
            } else {
                forwarding_notice = true;
            }
        }
        writer.request_shell(true).await?;
        request_accepted(&mut reader, "shell", &mut startup).await
    }
    .await;
    if let Err(error) = opened {
        // Keep any server explanation visible alongside the connection error.
        for message in startup.events {
            if let ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } = message {
                terminal.feed(&codec.decode(&data, false));
            }
        }
        *last_screen = Some(Arc::new(terminal.snapshot()?));
        return Err(error);
    }
    *online = true;
    if reconnecting {
        terminal.prepare_reconnect()?;
    }
    if forwarding_notice {
        terminal.feed(b"Ghostlink: agent forwarding unavailable; check the local agent and server policy.\r\n");
    }
    for name in refused_environment {
        terminal.feed(
            format!("Ghostlink: server refused environment variable {name}; check AcceptEnv.\r\n")
                .as_bytes(),
        );
    }
    for line in options
        .startup_snippet
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
    {
        writer
            .data_bytes(codec.encode_owned(format!("{line}\r").into_bytes()))
            .await?;
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
                let text_encoding = !matches!(&event, Input::Mouse(_));
                let bytes = match event {
                    Input::Text(text) => text.into_bytes(),
                    Input::Paste(text) => terminal.encode_paste(&text),
                    Input::Key(Key::Backspace, mods) if options.control_h_backspace && !mods.shift && !mods.control && !mods.alt && !mods.super_key => vec![0x08],
                    Input::Key(key, mods) => terminal.encode_key(key, mods)?,
                    Input::Mouse(event) => terminal.encode_mouse(event)?,
                    Input::Scroll(delta) => { terminal.scroll(delta); viewport_scrolled = true; dirty = true; continue; },
                    Input::Resize(new_size) => {
                        if size != new_size { terminal.resize(new_size)?; size = new_size; pending.push_back(Outgoing::Resize(size)); dirty = true; }
                        continue;
                    },
                    Input::Select(_) | Input::Copy(_) | Input::Reconnect => continue,
                };
                let bytes = if text_encoding { codec.encode_owned(bytes) } else { bytes };
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
                    let replies = terminal.feed(&codec.decode(&data, false));
                    let replies = codec.encode_owned(replies);
                    if !replies.is_empty() { pending_bytes += replies.len(); pending.push_back(Outgoing::Bytes(replies)); }
                    dirty = true;
                },
                Some(ChannelMsg::ExitStatus { exit_status: code }) => exit_status = Some(code),
                Some(ChannelMsg::ExitSignal { signal_name, .. }) => bail!("Remote shell terminated by {signal_name:?}"),
                Some(ChannelMsg::Close) | None => { terminal.feed(&codec.decode(&[], true)); break; },
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

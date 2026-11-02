//! A credential-free connection catalog with atomic, serialized updates.
use crate::host_options::HostOptions;
use anyhow::{Context as _, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use session::SshProfile;
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

const MAX_BYTES: usize = 1024 * 1024;
const MAX_CONNECTIONS: usize = 1000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedConnection {
    pub id: u64,
    pub label: String,
    pub address: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub options: HostOptions,
}
impl SavedConnection {
    pub fn from_destination(id: u64, label: &str, destination: &str) -> Result<Self> {
        let profile = validate_values(label, destination)?;
        Ok(Self {
            id,
            label: if label.trim().is_empty() {
                profile.label()
            } else {
                label.trim().to_owned()
            },
            address: profile.host,
            port: (profile.port != 0).then_some(profile.port),
            username: profile.username,
            options: HostOptions::default(),
        })
    }
    pub fn destination(&self) -> String {
        let address = if self.address.contains(':') {
            format!("[{}]", self.address)
        } else {
            self.address.clone()
        };
        format!(
            "{}{}{}",
            if self.username.is_empty() {
                String::new()
            } else {
                format!("{}@", self.username)
            },
            address,
            self.port.map(|port| format!(":{port}")).unwrap_or_default()
        )
    }
    pub fn to_profile(&self) -> Result<SshProfile> {
        let mut profile = SshProfile::from_destination(&self.destination())?;
        profile.options.display_name = Some(self.label.clone());
        profile.options.proxy = self.options.proxy.as_ref().map(|proxy| proxy.to_session());
        profile.options.theme = self.options.theme;
        profile.options.charset = match &self.options.charset {
            crate::host_options::Charset::Utf8 => None,
            crate::host_options::Charset::Named(name) => Some(name.clone()),
        };
        profile.options.agent_forwarding = self.options.agent_forwarding;
        profile.options.environment = self.options.environment.clone();
        profile.options.startup_snippet = self.options.startup_snippet.clone();
        profile.options.keepalive_secs = self.options.keepalive_secs;
        profile.options.control_h_backspace = matches!(
            self.options.backspace,
            crate::host_options::Backspace::ControlH
        );
        match &self.options.authentication {
            crate::host_options::Authentication::Automatic => {}
            crate::host_options::Authentication::Password { .. } => {
                profile.options.password_only = true
            }
            crate::host_options::Authentication::KeyFile { path, certificate } => {
                profile.options.identity_file = Some(path.clone());
                profile.options.certificate = certificate.clone();
            }
        }
        Ok(profile)
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.label.trim().is_empty(),
            "Saved connection has no label"
        );
        ensure!(
            !self.address.is_empty()
                && !self.address.contains('@')
                && !self.address.contains(['[', ']']),
            "Enter an address without username, brackets, or port"
        );
        if let Some((address, _)) = self.address.split_once('%') {
            ensure!(
                address.parse::<std::net::Ipv6Addr>().is_ok(),
                "Invalid scoped IPv6 address"
            );
        } else if self.address.contains(':') {
            ensure!(
                self.address.parse::<std::net::Ipv6Addr>().is_ok(),
                "Enter the port in the SSH port field"
            );
        }
        ensure!(self.port != Some(0), "SSH port must be between 1 and 65535");
        let profile = validate_values(&self.label, &self.destination())?;
        ensure!(
            profile.host == self.address
                && profile.username == self.username
                && profile.port == self.port.unwrap_or(0),
            "Invalid SSH address or username"
        );
        self.options.validate()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyCatalog {
    version: u32,
    next_id: u64,
    connections: Vec<LegacyConnection>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyConnection {
    id: u64,
    name: String,
    destination: String,
    #[serde(default)]
    options: HostOptions,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    version: u32,
    next_id: u64,
    connections: Vec<SavedConnection>,
}
impl Default for Catalog {
    fn default() -> Self {
        Self {
            version: 2,
            next_id: 1,
            connections: Vec::new(),
        }
    }
}
impl Catalog {
    fn validate(&self) -> Result<()> {
        ensure!(self.next_id > 0, "Invalid connection identifier counter");
        ensure!(
            self.version == 2,
            "Unsupported connections file version: {}",
            self.version
        );
        ensure!(
            self.connections.len() <= MAX_CONNECTIONS,
            "Too many saved connections"
        );
        let mut ids = HashSet::new();
        for entry in &self.connections {
            ensure!(
                entry.id > 0 && entry.id < self.next_id && ids.insert(entry.id),
                "Invalid or duplicate connection ID"
            );
            entry.validate()?;
        }
        for entry in &self.connections {
            self.validate_chain(entry.id, &mut Vec::new())?;
        }
        Ok(())
    }
    fn validate_chain(&self, id: u64, path: &mut Vec<u64>) -> Result<usize> {
        ensure!(!path.contains(&id), "Jump host chain contains a cycle");
        ensure!(path.len() <= 8, "Jump host chain exceeds 8 hops");
        let host = self
            .connections
            .iter()
            .find(|host| host.id == id)
            .context("A jump host no longer exists")?;
        path.push(id);
        let mut hops = 1;
        for hop in &host.options.jump_hosts {
            hops += self.validate_chain(*hop, path)?;
        }
        path.pop();
        ensure!(hops <= 9, "Jump host chain exceeds 8 hops");
        Ok(hops)
    }
    fn allocate_id(&mut self) -> Result<u64> {
        let id = self.next_id;
        self.next_id = id
            .checked_add(1)
            .context("Connection identifiers exhausted")?;
        Ok(id)
    }
    fn sort(&mut self) {
        self.connections
            .sort_by_cached_key(|entry| (entry.label.to_lowercase(), entry.destination()));
    }
}

#[derive(Clone)]
enum Storage {
    File(PathBuf),
    Memory(Arc<Mutex<Catalog>>),
    Unavailable(String),
}
#[derive(Clone)]
pub struct ConnectionStore(Storage);
impl ConnectionStore {
    pub fn local() -> Self {
        // GUI tests must never read or change the user's connection catalog.
        if cfg!(test) {
            return Self::memory();
        }
        match ghostlink_platform::data_dir() {
            Some(path) => Self::at(path.join("connections.json")),
            None => Self(Storage::Unavailable(
                "Could not find your application data directory; saved connections are unavailable"
                    .into(),
            )),
        }
    }
    pub fn at(path: PathBuf) -> Self {
        Self(Storage::File(path))
    }
    fn memory() -> Self {
        Self(Storage::Memory(Arc::new(Mutex::new(Catalog::default()))))
    }
    pub fn load(&self) -> Result<Vec<SavedConnection>> {
        let mut data = match &self.0 {
            Storage::File(path) => read(path)?,
            Storage::Memory(data) => data
                .lock()
                .map_err(|_| anyhow::anyhow!("Connection catalog lock failed"))?
                .clone(),
            Storage::Unavailable(error) => bail!("{error}"),
        };
        data.sort();
        Ok(data.connections)
    }
    /// Snapshot the ordered route, expanding each saved jump host's own route.
    pub fn route(&self, id: u64) -> Result<Vec<SavedConnection>> {
        fn visit(
            id: u64,
            hosts: &[SavedConnection],
            path: &mut Vec<u64>,
            route: &mut Vec<SavedConnection>,
        ) -> Result<()> {
            ensure!(!path.contains(&id), "Jump host chain contains a cycle");
            ensure!(
                path.len() <= 8 && route.len() < 9,
                "Jump host chain exceeds 8 hops"
            );
            let host = hosts
                .iter()
                .find(|host| host.id == id)
                .context("A host in this route no longer exists")?;
            path.push(id);
            for jump in &host.options.jump_hosts {
                visit(*jump, hosts, path, route)?;
            }
            path.pop();
            ensure!(route.len() < 9, "Jump host chain exceeds 8 hops");
            route.push(host.clone());
            Ok(())
        }
        let hosts = self.load()?;
        let mut route = Vec::new();
        visit(id, &hosts, &mut Vec::new(), &mut route)?;
        Ok(route)
    }
    fn update<T>(&self, operation: impl FnOnce(&mut Catalog) -> Result<T>) -> Result<T> {
        match &self.0 {
            Storage::File(path) => {
                let parent = path
                    .parent()
                    .context("Invalid connection catalog location")?;
                fs::create_dir_all(parent).context("Creating the connection catalog directory")?;
                let lock = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(path.with_extension("lock"))
                    .context("Opening the connection catalog lock")?;
                lock.try_lock()
                    .context("Another window is updating connections. Try again")?;
                // Reload under the lock so multiple app instances don't overwrite each other.
                let mut data = read(path)?;
                let result = operation(&mut data)?;
                data.validate()?;
                data.sort();
                let bytes =
                    serde_json::to_vec_pretty(&data).context("Encoding saved connections")?;
                ensure!(
                    bytes.len() < MAX_BYTES,
                    "Saved connections exceed the 1 MiB limit"
                );
                let mut temporary = tempfile::Builder::new()
                    .prefix(".connections-")
                    .suffix(".tmp")
                    .tempfile_in(parent)
                    .context("Creating a connection catalog update")?;
                temporary.write_all(&bytes)?;
                temporary.write_all(b"\n")?;
                temporary
                    .as_file()
                    .sync_all()
                    .context("Flushing saved connections")?;
                temporary
                    .persist(path)
                    .map_err(|e| e.error)
                    .context("Saving connections")?;
                Ok(result)
            }
            Storage::Memory(data) => {
                let mut current = data
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Connection catalog lock failed"))?;
                let mut next = current.clone();
                let result = operation(&mut next)?;
                next.validate()?;
                next.sort();
                *current = next;
                Ok(result)
            }
            Storage::Unavailable(error) => bail!("{error}"),
        }
    }
    #[cfg(test)]
    pub fn save(&self, id: Option<u64>, name: &str, destination: &str) -> Result<SavedConnection> {
        let mut entry = SavedConnection::from_destination(id.unwrap_or(0), name, destination)?;
        self.update(|data| {
            if let Some(current) = data.connections.iter().find(|host| Some(host.id) == id) {
                entry.options = current.options.clone();
            }
            Self::put(data, entry, id.is_some())
        })
    }
    pub fn save_host(&self, entry: SavedConnection) -> Result<SavedConnection> {
        let existing = entry.id != 0;
        self.update(|data| Self::put(data, entry, existing))
    }
    fn put(
        data: &mut Catalog,
        mut entry: SavedConnection,
        existing: bool,
    ) -> Result<SavedConnection> {
        entry.validate()?;
        if existing {
            let current = data
                .connections
                .iter_mut()
                .find(|host| host.id == entry.id)
                .context("This connection was removed in another window. Refresh the list")?;
            *current = entry.clone();
        } else {
            ensure!(
                data.connections.len() < MAX_CONNECTIONS,
                "You can save up to {MAX_CONNECTIONS} connections"
            );
            entry.id = data.allocate_id()?;
            data.connections.push(entry.clone());
        }
        Ok(entry)
    }
    pub fn delete(&self, id: u64) -> Result<SavedConnection> {
        self.update(|data| {
            let dependants = data
                .connections
                .iter()
                .filter(|host| host.options.jump_hosts.contains(&id))
                .map(|host| host.label.as_str())
                .collect::<Vec<_>>();
            ensure!(
                dependants.is_empty(),
                "This host is used as a jump host by: {}",
                dependants.join(", ")
            );
            let index = data
                .connections
                .iter()
                .position(|entry| entry.id == id)
                .context("This connection was already removed. Reopen the list to refresh")?;
            Ok(data.connections.remove(index))
        })
    }
    pub fn restore(&self, entry: SavedConnection) -> Result<SavedConnection> {
        self.update(|data| {
            ensure!(
                data.connections.len() < MAX_CONNECTIONS,
                "You can save up to {MAX_CONNECTIONS} connections"
            );
            let mut entry = entry;
            if entry.id >= data.next_id || data.connections.iter().any(|e| e.id == entry.id) {
                entry.id = data.allocate_id()?;
            }
            data.connections.push(entry.clone());
            Ok(entry)
        })
    }
}

fn validate_values(name: &str, destination: &str) -> Result<SshProfile> {
    ensure!(
        name.len() <= 1024 && !name.chars().any(char::is_control),
        "Enter a connection name without control characters (up to 1024 bytes)"
    );
    ensure!(destination.len() <= 1024, "The SSH destination is too long");
    let profile = SshProfile::from_destination(destination)?;
    ensure!(
        !profile.username.contains(':'),
        "Use user@server without a password in the destination"
    );
    Ok(profile)
}
fn read(path: &Path) -> Result<Catalog> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Catalog::default()),
        Err(error) => return Err(error).with_context(|| format!("Reading {}", path.display())),
    };
    let mut text = String::new();
    file.take(MAX_BYTES as u64 + 1)
        .read_to_string(&mut text)
        .context("Reading saved connections")?;
    ensure!(
        text.len() <= MAX_BYTES,
        "The connections file exceeds 1 MiB"
    );
    let value: serde_json::Value = serde_json::from_str(&text)
        .with_context(|| format!("Invalid connections file: {}", path.display()))?;
    let data: Catalog = match value.get("version").and_then(|version| version.as_u64()) {
        Some(1) => {
            let legacy: LegacyCatalog = serde_json::from_value(value)?;
            ensure!(legacy.version == 1, "Invalid legacy catalog version");
            let connections = legacy
                .connections
                .into_iter()
                .map(|host| {
                    let mut entry =
                        SavedConnection::from_destination(host.id, &host.name, &host.destination)?;
                    entry.options = host.options;
                    Ok(entry)
                })
                .collect::<Result<Vec<_>>>()?;
            Catalog {
                version: 2,
                next_id: legacy.next_id,
                connections,
            }
        }
        Some(2) => serde_json::from_value(value)?,
        _ => bail!("Unsupported connections file version"),
    };
    data.validate()?;
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_expand_nested_jump_hosts_in_order_and_keep_each_hosts_options() {
        let store = ConnectionStore::memory();
        let first = store.save(None, "Outer", "one@outer:2222").unwrap();
        let mut second = store.save(None, "Inner", "two@inner").unwrap();
        second.options.jump_hosts.push(first.id);
        second.options.keepalive_secs = Some(5);
        let second = store.save_host(second).unwrap();
        let mut root = store.save(None, "Application", "three@app").unwrap();
        root.options.jump_hosts.push(second.id);
        let root = store.save_host(root).unwrap();
        let route = store.route(root.id).unwrap();
        assert_eq!(route, vec![first, second, root]);
        assert_eq!(
            route[1].to_profile().unwrap().options.keepalive_secs,
            Some(5)
        );
    }
    #[test]
    fn host_shell_preferences_reach_the_session_and_invalid_names_are_rejected() {
        let store = ConnectionStore::memory();
        let mut host =
            SavedConnection::from_destination(0, "Shell preferences", "deploy@host").unwrap();
        host.options.backspace = crate::host_options::Backspace::ControlH;
        host.options.keepalive_secs = Some(0);
        host.options.environment = vec![("LANG".into(), "en_US.UTF-8".into())];
        host.options.startup_snippet = "echo ready".into();
        let host = store.save_host(host).unwrap();
        let profile = host.to_profile().unwrap();
        assert!(profile.options.control_h_backspace);
        assert_eq!(profile.options.keepalive_secs, Some(0));
        assert_eq!(profile.options.environment, host.options.environment);
        assert_eq!(profile.options.startup_snippet, "echo ready");
        let mut invalid = host.clone();
        invalid
            .options
            .environment
            .push(("invalid=name".into(), "value".into()));
        assert!(store.save_host(invalid).is_err());
        assert_eq!(store.load().unwrap(), vec![host]);
    }

    #[test]
    fn v1_migration_preserves_aliases_ipv6_and_explicit_ports_until_next_save() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("connections.json");
        let legacy = r#"{"version":1,"next_id":4,"connections":[{"id":1,"name":"Alias","destination":"prod"},{"id":2,"name":"IPv6","destination":"deploy@[::1]:2222"},{"id":3,"name":"Explicit default","destination":"host:22"}]}"#;
        fs::write(&path, legacy).unwrap();
        let store = ConnectionStore::at(path.clone());
        let hosts = store.load().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), legacy);
        let alias = hosts.iter().find(|host| host.id == 1).unwrap();
        assert_eq!(alias.address, "prod");
        assert!(alias.username.is_empty());
        assert_eq!(alias.port, None);
        let ipv6 = hosts.iter().find(|host| host.id == 2).unwrap();
        assert_eq!(ipv6.address, "::1");
        assert_eq!(ipv6.username, "deploy");
        assert_eq!(ipv6.port, Some(2222));
        let profile = ipv6.to_profile().unwrap();
        assert_eq!(profile.host, "::1");
        assert_eq!(profile.username, "deploy");
        assert_eq!(profile.port, 2222);
        assert_eq!(
            hosts.iter().find(|host| host.id == 3).unwrap().port,
            Some(22)
        );
        store.save_host(alias.clone()).unwrap();
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted["version"], 2);
        assert!(persisted["connections"][0].get("destination").is_none());
        assert_eq!(store.load().unwrap(), hosts);
    }

    #[test]
    fn jump_graph_rejects_missing_hosts_cycles_and_removal_without_mutating_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("connections.json");
        let store = ConnectionStore::at(path.clone());
        let mut first = store.save(None, "Bastion", "gateway").unwrap();
        let mut second = store.save(None, "Application", "app").unwrap();
        second.options.jump_hosts.push(first.id);
        let second = store.save_host(second).unwrap();
        let before = fs::read(&path).unwrap();
        assert!(
            store
                .delete(first.id)
                .unwrap_err()
                .to_string()
                .contains("Application")
        );
        first.options.jump_hosts.push(second.id);
        assert!(
            store
                .save_host(first)
                .unwrap_err()
                .to_string()
                .contains("cycle")
        );
        let mut missing = second.clone();
        missing.options.jump_hosts = vec![999];
        assert!(store.save_host(missing).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        let duplicate = store.save(None, "Another application", "app").unwrap();
        assert_ne!(duplicate.id, second.id);
    }

    #[test]
    fn jump_graph_caps_nested_hops_and_restoring_cannot_break_references() {
        let store = ConnectionStore::memory();
        let mut previous = store.save(None, "Hop 0", "host0").unwrap();
        for index in 1..=8 {
            let mut next = SavedConnection::from_destination(
                0,
                &format!("Hop {index}"),
                &format!("host{index}"),
            )
            .unwrap();
            next.options.jump_hosts.push(previous.id);
            previous = store.save_host(next).unwrap();
        }
        let mut excess = SavedConnection::from_destination(0, "Too deep", "extra").unwrap();
        excess.options.jump_hosts.push(previous.id);
        assert!(
            store
                .save_host(excess)
                .unwrap_err()
                .to_string()
                .contains("8 hops")
        );
    }
    #[test]
    fn legacy_hosts_default_options_and_edits_preserve_preferences() {
        let legacy = r#"{"id":1,"name":"old","destination":"deploy@example.test"}"#;
        let old: LegacyConnection = serde_json::from_str(legacy).unwrap();
        let mut host =
            SavedConnection::from_destination(old.id, &old.name, &old.destination).unwrap();
        assert_eq!(host.options, HostOptions::default());
        host.options.group = "Production".into();
        host.options.tags = vec!["database".into()];
        host.options.authentication =
            crate::host_options::Authentication::Password { remember: true };
        let store = ConnectionStore::memory();
        let host = store.restore(host).unwrap();
        let edited = store
            .save(Some(host.id), "renamed", "deploy@example.test:2222")
            .unwrap();
        assert_eq!(edited.options, host.options);
        let encoded = serde_json::to_string(&edited).unwrap();
        let decoded: SavedConnection = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, edited);
    }

    #[test]
    fn saved_connections_round_trip_edit_delete_and_undo_without_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        let store = ConnectionStore::at(path.clone());
        assert!(store.load().unwrap().is_empty());
        let added = store.save(None, "Production", " deploy@[::1]:22 ").unwrap();
        assert_eq!(added.destination(), "deploy@[::1]:22");
        let other = ConnectionStore::at(path.clone());
        assert_eq!(other.load().unwrap(), vec![added.clone()]);
        let edited = other
            .save(Some(added.id), "Live", "deploy@host:2222")
            .unwrap();
        assert_eq!(store.load().unwrap(), vec![edited.clone()]);
        let removed = store.delete(added.id).unwrap();
        assert!(other.load().unwrap().is_empty());
        store.restore(removed).unwrap();
        assert_eq!(other.load().unwrap(), vec![edited]);
        let text = fs::read_to_string(path).unwrap();
        for secret in [
            "password",
            "passphrase",
            "private_key",
            "known_hosts",
            "home",
        ] {
            assert!(!text.contains(secret));
        }
    }
    #[test]
    fn concurrent_catalog_handles_merge_and_validation_preserves_existing_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        let a = ConnectionStore::at(path.clone());
        let b = ConnectionStore::at(path.clone());
        a.save(None, "One", "one").unwrap();
        b.save(None, "Two", "two").unwrap();
        assert_eq!(a.load().unwrap().len(), 2);
        let before = fs::read(&path).unwrap();
        assert!(a.save(None, "Bad name\n", "one").is_err());
        assert!(a.save(None, "Bad", "ssh://user:password@host").is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.with_extension("lock"))
            .unwrap();
        lock.lock().unwrap();
        assert!(b.save(None, "Three", "three").is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    #[test]
    fn malformed_and_future_catalogs_fail_without_overwriting_them() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        let store = ConnectionStore::at(path.clone());
        for text in ["broken", r#"{"version":3,"next_id":1,"connections":[]}"#] {
            fs::write(&path, text).unwrap();
            assert!(store.load().is_err());
            assert!(store.save(None, "New", "new").is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), text);
        }
    }
}

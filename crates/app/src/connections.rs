//! A credential-free connection catalog with atomic, serialized updates.
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
    pub name: String,
    pub destination: String,
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
            version: 1,
            next_id: 1,
            connections: Vec::new(),
        }
    }
}
impl Catalog {
    fn validate(&self) -> Result<()> {
        ensure!(self.next_id > 0, "Invalid connection identifier counter");
        ensure!(
            self.version == 1,
            "Unsupported connections file version: {}",
            self.version
        );
        ensure!(
            self.connections.len() <= MAX_CONNECTIONS,
            "Too many saved connections"
        );
        let mut ids = HashSet::new();
        let mut destinations = HashSet::new();
        for entry in &self.connections {
            ensure!(
                entry.id > 0 && entry.id < self.next_id && ids.insert(entry.id),
                "Invalid or duplicate connection ID"
            );
            validate_values(&entry.name, &entry.destination)?;
            ensure!(
                !entry.name.trim().is_empty(),
                "Saved connection has no name"
            );
            ensure!(
                destinations.insert(&entry.destination),
                "Duplicate saved SSH destination"
            );
        }
        Ok(())
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
            .sort_by_cached_key(|entry| (entry.name.to_lowercase(), entry.destination.clone()));
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
    pub fn save(&self, id: Option<u64>, name: &str, destination: &str) -> Result<SavedConnection> {
        let name = name.trim();
        let destination = destination.trim();
        let destination = destination.strip_prefix("ssh://").unwrap_or(destination);
        let profile = validate_values(name, destination)?;
        let name = if name.is_empty() {
            profile.label()
        } else {
            name.to_owned()
        };
        self.update(|data| {
            ensure!(
                !data
                    .connections
                    .iter()
                    .any(|entry| Some(entry.id) != id && entry.destination == destination),
                "This SSH destination is already saved"
            );
            let entry = SavedConnection {
                id: match id {
                    Some(id) => id,
                    None => data.allocate_id()?,
                },
                name,
                destination: destination.to_owned(),
            };
            if let Some(id) = id {
                let current = data.connections.iter_mut().find(|e| e.id == id).context(
                    "This connection was removed in another window. Return to the list to refresh",
                )?;
                *current = entry.clone();
            } else {
                ensure!(
                    data.connections.len() < MAX_CONNECTIONS,
                    "You can save up to {MAX_CONNECTIONS} connections"
                );
                data.connections.push(entry.clone());
            }
            Ok(entry)
        })
    }
    pub fn delete(&self, id: u64) -> Result<SavedConnection> {
        self.update(|data| {
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
            ensure!(
                !data
                    .connections
                    .iter()
                    .any(|e| e.destination == entry.destination),
                "This SSH destination has already been saved again"
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
    let data: Catalog = serde_json::from_str(&text)
        .with_context(|| format!("Invalid connections file: {}", path.display()))?;
    data.validate()?;
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_connections_round_trip_edit_delete_and_undo_without_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        let store = ConnectionStore::at(path.clone());
        assert!(store.load().unwrap().is_empty());
        let added = store.save(None, "Production", " deploy@[::1]:22 ").unwrap();
        assert_eq!(added.destination, "deploy@[::1]:22");
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
        assert!(a.save(None, "Duplicate", "one").is_err());
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
        for text in ["broken", r#"{"version":2,"next_id":1,"connections":[]}"#] {
            fs::write(&path, text).unwrap();
            assert!(store.load().is_err());
            assert!(store.save(None, "New", "new").is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), text);
        }
    }
}

//! OS credential storage. Calls run on a dedicated thread, including Secret Service's
//! synchronous wrapper, so neither the UI nor a Tokio executor can be blocked.
use anyhow::{Context, Result};
use async_channel::{Receiver, Sender};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use zeroize::Zeroizing;

type Reply = Sender<Result<Option<Zeroizing<String>>>>;
enum Operation {
    Get,
    Set(Zeroizing<String>),
    Delete,
}
struct Request {
    account: String,
    operation: Operation,
    reply: Reply,
    guard: Option<Box<dyn FnOnce() -> bool + Send>>,
}
#[derive(Clone, Copy)]
pub enum SecretKind {
    Password,
    Proxy,
}
impl SecretKind {
    fn suffix(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::Proxy => "proxy",
        }
    }
}
fn failures() -> &'static Mutex<HashMap<String, String>> {
    static FAILURES: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    FAILURES.get_or_init(|| Mutex::new(HashMap::new()))
}
pub fn last_error(id: u64, kind: SecretKind) -> Option<String> {
    failures()
        .lock()
        .ok()?
        .get(&format!("host:{id}:{}", kind.suffix()))
        .cloned()
}
fn worker() -> Result<&'static Sender<Request>> {
    static QUEUE: OnceLock<Result<Sender<Request>, String>> = OnceLock::new();
    QUEUE
        .get_or_init(|| {
            let (sender, receiver) = async_channel::unbounded::<Request>();
            std::thread::Builder::new()
                .name("ghostlink-keychain".into())
                .spawn(move || {
                    while let Ok(request) = receiver.recv_blocking() {
                        let result = if request.guard.is_none_or(|guard| guard()) {
                            perform(&request.account, request.operation)
                        } else {
                            Ok(None)
                        };
                        if let Ok(mut errors) = failures().lock() {
                            if result.is_err() {
                                errors.insert(
                                    request.account.clone(),
                                    "System keychain unavailable — Ghostlink will ask each time."
                                        .into(),
                                );
                            } else {
                                errors.remove(&request.account);
                            }
                        }
                        let _ = request.reply.send_blocking(result);
                    }
                })
                .map_err(|error| error.to_string())?;
            Ok(sender)
        })
        .as_ref()
        .map_err(|error| anyhow::anyhow!("System keychain worker unavailable: {error}"))
}
fn perform(account: &str, operation: Operation) -> Result<Option<Zeroizing<String>>> {
    let entry =
        keyring::Entry::new(crate::APP_ID, account).context("Opening the system keychain")?;
    match operation {
        Operation::Get => match entry.get_password() {
            Ok(password) => Ok(Some(Zeroizing::new(password))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error)
                .context("System keychain unavailable; Ghostlink will ask for credentials"),
        },
        Operation::Set(password) => {
            entry
                .set_password(password.as_str())
                .context("Saving credentials in the system keychain")?;
            Ok(None)
        }
        Operation::Delete => match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error).context("Removing credentials from the system keychain"),
        },
    }
}
fn submit(
    id: u64,
    kind: SecretKind,
    operation: Operation,
    guard: Option<Box<dyn FnOnce() -> bool + Send>>,
) -> Receiver<Result<Option<Zeroizing<String>>>> {
    let (reply, receiver) = async_channel::bounded(1);
    let request = Request {
        account: format!("host:{id}:{}", kind.suffix()),
        operation,
        reply,
        guard,
    };
    match worker() {
        Ok(sender) => {
            if let Err(error) = sender.try_send(request) {
                let request = error.into_inner();
                let _ = request
                    .reply
                    .try_send(Err(anyhow::anyhow!("System keychain worker stopped")));
            }
        }
        Err(error) => {
            let _ = request.reply.try_send(Err(error));
        }
    }
    receiver
}
pub fn read(id: u64, kind: SecretKind) -> Receiver<Result<Option<Zeroizing<String>>>> {
    submit(id, kind, Operation::Get, None)
}
pub fn write(
    id: u64,
    kind: SecretKind,
    value: Zeroizing<String>,
) -> Receiver<Result<Option<Zeroizing<String>>>> {
    submit(id, kind, Operation::Set(value), None)
}
/// Recheck that the host still opts into storage on the keychain worker. This
/// prevents a late authentication from recreating a deleted host's credentials.
pub fn write_if(
    id: u64,
    kind: SecretKind,
    value: Zeroizing<String>,
    guard: impl FnOnce() -> bool + Send + 'static,
) -> Receiver<Result<Option<Zeroizing<String>>>> {
    submit(id, kind, Operation::Set(value), Some(Box::new(guard)))
}
pub fn delete(id: u64, kind: SecretKind) -> Receiver<Result<Option<Zeroizing<String>>>> {
    submit(id, kind, Operation::Delete, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_keychain_returns_an_error_and_a_notice_without_blocking_callers() {
        struct Unavailable;
        impl keyring::credential::CredentialBuilderApi for Unavailable {
            fn build(
                &self,
                _: Option<&str>,
                _: &str,
                _: &str,
            ) -> keyring::Result<Box<keyring::credential::Credential>> {
                Err(keyring::Error::NoStorageAccess(Box::new(
                    std::io::Error::other("fixture: no Secret Service"),
                )))
            }
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
        }
        struct Restore;
        impl Drop for Restore {
            fn drop(&mut self) {
                keyring::set_default_credential_builder(
                    keyring::default::default_credential_builder(),
                );
            }
        }
        keyring::set_default_credential_builder(Box::new(Unavailable));
        let _restore = Restore;
        let result = read(0, SecretKind::Password).recv_blocking().unwrap();
        assert!(result.is_err());
        assert!(
            last_error(0, SecretKind::Password)
                .unwrap()
                .contains("ask each time")
        );
        assert!(
            write_if(
                0,
                SecretKind::Password,
                Zeroizing::new("never store".into()),
                || false
            )
            .recv_blocking()
            .unwrap()
            .is_ok()
        );
        assert!(last_error(0, SecretKind::Password).is_none());
    }
    #[test]
    #[ignore = "manual OS keychain access; may display a native permission prompt"]
    fn native_password_round_trip_and_removal() {
        let id = u64::MAX;
        write(
            id,
            SecretKind::Password,
            Zeroizing::new("generated keychain fixture".into()),
        )
        .recv_blocking()
        .unwrap()
        .unwrap();
        assert_eq!(
            read(id, SecretKind::Password)
                .recv_blocking()
                .unwrap()
                .unwrap()
                .as_deref()
                .map(String::as_str),
            Some("generated keychain fixture")
        );
        delete(id, SecretKind::Password)
            .recv_blocking()
            .unwrap()
            .unwrap();
        assert!(
            read(id, SecretKind::Password)
                .recv_blocking()
                .unwrap()
                .unwrap()
                .is_none()
        );
    }
}

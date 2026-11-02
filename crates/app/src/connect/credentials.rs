//! Bind credential-store values to their reviewed endpoint, and prepare every
//! hop's credentials without blocking GPUI. No serialized secrets reach the catalog.
use super::*;
use crate::host_options::{Authentication, ProxyKind};
use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredSecret {
    context: String,
    value: Zeroizing<String>,
}
pub(super) fn context(host: &SavedConnection, kind: SecretKind) -> Option<String> {
    match kind {
        SecretKind::Password => Some(host.destination()),
        SecretKind::Proxy => host.options.proxy.as_ref().and_then(|proxy| {
            serde_json::to_string(&(&proxy.kind, &proxy.host, proxy.port, &proxy.username)).ok()
        }),
    }
}
pub(super) fn remembers(host: &SavedConnection, kind: SecretKind) -> bool {
    match kind {
        SecretKind::Password => matches!(
            host.options.authentication,
            Authentication::Password { remember: true }
        ),
        SecretKind::Proxy => host.options.proxy.as_ref().is_some_and(|proxy| {
            proxy.remember && proxy.kind != ProxyKind::Socks4 && !proxy.username.is_empty()
        }),
    }
}
fn encode(context: String, value: Zeroizing<String>) -> Result<Zeroizing<String>> {
    Ok(Zeroizing::new(serde_json::to_string(&StoredSecret {
        context,
        value,
    })?))
}
fn decode(value: &str, expected: &str) -> Option<Zeroizing<String>> {
    let stored: StoredSecret = serde_json::from_str(value).ok()?;
    (stored.context == expected).then_some(stored.value)
}
fn callback(
    host: &SavedConnection,
    kind: SecretKind,
    store: &ConnectionStore,
) -> Arc<dyn Fn(Zeroizing<String>) + Send + Sync> {
    let id = host.id;
    let expected = context(host, kind).unwrap_or_default();
    let store = store.clone();
    Arc::new(move |value| {
        let Ok(value) = encode(expected.clone(), value) else {
            return;
        };
        let store = store.clone();
        let expected = expected.clone();
        let _ = secrets::write_if(id, kind, value, move || {
            store.load().is_ok_and(|hosts| {
                hosts.iter().any(|host| {
                    host.id == id
                        && remembers(host, kind)
                        && context(host, kind).as_ref() == Some(&expected)
                })
            })
        });
    })
}
pub(super) fn build(
    route: &[SavedConnection],
    store: &ConnectionStore,
    password: Option<Zeroizing<String>>,
    proxy_password: Option<Zeroizing<String>>,
) -> Result<Vec<SshProfile>> {
    let mut profiles = route
        .iter()
        .map(|host| {
            let mut profile = host.to_profile()?;
            if remembers(host, SecretKind::Password) {
                profile.options.on_password_accepted =
                    Some(callback(host, SecretKind::Password, store));
            }
            if remembers(host, SecretKind::Proxy)
                && let Some(proxy) = &mut profile.options.proxy
            {
                proxy.on_password_accepted = Some(callback(host, SecretKind::Proxy, store));
            }
            Ok(profile)
        })
        .collect::<Result<Vec<_>>>()?;
    let root = profiles.last_mut().context("Host route is empty")?;
    if root.options.password_only {
        root.options.password = password.filter(|password| !password.is_empty());
    }
    if let Some(proxy) = &mut root.options.proxy
        && proxy.needs_password()
    {
        proxy.password = proxy_password.filter(|password| !password.is_empty());
    }
    Ok(profiles)
}
pub(super) fn needs_keychain(route: &[SavedConnection], profiles: &[SshProfile]) -> bool {
    route.iter().zip(profiles).any(|(host, profile)| {
        remembers(host, SecretKind::Password) && profile.options.password.is_none()
            || remembers(host, SecretKind::Proxy)
                && profile
                    .options
                    .proxy
                    .as_ref()
                    .is_some_and(|proxy| proxy.password.is_none())
    })
}
pub(super) async fn load_saved(route: &[SavedConnection], profiles: &mut [SshProfile]) {
    load_saved_with(route, profiles, |id, kind| async move {
        secrets::read(id, kind)
            .recv()
            .await
            .context("System keychain worker stopped")?
    })
    .await;
}
async fn load_saved_with<F, Fut>(route: &[SavedConnection], profiles: &mut [SshProfile], read: F)
where
    F: Fn(u64, SecretKind) -> Fut,
    Fut: std::future::Future<Output = Result<Option<Zeroizing<String>>>>,
{
    for (host, profile) in route.iter().zip(profiles) {
        for kind in [SecretKind::Password, SecretKind::Proxy] {
            if !remembers(host, kind) {
                continue;
            }
            let target = match kind {
                SecretKind::Password => &mut profile.options.password,
                SecretKind::Proxy => {
                    &mut profile
                        .options
                        .proxy
                        .as_mut()
                        .expect("proxy settings")
                        .password
                }
            };
            if target.is_some() {
                continue;
            }
            match read(host.id, kind).await {
                Ok(Some(value)) => {
                    *target = context(host, kind).and_then(|context| decode(&value, &context))
                }
                Ok(None) => {}
                _ => {
                    profile.options.credential_notice =
                        Some("System keychain unavailable — Ghostlink will ask each time.".into())
                }
            }
        }
    }
}
pub(super) fn assemble(mut profiles: Vec<SshProfile>) -> Result<SshProfile> {
    let mut root = profiles.pop().context("Host route is empty")?;
    root.options.jump_hosts = profiles;
    root.validate()?;
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    async fn every_hop_receives_its_own_remembered_password_and_proxy_login() {
        let store = ConnectionStore::local();
        let mut gateway = store.save(None, "Gateway", "gateway-user@gateway").unwrap();
        gateway.options.authentication = Authentication::Password { remember: true };
        let gateway = store.save_host(gateway).unwrap();
        let mut root = store.save(None, "Application", "app-user@app").unwrap();
        root.options.authentication = Authentication::Password { remember: true };
        root.options.jump_hosts.push(gateway.id);
        root.options.proxy = Some(crate::host_options::Proxy {
            kind: ProxyKind::Socks5,
            host: "proxy".into(),
            port: 1080,
            username: "proxy-user".into(),
            remember: true,
        });
        let root = store.save_host(root).unwrap();
        let route = store.route(root.id).unwrap();
        let mut profiles = build(&route, &store, None, None).unwrap();
        assert!(needs_keychain(&route, &profiles));
        load_saved_with(&route, &mut profiles, |id, kind| {
            let host = route.iter().find(|host| host.id == id).unwrap();
            let value = match kind {
                SecretKind::Password => format!("password-{id}"),
                SecretKind::Proxy => "proxy-password".into(),
            };
            std::future::ready(Ok(Some(
                encode(context(host, kind).unwrap(), Zeroizing::new(value)).unwrap(),
            )))
        })
        .await;
        assert!(!needs_keychain(&route, &profiles));
        let profile = assemble(profiles).unwrap();
        assert_eq!(
            profile.options.password.as_ref().unwrap().as_str(),
            format!("password-{}", root.id)
        );
        assert_eq!(
            profile.options.jump_hosts[0]
                .options
                .password
                .as_ref()
                .unwrap()
                .as_str(),
            format!("password-{}", gateway.id)
        );
        assert_eq!(
            profile
                .options
                .proxy
                .as_ref()
                .unwrap()
                .password
                .as_ref()
                .unwrap()
                .as_str(),
            "proxy-password"
        );
        assert!(
            profile.options.jump_hosts[0]
                .options
                .on_password_accepted
                .is_some()
        );
        assert!(
            profile
                .options
                .proxy
                .as_ref()
                .unwrap()
                .on_password_accepted
                .is_some()
        );
    }
    #[test]
    fn stored_secrets_are_bound_to_the_endpoint_and_proxy_identity() {
        let mut host = SavedConnection::from_destination(1, "Host", "deploy@host:2222").unwrap();
        let expected = context(&host, SecretKind::Password).unwrap();
        let encoded = encode(expected.clone(), Zeroizing::new("fixture password".into())).unwrap();
        assert_eq!(
            decode(&encoded, &expected).unwrap().as_str(),
            "fixture password"
        );
        host.address = "another-host".into();
        assert!(decode(&encoded, &context(&host, SecretKind::Password).unwrap()).is_none());
        assert!(decode("unbound legacy password", &expected).is_none());
        host.options.proxy = Some(crate::host_options::Proxy {
            kind: ProxyKind::Socks5,
            host: "proxy".into(),
            port: 1080,
            username: "user".into(),
            remember: true,
        });
        let expected = context(&host, SecretKind::Proxy).unwrap();
        let encoded = encode(expected, Zeroizing::new("proxy fixture".into())).unwrap();
        host.options.proxy.as_mut().unwrap().username = "different".into();
        assert!(decode(&encoded, &context(&host, SecretKind::Proxy).unwrap()).is_none());
    }
}

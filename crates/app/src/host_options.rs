//! Persisted per-host preferences. Authentication secrets live in the OS keychain.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HostOptions {
    pub group: String,
    pub tags: Vec<String>,
    pub backspace: Backspace,
    pub authentication: Authentication,
    pub agent_forwarding: bool,
    pub startup_snippet: String,
    pub jump_hosts: Vec<u64>,
    pub proxy: Option<Proxy>,
    pub environment: Vec<(String, String)>,
    pub charset: Charset,
    pub theme: Theme,
    pub keepalive_secs: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Backspace {
    #[default]
    Delete,
    ControlH,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Authentication {
    #[default]
    Automatic,
    Password {
        remember: bool,
    },
    KeyFile {
        path: PathBuf,
        certificate: Option<PathBuf>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proxy {
    pub kind: ProxyKind,
    pub host: String,
    pub port: u16,
    pub username: String,
    #[serde(default)]
    pub remember: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProxyKind {
    Socks5,
    Socks4,
    Http,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Charset {
    #[default]
    Utf8,
    Named(String),
}

pub use terminal_core::themes::Theme;

impl HostOptions {
    pub fn validate(&self) -> anyhow::Result<()> {
        use anyhow::ensure;
        let single_line =
            |text: &str, max: usize| text.len() <= max && !text.chars().any(char::is_control);
        ensure!(
            single_line(&self.group, 256),
            "Group must be a single line of up to 256 bytes"
        );
        ensure!(
            self.tags.len() <= 64
                && self
                    .tags
                    .iter()
                    .all(|tag| !tag.trim().is_empty() && single_line(tag, 128)),
            "Use up to 64 tags, each up to 128 bytes"
        );
        ensure!(self.jump_hosts.len() <= 8, "Use up to 8 jump hosts");
        ensure!(
            self.jump_hosts
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                == self.jump_hosts.len(),
            "A jump host is listed more than once"
        );
        ensure!(
            self.startup_snippet.len() <= 64 * 1024 && !self.startup_snippet.contains('\0'),
            "Startup snippet exceeds 64 KiB or contains a NUL character"
        );
        ensure!(
            self.keepalive_secs.is_none_or(|seconds| seconds <= 86400),
            "Keep-alive must be between 0 and 86400 seconds"
        );
        ensure!(
            self.environment.len() <= 128,
            "Use up to 128 environment variables"
        );
        let mut names = std::collections::HashSet::new();
        for (name, value) in &self.environment {
            ensure!(
                !name.is_empty()
                    && name.len() <= 256
                    && name.bytes().enumerate().all(|(index, byte)| byte == b'_'
                        || byte.is_ascii_alphabetic()
                        || index > 0 && byte.is_ascii_digit()),
                "Invalid environment variable name: {name}"
            );
            ensure!(names.insert(name), "Duplicate environment variable: {name}");
            ensure!(
                value.len() <= 8192 && !value.contains('\0'),
                "Environment variable {name} is too long or contains NUL"
            );
        }
        if let Authentication::KeyFile { path, certificate } = &self.authentication {
            ensure!(!path.as_os_str().is_empty(), "Choose an SSH key file");
            ensure!(
                certificate
                    .as_ref()
                    .is_none_or(|path| !path.as_os_str().is_empty()),
                "Choose a certificate file or leave it empty"
            );
        }
        if let Some(proxy) = &self.proxy {
            proxy.to_session().validate()?;
        }
        if let Charset::Named(name) = &self.charset {
            session::charset::validate(name)?;
        }
        Ok(())
    }
}

impl Proxy {
    pub fn to_session(&self) -> session::ProxyOptions {
        session::ProxyOptions {
            kind: match self.kind {
                ProxyKind::Socks5 => session::ProxyKind::Socks5,
                ProxyKind::Socks4 => session::ProxyKind::Socks4,
                ProxyKind::Http => session::ProxyKind::Http,
            },
            host: self.host.clone(),
            port: self.port,
            username: self.username.clone(),
            password: None,
            on_password_accepted: None,
        }
    }
}

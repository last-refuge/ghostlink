//! Resolve the documented, non-executing subset of the user's SSH config.
use super::*;

#[derive(Debug)]
pub(super) struct Resolved {
    pub host: String,
    pub user: String,
    /// True when neither the destination nor SSH config named a remote user,
    /// so the local login name was used, as OpenSSH does.
    pub implicit_user: bool,
    pub port: u16,
    pub identities: Vec<PathBuf>,
    pub agent: Option<PathBuf>,
    pub identities_only: bool,
    pub password: bool,
    pub keyboard: bool,
    pub interactive: bool,
    pub methods: Vec<russh::MethodKind>,
}
#[derive(Default)]
struct Options {
    host: Option<String>,
    user: Option<String>,
    port: Option<u16>,
    identities: Vec<String>,
    agent: Option<String>,
    identities_only: Option<bool>,
    password: Option<bool>,
    keyboard: Option<bool>,
    public_key: Option<bool>,
    batch: Option<bool>,
    methods: Option<Vec<russh::MethodKind>>,
}
pub(super) fn resolve(profile: &SshProfile, home: &Path) -> Result<Resolved> {
    let local_user = if cfg!(windows) {
        std::env::var("USERNAME").or_else(|_| std::env::var("USER"))
    } else {
        std::env::var("USER").or_else(|_| std::env::var("LOGNAME"))
    }
    .context("Could not determine your SSH username")?;
    resolve_with_user(profile, home, &local_user)
}
fn resolve_with_user(profile: &SshProfile, home: &Path, local_user: &str) -> Result<Resolved> {
    let mut options = Options::default();
    let mut active = true;
    let mut stack = Vec::new();
    let mut count = 0;
    parse_file(
        &home.join(".ssh/config"),
        &profile.host,
        home,
        &mut options,
        &mut active,
        &mut stack,
        &mut count,
    )?;
    let host = options
        .host
        .unwrap_or_else(|| profile.host.clone())
        .replace("%h", &profile.host)
        .replace("%%", "%");
    let implicit_user = profile.username.is_empty() && options.user.is_none();
    let user = if profile.username.is_empty() {
        options.user.unwrap_or_else(|| local_user.into())
    } else {
        profile.username.clone()
    };
    let port = if profile.port == 0 {
        options.port.unwrap_or(22)
    } else {
        profile.port
    };
    let expand = |value: &str| expand(value, home, &host, &profile.host, &user, local_user, port);
    let identity_names = if options.identities.is_empty() {
        vec![
            "~/.ssh/id_ed25519".into(),
            "~/.ssh/id_ecdsa".into(),
            "~/.ssh/id_rsa".into(),
        ]
    } else {
        options.identities
    };
    let mut identities = Vec::new();
    if options.public_key != Some(false) {
        for name in identity_names {
            if name.eq_ignore_ascii_case("none") {
                continue;
            }
            let path = PathBuf::from(expand(&name)?);
            let path = if path.is_absolute() {
                path
            } else {
                home.join(path)
            };
            if !identities.contains(&path) {
                identities.push(path);
            }
        }
    }
    let agent = if options.public_key == Some(false) || options.agent.as_deref() == Some("none") {
        None
    } else {
        let configured = options.agent.filter(|value| value != "SSH_AUTH_SOCK");
        configured
            .map(|value| expand(&value).map(PathBuf::from))
            .transpose()?
            .or_else(|| std::env::var_os("SSH_AUTH_SOCK").map(PathBuf::from))
            .or_else(default_agent)
    };
    Ok(Resolved {
        host,
        user,
        implicit_user,
        port,
        identities,
        agent,
        identities_only: options.identities_only.unwrap_or(false),
        password: options.password.unwrap_or(true),
        keyboard: options.keyboard.unwrap_or(true),
        interactive: !options.batch.unwrap_or(false),
        methods: options.methods.unwrap_or_else(|| {
            vec![
                russh::MethodKind::PublicKey,
                russh::MethodKind::KeyboardInteractive,
                russh::MethodKind::Password,
            ]
        }),
    })
}
fn default_agent() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        Some(PathBuf::from(r"\\.\pipe\openssh-ssh-agent"))
    }
    #[cfg(not(windows))]
    {
        None
    }
}
fn host_matches(patterns: &[String], host: &str) -> Result<bool> {
    let mut positive = false;
    for pattern in patterns {
        let (negative, pattern) = pattern
            .strip_prefix('!')
            .map_or((false, pattern.as_str()), |p| (true, p));
        let matched = glob::Pattern::new(pattern)?.matches_with(
            host,
            glob::MatchOptions {
                case_sensitive: false,
                require_literal_separator: false,
                require_literal_leading_dot: false,
            },
        );
        if matched && negative {
            return Ok(false);
        }
        positive |= matched && !negative;
    }
    Ok(positive)
}
#[allow(clippy::too_many_arguments)]
fn parse_file(
    path: &Path,
    host: &str,
    home: &Path,
    options: &mut Options,
    active: &mut bool,
    stack: &mut Vec<PathBuf>,
    count: &mut usize,
) -> Result<()> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).with_context(|| format!("Reading SSH config {}", path.display())),
    };
    ensure!(
        text.len() <= 1024 * 1024,
        "SSH config is too large: {}",
        path.display()
    );
    let path = fs::canonicalize(path)?;
    ensure!(
        !stack.contains(&path) && stack.len() < 8 && *count < 64,
        "SSH config Include cycle or limit exceeded: {}",
        path.display()
    );
    stack.push(path.clone());
    *count += 1;
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let split = line
            .find(|c: char| c.is_whitespace() || c == '=')
            .unwrap_or(line.len());
        let name = line[..split].to_ascii_lowercase();
        let rest = line[split..]
            .trim_start()
            .strip_prefix('=')
            .unwrap_or(line[split..].trim_start())
            .trim_start();
        let args =
            words(rest).with_context(|| format!("SSH config {}:{}", path.display(), index + 1))?;
        if args.is_empty() {
            continue;
        }
        if name == "host" {
            *active = host_matches(&args, host)?;
            continue;
        }
        if name == "match" {
            bail!(
                "SSH config Match blocks are not supported yet ({}:{})",
                path.display(),
                index + 1
            );
        }
        if !*active {
            continue;
        }
        let one = || -> Result<&str> {
            ensure!(
                args.len() == 1,
                "Expected one value for {name} at {}:{}",
                path.display(),
                index + 1
            );
            Ok(&args[0])
        };
        match name.as_str() {
            "hostname" => {
                if options.host.is_none() {
                    options.host = Some(one()?.into());
                }
            }
            "user" => {
                if options.user.is_none() {
                    options.user = Some(one()?.into());
                }
            }
            "port" => {
                if options.port.is_none() {
                    let port = one()?.parse::<u16>().context("Invalid SSH config Port")?;
                    ensure!(port > 0, "Invalid SSH config Port");
                    options.port = Some(port);
                }
            }
            "identityfile" => options.identities.push(one()?.into()),
            "batchmode" => {
                if options.batch.is_none() {
                    options.batch = Some(boolean(one()?)?);
                }
            }
            "preferredauthentications" => {
                if options.methods.is_none() {
                    let mut methods = Vec::new();
                    for method in one()?.split(',') {
                        let kind = method.parse::<russh::MethodKind>().map_err(|_| {
                            anyhow::anyhow!("Unsupported SSH authentication method: {method}")
                        })?;
                        ensure!(
                            matches!(
                                kind,
                                russh::MethodKind::PublicKey
                                    | russh::MethodKind::Password
                                    | russh::MethodKind::KeyboardInteractive
                            ),
                            "SSH authentication method {method} is not supported yet"
                        );
                        if !methods.contains(&kind) {
                            methods.push(kind);
                        }
                    }
                    options.methods = Some(methods);
                }
            }
            "identityagent" => {
                if options.agent.is_none() {
                    options.agent = Some(one()?.into());
                }
            }
            "identitiesonly" => {
                if options.identities_only.is_none() {
                    options.identities_only = Some(boolean(one()?)?);
                }
            }
            "passwordauthentication" => {
                if options.password.is_none() {
                    options.password = Some(boolean(one()?)?);
                }
            }
            "kbdinteractiveauthentication" | "challengeresponseauthentication" => {
                if options.keyboard.is_none() {
                    options.keyboard = Some(boolean(one()?)?);
                }
            }
            "pubkeyauthentication" => {
                if options.public_key.is_none() {
                    options.public_key = Some(boolean(one()?)?);
                }
            }
            "include" => {
                for pattern in args {
                    let pattern = if let Some(suffix) = pattern
                        .strip_prefix("~/")
                        .or_else(|| pattern.strip_prefix("~\\"))
                    {
                        home.join(suffix)
                    } else {
                        let p = PathBuf::from(pattern);
                        if p.is_absolute() {
                            p
                        } else {
                            home.join(".ssh").join(p)
                        }
                    };
                    let mut paths = glob::glob(&pattern.to_string_lossy().replace('\\', "/"))?
                        .collect::<std::result::Result<Vec<_>, _>>()?;
                    paths.sort();
                    for included in paths {
                        // Like OpenSSH, Host lines inside an included file do not
                        // change which block the including file's later lines are in.
                        let outer = *active;
                        parse_file(&included, host, home, options, active, stack, count)?;
                        *active = outer;
                    }
                }
            }
            "proxyjump"
            | "proxycommand"
            | "certificatefile"
            | "pkcs11provider"
            | "securitykeyprovider"
            | "hostkeyalias"
            | "userknownhostsfile"
            | "globalknownhostsfile"
            | "knownhostscommand"
            | "ciphers"
            | "kexalgorithms"
            | "hostkeyalgorithms"
            | "pubkeyacceptedalgorithms" => bail!(
                "SSH config {name} is not supported yet ({}:{}); Ghostlink cannot apply this connection setting",
                path.display(),
                index + 1
            ),
            // Shell integration, forwarding, presentation, and OpenSSH process settings
            // do not change this direct terminal transport's address or identity discovery.
            _ => {}
        }
    }
    stack.pop();
    Ok(())
}
fn boolean(value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "yes" => Ok(true),
        "no" => Ok(false),
        _ => bail!("Expected yes or no in SSH config"),
    }
}
fn words(line: &str) -> Result<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' && quote.is_some() && chars.peek().copied() == quote {
            word.push(chars.next().unwrap());
            continue;
        }
        if quote == Some(ch) {
            quote = None;
            continue;
        }
        if quote.is_none() {
            match ch {
                '#' => break,
                '\'' | '"' => {
                    quote = Some(ch);
                    continue;
                }
                c if c.is_whitespace() => {
                    if !word.is_empty() {
                        words.push(std::mem::take(&mut word));
                    }
                    continue;
                }
                _ => {}
            }
        }
        word.push(ch);
    }
    ensure!(quote.is_none(), "Unclosed quote in SSH config");
    if !word.is_empty() {
        words.push(word);
    }
    Ok(words)
}
#[allow(clippy::too_many_arguments)]
fn expand(
    value: &str,
    home: &Path,
    host: &str,
    original: &str,
    user: &str,
    local: &str,
    port: u16,
) -> Result<String> {
    let home = home.to_string_lossy();
    let mut result = String::new();
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '%' {
            result.push_str(match chars.next() {
                Some('%') => "%",
                Some('d') => &home,
                Some('h') => host,
                Some('n') => original,
                Some('r') => user,
                Some('u') => local,
                Some('p') => {
                    result.push_str(&port.to_string());
                    continue;
                }
                _ => bail!("Unsupported token in SSH identity path"),
            });
        } else if ch == '$' && chars.peek() == Some(&'{') {
            chars.next();
            let mut name = String::new();
            loop {
                match chars.next() {
                    Some('}') => break,
                    Some(c) => name.push(c),
                    None => bail!("Unclosed environment variable in SSH config"),
                }
            }
            result.push_str(
                &std::env::var(&name).with_context(|| {
                    format!("SSH config environment variable {name} is not set")
                })?,
            );
        } else {
            result.push(ch);
        }
    }
    if result == "~" {
        return Ok(home.into_owned());
    }
    if let Some(suffix) = result
        .strip_prefix("~/")
        .or_else(|| result.strip_prefix("~\\"))
    {
        return Ok(PathBuf::from(home.as_ref())
            .join(suffix)
            .to_string_lossy()
            .into_owned());
    }
    ensure!(
        !result.starts_with('~'),
        "Only the current user's ~ home expansion is supported"
    );
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile() -> SshProfile {
        SshProfile {
            host: "prod".into(),
            port: 0,
            username: String::new(),
            auth: SshAuth::Automatic {
                home: PathBuf::new(),
            },
            known_hosts: "unused".into(),
        }
    }
    #[test]
    fn aliases_includes_precedence_and_identity_tokens() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        fs::create_dir(home.join(".ssh")).unwrap();
        fs::create_dir(home.join(".ssh/config.d")).unwrap();
        fs::write(
            home.join(".ssh/config"),
            "Include config.d/*\nHost *\n User default\n Port 22\n IdentityFile ~/.ssh/id_rsa\n",
        )
        .unwrap();
        fs::write(home.join(".ssh/config.d/01-prod"),"Host prod !staging\n HostName=127.0.0.1\n User deploy\n Port 2222\n IdentityFile \"~/.ssh/%n key\"\n IdentitiesOnly yes\n IdentityAgent none\n").unwrap();
        let result = resolve_with_user(&profile(), home, "local").unwrap();
        assert_eq!(result.host, "127.0.0.1");
        assert_eq!(result.user, "deploy");
        assert_eq!(result.port, 2222);
        assert_eq!(
            result.identities,
            vec![home.join(".ssh/prod key"), home.join(".ssh/id_rsa")]
        );
        assert!(result.identities_only);
        assert!(result.agent.is_none());
        let mut explicit = profile();
        explicit.username = "override".into();
        explicit.port = 2200;
        let result = resolve_with_user(&explicit, home, "local").unwrap();
        assert_eq!(result.user, "override");
        assert_eq!(result.port, 2200);
    }
    #[test]
    fn included_host_blocks_do_not_capture_the_including_files_later_lines() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        fs::create_dir_all(home.join(".ssh/config.d")).unwrap();
        fs::write(
            home.join(".ssh/config"),
            "Include config.d/*\nUser remote\n",
        )
        .unwrap();
        fs::write(home.join(".ssh/config.d/other"), "Host other\n Port 2222\n").unwrap();
        let result = resolve_with_user(&profile(), home, "local").unwrap();
        assert_eq!(result.user, "remote");
        assert!(!result.implicit_user);
        assert_eq!(result.port, 22);
    }
    #[test]
    fn defaults_do_not_require_config_or_agent() {
        let temp = tempfile::tempdir().unwrap();
        let result = resolve_with_user(&profile(), temp.path(), "local").unwrap();
        assert_eq!(result.user, "local");
        assert!(result.implicit_user);
        assert_eq!(result.port, 22);
        assert_eq!(result.identities.len(), 3);
        assert!(result.identities[0].ends_with(".ssh/id_ed25519"));
    }
    #[test]
    fn unsafe_or_unimplemented_routing_and_include_cycles_are_not_silently_ignored() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join(".ssh")).unwrap();
        let path = temp.path().join(".ssh/config");
        for text in [
            "Host prod\n ProxyCommand arbitrary-command\n",
            "Include config\n",
        ] {
            fs::write(&path, text).unwrap();
            assert!(resolve_with_user(&profile(), temp.path(), "local").is_err());
        }
    }
    #[test]
    fn negation_and_quoted_windows_paths() {
        assert!(!host_matches(&["*".into(), "!prod".into()], "prod").unwrap());
        assert!(host_matches(&["PRO?".into()], "prod").unwrap());
        assert_eq!(
            words(r#""C:\Users\name\my key" # comment"#).unwrap(),
            vec![r"C:\Users\name\my key"]
        );
    }
}

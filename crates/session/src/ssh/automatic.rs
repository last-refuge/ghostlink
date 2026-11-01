//! Opinionated authentication: discovered identities, then server-requested credentials.
use super::*;
use keys::{agent::AgentIdentity, ssh_encoding::Encode};
use russh::{MethodKind, MethodSet, Signer};

pub(super) struct Prompter {
    updates: Sender<Update>,
    stale: Receiver<Update>,
    replies: Receiver<CredentialReply>,
    next_id: u64,
}
impl Prompter {
    pub fn new(
        updates: &Sender<Update>,
        stale: &Receiver<Update>,
        replies: Receiver<CredentialReply>,
    ) -> Self {
        Self {
            updates: updates.clone(),
            stale: stale.clone(),
            replies,
            next_id: 0,
        }
    }
    async fn ask(
        &mut self,
        label: String,
        detail: String,
        secret: bool,
    ) -> Result<Zeroizing<String>> {
        self.next_id += 1;
        let id = self.next_id;
        publish(
            &self.updates,
            &self.stale,
            Update {
                screen: None,
                state: State::AwaitingCredential(CredentialPrompt {
                    id,
                    label,
                    detail,
                    secret,
                }),
            },
        );
        let reply = tokio::time::timeout(Duration::from_secs(300), self.replies.recv())
            .await
            .context("Authentication prompt timed out")?
            .context("Authentication cancelled")?;
        ensure!(reply.id == id, "Stale authentication response");
        let value = reply.value.context("Authentication cancelled")?;
        publish(
            &self.updates,
            &self.stale,
            Update {
                screen: None,
                state: State::Authenticating,
            },
        );
        Ok(value)
    }
}
fn auth_result(result: client::AuthResult, remaining: &mut MethodSet) -> bool {
    match result {
        client::AuthResult::Success => true,
        client::AuthResult::Failure {
            remaining_methods, ..
        } => {
            *remaining = remaining_methods;
            false
        }
    }
}
async fn network<T>(
    future: impl std::future::Future<Output = std::result::Result<T, russh::Error>>,
) -> Result<T> {
    tokio::time::timeout(Duration::from_secs(30), future)
        .await
        .context("SSH authentication timed out")?
        .map_err(Into::into)
}
pub(super) async fn authenticate(
    connection: &mut client::Handle<Verifier>,
    username: &str,
    plan: config::Resolved,
    mut prompts: Prompter,
) -> Result<()> {
    let mut remaining = MethodSet::empty();
    if auth_result(
        network(connection.authenticate_none(username)).await?,
        &mut remaining,
    ) {
        return Ok(());
    }
    let mut candidates = Vec::<(PathBuf, PublicKey)>::new();
    let mut errors = Vec::new();
    for path in &plan.identities {
        if !path.is_file() {
            continue;
        }
        let public = public_key(path);
        match public {
            Ok(key) => candidates.push((path.clone(), key)),
            Err(error) => errors.push(format!("{}: {error}", path.display())),
        }
    }
    let mut rejected = Vec::<String>::new();
    let mut agent_rejected = false;
    for method in plan.methods.iter().copied() {
        if method == MethodKind::PublicKey && remaining.contains(&MethodKind::PublicKey) {
            let (result, offered) = agent_auth(connection, username, &plan, &candidates).await?;
            if let Some(result) = result {
                if auth_result(result, &mut remaining) {
                    return Ok(());
                }
                agent_rejected |= offered > 0;
            }
            let hash = network(connection.best_supported_rsa_hash())
                .await?
                .flatten();
            for (path, public) in &candidates {
                if !remaining.contains(&MethodKind::PublicKey) {
                    break;
                }
                let mut signer = FileSigner {
                    path: path.clone(),
                    prompts: &mut prompts,
                    interactive: plan.interactive,
                };
                // The signer is called only after the server accepts this public key.
                // A passphrase prompt therefore never appears for an unrelated key.
                let result = tokio::time::timeout(
                    Duration::from_secs(360),
                    connection.authenticate_publickey_with(
                        username,
                        public.clone(),
                        hash,
                        &mut signer,
                    ),
                )
                .await
                .context("Key authentication timed out")??;
                if auth_result(result, &mut remaining) {
                    return Ok(());
                }
                rejected.push(
                    path.file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
        let hint = rejection_hint(username, &plan, &rejected, agent_rejected);
        if method == MethodKind::KeyboardInteractive
            && plan.interactive
            && plan.keyboard
            && remaining.contains(&MethodKind::KeyboardInteractive)
        {
            let mut response =
                network(connection.authenticate_keyboard_interactive_start(username, None)).await?;
            let mut rounds = 0;
            loop {
                response = match response {
                    client::KeyboardInteractiveAuthResponse::Success => return Ok(()),
                    client::KeyboardInteractiveAuthResponse::Failure {
                        remaining_methods, ..
                    } => {
                        remaining = remaining_methods;
                        break;
                    }
                    client::KeyboardInteractiveAuthResponse::InfoRequest {
                        prompts: questions,
                        ..
                    } => {
                        rounds += 1;
                        ensure!(
                            rounds <= 12,
                            "SSH server requested too many authentication rounds"
                        );
                        ensure!(
                            questions.len() <= 8,
                            "SSH server requested too many authentication fields"
                        );
                        let mut answers = Vec::new();
                        for question in questions {
                            let value = prompts
                                .ask(
                                    question.prompt.chars().take(256).collect(),
                                    join(
                                        format!("SSH authentication for {username}@{}.", plan.host),
                                        &hint,
                                    ),
                                    !question.echo,
                                )
                                .await?;
                            answers.push(value.to_string());
                        }
                        network(connection.authenticate_keyboard_interactive_respond(answers))
                            .await?
                    }
                };
            }
        }
        if method == MethodKind::Password
            && plan.interactive
            && plan.password
            && remaining.contains(&MethodKind::Password)
        {
            for attempt in 0..3 {
                let detail = if attempt == 0 {
                    join(format!("Password for {username}@{}.", plan.host), &hint)
                } else {
                    join(
                        format!(
                            "That password was not accepted for {username}@{}. Try again.",
                            plan.host
                        ),
                        &hint,
                    )
                };
                let password = prompts.ask("Password".into(), detail, true).await?;
                if auth_result(
                    network(connection.authenticate_password(username, password.as_str())).await?,
                    &mut remaining,
                ) {
                    return Ok(());
                }
                if !remaining.contains(&MethodKind::Password) {
                    break;
                }
            }
        }
    }
    let mut message = format!(
        "SSH authentication failed for {username}@{}. No available identity was accepted.",
        plan.host
    );
    let hint = rejection_hint(username, &plan, &rejected, agent_rejected);
    if hint.is_empty() {
        message.push_str(" Check this host's IdentityFile and User in ~/.ssh/config.");
    } else {
        message = join(message, &hint);
    }
    if !errors.is_empty() {
        message.push_str(&format!(
            " Key files that could not be read: {}",
            errors.join("; ")
        ));
    }
    bail!(message)
}

/// Explain a fallback to typed credentials. Without this, a key rejected because
/// Ghostlink signed in as the wrong account looks like a broken password prompt.
fn rejection_hint(
    username: &str,
    plan: &config::Resolved,
    rejected: &[String],
    agent_rejected: bool,
) -> String {
    let mut hint = String::new();
    if !rejected.is_empty() {
        hint = format!(
            "The server did not accept your SSH key{} ({}) for {username}.",
            if rejected.len() == 1 { "" } else { "s" },
            rejected.join(", ")
        );
    } else if agent_rejected {
        hint = format!("The server did not accept the keys in your SSH agent for {username}.");
    }
    if plan.implicit_user {
        hint = join(
            hint,
            &format!(
                "This connection does not name a remote user, so Ghostlink signed in as your local username \"{username}\". If your account on this server has a different name, cancel and change the destination to user@{}.",
                plan.host
            ),
        );
    }
    hint
}
fn join(first: String, second: &str) -> String {
    match (first.is_empty(), second.is_empty()) {
        (_, true) => first,
        (true, false) => second.into(),
        (false, false) => format!("{first} {second}"),
    }
}

fn public_key(path: &Path) -> Result<PublicKey> {
    let text = Zeroizing::new(fs::read_to_string(path).context("Reading SSH identity")?);
    if let Ok(key) = keys::PrivateKey::from_openssh(text.as_bytes()) {
        return Ok(key.public_key().clone());
    }
    let public = PathBuf::from(format!("{}.pub", path.display()));
    if public.is_file() {
        return keys::load_public_key(public).context("Reading public identity");
    }
    Ok(keys::decode_secret_key(&text, None)
        .context("Reading private identity; encrypted PEM keys need a matching .pub file")?
        .public_key()
        .clone())
}
struct FileSigner<'a> {
    path: PathBuf,
    prompts: &'a mut Prompter,
    interactive: bool,
}
impl Signer for FileSigner<'_> {
    type Error = anyhow::Error;
    async fn auth_sign(
        &mut self,
        identity: &AgentIdentity,
        hash: Option<HashAlg>,
        mut data: Vec<u8>,
    ) -> Result<Vec<u8>> {
        let text =
            Zeroizing::new(fs::read_to_string(&self.path).context("Reading private identity")?);
        let mut key = keys::decode_secret_key(&text, None);
        let encrypted = matches!(key, Err(keys::Error::KeyIsEncrypted))
            || text.contains("ENCRYPTED")
            || keys::PrivateKey::from_openssh(text.as_bytes()).is_ok_and(|key| key.is_encrypted());
        if key.is_err() && encrypted {
            ensure!(
                self.interactive,
                "This identity is encrypted and SSH config BatchMode disables passphrase prompts"
            );
            for attempt in 0..3 {
                let label = if attempt == 0 {
                    "Key passphrase"
                } else {
                    "Passphrase not accepted. Try again"
                };
                let passphrase = self
                    .prompts
                    .ask(
                        label.into(),
                        format!(
                            "Unlock {}",
                            self.path.file_name().unwrap_or_default().to_string_lossy()
                        ),
                        true,
                    )
                    .await?;
                key = keys::decode_secret_key(&text, Some(passphrase.as_str()));
                if key.is_ok() {
                    break;
                }
            }
        }
        let key = key.context("Could not unlock SSH identity")?;
        ensure!(
            key.public_key().key_data() == identity.public_key().key_data(),
            "The private key does not match its public identity"
        );
        let signature: keys::ssh_key::Signature = match key.key_data() {
            keys::ssh_key::private::KeypairData::Rsa(rsa) => {
                keys::signature::Signer::try_sign(&(rsa, hash), &data)?
            }
            key => keys::signature::Signer::try_sign(key, &data)?,
        };
        let mut encoded = Vec::new();
        signature.encode(&mut encoded)?;
        data.extend_from_slice(&(encoded.len() as u32).to_be_bytes());
        data.extend_from_slice(&encoded);
        Ok(data)
    }
}
async fn agent_auth(
    connection: &mut client::Handle<Verifier>,
    username: &str,
    plan: &config::Resolved,
    candidates: &[(PathBuf, PublicKey)],
) -> Result<(Option<client::AuthResult>, usize)> {
    let Some(path) = &plan.agent else {
        return Ok((None, 0));
    };
    #[cfg(windows)]
    let agent = tokio::time::timeout(
        Duration::from_millis(500),
        keys::agent::client::AgentClient::connect_named_pipe(path),
    )
    .await;
    #[cfg(unix)]
    let agent = tokio::time::timeout(
        Duration::from_millis(500),
        keys::agent::client::AgentClient::connect_uds(path),
    )
    .await;
    let Ok(Ok(mut agent)) = agent else {
        return Ok((None, 0));
    };
    let identities = tokio::time::timeout(Duration::from_secs(2), agent.request_identities()).await;
    let Ok(Ok(identities)) = identities else {
        return Ok((None, 0));
    };
    let mut keys = identities
        .into_iter()
        .filter_map(|identity| {
            if let AgentIdentity::PublicKey { key, .. } = identity {
                Some(key)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let matching = |key: &PublicKey| {
        candidates
            .iter()
            .any(|(_, candidate)| candidate.key_data() == key.key_data())
    };
    if plan.identities_only {
        keys.retain(matching);
    }
    keys.sort_by_key(|key| !matching(key));
    let hash = network(connection.best_supported_rsa_hash())
        .await?
        .flatten();
    let mut last = None;
    let mut offered = 0;
    for key in keys.into_iter().take(5) {
        offered += 1;
        let result = tokio::time::timeout(
            Duration::from_secs(30),
            connection.authenticate_publickey_with(username, key, hash, &mut agent),
        )
        .await
        .context("SSH agent authentication timed out")??;
        let done = result.success();
        last = Some(result);
        if done {
            break;
        }
    }
    Ok((last, offered))
}

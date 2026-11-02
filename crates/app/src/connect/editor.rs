use super::*;
use crate::host_options::{Authentication, Backspace, HostOptions};

pub(super) struct Editor {
    pub id: Option<u64>,
    pub charset_open: bool,
    pub charset_focus: FocusHandle,
    pub theme_focus: FocusHandle,
    pub appearance_anchor: Option<gpui::ScrollAnchor>,
    pub charset_cursor: usize,
    pub theme_cursor: usize,
    pub charset_scroll: gpui::ScrollHandle,
    pub theme_scroll: gpui::ScrollHandle,
    pub theme_open: bool,
    pub name: Entity<Field>,
    pub address: Entity<Field>,
    pub username: Entity<Field>,
    pub port: Entity<Field>,
    pub group: Entity<Field>,
    pub tags: Entity<Field>,
    pub password: Entity<Field>,
    pub key_file: Entity<Field>,
    pub keepalive: Entity<Field>,
    pub snippet: Entity<Field>,
    pub environment: Vec<(Entity<Field>, Entity<Field>)>,
    pub certificate: Entity<Field>,
    pub proxy_host: Entity<Field>,
    pub proxy_port: Entity<Field>,
    pub proxy_username: Entity<Field>,
    pub proxy_password: Entity<Field>,
    pub initial_proxy_password: zeroize::Zeroizing<String>,
    pub jump_search: Entity<Field>,
    pub options: HostOptions,
    initial_options: HostOptions,
    pub initial_password: zeroize::Zeroizing<String>,
    initial: Vec<String>,
}
impl Editor {
    pub fn new(
        host: Option<&SavedConnection>,
        address: &str,
        cx: &mut Context<ConnectForm>,
    ) -> Self {
        let mut editor = Self {
            id: host.map(|host| host.id),
            charset_open: false,
            charset_focus: cx.focus_handle(),
            theme_focus: cx.focus_handle(),
            appearance_anchor: None,
            charset_cursor: 0,
            theme_cursor: 0,
            charset_scroll: gpui::ScrollHandle::new(),
            theme_scroll: gpui::ScrollHandle::new(),
            theme_open: false,
            name: cx.new(|cx| Field::new(host.map_or("", |host| &host.label), false, cx)),
            address: cx.new(|cx| Field::new(host.map_or(address, |host| &host.address), false, cx)),
            username: cx.new(|cx| Field::new(host.map_or("", |host| &host.username), false, cx)),
            port: cx.new(|cx| {
                Field::new(
                    &host
                        .and_then(|host| host.port)
                        .map(|port| port.to_string())
                        .unwrap_or_default(),
                    false,
                    cx,
                )
            }),
            group: cx.new(|cx| Field::new(host.map_or("", |host| &host.options.group), false, cx)),
            tags: cx.new(|cx| {
                Field::new(
                    &host
                        .map(|host| host.options.tags.join(", "))
                        .unwrap_or_default(),
                    false,
                    cx,
                )
            }),
            password: cx.new(|cx| Field::new("", true, cx)),
            key_file: cx.new(|cx| {
                Field::new(
                    &host
                        .and_then(|host| match &host.options.authentication {
                            Authentication::KeyFile { path, .. } => {
                                Some(path.to_string_lossy().into_owned())
                            }
                            _ => None,
                        })
                        .unwrap_or_default(),
                    false,
                    cx,
                )
            }),
            keepalive: cx.new(|cx| {
                Field::new(
                    &host
                        .and_then(|host| host.options.keepalive_secs)
                        .map(|seconds| seconds.to_string())
                        .unwrap_or_default(),
                    false,
                    cx,
                )
            }),
            snippet: cx.new(|cx| {
                Field::new_multiline(host.map_or("", |host| &host.options.startup_snippet), cx)
            }),
            environment: host
                .map(|host| {
                    host.options
                        .environment
                        .iter()
                        .map(|(name, value)| {
                            (
                                cx.new(|cx| Field::new(name, false, cx)),
                                cx.new(|cx| Field::new(value, false, cx)),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
            certificate: cx.new(|cx| {
                Field::new(
                    &host
                        .and_then(|host| match &host.options.authentication {
                            Authentication::KeyFile { certificate, .. } => certificate
                                .as_ref()
                                .map(|path| path.to_string_lossy().into_owned()),
                            _ => None,
                        })
                        .unwrap_or_default(),
                    false,
                    cx,
                )
            }),
            proxy_host: cx.new(|cx| {
                Field::new(
                    host.and_then(|host| host.options.proxy.as_ref())
                        .map_or("", |proxy| &proxy.host),
                    false,
                    cx,
                )
            }),
            proxy_port: cx.new(|cx| {
                Field::new(
                    &host
                        .and_then(|host| host.options.proxy.as_ref())
                        .map(|proxy| proxy.port.to_string())
                        .unwrap_or_default(),
                    false,
                    cx,
                )
            }),
            proxy_username: cx.new(|cx| {
                Field::new(
                    host.and_then(|host| host.options.proxy.as_ref())
                        .map_or("", |proxy| &proxy.username),
                    false,
                    cx,
                )
            }),
            proxy_password: cx.new(|cx| Field::new("", true, cx)),
            initial_proxy_password: zeroize::Zeroizing::new(String::new()),
            jump_search: cx.new(|cx| Field::new("", false, cx)),
            initial_options: host.map(|host| host.options.clone()).unwrap_or_default(),
            initial_password: zeroize::Zeroizing::new(String::new()),
            options: host.map(|host| host.options.clone()).unwrap_or_default(),
            initial: Vec::new(),
        };
        editor.initial = editor.values(cx);
        editor
    }
    pub fn fields(&self) -> Vec<&Entity<Field>> {
        let mut fields = vec![
            &self.address,
            &self.name,
            &self.group,
            &self.tags,
            &self.username,
            &self.port,
        ];
        match self.options.authentication {
            Authentication::Password { .. } => fields.push(&self.password),
            Authentication::KeyFile { .. } => {
                fields.push(&self.key_file);
                fields.push(&self.certificate);
            }
            Authentication::Automatic => {}
        }
        fields.push(&self.jump_search);
        if let Some(proxy) = &self.options.proxy {
            fields.extend([&self.proxy_host, &self.proxy_port, &self.proxy_username]);
            if proxy.kind != crate::host_options::ProxyKind::Socks4 {
                fields.push(&self.proxy_password);
            }
        }
        fields.push(&self.snippet);
        for (name, value) in &self.environment {
            fields.push(name);
            fields.push(value);
        }
        fields.push(&self.keepalive);
        fields
    }
    pub fn all_fields(&self) -> Vec<&Entity<Field>> {
        let mut fields = vec![
            &self.address,
            &self.name,
            &self.group,
            &self.tags,
            &self.username,
            &self.port,
            &self.password,
            &self.key_file,
            &self.certificate,
            &self.snippet,
            &self.keepalive,
            &self.proxy_host,
            &self.proxy_port,
            &self.proxy_username,
            &self.proxy_password,
            &self.jump_search,
        ];
        for (name, value) in &self.environment {
            fields.push(name);
            fields.push(value);
        }
        fields
    }
    fn values(&self, cx: &App) -> Vec<String> {
        self.fields()
            .into_iter()
            .filter(|field| {
                field.entity_id() != self.password.entity_id()
                    && field.entity_id() != self.proxy_password.entity_id()
                    && field.entity_id() != self.jump_search.entity_id()
            })
            .map(|field| field.read(cx).value().to_owned())
            .collect()
    }
    pub fn dirty(&self, cx: &App) -> bool {
        self.values(cx) != self.initial
            || self.options != self.initial_options
            || self.password.read(cx).value() != self.initial_password.as_str()
            || self.proxy_password.read(cx).value() != self.initial_proxy_password.as_str()
    }
    pub fn host(&self, cx: &App) -> anyhow::Result<SavedConnection> {
        let port = self.port.read(cx).value().trim();
        let port = if port.is_empty() {
            None
        } else {
            Some(
                port.parse::<u16>()
                    .map_err(|_| anyhow::anyhow!("SSH port must be between 1 and 65535"))?,
            )
        };
        let address = self.address.read(cx).value().trim().to_owned();
        let username = self.username.read(cx).value().trim().to_owned();
        let label = self.name.read(cx).value().trim().to_owned();
        let mut options = self.options.clone();
        if let Authentication::KeyFile { path, certificate } = &mut options.authentication {
            *path = self.key_file.read(cx).value().trim().into();
            let selected = self.certificate.read(cx).value().trim();
            *certificate = if selected.is_empty() {
                None
            } else {
                Some(selected.into())
            };
        }
        if let Some(proxy) = &mut options.proxy {
            proxy.host = self.proxy_host.read(cx).value().trim().to_owned();
            proxy.port = self
                .proxy_port
                .read(cx)
                .value()
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("Proxy port must be between 1 and 65535"))?;
            proxy.username = self.proxy_username.read(cx).value().trim().to_owned();
        }
        options.startup_snippet = self.snippet.read(cx).value().to_owned();
        options.environment = self
            .environment
            .iter()
            .map(|(name, value)| {
                (
                    name.read(cx).value().trim().to_owned(),
                    value.read(cx).value().to_owned(),
                )
            })
            .collect();
        let keepalive = self.keepalive.read(cx).value().trim();
        options.keepalive_secs =
            if keepalive.is_empty() {
                None
            } else {
                Some(keepalive.parse::<u64>().map_err(|_| {
                    anyhow::anyhow!("Keep-alive must be between 0 and 86400 seconds")
                })?)
            };
        options.group = self.group.read(cx).value().trim().to_owned();
        options.tags = self
            .tags
            .read(cx)
            .value()
            .split(',')
            .map(str::trim)
            .filter(|tag| !tag.is_empty())
            .map(str::to_owned)
            .collect();
        let mut host = SavedConnection {
            id: self.id.unwrap_or(0),
            label,
            address,
            port,
            username,
            options,
        };
        if host.label.is_empty() {
            host.label = host.destination();
        }
        Ok(host)
    }
}

pub(super) fn field(id: &'static str, label: &str, input: &Entity<Field>) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .gap_1()
        .child(
            div()
                .text_xs()
                .text_color(rgb(0x9aada4))
                .child(label.to_owned()),
        )
        .child(
            div()
                .id(id)
                .debug_selector(move || id.into())
                .child(input.clone()),
        )
}
impl ConnectForm {
    pub(super) fn render_editor(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let Some(editor) = &self.editor else {
            return div()
                .flex_1()
                .p_5()
                .text_color(rgb(0x9aada4))
                .child("Choose a host or add a new connection.");
        };
        let local_user = if cfg!(windows) {
            std::env::var("USERNAME").or_else(|_| std::env::var("USER"))
        } else {
            std::env::var("USER").or_else(|_| std::env::var("LOGNAME"))
        }
        .unwrap_or_else(|_| "current user".into());
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child(
                if editor.id.is_some() {
                    "Host details"
                } else {
                    "New host"
                },
            ))
            .child(field(
                "connection-destination",
                "Address or SSH config alias",
                &editor.address,
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0x8e9b9e))
                    .child("Enter only the hostname or IP address. Set the remote account below."),
            )
            .child(
                div()
                    .mt_2()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("General"),
            )
            .child(field("connection-name", "Label", &editor.name))
            .child(field("connection-group", "Parent group", &editor.group))
            .child(div().flex().flex_wrap().gap_1().children(self.entries.iter()
                .map(|host| host.options.group.clone()).filter(|group| !group.is_empty())
                .collect::<std::collections::BTreeSet<_>>().into_iter()
                .filter(|group| group != editor.group.read(cx).value() && group.to_lowercase().starts_with(&editor.group.read(cx).value().to_lowercase()))
                .take(5).map(|group| {
                    div().id(gpui::SharedString::from(format!("suggest-group-{group}"))).px_2().py_1().text_xs().rounded_sm().bg(rgb(0x293438)).cursor_pointer().child(group.clone())
                        .on_click(cx.listener(move |this, _, _, cx| { if let Some(editor) = &this.editor { editor.group.update(cx, |field, cx| field.set_value(&group, cx)); } cx.notify(); }))
                })))
            .child(field(
                "connection-tags",
                "Tags (separate with commas)",
                &editor.tags,
            ))
            .child(div().flex().flex_wrap().gap_1().children(editor.tags.read(cx).value().split(',').map(str::trim).filter(|tag| !tag.is_empty()).map(str::to_owned).collect::<std::collections::BTreeSet<_>>().into_iter().map(|tag| {
                div().id(gpui::SharedString::from(format!("tag-{tag}"))).px_2().py_1().text_xs().rounded_sm().bg(rgb(0x293e36)).cursor_pointer().child(format!("{tag} ×"))
                    .on_click(cx.listener(move |this, _, _, cx| { if let Some(editor) = &this.editor {
                        let tags = editor.tags.read(cx).value().split(',').map(str::trim).filter(|value| !value.is_empty() && *value != tag).collect::<Vec<_>>().join(", ");
                        editor.tags.update(cx, |field, cx| field.set_value(&tags, cx));
                    } cx.notify(); }))
            })))
            .child(div().text_xs().text_color(rgb(0x9aada4)).child("Backspace mode"))
            .child(div().flex().gap_2().children([(false, "Delete (DEL)"), (true, "Control-H")].into_iter().map(|(control_h, label)| {
                let selected = matches!(editor.options.backspace, Backspace::ControlH) == control_h;
                div().id(("backspace-mode", control_h as usize)).px_3().py_2().rounded_sm().cursor_pointer().bg(rgb(if selected { 0x293e36 } else { 0x171c1f })).child(label)
                    .on_click(cx.listener(move |this, _, _, cx| { if let Some(editor) = &mut this.editor { editor.options.backspace = if control_h { Backspace::ControlH } else { Backspace::Delete }; } cx.notify(); }))
            })))
            .child(div().mt_2().font_weight(FontWeight::SEMIBOLD).child("SSH"))
            .child(field("connection-username", "Username", &editor.username))
            .child(div().text_xs().text_color(rgb(0x8e9b9e)).child(format!(
                "When blank, uses SSH config or local user ({local_user})."
            )))
            .child(field(
                "connection-port",
                "SSH port (blank uses SSH config or 22)",
                &editor.port,
            ))
            .child(div().mt_2().font_weight(FontWeight::SEMIBOLD).child("Authentication"))
            .child(div().flex().flex_wrap().gap_1()
                .children([(0, "Automatic"), (1, "Password"), (2, "Key file")].into_iter().map(|(mode, title)| {
                    let selected = matches!((&editor.options.authentication, mode), (Authentication::Automatic, 0) | (Authentication::Password { .. }, 1) | (Authentication::KeyFile { .. }, 2));
                    div().id(("auth-mode", mode as usize)).px_3().py_2().rounded_sm().cursor_pointer()
                        .bg(rgb(if selected { 0x293e36 } else { 0x171c1f })).child(title)
                        .on_click(cx.listener(move |this, _, window, cx| { if let Some(editor) = &mut this.editor {
                            editor.options.authentication = match mode { 1 => Authentication::Password { remember: false }, 2 => Authentication::KeyFile { path: editor.key_file.read(cx).value().into(), certificate: None }, _ => Authentication::Automatic };
                        } this.focus_editor(window, cx); cx.notify(); }))
                })))
            .when(matches!(editor.options.authentication, Authentication::Password { .. }), |root| {
                let remember = matches!(editor.options.authentication, Authentication::Password { remember: true });
                root.child(field("connection-password", "Password (leave blank to ask when connecting)", &editor.password))
                    .child(div().id("remember-password").py_2().cursor_pointer()
                        .child(if remember { "☑ Remember accepted password in system keychain" } else { "☐ Remember accepted password in system keychain" })
                        .on_click(cx.listener(|this, _, _, cx| { if let Some(editor) = &mut this.editor && let Authentication::Password { remember } = &mut editor.options.authentication { *remember = !*remember; } cx.notify(); })))
                    .when_some(editor.id.and_then(|id| ghostlink_platform::secrets::last_error(id, ghostlink_platform::secrets::SecretKind::Password)), |root, error| root.child(div().text_xs().text_color(rgb(0xd7b77d)).child(error)))
            })
            .when(matches!(editor.options.authentication, Authentication::KeyFile { .. }), |root| {
                root.child(field("connection-key", "SSH private key file", &editor.key_file))
                    .child(action("browse-key", "Browse…", false).on_click(cx.listener(|this, _, _, cx| this.browse_key(cx))))
                    .child(field("connection-certificate", "OpenSSH user certificate (optional)", &editor.certificate))
                    .child(action("browse-certificate", "Browse certificate…", false).on_click(cx.listener(|this, _, _, cx| this.browse_certificate(cx))))
                    .child(div().text_xs().text_color(rgb(0x9aada4)).child("When blank, a matching <key>-cert.pub file is discovered automatically."))
            })
            .child(div().mt_2().font_weight(FontWeight::SEMIBOLD).child("Advanced"))
            .child(self.render_appearance(window, cx))
            .child(self.render_routing(cx))
            .child(div().id("agent-forwarding").py_2().cursor_pointer()
                .child(if editor.options.agent_forwarding { "☑ Forward local SSH agent" } else { "☐ Forward local SSH agent" })
                .on_click(cx.listener(|this, _, _, cx| { if let Some(editor) = &mut this.editor { editor.options.agent_forwarding = !editor.options.agent_forwarding; } cx.notify(); })))
            .child(div().text_xs().text_color(rgb(0x9aada4)).child("Enable only for hosts you trust to request signatures from your local agent."))
            .child(field("connection-startup", "Startup snippet (runs in each new shell)", &editor.snippet))
            .child(div().text_xs().text_color(rgb(0x9aada4)).child("Environment variables"))
            .children(editor.environment.iter().enumerate().map(|(index, (name, value))| {
                div().id(("environment", name.entity_id())).flex().items_end().gap_2()
                    .child(field("env-name", "Name", name).flex_1().min_w_0())
                    .child(field("env-value", "Value", value).flex_1().min_w_0())
                    .child(div().id("remove-env").px_2().py_2().cursor_pointer().child("×")
                        .on_click(cx.listener(move |this, _, window, cx| { if let Some(editor) = &mut this.editor { editor.environment.remove(index); } this.watch_editor(cx); this.focus_editor(window, cx); cx.notify(); })))
            }))
            .child(action("add-environment", "+ Add variable", false).on_click(cx.listener(|this, _, _, cx| {
                if let Some(editor) = &mut this.editor {
                    if editor.environment.len() >= 128 { this.error = Some("Use up to 128 environment variables.".into()); }
                    else { editor.environment.push((cx.new(|cx| Field::new("", false, cx)), cx.new(|cx| Field::new("", false, cx)))); }
                }
                this.watch_editor(cx); cx.notify();
            })))
            .child(div().text_xs().text_color(rgb(0x9aada4)).child("The server must permit these names in AcceptEnv. Values are stored with host details."))
            .child(field("connection-keepalive", "Keep-alive seconds (blank: 30, zero: disabled)", &editor.keepalive))
            .child(div().text_xs().text_color(rgb(0x9aada4)).child("Passwords are held only in memory unless you choose the system keychain. SSH keys and agent access follow your SSH config."))
    }
}

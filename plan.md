# Plan: Termius-style connection editor + per-host SSH options

## Context
Ghostlink's connection editor is two text boxes (name + free-form "Host or SSH alias") and the saved record is just `{id, name, destination}` (`crates/app/src/connections.rs`). That hid the username inside a string and made the app’s authentication behavior difficult to inspect. The confirmed terminal login used public-key authentication; Ghostlink must discover and use the same available identities. The user wants a friendlier editor modeled on Termius's Host Details panel, with the same per-host options, working on Windows, macOS, and Linux/Arch/Omarchy.

Decisions already made with the user:
- Layout: **connections list on the left, editor panel on the right**.
- Passwords: **optionally saved in the OS keychain** (never in Ghostlink's files).
- **Mosh: deferred.** **Share this host: left out.**

In scope (Termius parity): Address · Label · Parent group · Tags · Backspace mode · SSH port · Username · Password · SSH key / certificate (FIDO2 keys via the SSH agent) · Agent forwarding · Startup snippet · Host chaining (jump hosts) · Proxy · Environment variables · Charset · Per-host terminal theme. Plus keep-alive.

What already exists to reuse (verified):
- russh 0.64.1 has `channel_open_direct_tcpip` + `ChannelStream` + `client::connect_stream` (jump hosts), `set_env`, `agent_forward` + `Handler::server_channel_open_agent_forward`, `authenticate_openssh_cert`.
- libghostty exposes `GHOSTTY_TERMINAL_OPT_COLOR_PALETTE` (256 colors) next to the FG/BG/cursor options already set in `crates/ghostty-sys/src/bridge.c` (`gl_terminal_new`).
- GPUI 0.2.2 `cx.prompt_for_paths` = native file picker on all 3 OSes (Linux via xdg-desktop-portal, already a documented runtime dependency).
- `encoding_rs` and `tokio-socks` are already in the dependency cache.
- Existing automatic auth (`crates/session/src/ssh/automatic.rs`), SSH config resolution (`ssh/config.rs`), host-key `Verifier` (`ssh.rs`), and the russh test fixture (`ssh/tests.rs`).

## Design

### 1. Saved-connection model v2 — `crates/app/src/connections.rs`
- `Catalog.version = 2`. `SavedConnection` fields: `id, label, address, port: Option<u16>, username, group, tags: Vec<String>, backspace (Delete|ControlH), auth (Automatic | Password{remember} | KeyFile{path, certificate: Option<PathBuf>}), agent_forwarding, startup_snippet, jump_hosts: Vec<u64>, proxy: Option<Proxy{kind: Socks5|Socks4|Http, host, port, username}>, env: Vec<(String,String)>, charset, theme, keepalive_secs`. New fields use `serde(default)`.
- **Automatic migration v1 → v2**: split `destination` with the existing parser in `SshProfile::from_destination` into address/port/username; `name` → `label`. SSH-config aliases stay valid in `address`.
- Validation: jump-host ids must exist, no cycles, max 8 hops; deleting a host used as a jump host is blocked with a message naming the hosts that use it. Drop the "unique destination" rule (Termius allows duplicates).
- `SavedConnection::to_profile(&catalog)` builds the `SshProfile` (including jump-hop profiles) — single place the UI turns a saved host into a session.
- No secrets are ever written to `connections.json`.

### 2. OS keychain — `crates/platform/src/secrets.rs` (new)
- `keyring` v3 with `apple-native` (macOS Keychain), `windows-native` (Credential Manager), `async-secret-service` + `tokio` + `crypto-rust` (pure-Rust D-Bus Secret Service: GNOME Keyring/KWallet, no libdbus build dependency).
- Entries: service `io.github.last-refuge.ghostlink`, account `host:<id>:password` / `host:<id>:proxy`.
- Any keychain error = "unavailable": Ghostlink falls back to prompting and shows a one-line notice in the editor ("No system keychain found — Ghostlink will ask each time"). Secrets deleted when the host is deleted.

### 3. Session layer — `crates/session/src/ssh.rs` (+ new modules)
- Add `SshOptions` to `SshProfile` (default = today's behavior): `identity_file, certificate, saved_password, on_password_accepted (callback to store in keychain), agent_forwarding, startup_snippet, env, backspace, charset, keepalive, proxy, jump: Vec<SshProfile>`.
- **Key file / certificate**: explicit key goes first in `config::Resolved.identities` (like `ssh -i`); a certificate (explicit or `<key>-cert.pub`) uses `authenticate_openssh_cert`.
- **Saved password**: in `automatic.rs` password step, try the saved password once before prompting; when a typed password succeeds and "remember" is on, call `on_password_accepted` (keychain write off the UI thread).
- **Proxy** — `ssh/proxy.rs`: SOCKS5 (no-auth / user+pass) and SOCKS4a via `tokio-socks`; HTTP CONNECT (Basic auth) hand-written. Replaces the direct `TcpStream::connect` in `authenticate_connection`.
- **Host chaining** — refactor `authenticate_connection` into a per-hop `open_hop(profile, stream)`: connect hop 1 (via its proxy if any), authenticate with that hop's own settings, then `channel_open_direct_tcpip(next)` → `into_stream()` → `connect_stream` for the next hop. Host-key and credential prompts name the hop. All hop handles are kept in `ConnectionInner` so tabs/reconnect keep working.
- **Agent forwarding**: `agent_forward(true)` on each session channel when enabled (refusal is non-fatal); `Verifier::server_channel_open_agent_forward` bridges to the local agent — Unix socket (`SSH_AUTH_SOCK`/`IdentityAgent`, reusing `config.rs` resolution) or Windows `\\.\pipe\openssh-ssh-agent` — with `copy_bidirectional`.
- **Environment variables**: `set_env` before `request_shell`; refused names shown as one notice line in the terminal (servers only accept names listed in `AcceptEnv`).
- **Startup snippet**: sent line by line after the shell is accepted, for every new terminal channel.
- **Backspace**: `ControlH` sends `0x08` instead of `0x7f` for an unmodified Backspace in the SSH input path (`Input::Key` handling in `ssh.rs`).
- **Charset** — `session/src/charset.rs`: stateful `encoding_rs` decoder before `terminal.feed` and encoder on outgoing bytes (keys, paste, terminal replies). UTF-8 is a zero-cost passthrough. List: UTF-8, ISO-8859-1…15, Windows-1250…1258, KOI8-R/U, Shift_JIS, EUC-JP, EUC-KR, GB18030, Big5.
- **Keep-alive**: replaces the hard-coded 30 s in `client::Config`.

### 4. Per-host themes — `bridge.c`, `crates/terminal-core`, `crates/terminal-view`
- `gl_terminal_set_colors(fg, bg, cursor, palette16)` sets the default 256-color palette (first 16 from the theme, rest from libghostty's defaults).
- `terminal-core/src/themes.rs`: Ghostlink (current), Solarized Dark/Light, Dracula, Nord, Gruvbox Dark, One Dark, Tokyo Night, Catppuccin Mocha (public, MIT/permissive palettes).
- Terminal pane background and `render_cache.rs` `DEFAULT_BACKGROUND` follow the theme instead of constants.

### 5. Connections UI — split `crates/app/src/connect.rs` into `connect/{mod,list,editor}.rs` + `widgets.rs`
- Wide modal (~960 px; collapses to one column below ~760 px). **Left**: search (label, address, user, group, tags), hosts grouped by Parent group (collapsible), each row shows label, `user@address:port`, tag chips; "+ New host".
- **Right**, in Termius order: **Address** · **General** (Label, Parent group with existing-group suggestions, Tags chip input, Backspace select) · **SSH** (Port, Username — placeholder shows the fallback, e.g. "SSH config or local user (jason)", Password + "Remember in keychain", Key file [Browse…] + Certificate) · **Advanced** (Agent forwarding toggle, Startup snippet, Host chaining ordered picker of other hosts, Proxy, Environment variables key/value rows, Charset, Theme with swatches, Keep-alive).
- Footer: Delete (with existing undo), Save, Connect (saves first). Unsaved-changes guard when switching hosts.
- New widgets: Toggle, Select (popover list), ChipInput, key/value rows; extend `terminal-view/src/field.rs` `Field` with a multi-line mode for the snippet.
- Keys: ↑/↓ list, Enter connect, Ctrl/Cmd+N new, Ctrl/Cmd+S save, Ctrl/Cmd+F search, Tab through fields, Esc close.

### 6. Docs
README SSH section and `docs/building.md` (keychain requirements per OS, supported options, Mosh not yet supported).

## Delivery phases (each one builds, passes tests, and is usable on its own)
1. Model v2 + migration + keychain + new list/editor UI for Address, General, Port, Username, Password, Key file.
2. Session options: key/cert, saved password, env, startup snippet, backspace, keep-alive, agent forwarding (+ their editor fields).
3. Proxy + host chaining.
4. Charset + themes.

I'll check in with progress (and ask before any commit/push) at the end of each phase.

## Verification
- **Unit**: v1→v2 migration round trip from a real v1 file; validation (jump cycles, missing ids, delete-in-use); `to_profile`; charset decoder across split multibyte chunks; proxy handshakes against in-test SOCKS5/HTTP servers.
- **Session (russh fixture in `ssh/tests.rs`)**: env requests, startup snippet bytes, `^H` backspace, agent-forward request + bridged channel, saved-password-first then prompt, two-hop chain via a fixture that implements `direct-tcpip`.
- **Real OpenSSH on loopback** (as done for the auth bug: unprivileged `/usr/sbin/sshd` on 127.0.0.1): jump host with `AllowTcpForwarding yes`, `AcceptEnv`, agent forwarding (`ssh-add -l` through the forwarded agent), certificate login.
- **GPUI tests** (`connect` module, debug selectors): create/edit/delete/undo, grouping/search, unsaved-changes guard, migration of existing saved hosts.
- **Keychain**: macOS Keychain round trip (ignored/manual test); Linux CI runners have no Secret Service → exercises the fallback path.
- `cargo fmt`, `clippy -D warnings`, full workspace tests locally on macOS; then the **Tests** and **Build installers** workflows for Windows/macOS/Linux/Arch.
- **Manual on your machines**: screen capture is blocked on this Mac, so a visual check on Omarchy (Secret Service via gnome-keyring, portal file picker) and Windows is needed per phase.

## Implementation status (2026-11-01)

Phase 1 implementation is present in the worktree:
- `connections.rs`: v2 structured endpoints and host preferences, in-memory v1 migration with atomic v2 writes, duplicate destinations, jump-reference/cycle/depth validation, dependent-host deletion protection.
- `connect/{mod,editor,list,tests}.rs`: responsive list/editor, explicit username/port, groups, group suggestions, removable tags, search, unsaved-change guard, CRUD/undo, password and key-file modes, native key picker, keep-alive and Backspace controls.
- `platform/src/secrets.rs`: dedicated credential worker with macOS Keychain, Windows Credential Manager, and pure-Rust Linux Secret Service; unavailable-store fallback; guarded writes prevent late authentication from recreating credentials for a deleted/changed host.
- Session options support explicit key files, saved/in-memory passwords, remembering only accepted passwords, keep-alive, Control-H Backspace, environment requests and startup commands for every shell channel. Environment/startup controls are not yet exposed in the editor.

Verified: 80 workspace tests pass, Clippy with warnings denied passes (existing vendored xattr deprecation warnings remain outside workspace linting), formatting and diff checks pass, the platform/keychain crate compiles for Linux GNU and Windows MSVC targets, and macOS release packaging/signature verification and process startup smoke pass. A credential-store failure fixture verifies fallback without touching personal credentials. Two manual probes remain ignored, including the native OS-keychain round trip.

Still required: certificate authentication; environment/startup editor controls and multiline input; agent forwarding; proxy and chained-hop transports plus their editors/tests; charset conversion; per-host themes; final documentation and full native Windows/Linux/Omarchy gates. Native keychain consent, actual portal dialogs, and visual screenshot checks are unverified. The Mac computer-use tool previously could not discover application windows. No commits or pushes have been made for this implementation.

## Phase 2 implementation status (2026-11-01)

Implemented certificate login (explicit certificate and `<key>-cert.pub` discovery), matching-key validation, encrypted private-key unlock, and plain-key fallback when an automatically discovered certificate is malformed. An explicitly selected invalid/mismatched certificate reports an error. The editor includes certificate browsing, multiline startup commands, environment-variable rows, keep-alive, Control-H Backspace, and opt-in agent forwarding. Text fields now maintain multiline caret/IME geometry, clipboard line breaks, vertical movement, internal scrolling, and scroll anchors when focused in the host editor; the editor observes field changes for live group/tag updates.

Agent forwarding uses bounded Unix socket / Windows named-pipe opening and bidirectional SSH-channel bridging. Access is disabled by default, gated until authentication succeeds, and limited to 16 simultaneous forwarded channels with a 300-second channel lifetime bound. Missing agents and explicit server refusal keep the shell usable. Tests verify both permitted forwarding and rejected unsolicited access against a private generated agent.

Verification: 89 automated workspace tests pass; formatting and Clippy pass; Linux GNU and Windows MSVC platform-crate tests compile (including the named-pipe transport test); macOS release packaging and ad-hoc signature verification pass. The separate native `/usr/sbin/sshd` loopback gate passes for generated CA/certificate login, accepted environment values, startup commands, and `ssh-add -l` through the forwarded private agent. The native gate uses no personal SSH keys/config and changes no system SSH server settings. Native keychain consent, portal interactions, screenshots, and full Windows/Linux runtime checks remain open.

Next: implement SOCKS5/SOCKS4a/HTTP CONNECT proxies and chained SSH hops, retain every hop transport for tabs/reconnect, and expose ordered jump-host and proxy editors. Then implement charset conversion and terminal themes, and finish the cross-platform/native completion audit. Nothing has been committed or pushed for these phases.

## Phases 3 and 4 implementation status (2026-11-01)

Implemented SOCKS5, SOCKS4a, and HTTP CONNECT over TCP or a preceding SSH tunnel; ordered/nested saved jump-host routes; per-hop trust and credential prompts with independent IDs; route retention across tabs and complete-route recovery; proxy and jump-host editor controls; and optional proxy credentials in the OS keychain. Stored credentials are bound to the SSH endpoint or proxy identity, and only accepted passwords are remembered. Tests cover proxy authentication/remote DNS/IPv6, coalesced SSH banners, refused forwarding, proxy-before/after-jump routes, stale decisions, catalog expansion, all-hop credential loading, and secret-free editor-to-session persistence.

Implemented exact ISO-8859 tables (1–15 excluding the unassigned 12), encoding_rs multibyte and Windows/KOI8 codecs, UTF-8 passthrough, per-host charset selection, input/paste/startup conversion, and packet-stateful output decoding. Unrepresentable input becomes `?`; invalid remote sequences use U+FFFD. Added nine terminal themes with editor swatches, default/ANSI/cursor colors, preservation of the 256-color cube, remote OSC background support, and reconnect persistence. Palette source/license notices are included in macOS, Linux, Windows, and Arch distributions. Arch CI now executes makepkg check() instead of skipping it.

Verified on this Mac:
- 109 automated workspace tests pass; Clippy with workspace warnings denied passes; formatting and diff checks pass. Existing vendor/xattr deprecation warnings remain outside workspace linting.
- Native macOS Keychain write/read/delete round trip passes using the test credential.
- Native OpenSSH certificate/environment/startup/forwarded-agent gate passes both directly and through a jump host. The generated certificate includes explicit port-forwarding permission.
- Windows MSVC and Linux GNU platform/keychain/agent test targets compile. This is a platform-crate cross-check, not a full native application test.
- Latest macOS release ZIP builds, passes ad-hoc signature verification, contains theme license notices, and its executable stays alive through the 3-second startup smoke. Rendering is not established by process survival.
- Release manifest/checksum script regression test passes; modified Unix packaging scripts parse successfully.

Required completion gates still open:
1. Commit/push approval, as promised in this plan, then the Tests and Build installers workflows for the exact reviewed revision. A feature branch/PR can run these without publishing a main-branch preview.
2. Native Windows and Linux/Arch results from those workflows; actual Omarchy Secret Service/portal and Windows agent/editor runtime interactions.
3. Actual editor screenshots and native visual/keyboard checks. The current Mac computer-use tool still returns `cgWindowNotFound` for Ghostlink, so no screenshot or native visual pass is claimed. Native keychain consent UI and file-picker dialogs have not been visually inspected.

No commits or pushes have been made. The goal remains active until these gates are resolved; implementation completion does not establish full cross-platform runtime verification.

## Final local audit follow-up (2026-11-01)

The completion audit found and fixed keyboard gaps in the charset/theme selectors. They now participate in Tab order, show focus, scroll the selected item into view, and support Enter/Space, arrows, Home/End, and Escape without accidentally saving the host. Narrow editors no longer Tab into an invisible search box; Find opens the host list, and widening the window releases list-only focus navigation. Added GPUI regressions for selection/accept/cancel/save and narrow-to-wide navigation.

Also fixed legacy mouse protocol bytes passing through text transcoding when their coordinate bytes happened to form valid UTF-8. A real loopback SSH regression checks the exact raw coordinate sequence with a legacy charset. UTF-8 input now retains its owned buffer allocation instead of making a second copy.

Latest full workspace suite: 113 passed, 3 ignored manual probes. Clippy, formatting, diff checks, and refreshed macOS release packaging/signature verification pass. The native Keychain and OpenSSH manual gate results above remain separately recorded. The commit/push approval requested in chat is still pending; no publication or commit has occurred. Remaining native/CI/visual gates above are unchanged.

## Build authorization (2026-11-02)

The user approved committing and pushing this implementation and cutting a build for testing. The exact committed revision will be checked by the Tests and Build installers workflows. Native screenshot and desktop interaction gates remain open.

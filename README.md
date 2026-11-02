# Ghostlink

An SSH workspace for Windows, macOS, and Linux, built with Rust, GPUI, and libghostty-vt.

The app opens a connection picker where you can search saved servers, connect, or add, edit, and delete entries. The workspace's left sidebar lists open sessions; each session owns its tabs, split layouts, and focused pane. Tabs and panes share one authenticated SSH connection and keep independent remote shells, terminal state, scrollback, and selection. Local-shell backends (ConPTY on Windows, PTYs on Unix) are retained for testing and later work, without local launchers in the current interface.

## Install

On **Linux / Arch / Omarchy**, install the latest tested preview:

```bash
curl -fsSL https://github.com/last-refuge/ghostlink/releases/download/preview/install.sh | bash
```

Run the same command to update. The installer detects your CPU, verifies the download's SHA-256 checksum, and installs a launcher. On Arch/Omarchy x86_64 it uses **pacman**, including dependencies; other Linux systems install to `~/.local/bin` with a per-user desktop entry. Missing runtime libraries are installed through apt or dnf when available, with sudo requested only for system packages. Linux archives require Ubuntu 24.04 or a compatible newer system; a working Vulkan driver is required. Arch ARM uses the aarch64 Linux archive.

Download the **Windows installer**, portable ZIP, or **macOS app** from [Ghostlink Preview](https://github.com/last-refuge/ghostlink/releases/tag/preview). Windows installers are unsigned; macOS apps are ad-hoc signed and not notarized. The preview channel follows successful builds of `main`. You can [review the Linux installer](install.sh) before running it.

## Build and run

Build natively on the destination OS. Windows targets x86_64; macOS and Linux helpers support x86_64 and aarch64. Linux builds enable both Wayland and X11, including the Arch/Omarchy workflow. Install Git, Rust through [rustup](https://rustup.rs/), and the platform prerequisites in the [build guide](docs/building.md).

On **Windows 11 x64**, install Visual Studio C++ Build Tools with the Windows SDK, then run from the repository root:

```powershell
./scripts/bootstrap.ps1
./scripts/dev.ps1 run
```

Create the Windows installer and portable ZIP with `./scripts/package-windows.ps1`. The installer uses a per-user location, creates a Start menu shortcut, bundles the Microsoft runtime, and supports in-place upgrades. Uninstall preserves saved connections and trusted host keys. Close Ghostlink before updating or uninstalling; setup will not terminate active sessions. Current installers are unsigned development builds.

On **macOS or Linux**, after installing the platform development libraries:

```bash
bash scripts/bootstrap.sh
bash scripts/dev.sh run
```

Build distributable artifacts with `bash scripts/package.sh`: a macOS `.app` in a ZIP, or a Linux tarball with the executable and desktop launcher, written under `dist/`. The macOS bundle is ad-hoc signed for development; Developer ID signing and notarization are separate release work. Arch/Omarchy users can build a pacman package using `packaging/arch/PKGBUILD`. See the build guide for dependencies, installation, and source selection.

Bootstrap downloads the checksum-verified **Zig 0.16.0** archive for the host into `.tools`, fetches a sparse checkout of the pinned Ghostty commit into `.deps`, and builds the native libraries. It does not install Zig globally. The first run needs network access and can take several minutes; interrupted Zig downloads can be resumed by rerunning the script. Checksums come from the [official Zig release index](https://ziglang.org/download/index.json).

Build output and Zig package extraction stay outside the source tree: `%TEMP%` on Windows, `~/Library/Caches/Ghostlink/build` on macOS, and `$XDG_CACHE_HOME/ghostlink/build` (default `~/.cache/ghostlink/build`) on Linux. Set `CARGO_TARGET_DIR` to override Rust's output location. The Windows development executable normally lives at `%TEMP%\ghostlink-target\debug\ghostlink.exe`. Native terminal archives are separated by Rust target triple, so Windows and WSL can share a checkout. libghostty-vt is linked statically; no terminal DLL/shared library needs to be copied beside the executable.

Rust is pinned in `rust-toolchain.toml`, direct crate versions are exact, and `Cargo.lock` pins the dependency graph. The helper reuses an installed stable compiler only if it matches the pinned Rust version exactly.

The [Build installers workflow](https://github.com/last-refuge/ghostlink/actions/workflows/installers.yml) builds Windows, native macOS/Linux on both architectures, and an Arch package. After every platform builds, it publishes all installers and SHA-256 checksums to the Preview release. It verifies the packages themselves: Windows tests installation, upgrade, and uninstall; Linux smoke-tests application startup on a virtual display. It does not run the test suite.

The [Tests workflow](https://github.com/last-refuge/ghostlink/actions/workflows/tests.yml) runs formatting, Clippy, the Rust test suite, and the installer script tests on every platform, separately from the builds, so a test failure is reported without blocking the Preview release. Both workflows run on pushes to `main` and pull requests, and either can be started manually from the Actions tab or with `gh workflow run tests.yml`. Run the same checks locally with the `dev` script commands below.

## Controls

| Action | Control |
|---|---|
| New tab in the current SSH session | `Ctrl Shift T` or `+` in its tab strip |
| Switch session | Click its sidebar entry, or `Ctrl Shift Page Up` / `Page Down` |
| Switch tab within the session | Click its title, or `Ctrl Tab` / `Ctrl Shift Tab` |
| Close focused pane | `Ctrl Shift W`; closes the tab if it has one pane |
| Close entire tab | Its `×` button |
| Close an SSH session and all its tabs | Its sidebar `×` button |
| Split right / below | `Ctrl Shift D` / `Ctrl Shift E`, or toolbar buttons |
| Resize split | Drag its divider |
| Focus next pane | `Ctrl Shift J` |
| Zoom / restore focused pane | `Ctrl Shift Z` |
| New SSH session | `Ctrl Shift K` or New session; opens the connection picker |
| Reconnect a disconnected session | `Ctrl Shift R` or Reconnect in its pane |
| Toggle sidebar | `Ctrl Shift B` |
| Select text | Drag; hold near or beyond the top/bottom edge to scroll through history |
| Select word / logical line | Double-click / triple-click; drag to extend by words / lines |
| Copy selection | `Ctrl Shift C` |
| Clear selection | `Escape`, a new click, or typing |
| Paste clipboard | `Ctrl Shift V` |
| Scroll history | Mouse wheel |
| Select / scroll history inside a mouse-enabled application | Hold `Shift` while dragging / scrolling |

On macOS, use Command for workspace and connection-picker shortcuts; Control aliases remain available. `Cmd T` opens a related tab, `Cmd W` closes a pane, `Cmd N` opens the connection picker, `Cmd C` / `Cmd V` copy and paste, and `Cmd Q` quits. Tab switching remains `Ctrl Tab` / `Ctrl Shift Tab` to avoid the OS's Command-Tab shortcut.

Copy joins soft-wrapped terminal rows while preserving real line breaks, Unicode, and indentation. Scrolling keeps the selection attached to its text. Selection and copy remain available after the shell exits or disconnects, until its pane is closed. Copying with no selection leaves the clipboard unchanged.

Applications such as Neovim and tmux receive mouse clicks, drag/release events, motion, and vertical/horizontal scrolling when they request mouse reporting. Hold Shift for terminal text selection or history scrolling. Alternate-screen applications that request alternate scrolling receive cursor keys when mouse reporting is off.

Closing a pane terminates its remote shell. Closing a tab terminates its panes; closing a sidebar session terminates all its tabs. Other sessions keep running. Closing a session's final tab removes its sidebar entry, and closing the final session returns to the connection picker. Application shutdown requests worker cancellation through GPUI's asynchronous quit hook. Tabs and layouts are retained while switching sessions within the running app; restart recovery remains future work.

On Linux/Omarchy, terminal copy/paste also accepts **Super+C / Super+V** alongside **Ctrl+Shift+C / Ctrl+Shift+V**. Right-click opens a Copy/Paste menu. When a terminal application captures mouse events, use **Shift+right-click** to open Ghostlink's menu. **Shift+Insert** also pastes.

## Connect with SSH

The connection picker opens at startup and whenever you choose **New session**. Saved hosts appear in a searchable list on the left, with host details on the right. Small windows show one panel at a time; choose **Browse hosts** to return to the list. Hosts can be organized into collapsible parent groups and tagged; search matches labels, addresses, usernames, groups, and tags.

Choose **New host**, enter an **Address** (a hostname, IP address, or SSH config alias), and set the **Username** and **SSH port** separately. A blank username uses SSH config or the local account shown below the field; a blank port uses SSH config or 22. IPv6 addresses belong in the address field without brackets. **Label** defaults to the destination when empty. **Save** retains the details; **Connect** saves and opens the host. Switching hosts or closing the editor asks what to do with unsaved changes. **Delete** offers **Undo**; a host used in a jump-host chain cannot be deleted until its references are removed.

Choose **Automatic** to discover SSH keys and the agent, **Password** to enter a password or ask when connecting, or **Key file** to use a specific private key and optional OpenSSH user certificate. A matching `<key>-cert.pub` file is discovered automatically when no certificate is selected. **Browse…** opens the OS file picker; Linux uses the desktop portal. Encrypted keys still ask for their passphrase when required. Certificate login may need to unlock its private key before the server can accept the certificate. Password mode offers **Remember accepted password in system keychain**. Only a server-accepted password is saved. Unchecking remember, changing the destination, or deleting the host removes its saved password. Undo restores host details and asks for credentials again. A missing or locked keychain falls back to prompting. Keyboard-interactive challenges remain interactive. **Backspace mode** selects DEL or Control-H for an unmodified Backspace. **Keep-alive seconds** defaults to 30 when blank; zero disables it. **Startup snippet** accepts multiple lines and runs in every new shell, including tabs, splits and reconnects. **Environment variables** are sent before the shell starts; a refused name produces a notice without blocking the shell (the server controls `AcceptEnv`). Values and startup commands are saved with the host details.

**Forward local SSH agent** is off by default. Enable it only for a trusted host that should be allowed to request signatures from your local agent. It uses the host's resolved `IdentityAgent`/`SSH_AUTH_SOCK` on Unix and the OpenSSH named pipe on Windows. A missing agent or server refusal leaves the shell usable. Hardware/FIDO2 identities should be loaded in your SSH agent and used with Automatic authentication; hardware-key provider libraries are not loaded by Ghostlink.

**Host chaining** connects through saved hosts in the displayed order. Add, reorder, or remove jump hosts; each hop uses its own username, key/certificate, proxy, trust decision, and remembered credentials. Nested routes expand in order, with at most eight intermediate hosts. Tabs reuse the complete authenticated route; reconnect rebuilds it after a transport disconnect. Every gateway must allow TCP forwarding to the next endpoint.

**Proxy** supports SOCKS5 (anonymous or username/password), SOCKS4a (user ID and remote DNS), and HTTP CONNECT (anonymous or Basic authentication). Enter its hostname/IP, separate port, and login. A proxy on a jump host runs before that hop; a target's proxy is reached through the preceding jump. Proxy passwords can be remembered in the system keychain after a successful proxy handshake. Changing proxy identity or disabling remember removes its saved credential. HTTP Basic credentials require a trusted network/proxy; this option opens a plain TCP proxy connection.

**Character set** defaults to UTF-8. Legacy choices include ISO-8859-1 through 15 (except the unassigned ISO-8859-12), Windows-1250 through 1258, KOI8-R/U, Shift_JIS, EUC-JP/KR, GB18030, and Big5. Output conversion retains split multibyte characters across network packets; text, paste, and startup commands are encoded for the remote host. Unrepresentable input becomes `?`; malformed remote sequences become the replacement character. **Terminal theme** offers Ghostlink, Solarized Dark/Light, Dracula, Nord, Gruvbox Dark, One Dark, Tokyo Night, and Catppuccin Mocha, with palette swatches. Tab reaches both appearance selectors; Enter/Space opens a menu, arrows or Home/End move through choices, Enter accepts, and Escape cancels. Themes apply to every terminal channel and survive reconnects. See [palette sources and licenses](docs/theme-licenses.md).

Saved host details persist in `connections.json` under `%LOCALAPPDATA%\Ghostlink` on Windows, `~/Library/Application Support/Ghostlink` on macOS, and `$XDG_DATA_HOME/ghostlink` (default `~/.local/share/ghostlink`) on Linux. The v2 catalog stores separate address, username, and port fields plus per-host preferences; existing v1 names/destinations are migrated in memory and written as v2 on the next save. It never stores passwords or key passphrases. Remembered passwords use Windows Credential Manager, macOS Keychain, or Linux Secret Service (such as GNOME Keyring or KWallet). Updates are atomic and serialized across app windows; an unreadable or unsupported file is reported without overwriting it. Editing or deleting a host does not interrupt its open sessions.

In the picker, `Ctrl N` adds a host, `Ctrl S` saves, `Ctrl F` focuses search, and `Ctrl E` focuses the selected host's editor. Use Tab/Shift-Tab through text fields, Up/Down in search to select a host, and Enter in search to connect. Escape closes the picker when sessions are open, with an unsaved-changes guard. Use Command for these shortcuts on macOS.

Connecting adds a session to the sidebar using its saved name, with its first tab. Use `+` or `Ctrl Shift T` for another tab on that server, or split the current tab for another pane. These shells reuse the existing authenticated transport, without another login prompt or stored password. Switching sidebar entries restores each session's selected tab, layout, zoom, and focus. Authentication prompts reveal their owning tab within the active session; a background session displays **Authentication needed** without taking focus. The current destination stays visible in the app header, including when the sidebar is hidden. Every pane has its own remote PTY and resizing.

Disconnected panes and their sidebar session show their status. Choose **Reconnect** or press `Ctrl Shift R` in a disconnected pane. That pane authenticates first; other panes disconnected from the session then reuse the recovered connection. Tabs, split layouts, zoom, focus, and each pane's previous output are retained. An intentionally exited shell can also be reopened in place. Previous styled output remains in scrollback, including the last full-screen application display; fresh terminal modes prevent an interrupted application from affecting the new shell. Reconnecting creates fresh remote shells. Use tmux when remote processes need to survive a disconnect.

An unknown server presents its algorithm and SHA-256 fingerprint **before authentication**. Verify the fingerprint with a trusted source, then select **Trust key and connect**. Accepted keys are stored in Ghostlink's own OpenSSH-format `known_hosts` file in the same app data directory; the user's OpenSSH file is not modified. Changed keys block the connection, with no automatic replacement. Malformed or unreadable trust files also block connections.

Ghostlink reads the user's `~/.ssh/config`, including `Host` patterns and negation, `HostName`, `User`, `Port`, `IdentityFile`, `IdentitiesOnly`, `IdentityAgent`, and nested `Include` globs. It honors public-key/password/keyboard-interactive enablement, `BatchMode`, and the supported methods in `PreferredAuthentications`. Explicit per-host usernames and ports take precedence. Identity paths support the current user's `~`, `%d`, `%h`, `%n`, `%r`, `%u`, `%p`, `%%`, and `${ENV}` expansion. A key selected in the connection editor is tried before configured identities.

Without configured identity files, discovery checks `~/.ssh/id_ed25519`, `id_ecdsa`, and `id_rsa`. An available agent is used first: Windows OpenSSH on Windows, or `SSH_AUTH_SOCK` / configured `IdentityAgent` sockets on macOS and Linux. A missing agent is skipped automatically. For plain-key authentication, an encrypted key prompts for its passphrase only after the server accepts its public identity. Encrypted PEM identities require a matching `.pub` file. Password and keyboard-interactive authentication follow when offered by the server and allowed by config. Enter submits a prompt; Escape cancels it. A connection failure remains visible in its pane, alongside any previous terminal output.

This is a documented subset of SSH config, not full OpenSSH compatibility. System-wide config, forwarding, and OpenSSH process/presentation settings are not applied. `Match` blocks, `ProxyCommand`/`ProxyJump`, `CertificateFile`, hardware-key provider directives, custom trust-file directives, and algorithm overrides in SSH config produce explicit unsupported-setting errors. Proxies, jump hosts, user certificates, and agent forwarding are selected per host in the editor; system-wide SSH config and host certificate trust remain unsupported.

## Check the implementation

```powershell
./scripts/dev.ps1 check
./scripts/dev.ps1 test
./scripts/dev.ps1 clippy
./scripts/dev.ps1 fmt
```

On macOS/Linux, run the corresponding `bash scripts/dev.sh check|test|clippy|fmt` commands. Unix tests require the OpenSSH client tools (`ssh-agent` and `ssh-add`) and use a private temporary agent with generated keys. Real Unix PTY checks cover input, Unicode, resizing, output drain, exit, copy after exit, and worker shutdown. Windows-specific PowerShell/ConPTY and pointer-selection tests stay gated to Windows.

The suite includes real ConPTY/PowerShell round trips; an in-process russh server for trust, automatic identity discovery, encrypted keys, RSA signatures, password and keyboard-interactive prompts, shared authentication and independent channel lifetimes, interleaved startup messages, genuine shell refusal/channel closure, PTY sizing, terminal replies, large output, exit status, and cancellation; SSH config precedence/includes; saved-connection persistence, concurrent catalog updates, malformed-file protection, and picker CRUD/search/keyboard behavior; simulated GPUI session/tab switching, close/focus restoration, pane layouts, credential interactions, selection, clipboard, and drag auto-scroll; responsive picker/tab-bar layout and selected-tab visibility, retained row layout and Unicode/style boundaries; and recorded VT sequences covering Unicode, alternate screens, scrollback, wrapped-line copying, cursor-key modes, and bracketed paste. Selection tests cover copying after shell exit, tracked anchors during output/resize, word/line dragging, wide/combining graphemes, release outside the pane, focus changes, and retained-worker shutdown. Tests use generated identities and temporary trust/config files and never connect to an external host or read the user's keys.

Two additional manual probes measure local and loopback SSH input-to-snapshot latency. A native Windows renderer probe measures preparation and paint work. See [performance measurements](llm-docs/performance.md) for results and reproduction commands.

A separate manual gate runs real Neovim, both directly and inside tmux, through WSL Ubuntu and the terminal backend. It checks cursor keys, mouse clicks, scrolling, resizing, Unicode paste, tmux's command prefix, process exit, and alternate-screen restoration. It passed with **Neovim 0.12.6 and tmux 3.4**. SSH loopback tests independently check mouse bytes, shared-transport recovery, preserved history/size, shell-exit retry, and changed-key blocking. A live external-server/full-screen pass and native Windows pointer interaction remain separate validation steps.

```powershell
$env:CARGO_TARGET_DIR = Join-Path $env:TEMP 'ghostlink-target'
$env:GHOSTLINK_NVIM = '/usr/bin/nvim' # Or a portable Neovim executable inside Ubuntu
cargo +stable test --locked -p session full_screen:: -- --ignored --nocapture
```

## Structure

| Crate | Responsibility |
|---|---|
| `platform` | Native app-data/home directories, fonts, app identity, and shortcut defaults |
| `ghostty-sys` | Small C bridge compiled against the pinned upstream headers; private Rust ABI |
| `terminal-core` | Exclusive native ownership, input encoding, checked dimensions, owned snapshots |
| `session` | ConPTY/Unix PTY and direct russh transports, authentication, host-key trust, bounded queues, cancellation |
| `terminal-view` | GPUI canvas, fixed cell placement, focus, selection, clipboard, preliminary IME composition |
| `app` | Tabs, recursive pane layouts, divider dragging, saved-connection catalog and picker, workspace controls |

Each terminal lives on one session worker. ConPTY output is bounded and lossless; only obsolete render snapshots are superseded. Input writes never block the parser. The output reader keeps draining while the pseudoconsole closes. Errors and process exit states are shown in the pane.

The native bridge uses [Ghostty commit f64f4aca](https://github.com/ghostty-org/ghostty/tree/f64f4aca2c29b554d111b36c3d946a9bddd159ff) and Zig 0.16.0. The UI uses [GPUI 0.2.2](https://docs.rs/gpui/0.2.2/gpui/). Review these pins together when updating either integration.

## Remaining prototype work

- Live Windows-agent validation and native cross-platform checks for the new host editor/options.
- A real remote full-screen application compatibility pass (for example, Neovim).
- Full keyboard protocol coverage, complete key-event metadata, and cursor-shape fidelity.
- Visual verification of selection, clipboard, font fallback, emoji, and mixed-DPI rendering.
- Japanese/Chinese IME validation; composition and candidate bounds are wired but unverified.
- Screen-reader integration and keyboard-accessible application controls.
- Sustained-output performance, many-pane scalability, and GPU recovery.

Preferences, workspace restoration, full OpenSSH config compatibility, terminal text search, and WSL discovery have not been implemented. Selection and IME checks against the actual Windows renderer remain pending; GPUI test-backend coverage does not replace those checks.

See [the implementation plan](llm-docs/implementation-plan.md) for the wider product scope and [implementation progress](llm-docs/implementation-progress.md) for validation and the next tasks.

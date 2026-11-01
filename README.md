# Ghostlink

A Windows SSH workspace built with Rust, GPUI, and libghostty-vt.

The app opens into an SSH workspace. The left sidebar lists open sessions; each session owns its tabs, split layouts, and focused pane. Tabs and panes share one authenticated SSH connection and keep independent remote shells, terminal state, scrollback, and selection. The ConPTY local-shell backend is retained for later work and testing, without local launchers in the current interface.

## Build and run

Development currently targets **Windows 11 x64**. Install Git, Rust through rustup, and Visual Studio C++ Build Tools with the Windows SDK. Run these commands in PowerShell from the repository root:

```powershell
./scripts/bootstrap.ps1
./scripts/dev.ps1 run
```

Bootstrap downloads a checksum-verified Zig compiler into `.tools`, fetches a sparse checkout of the pinned Ghostty commit into `.deps`, and builds the native libraries. It does not install Zig globally. The first run needs network access and can take several minutes; interrupted Zig downloads can be resumed by rerunning the script.

Build output and Zig package extraction go to `%TEMP%` to avoid Dropbox locking temporary compiler files. Set `CARGO_TARGET_DIR` to override Rust's output location. The development executable normally lives at `%TEMP%\ghostlink-target\debug\ghostlink.exe`. libghostty-vt is linked statically; no terminal DLL needs to be copied beside the executable.

Rust is pinned in `rust-toolchain.toml`, direct crate versions are exact, and `Cargo.lock` pins the dependency graph. The helper reuses an installed stable compiler only if it matches the pinned Rust version exactly.

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
| Connect SSH | `Ctrl Shift K` or Connect SSH |
| Reconnect a disconnected session | `Ctrl Shift R` or Reconnect in its pane |
| Toggle sidebar | `Ctrl Shift B` |
| Select text | Drag; hold near or beyond the top/bottom edge to scroll through history |
| Select word / logical line | Double-click / triple-click; drag to extend by words / lines |
| Copy selection | `Ctrl Shift C` |
| Clear selection | `Escape`, a new click, or typing |
| Paste clipboard | `Ctrl Shift V` |
| Scroll history | Mouse wheel |
| Select / scroll history inside a mouse-enabled application | Hold `Shift` while dragging / scrolling |

Copy joins soft-wrapped terminal rows while preserving real line breaks, Unicode, and indentation. Scrolling keeps the selection attached to its text. Selection and copy remain available after the shell exits or disconnects, until its pane is closed. Copying with no selection leaves the clipboard unchanged.

Applications such as Neovim and tmux receive mouse clicks, drag/release events, motion, and vertical/horizontal scrolling when they request mouse reporting. Hold Shift for terminal text selection or history scrolling. Alternate-screen applications that request alternate scrolling receive cursor keys when mouse reporting is off.

Closing a pane terminates its remote shell. Closing a tab terminates its panes; closing a sidebar session terminates all its tabs. Other sessions keep running. Closing a session's final tab removes its sidebar entry, and closing the final session returns to the welcome view. Application shutdown requests worker cancellation through GPUI's asynchronous quit hook. Tabs and layouts are retained while switching sessions within the running app; restart recovery remains future work.

## Connect with SSH

Open **Connect SSH**, enter a server or SSH alias, and press Enter. The single destination field accepts `server`, `user@server`, `user@server:2222`, and bracketed IPv6 addresses. Ghostlink resolves the username, port, and identities from your SSH config, then tries available keys automatically. Passphrases, passwords, and keyboard-interactive challenges appear only when needed. Secrets are masked when appropriate and are never saved to settings, workspace files, or logs.

Connecting adds a session to the sidebar with its first tab. Use `+` or `Ctrl Shift T` for another tab on that server, or split the current tab for another pane. These shells reuse the existing authenticated transport, without another login prompt or stored password. Switching sidebar entries restores each session's selected tab, layout, zoom, and focus. Authentication prompts reveal their owning tab within the active session; a background session displays **Authentication needed** without taking focus. The current destination stays visible in the app header, including when the sidebar is hidden. Every pane has its own remote PTY and resizing.

Disconnected panes and their sidebar session show their status. Choose **Reconnect** or press `Ctrl Shift R` in a disconnected pane. That pane authenticates first; other panes disconnected from the session then reuse the recovered connection. Tabs, split layouts, zoom, focus, and each pane's previous output are retained. An intentionally exited shell can also be reopened in place. Previous styled output remains in scrollback, including the last full-screen application display; fresh terminal modes prevent an interrupted application from affecting the new shell. Reconnecting creates fresh remote shells. Use tmux when remote processes need to survive a disconnect.

An unknown server presents its algorithm and SHA-256 fingerprint **before authentication**. Verify the fingerprint with a trusted source, then select **Trust key and connect**. Accepted keys are stored in Ghostlink's own OpenSSH-format file at `%LOCALAPPDATA%\Ghostlink\known_hosts`; the user's OpenSSH file is not modified. Changed keys block the connection, with no automatic replacement. Malformed or unreadable trust files also block connections.

Ghostlink reads the user's `~/.ssh/config`, including `Host` patterns and negation, `HostName`, `User`, `Port`, `IdentityFile`, `IdentitiesOnly`, `IdentityAgent`, and nested `Include` globs. It honors public-key/password/keyboard-interactive enablement, `BatchMode`, and the supported methods in `PreferredAuthentications`. Explicit destination usernames and ports take precedence. Identity paths support the current user's `~`, `%d`, `%h`, `%n`, `%r`, `%u`, `%p`, `%%`, and `${ENV}` expansion. Custom key paths belong in this config rather than the connection form.

Without configured identity files, discovery checks `~/.ssh/id_ed25519`, `id_ecdsa`, and `id_rsa`. An available agent is used first; a missing Windows OpenSSH agent is skipped automatically. An encrypted key prompts for its passphrase only after the server accepts its public identity. Encrypted PEM identities require a matching `.pub` file. Password and keyboard-interactive authentication follow when offered by the server and allowed by config. Enter submits a prompt; Escape cancels it. A connection failure remains visible in its pane, alongside any previous terminal output.

This is a documented subset of SSH config, not full OpenSSH compatibility. System-wide config, forwarding, and OpenSSH process/presentation settings are not applied. `Match` blocks, proxies/jump hosts, certificate/hardware-key providers, custom trust-file directives, and algorithm overrides produce explicit unsupported-setting errors. Agent forwarding and certificate authentication are not enabled.

## Check the implementation

```powershell
./scripts/dev.ps1 check
./scripts/dev.ps1 test
./scripts/dev.ps1 clippy
./scripts/dev.ps1 fmt
```

The 66-test suite includes real ConPTY/PowerShell round trips; an in-process russh server for trust, automatic identity discovery, encrypted keys, RSA signatures, password and keyboard-interactive prompts, shared authentication and independent channel lifetimes, interleaved startup messages, genuine shell refusal/channel closure, PTY sizing, terminal replies, large output, exit status, and cancellation; SSH config precedence/includes; simulated GPUI session/tab switching, close/focus restoration, pane layouts, destination forms, credential interactions, selection, clipboard, and drag auto-scroll; responsive tab-bar layout and selected-tab visibility, retained row layout and Unicode/style boundaries; and recorded VT sequences covering Unicode, alternate screens, scrollback, wrapped-line copying, cursor-key modes, and bracketed paste. Selection tests cover copying after shell exit, tracked anchors during output/resize, word/line dragging, wide/combining graphemes, release outside the pane, focus changes, and retained-worker shutdown. Tests use generated identities and temporary trust/config files and never connect to an external host or read the user's keys.

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
| `ghostty-sys` | Small C bridge compiled against the pinned upstream headers; private Rust ABI |
| `terminal-core` | Exclusive native ownership, input encoding, checked dimensions, owned snapshots |
| `session` | ConPTY and direct russh transports, authentication, host-key trust, bounded queues, cancellation |
| `terminal-view` | GPUI canvas, fixed cell placement, focus, selection, clipboard, preliminary IME composition |
| `app` | Tabs, recursive pane layouts, divider dragging, SSH connection form, workspace controls |

Each terminal lives on one session worker. ConPTY output is bounded and lossless; only obsolete render snapshots are superseded. Input writes never block the parser. The output reader keeps draining while the pseudoconsole closes. Errors and process exit states are shown in the pane.

The native bridge uses [Ghostty commit f64f4aca](https://github.com/ghostty-org/ghostty/tree/f64f4aca2c29b554d111b36c3d946a9bddd159ff) and Zig 0.16.0. The UI uses [GPUI 0.2.2](https://docs.rs/gpui/0.2.2/gpui/). Review these pins together when updating either integration.

## Remaining prototype work

- Live Windows-agent validation and jump hosts.
- A real remote full-screen application compatibility pass (for example, Neovim).
- Full keyboard protocol coverage, complete key-event metadata, and cursor-shape fidelity.
- Visual verification of selection, clipboard, font fallback, emoji, and mixed-DPI rendering.
- Japanese/Chinese IME validation; composition and candidate bounds are wired but unverified.
- Screen-reader integration and keyboard-accessible application controls.
- Sustained-output performance, many-pane scalability, and GPU recovery.

Saved hosts, preferences, workspace restoration, full OpenSSH config compatibility, search, and WSL discovery have not been implemented. Selection and IME checks against the actual Windows renderer remain pending; GPUI test-backend coverage does not replace those checks.

See [the implementation plan](llm-docs/implementation-plan.md) for the wider product scope and [implementation progress](llm-docs/implementation-progress.md) for validation and the next tasks.

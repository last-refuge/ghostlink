# Implementation progress

Date: 2026-10-31

## Increment 1: local terminal foundation

Implemented the five-crate Rust workspace, exact Rust/crate pins and lockfile, a Windows bootstrap script, and a graphite GPUI application based on the existing visual reference.

The native layer builds Ghostty `f64f4aca2c29b554d111b36c3d946a9bddd159ff` with Zig 0.16.0 and statically links it. A C bridge keeps upstream enums, sized structs, and tagged unions out of the Rust ABI. The safe wrapper never shares a native handle across threads and copies all render data into owned snapshots.

The local session runs Windows PowerShell or Command Prompt through portable-pty's ConPTY implementation. Separate input and output workers service the pipes; the session owns parsing and resize operations. Output bytes pass through a bounded queue without dropping. Snapshots are coalesced independently. Terminal-generated replies return to ConPTY. Cancellation, process exit, and spawn errors are represented explicitly.

The GPUI implementation includes local tabs, shell launch entries, terminal resize, fixed cell origins, basic styles and colors, scrollback, viewport selection, copying, pasting, and a composition overlay with IME candidate bounds. Selection and UI behavior need visual validation.

## Validation

- Native bootstrap completes and can be rerun using cached dependencies.
- `cargo check --workspace` passes.
- Ten tests pass: six libghostty integration tests and four Windows ConPTY tests.
- ConPTY tests cover input, resizing, spawn failure, cancellation, a 5,000-line output stream, and preservation of final output/exit status.
- Clippy passes across all targets with warnings denied.
- Rust formatting passes.
- A Windows GitHub Actions workflow is checked in; it has not been run on a hosted runner yet.
- The executable launches and exposes a Ghostlink window.
- Computer-use app approval timed out before screenshot inspection. Visual layout, user-driven keyboard/mouse behavior, and real IME interaction have not been verified.

Native builds and Rust compilation use local temporary directories because Dropbox locked compiler temporary files during the initial build attempts. The checked-in scripts include that workaround.

## Increment 2: SSH and split panes

Implemented direct russh 0.64.1 connections with password, encrypted/unencrypted private-key, and Windows OpenSSH-agent authentication. Each SSH session owns its terminal on a worker with a current-thread Tokio runtime. Channel reads and writes run independently, with bounded input queues, terminal-generated replies, remote PTY/window-size requests, keepalives, timeouts, cancellation, final-output retention, and explicit disconnect errors.

First-use trust presents the server's SHA-256 fingerprint before credentials are sent. An accepted plain host key is persisted to Ghostlink's own `%LOCALAPPDATA%\Ghostlink\known_hosts`. Changed keys, malformed/unreadable stores, and unsupported host certificates block the connection. A pending trust decision can be cancelled. Passwords/passphrases are masked in the form, redacted from profile debugging, and held in zeroizing strings; no credential persistence was added.

Tabs now own binary pane trees with horizontal/vertical splits, draggable dividers, focus cycling, zoom/restore, and pane removal that promotes the sibling layout. Every pane keeps its session and resize path. The SSH form can connect in a split beside the focused pane or in a new tab. Local and SSH panes can be mixed in the same tree.

Validation: **23 tests pass**, comprising six terminal-core tests, four ConPTY tests, seven SSH tests, two pane-tree tests, and four GPUI interaction/text-input tests. The SSH fixture binds only to loopback and uses generated keys and temporary trust files. It exercises first-use acceptance/rejection, changed keys before authentication, password rejection, encrypted-key authentication, terminal cursor-key modes and replies, stderr, resizing, 5,000 output lines, final output/exit status, and cancellation. GPUI's test backend exercises split shortcuts, zoom/focus/close, divider dragging, native form input, and masked-field Unicode editing/copy prevention. Clippy with warnings denied and formatting pass.

Live Windows-agent authentication, external-server/Neovim compatibility, and the actual Windows renderer/IME still require validation. The integration milestone is not marked complete.

Authentication correction: the connection form now defaults to **Private key**, with the key path visible and a note that direct key-file authentication does not require an agent. Missing Windows-agent pipes and agents with no supported keys produce guidance to choose a key file or another authentication method. Two new regression tests verify the form submits a private-key profile by default and a missing named pipe produces actionable guidance. **25 tests**, Clippy, and formatting pass.

## Increment 3: automatic SSH identity discovery

Replaced the authentication selector, key-file field, separate connection-detail fields, and tab/split choice with one destination field. Enter a server or SSH alias; the connection opens beside the focused pane. User SSH config supplies aliases, usernames, ports, and custom identities, with standard key locations used by default. Missing agents fall through to file identities automatically. No credential-selection controls were added.

Implemented the documented user-config subset described in the README, including Host/Include precedence, identity expansion, agent selection, authentication preferences, and BatchMode. Unsupported routing, certificate, trust-file, and algorithm settings fail explicitly. Config resolution and identity reads run on the session worker. Server trust is checked before authentication or identity reads.

Passphrases appear only after the server accepts an encrypted key's public identity. Password and keyboard-interactive prompts follow the server's supported methods and user config. Prompts accept native keyboard/IME input, hide secrets, clear their fields afterward, and restore terminal focus. Credential values use zeroizing storage and are not persisted.

Validation: **39 tests pass**, including automatic standard/configured identities, missing-agent fallback, accepted and unrelated encrypted keys, negotiated RSA signatures, password cancellation, keyboard-interactive challenges, BatchMode/authentication preferences, config precedence/includes, the single-field connection form, and credential input/focus restoration. Clippy with warnings denied, formatting, and the development build pass. External-server compatibility and actual Windows renderer/IME validation remain pending.

## SSH startup correction

Fixed a false shell-request refusal after successful authentication. The startup handshake previously treated the first channel message as the request reply, so an informational message or early output caused a misleading refusal. It now waits for an actual success/failure reply under one fixed timeout, skips flow-control notifications already handled by russh, and preserves other startup events in a bounded queue for the terminal read loop. Queued startup events are processed before user input and the first running frame. Actual rejection and channel closure remain distinct, and already-received server output stays visible when startup fails.

The loopback fixture reproduces the exact shell-refusal error before the fix by interleaving flow-control messages, stdout, and stderr with request replies. It now verifies successful password authentication, retained startup text, terminal-generated replies, and interactive round trips. A second regression covers genuine refusal and closure with retained diagnostics. **41 tests**, Clippy with warnings denied, formatting, and the development build pass. The user confirmed that SSH now works against their external server. Remote full-screen applications, live Windows-agent authentication, and Windows renderer/IME behavior still require validation.

## Increment 4: typing latency

Prioritized input responsiveness after the user reported visible delay in both local and SSH panes. A native Windows renderer probe found approximately 200 ms of terminal paint work on a dense viewport. Retained row layout, batched ASCII drawing, merged background/selection fills, and removal of redundant input invalidations reduced median paint work to 3.222 ms, with 0.343 ms of preparation. Non-ASCII graphemes retain their parser-assigned placement. Local input now wakes the worker and is flushed before waiting for output; SSH sockets use TCP_NODELAY. Both transports avoid unchanged pre-echo snapshots while following output.

Validation: **43 correctness tests pass**, including new row-cache, Unicode/style, and invalidation regressions. Two separate manual transport probes pass. Clippy with warnings denied, formatting, and the development build pass. Native timings and the limits of the measurements are documented in [performance measurements](performance.md). On 2026-10-31 the user confirmed that typing is much better. Physical keyboard-to-display latency, external network timing, and sustained-output/many-pane profiling remain open.

## Increment 5: SSH session workspace

Reworked the app around open SSH sessions in the default left sidebar. Every session owns its tabs, selected tab, split trees, zoom, and focused pane. Switching sessions restores those entities without recreating their shells. New tabs and splits target the active SSH session. The app starts at a connection welcome view, and local PowerShell/CMD launchers have been removed from the current interface while their backend remains intact.

Added shared authenticated SSH transports. Tabs and panes open independent PTY channels on the session's transport, so authentication occurs once. Transport I/O runs on a shared runtime while each terminal parser stays on its owning worker. Closing a pane closes only its channel; closing the original tab leaves siblings usable. The final connection owner shuts down transport work without joining it on GPUI. Closing a session removes its tabs; closing the final session returns to the welcome view. Tabs/layouts are retained within the running app, with restart persistence still deferred.

Authentication events reveal the owning tab in the active session. Background sessions show an attention label without stealing focus, and selecting one reveals its pending prompt. Added keyboard cycling for session-local tabs and sidebar sessions. Existing split, divider, pane focus, zoom, and close behavior now follows the active session.

Validation: **49 correctness tests pass**, with GPUI regressions for session/tab restoration, keyboard navigation, focus, authentication attention, close boundaries, and the empty state. Loopback SSH regressions verify a single authentication across channels, continued use after the original tab exits, client-initiated channel cleanup, and authentication handoff after its original pane is cancelled. Both manual latency probes pass with results comparable to the previous increment. Clippy with warnings denied, formatting, and build pass. An existing ConPTY test transiently returned Windows control-C exit status during verification; the isolated diagnostic rerun and subsequent full suite passed. Actual external-server session/tab behavior and native layout still need user validation.

## Increment 6: UI cleanup

Polished the SSH workspace using the user's screenshot as the reference. Removed the redundant SSH pane header and badge, combined split/zoom controls with the tab bar, placed new-tab beside the tabs, and removed the duplicate application footer. The current destination appears in the compact app header so it remains visible with the sidebar hidden. Session rows have consistent line heights, truncated long labels with full-label tooltips, quieter selection styling, and larger close targets. Tabs have consistent widths, aligned baselines, and shortcut tooltips. Native Windows caption colors match the graphite app through DWM attributes; unsupported cosmetic attributes are nonfatal.

Added per-session tab scrolling and deferred post-layout correction so selected tabs remain visible after navigation and resizing. A GPUI layout regression checks controls at 700, 1280, and 1947 logical pixels wide, verifies that new-tab stays beside the tabs and does not overlap pane actions, and checks a selected tab after growing the tab list and shrinking the window. **50 correctness tests**, Clippy with warnings denied, formatting, and build pass. Native appearance still needs user review; the supplied screenshot documents the pre-change appearance. SSH/session behavior and the terminal rendering path were preserved.

## Increment 7: selection and copy

The user confirmed the UI cleanup and requested the selection/copy improvements next. Replaced viewport cell offsets with libghostty-vt's tracked selection gestures, row highlights, and plain-text selection formatter. Selection stays attached to scrollback across scrolling, new output, and resizing. Dragging near or beyond the top/bottom edge auto-scrolls at a distance-dependent rate. Double-click selects words and triple-click selects complete logical lines, including wrapped content; subsequent dragging extends by the same unit.

Copy remains `Ctrl Shift C`. It joins soft wraps while retaining real newlines, indentation, and wide/combining Unicode graphemes, and trims trailing padding on nonblank lines. An empty selection leaves the clipboard unchanged. Copy requests run on the parser's owning worker and return asynchronously, with a 16 MiB limit and protection against an older request overwriting a newer copy. Escape, a new click, or typing clears selection. Mouse release outside the pane, focus changes, window deactivation, and closing a pane stop an active drag. No new interface controls or preferences were added.

Retained the terminal parser after shell exit/disconnection so scrolling, selection, resizing, and copy keep working until the pane closes. Transport work still stops when the shell exits. Pane close and asynchronous app quit cancel the retained worker and wake its blocking receive. Selection highlights remain separate from cached text layout, so dragging does not reshape unchanged rows.

Validation: **58 correctness tests pass**, with native regressions for soft-wrap/newline copying, word/line dragging, scrollback anchors, output/resize, alternate-screen isolation, reverse selection, and Unicode graphemes. Real ConPTY and loopback SSH checks verify copying after exit, with a retained-worker quit regression. GPUI tests exercise double/triple-click, copy, Escape, empty-selection clipboard behavior, drag auto-scroll through history, release outside the pane, and focus loss. Cache tests verify unchanged row layouts are reused during selection. Clippy with warnings denied and formatting pass. The native renderer probe remains responsive: median paint **3.050 ms**, preparation **0.348 ms**. Actual Windows mouse/clipboard behavior, mixed DPI, and IME still need user validation.

The development build passes. Both manual latency probes pass with comparable results: SSH median **15.63 ms**, ConPTY median **31.01 ms** from input submission to terminal snapshot. See [performance measurements](performance.md) for percentiles and measurement limits.

## Increment 8: full-screen tools and connection recovery

Implemented items 2 and 3 from the earlier chat roadmap: full-screen tools and connection recovery. Added libghostty-vt mouse encoding for X10, UTF-8, SGR, URxvt, and SGR-pixel protocols, with button/modifier handling, drag/release, hover motion, and vertical/horizontal wheel input. Mouse modes come from the terminal parser. Native encoder state is retained for motion deduplication, with geometry refreshed on resize. Shift overrides application mouse reporting for terminal selection and history scrolling. Fractional wheel input accumulates; alternate-screen scrolling falls back to application cursor keys when requested. Focus loss, deactivation, and release outside the pane finish mouse capture.

SSH transport loss now has an explicit Disconnected state and sidebar attention. A contextual Reconnect action and `Ctrl Shift R` retry in the existing pane. Its terminal stays visible while connecting. The chosen pane authenticates first; its disconnected siblings then reuse the recovered transport, preserving session/tab identities, split layout, zoom, and focus. An intentionally exited shell can be reopened independently. A closed cached transport is replaced under a shared asynchronous lock, keeping authentication serialized. Trust checks and interactive authentication run again for a new transport; stale credential/trust replies are discarded between attempts.

Each worker retains its parser until retry succeeds. The new connection starts with fresh parser, input, keyboard, mouse, and application modes. The previous primary-screen history and last alternate-screen display are replayed as styled, wrapped text into its scrollback, protected from the new application's initial screen clear. Archive size is bounded at 64 MiB. Failed authentication/trust leaves the previous output available. Reconnection opens fresh shells; tmux is still needed to retain remote processes across a dropped transport.

Validation: **66 correctness tests pass**, including native mouse protocols/modifiers/motion/wheels, alternate-scroll cursor encoding, styled history retention, interrupted parser/mode reset, remote mouse byte delivery, shared-transport recovery with one authentication, retained PTY size/history, shell-exit retry, changed-key blocking, simulated Shift selection/mouse capture cleanup, and reconnect UI/shortcut behavior. Clippy with warnings denied and formatting pass. The separate real-application gate passes with **Neovim 0.12.6** directly and inside **tmux 3.4** through WSL Ubuntu/ConPTY, covering cursor keys, mouse clicks, scrolling, resizing, Unicode paste, tmux's command prefix, exit, and alternate-screen restoration. The probe uses a private tmux socket and temporary editor state; regular CI does not require WSL or editor packages. See the README for reproduction.

The real-application check exercises the shared terminal core and local transport; loopback SSH tests independently verify the protocol and recovery paths. A live external-server Neovim/tmux pass, native Windows pointer verification, Windows-agent validation, mixed DPI, IME, and accessibility remain open integration gates.

The development build passes. All three manual tests pass: real Neovim/tmux compatibility and both transport latency probes. The native renderer probe remains comparable: preparation median **0.334 ms**, paint median **3.128 ms**. SSH input-to-snapshot median is **15.55 ms**, ConPTY **31.27 ms**. Full results and measurement limits are recorded in [performance measurements](performance.md).

## Increment 9: brand artwork in the app

Replaced the placeholder header emblem/text with the approved Ghostlink logo and added the matching illustration above the SSH welcome message. The artwork scales with available width and height, with a scrollable welcome area for constrained windows. The graphite header matches the logo's background. Both PNGs are embedded in the executable and shared across renders through cached image handles, so launch location does not affect loading.

Validation: all **11 existing app tests**, Clippy with warnings denied, formatting, and the development build pass. The branded executable launches from outside the repository. The underlying terminal/session implementation is unchanged by this increment.

Header alignment correction: the user's screenshot exposed a 60-pixel intrinsic image laid out at the top of a 40-pixel slot. The logo now uses explicit dimensions and flex centering. Layout inspection at 700, 1280, and 1947 logical pixels confirms matching image/container centers; the complete foreground mark fits within the clipping slot, with only background padding removed.

## Next increment

1. Validate the new mouse and reconnect behavior on a live SSH server, then refine native Windows pointer/IME, font fallback, and mixed DPI.
2. Add recent servers, followed by session/layout restoration after restart.
3. Add saved hosts, jump hosts, and live Windows-agent validation.
4. Add full key-event metadata, screen-reader access, and further sustained-output rendering optimization.

The integration milestone remains open. SSH protocol behavior and pane interactions have automated coverage; a live remote full-screen application gate has not been completed.

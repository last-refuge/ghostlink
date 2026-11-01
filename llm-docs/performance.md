# Typing latency investigation

Date: 2026-10-30

Typing responsiveness takes priority over additional features. The shared renderer was doing enough synchronous work on each frame to delay input dispatch in both local and SSH panes.

## Native rendering measurements

Measured with the actual Windows GPUI renderer in the existing unoptimized development build. The probe opens a 1500 × 1000 window, fills the terminal with 42 dense text lines, and makes 80 small updates at approximately 40 ms intervals. It exits after six seconds. Measurements cover terminal preparation and paint callbacks on the CPU; they do not measure GPU presentation or physical keyboard-to-display latency.

| Work | Before | After, median | After, 95th percentile |
|---|---:|---:|---:|
| Terminal paint, including per-cell layout before the change | approximately 200 ms | 3.222 ms | 3.616 ms |
| Preparation, including retained row layout after the change | included above | 0.343 ms | 0.417 ms |

The observed maximum after the change was 9.145 ms for paint and 3.332 ms for preparation, including cold work. These are measurements on the development machine, not release guarantees. Larger grids, Unicode-heavy screens, many panes, and other hardware still need characterization.

## Changes

- Retain shaped rows across snapshots. Cursor-only updates reuse every row; changing a character rebuilds its row.
- Batch adjacent ASCII cells with matching text styles. Disable ligatures to retain the previous cell placement; non-ASCII graphemes keep their exact parser-assigned positions and widths.
- Merge background fills and selection highlights into spans instead of issuing a quad per cell.
- Avoid requesting a redraw merely because input was sent. Output, selection, composition, and errors still invalidate the view when they change.
- Wake local session workers when input arrives, flush new input before waiting for output, and drain accepted input even when wakeups were coalesced. Preserve bounded queues and independent ConPTY output draining.
- Stop publishing unchanged pre-echo snapshots while the viewport is already following output.
- Set TCP_NODELAY directly on the SSH socket. This path uses russh's `connect_stream`, which leaves configuration of the pre-opened socket to the caller.

## Transport measurements

Separate manual probes send 32 inputs and observe the matching terminal snapshot through a blocking receiver. They include transport, terminal parsing, and snapshot pacing, and exclude the GPUI/render/GPU path. The local fixture uses PowerShell's console key reader; the SSH fixture runs on loopback with generated credentials.

| Probe | Median | 95th percentile | Maximum |
|---|---:|---:|---:|
| ConPTY send to snapshot | 31.12 ms | 32.07 ms | 32.19 ms |
| Loopback SSH send to snapshot | 15.64 ms | 30.96 ms | 31.93 ms |

The probes send the next input immediately after each snapshot, rather than simulating human pauses. External SSH network latency and server echo behavior have not been measured. Timing probes are manual rather than timing assertions in CI, so scheduler differences do not create flaky correctness tests.

## Reproduce

From PowerShell, using the pinned Rust compiler and the same temporary build cache as the development helper:

```powershell
$env:CARGO_TARGET_DIR = Join-Path $env:TEMP 'ghostlink-target'
$env:GHOSTLINK_PROFILE = '1'
cargo +stable run --locked -p terminal-view --example render_probe
Remove-Item Env:GHOSTLINK_PROFILE
cargo +stable test --locked -p session input_latency_probe -- --ignored --nocapture
```

Use `+1.93.0` instead of `+stable` if the installed stable compiler does not match the pinned version. Profiling is disabled by default. It records durations only, with no terminal contents, keystrokes, or credentials.

Validation at the performance increment: 43 correctness tests pass, both manual transport probes pass, and Clippy with warnings denied, formatting, and the development build pass. Row-cache regressions cover cursor-only reuse, changed-row and width invalidation, style boundaries, background merging, wide characters, and combining graphemes. On 2026-10-31 the user confirmed that typing feels much better. After the shared SSH transport change, the probes remained comparable: loopback SSH median 15.55 ms / p95 26.09 ms, and ConPTY median 30.86 ms / p95 31.19 ms.

After the selection/copy improvements on 2026-10-31, the same native probe recorded 89 samples: preparation median **0.348 ms**, p95 **0.552 ms**, maximum **2.799 ms**; paint median **3.050 ms**, p95 **3.755 ms**, maximum **8.669 ms**. Selection highlights use native row ranges without reshaping cached text; a cache regression verifies this reuse. Clipboard formatting runs on the session worker only when copying. These measurements use the same development probe and retain the limits described above.

Both transport probes also pass after the selection changes: loopback SSH median **15.63 ms**, p95 **30.81 ms**, maximum **30.87 ms**; ConPTY median **31.01 ms**, p95 **31.46 ms**, maximum **32.14 ms**.

After full-screen mouse support and connection recovery, the same native probe recorded 89 samples: preparation median **0.334 ms**, p95 **0.462 ms**, maximum **2.850 ms**; paint median **3.128 ms**, p95 **3.587 ms**, maximum **9.141 ms**. Transport probes remain comparable: loopback SSH median **15.55 ms**, p95 **16.19 ms**, maximum **28.01 ms**; ConPTY median **31.27 ms**, p95 **32.16 ms**, maximum **32.33 ms**. Mouse events use retained encoder state; reconnect history formatting runs only during successful recovery on the owning worker.

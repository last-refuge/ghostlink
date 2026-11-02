# Ghostlink GPUI patch

Vendored from crates.io gpui 0.2.2 with its original licenses.

- `src/platform/linux/wayland/clipboard.rs`: accept `text/plain` in addition to UTF-8-labelled clipboard offers. GPUI already advertises `text/plain` when copying text, but its receive allowlist omitted it. This fixes text-only Wayland clipboard sources being silently rejected in terminals and text fields. A small `clipboard_mime.rs` helper prefers explicit UTF-8 formats and has standalone regression tests, run by the Linux Tests workflow.

Remove this patch when upgrading to a release with this fix.

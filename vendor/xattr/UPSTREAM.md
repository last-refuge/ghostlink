# xattr 0.2.3 compatibility patch

Source: the checksum-verified [xattr 0.2.3 crate](https://crates.io/crates/xattr/0.2.3), required by GPUI 0.2.2 through `zed-async-tar 0.5.0-zed`. Original source, tests, and MIT/Apache licenses are retained here.

The only source change is in `src/sys/mod.rs`: Linux/Android use `libc::ENODATA` for a missing extended attribute. Modern libc no longer exports the non-native `ENOATTR` alias on Linux. macOS and BSD retain `libc::ENOATTR`. Current upstream xattr uses the same platform distinction, but GPUI's transitive dependency requires the older 0.2 API.

This path patch keeps the existing libc version and avoids a private registry-cache edit or network Git dependency. Remove it when the pinned GPUI dependency graph accepts a maintained xattr release. The vendored crate is excluded from the application workspace; its upstream integration tests can be run separately on Unix with `cargo test --manifest-path vendor/xattr/Cargo.toml`.

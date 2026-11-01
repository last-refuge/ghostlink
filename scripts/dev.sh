#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
cd "$ghostlink_root"

command -v cargo >/dev/null || ghostlink_fail 'Install Rust using rustup first. See README.md.'
version="$(sed -n 's/^channel = "\([^"]*\)"/\1/p' rust-toolchain.toml)"
toolchain="+$version"
if rustup toolchain list | grep -q "^stable-$ghostlink_target"; then
    case "$(rustc +stable --version)" in
        "rustc $version "*) toolchain=+stable ;;
    esac
fi
[[ $# -le 1 ]] || ghostlink_fail 'Usage: bash scripts/dev.sh [run|build|release|check|test|clippy|fmt]'
case "${1:-run}" in
    run) exec cargo "$toolchain" run --locked -p ghostlink ;;
    build) exec cargo "$toolchain" build --locked --workspace ;;
    release) exec cargo "$toolchain" build --locked --release -p ghostlink ;;
    check) exec cargo "$toolchain" check --locked --workspace ;;
    test) exec cargo "$toolchain" test --locked --workspace ;;
    clippy) exec cargo "$toolchain" clippy --locked --workspace --all-targets -- -D warnings ;;
    fmt) exec cargo "$toolchain" fmt --all -- --check ;;
    *) ghostlink_fail 'Usage: bash scripts/dev.sh [run|build|release|check|test|clippy|fmt]' ;;
esac

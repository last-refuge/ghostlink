#!/usr/bin/env bash
# Sourced by the native macOS/Linux build helpers. Compatible with macOS Bash 3.2.
# These variables are the interface consumed by the importing scripts.
# shellcheck disable=SC2034
ghostlink_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
ghostlink_revision=f64f4aca2c29b554d111b36c3d946a9bddd159ff
ghostlink_zig_version=0.16.0

ghostlink_fail() { printf '%s\n' "$*" >&2; exit 1; }

case "$(uname -m)" in
    x86_64|amd64) ghostlink_arch=x86_64 ;;
    arm64|aarch64) ghostlink_arch=aarch64 ;;
    *) ghostlink_fail 'Supported architectures: x86_64 and aarch64.' ;;
esac
case "$(uname -s)" in
    Linux)
        ghostlink_os=linux
        ghostlink_target="$ghostlink_arch-unknown-linux-gnu"
        ghostlink_zig_target="$ghostlink_arch-linux-gnu"
        case "${XDG_CACHE_HOME:-}" in
            /*) ghostlink_cache="$XDG_CACHE_HOME/ghostlink/build" ;;
            *) ghostlink_cache="$HOME/.cache/ghostlink/build" ;;
        esac
        ;;
    Darwin)
        ghostlink_os=macos
        ghostlink_target="$ghostlink_arch-apple-darwin"
        export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-12.0}"
        ghostlink_zig_target="$ghostlink_arch-macos.$MACOSX_DEPLOYMENT_TARGET"
        ghostlink_cache="$HOME/Library/Caches/Ghostlink/build"
        ;;
    *) ghostlink_fail 'Use these helpers on macOS/Linux, or scripts/*.ps1 on Windows.' ;;
esac
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ghostlink_cache/target/$ghostlink_target}"
case "$CARGO_TARGET_DIR" in
    /*) ;;
    *) export CARGO_TARGET_DIR="$ghostlink_root/$CARGO_TARGET_DIR" ;;
esac

#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

skip_build=false
case "${1:-}" in
    '') ;;
    --skip-build) skip_build=true ;;
    *) ghostlink_fail 'Usage: bash scripts/bootstrap.sh [--skip-build]' ;;
esac
[[ $# -le 1 ]] || ghostlink_fail 'Too many arguments.'
for tool in git curl tar cc; do
    command -v "$tool" >/dev/null || ghostlink_fail "Install $tool before bootstrapping. See README.md."
done
case "$ghostlink_arch-$ghostlink_os" in
    x86_64-linux) hash=70e49664a74374b48b51e6f3fdfbf437f6395d42509050588bd49abe52ba3d00 ;;
    aarch64-linux) hash=ea4b09bfb22ec6f6c6ceac57ab63efb6b46e17ab08d21f69f3a48b38e1534f17 ;;
    x86_64-macos) hash=0387557ed1877bc6a2e1802c8391953baddba76081876301c522f52977b52ba7 ;;
    aarch64-macos) hash=b23d70deaa879b5c2d486ed3316f7eaa53e84acf6fc9cc747de152450d401489 ;;
esac
tools="$ghostlink_root/.tools"
deps="$ghostlink_root/.deps"
distribution="zig-$ghostlink_arch-$ghostlink_os-$ghostlink_zig_version"
archive="$tools/$distribution.tar.xz"
zig="$tools/$distribution/zig"
mkdir -p "$tools" "$deps"
if [[ ! -f "$tools/$distribution/.ghostlink-complete" ]]; then
    printf 'Downloading Zig %s for %s/%s...\n' "$ghostlink_zig_version" "$ghostlink_os" "$ghostlink_arch"
    curl --fail --location --retry 3 --continue-at - --output "$archive" \
        "https://ziglang.org/download/$ghostlink_zig_version/$distribution.tar.xz"
    if command -v sha256sum >/dev/null; then
        actual="$(sha256sum "$archive")"
    else
        actual="$(shasum -a 256 "$archive")"
    fi
    [[ "${actual%% *}" == "$hash" ]] || ghostlink_fail "Zig checksum mismatch: $archive. Remove that archive and rerun."
    tar -xf "$archive" -C "$tools"
    touch "$tools/$distribution/.ghostlink-complete"
fi
[[ "$("$zig" version)" == "$ghostlink_zig_version" ]] || ghostlink_fail 'Cached Zig has the wrong version.'

source_dir="$deps/ghostty-$ghostlink_revision"
if [[ ! -d "$source_dir/.git" ]]; then
    git init "$source_dir"
    git -C "$source_dir" remote add origin https://github.com/ghostty-org/ghostty.git
    git -C "$source_dir" sparse-checkout init --no-cone
    git -C "$source_dir" sparse-checkout set /build.zig /build.zig.zon /src/ /include/ /pkg/
fi
head="$(git -C "$source_dir" rev-parse --verify HEAD 2>/dev/null || true)"
if [[ "$head" != "$ghostlink_revision" ]]; then
    git -C "$source_dir" fetch --depth 1 --filter=blob:none origin "$ghostlink_revision"
    git -C "$source_dir" checkout --detach "$ghostlink_revision"
fi

if [[ "$skip_build" == false ]]; then
    # Keep compiler/package churn out of synced source directories.
    native="$ghostlink_cache/native/$ghostlink_revision/$ghostlink_target"
    mkdir -p "$native"
    tar -C "$source_dir" --exclude=./.git --exclude=./zig-out --exclude=./zig-pkg \
        --exclude=./.zig-cache -cf - . | tar -C "$native" -xf -
    (
        cd "$native"
        "$zig" build -Demit-lib-vt -Demit-xcframework=false -Doptimize=ReleaseFast \
            "-Dtarget=$ghostlink_zig_target" \
            --cache-dir "$ghostlink_cache/zig-local/$ghostlink_target" \
            --global-cache-dir "$ghostlink_cache/zig-global" \
            --prefix "$source_dir/zig-out/$ghostlink_target"
    )
fi
printf 'Ghostty source: %s\nNative library: %s\n' "$source_dir" "$source_dir/zig-out/$ghostlink_target/lib"

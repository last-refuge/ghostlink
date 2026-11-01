#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
[[ $# == 0 ]] || ghostlink_fail 'Usage: bash scripts/package.sh'
bash "$ghostlink_root/scripts/dev.sh" release
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ghostlink_root/Cargo.toml")"
dist="$ghostlink_root/dist"
mkdir -p "$dist" "$ghostlink_cache/packaging"
# Temporary packaging files also stay out of synced worktrees.
stage="$(mktemp -d "$ghostlink_cache/packaging/.package.XXXXXX")"
cleanup() {
    # Only remove the directory created by this packaging run.
    case "$stage" in "$ghostlink_cache/packaging/.package."*) rm -rf -- "$stage" ;; esac
}
trap cleanup EXIT
name="ghostlink-$version-$ghostlink_os-$ghostlink_arch"
binary="$CARGO_TARGET_DIR/release/ghostlink"
if [[ "$ghostlink_os" == macos ]]; then
    app="$stage/Ghostlink.app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    cp "$binary" "$app/Contents/MacOS/ghostlink"
    chmod 755 "$app/Contents/MacOS/ghostlink"
    cp "$ghostlink_root/packaging/macos/Info.plist" "$app/Contents/Info.plist"
    /usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$app/Contents/Info.plist"
    /usr/libexec/PlistBuddy -c "Set :CFBundleVersion $version" "$app/Contents/Info.plist"
    /usr/libexec/PlistBuddy -c "Set :LSMinimumSystemVersion $MACOSX_DEPLOYMENT_TARGET" "$app/Contents/Info.plist"
    codesign --force --sign - "$app"
    codesign --verify --deep --strict "$app"
    ditto -c -k --sequesterRsrc --keepParent "$app" "$dist/$name.zip"
    printf 'App bundle: %s\n' "$dist/$name.zip"
else
    package="$stage/$name"
    mkdir -p "$package/bin" "$package/share/applications" "$package/share/pixmaps" "$package/docs"
    install -m755 "$binary" "$package/bin/ghostlink"
    install -m644 "$ghostlink_root/packaging/linux/io.github.last-refuge.ghostlink.desktop" "$package/share/applications/"
    install -m644 "$ghostlink_root/branding/ghostlink-logo-v1.png" "$package/share/pixmaps/ghostlink.png"
    cp "$ghostlink_root/README.md" "$package/README.md"
    cp "$ghostlink_root/docs/building.md" "$package/docs/building.md"
    tar -czf "$dist/$name.tar.gz" -C "$stage" "$name"
    printf 'Linux archive: %s\n' "$dist/$name.tar.gz"
fi

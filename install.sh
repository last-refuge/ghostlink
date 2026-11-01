#!/usr/bin/env bash
# Install the latest tested preview. Safe to run again to update.
set -euo pipefail

fail() { printf 'Ghostlink: %s\n' "$*" >&2; exit 1; }
download() { curl --fail --silent --show-error --location --retry 3 --proto '=https,file' --proto-redir '=https' "$1" --output "$2"; }
as_root() {
    if [[ $(id -u) == 0 ]]; then "$@"; else sudo "$@"; fi
}

main() (
    [[ $(uname -s) == Linux ]] || fail 'This installer supports Linux. Download Windows/macOS builds from GitHub Releases.'
    for tool in curl tar sha256sum awk mktemp; do
        command -v "$tool" >/dev/null || fail "Install $tool and try again."
    done
    case "$(uname -m)" in
        x86_64|amd64) arch=x86_64 ;;
        aarch64|arm64) arch=aarch64 ;;
        *) fail 'Supported CPU architectures: x86_64 and aarch64.' ;;
    esac
    base=${GHOSTLINK_RELEASE_BASE_URL:-https://github.com/last-refuge/ghostlink/releases/download/preview}
    stage=$(mktemp -d)
    trap 'rm -rf -- "$stage"' EXIT
    asset="ghostlink-linux-$arch.tar.gz"
    native_arch=false
    if [[ $arch == x86_64 ]] && command -v pacman >/dev/null; then
        asset=ghostlink-arch-x86_64.pkg.tar.zst
        native_arch=true
    fi
    printf 'Downloading Ghostlink for %s...\n' "$arch"
    download "$base/SHA256SUMS" "$stage/SHA256SUMS"
    download "$base/$asset" "$stage/$asset"
    expected=$(awk -v name="$asset" '$2 == name {print $1}' "$stage/SHA256SUMS")
    [[ $expected =~ ^[[:xdigit:]]{64}$ ]] || fail "Missing or invalid checksum for $asset."
    actual=$(sha256sum "$stage/$asset")
    [[ ${actual%% *} == "$expected" ]] || fail 'Download checksum mismatch; nothing was installed.'

    if [[ $native_arch == true ]]; then
        # Dependencies and subsequent removal are managed by pacman.
        as_root pacman -U --needed --noconfirm "$stage/$asset"
        printf 'Ghostlink installed. Open it from your app launcher or run ghostlink.\n'
        exit
    fi

    command -v ldd >/dev/null || fail 'Install glibc tools (ldd) and try again.'
    root=$(tar -tzf "$stage/$asset" | awk 'NR == 1 {sub(/\/$/, ""); print} {next}')
    [[ $root =~ ^ghostlink-[0-9][a-zA-Z0-9.+-]*-linux-$arch$ ]] || fail 'Unexpected archive layout.'
    tar -xzf "$stage/$asset" -C "$stage" --no-same-owner --no-same-permissions \
        "$root/bin/ghostlink" "$root/share/pixmaps/ghostlink.png"
    binary="$stage/$root/bin/ghostlink"
    [[ -f $binary && ! -L $binary ]] || fail 'Archive is missing the executable.'
    dependencies=$(ldd "$binary" 2>&1) || true
    if [[ $dependencies == *'GLIBC_'*'not found'* ]]; then
        fail 'This build requires Ubuntu 24.04 or a newer compatible glibc. Build from source on older systems.'
    fi
    if [[ $dependencies == *'not found'* ]]; then
        if command -v apt-get >/dev/null; then
            as_root apt-get update
            as_root apt-get install -y libfontconfig1 libfreetype6 libxcb1 libx11-xcb1 \
                libxcb-shape0 libxcb-xfixes0 libxkbcommon0 libxkbcommon-x11-0 \
                libwayland-client0 libvulkan1 fonts-dejavu-core
        elif command -v dnf >/dev/null; then
            as_root dnf install -y fontconfig freetype libxcb libX11 libxkbcommon \
                libxkbcommon-x11 wayland-libs vulkan-loader dejavu-sans-mono-fonts
        else
            printf '%s\n' "$dependencies" >&2
            fail 'Install the missing runtime libraries listed above and try again.'
        fi
    fi
    ldd "$binary" > "$stage/dependencies" 2>&1 || { cat "$stage/dependencies" >&2; fail 'Runtime libraries are incompatible.'; }
    if [[ $(< "$stage/dependencies") == *'not found'* ]]; then
        cat "$stage/dependencies" >&2
        fail 'Runtime libraries are still missing; nothing was installed.'
    fi

    bindir="$HOME/.local/bin"
    data=${XDG_DATA_HOME:-$HOME/.local/share}
    [[ $data == /* ]] || fail 'XDG_DATA_HOME must be an absolute path.'
    # Desktop entry quoting cannot represent literal newlines in paths.
    [[ $bindir != *$'\n'* && $data != *$'\n'* ]] || fail 'Install paths cannot contain newlines.'
    mkdir -p "$bindir" "$data/applications" "$data/pixmaps"
    # Replace the inode so updating a running app does not truncate its binary.
    pending=$(mktemp "$bindir/.ghostlink.XXXXXX")
    trap 'rm -f -- "${pending:-}"; rm -rf -- "$stage"' EXIT
    install -m755 "$binary" "$pending"
    mv -f -- "$pending" "$bindir/ghostlink"
    install -m644 "$stage/$root/share/pixmaps/ghostlink.png" "$data/pixmaps/ghostlink.png"
    exec_path=${bindir//\\/\\\\}
    exec_path=${exec_path//\"/\\\"}
    exec_path=${exec_path//\$/\\\$}
    exec_path=${exec_path//\`/\\\`}
    exec_path=${exec_path//%/%%}
    cat > "$data/applications/io.github.last-refuge.ghostlink.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Ghostlink
Comment=An SSH workspace
Exec="$exec_path/ghostlink"
Icon=ghostlink
Terminal=false
Categories=Network;RemoteAccess;
StartupWMClass=io.github.last-refuge.ghostlink
EOF
    if command -v update-desktop-database >/dev/null; then
        update-desktop-database "$data/applications" || true
    fi
    printf 'Ghostlink installed. Open it from your app launcher.\n'
    case ":$PATH:" in
        *":$bindir:"*) printf 'Or run: ghostlink\n' ;;
        *) printf 'Or run: %s/ghostlink\n' "$bindir" ;;
    esac
)

if [[ -z ${BASH_SOURCE[0]:-} || ${BASH_SOURCE[0]} == "$0" ]]; then
    main "$@"
fi

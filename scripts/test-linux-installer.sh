#!/usr/bin/env bash
# Exercise installation and failure paths without touching the user's files.
set -euo pipefail
repository=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
stage=$(mktemp -d)
trap 'rm -rf -- "$stage"' EXIT
export HOME="$stage/home with spaces"
export XDG_DATA_HOME="$HOME/data"
export GHOSTLINK_RELEASE_BASE_URL="file://$stage/release"
mkdir -p "$HOME" "$stage/release" "$stage/mock" "$stage/payload/ghostlink-0.1.0-linux-x86_64/"{bin,share/pixmaps}
cp /bin/true "$stage/payload/ghostlink-0.1.0-linux-x86_64/bin/ghostlink"
printf 'fixture-icon' > "$stage/payload/ghostlink-0.1.0-linux-x86_64/share/pixmaps/ghostlink.png"
tar -czf "$stage/release/ghostlink-linux-x86_64.tar.gz" -C "$stage/payload" ghostlink-0.1.0-linux-x86_64
printf 'fixture-package' > "$stage/release/ghostlink-arch-x86_64.pkg.tar.zst"
(
    cd "$stage/release"
    sha256sum ghostlink-* > SHA256SUMS
)
cat > "$stage/mock/uname" <<'EOF'
#!/bin/sh
case "$1" in -s) echo Linux ;; -m) echo x86_64 ;; *) exit 1 ;; esac
EOF
chmod +x "$stage/mock/uname"
export PATH="$stage/mock:$PATH"
bash < "$repository/install.sh"
cmp /bin/true "$HOME/.local/bin/ghostlink"
desktop="$XDG_DATA_HOME/applications/io.github.last-refuge.ghostlink.desktop"
desktop-file-validate "$desktop"
[[ $(< "$desktop") == *"Exec=\"$HOME/.local/bin/ghostlink\""* ]]
# Updates preserve user data, and are safe while the old binary is open.
mkdir -p "$XDG_DATA_HOME/ghostlink"
printf 'saved-connection' > "$XDG_DATA_HOME/ghostlink/connections.json"
exec 9< "$HOME/.local/bin/ghostlink"
bash "$repository/install.sh"
cmp /bin/true "$HOME/.local/bin/ghostlink"
[[ $(< "$XDG_DATA_HOME/ghostlink/connections.json") == saved-connection ]]
printf 'corrupt' >> "$stage/release/ghostlink-linux-x86_64.tar.gz"
if bash "$repository/install.sh" > "$stage/failure.log" 2>&1; then
    echo 'Corrupt download was accepted' >&2; exit 1
fi
[[ $(< "$stage/failure.log") == *'checksum mismatch'* ]]
cmp /bin/true "$HOME/.local/bin/ghostlink"
# Arch selects the native package and passes only the verified path to pacman.
cat > "$stage/mock/pacman" <<'EOF'
#!/bin/sh
printf '%s\n' "$@" > "$HOME/pacman-args"
test "$1" = -U && test "$2" = --needed && test "$3" = --noconfirm
test "$(cat "$4")" = fixture-package
EOF
cat > "$stage/mock/sudo" <<'EOF'
#!/bin/sh
exec "$@"
EOF
chmod +x "$stage/mock/pacman" "$stage/mock/sudo"
bash "$repository/install.sh"
[[ $(< "$HOME/pacman-args") == *ghostlink-arch-x86_64.pkg.tar.zst* ]]
printf 'Installer checks passed (install, update, checksum failure, Arch selection).\n'

# Building Ghostlink

Build on the target OS with Rust **1.93.0** via rustup. Bootstrap pins Ghostty to `f64f4aca2c29b554d111b36c3d946a9bddd159ff` and downloads checksum-verified Zig **0.16.0** into the checkout. System Zig versions are not used. Native scripts support x86_64 Windows and x86_64/aarch64 macOS and Linux; these helpers do not cross-compile the GPUI app between operating systems.

## Windows

Install Git, rustup, and Visual Studio C++ Build Tools with the Windows SDK. In PowerShell:

```powershell
./scripts/bootstrap.ps1
./scripts/dev.ps1 run
```

The helpers use `%TEMP%\ghostlink-target` for Cargo output and temporary native build caches outside the checkout. The development executable is `%TEMP%\ghostlink-target\debug\ghostlink.exe`.

### Windows installer

```powershell
./scripts/package-windows.ps1
```

This builds an optimized release app, discovers the installed SDK's shader compiler, and creates:

- `dist/ghostlink-<version>-windows-x86_64-setup.exe`
- `dist/ghostlink-<version>-windows-x86_64.zip`

The setup executable installs for the current user under `%LOCALAPPDATA%\Programs\Ghostlink`, creates one Start menu shortcut, and registers an uninstaller in Windows Settings. It needs no administrator prompt or network connection during installation. The portable ZIP includes the same release executable and app-local Microsoft runtime DLLs; extract the whole ZIP before running `ghostlink.exe`.

Visual Studio's redistributable CRT files are copied into the app directory, following [Microsoft's app-local deployment model](https://learn.microsoft.com/en-us/cpp/windows/walkthrough-deploying-a-visual-cpp-application-to-an-application-local-folder?view=msvc-170). The Microsoft C++ Build Tools installation must include its x64 redist components. No global redistributable installer is run. libghostty-vt and brand assets are embedded in the application.

The helper downloads pinned **Inno Setup 7.1.1**, verifies its SHA-256 and publisher signature, and prepares it in portable mode under `.tools/inno-7.1.1-x64`. This compiler preparation does not create file associations, shortcuts, or an Inno Setup uninstall entry. The app's installer recipe is `packaging/windows/ghostlink.iss`; its stable app ID preserves upgrade/uninstall identity across versions. Temporary payloads and installer-test fixtures live under `%TEMP%\ghostlink-packaging` rather than the synced source tree.

Close all Ghostlink instances before updating or uninstalling. A running-process marker makes Setup/Uninstall wait for the user to close the app; silent installs fail instead. Restart Manager auto-close is disabled, protecting active SSH sessions. Uninstall removes installer-owned program files, runtime DLLs, shortcut, and uninstall entry. Saved connections and `known_hosts` stay under `%LOCALAPPDATA%\Ghostlink`; user-created files are not recursively deleted.

These are **unsigned development installers**. Windows may display an unknown-publisher/SmartScreen prompt when downloading them. Public release signing requires a publisher code-signing certificate and is separate release setup.

Run the installer integration check after packaging:

```powershell
./scripts/test-windows-installer.ps1
```

It uses a temporary directory and test-only app ID, shortcut, and uninstall registration, then removes the fixture. It checks installed file hashes/runtime deployment, running-app protection, an in-place version upgrade, shortcuts, startup, uninstall, and preservation of test-owned data. It does not close existing Ghostlink processes. `-SkipLaunch` skips only the GUI startup step for CI machines without a display/GPU; install/upgrade/uninstall and running-app protection still run. The Windows CI job uses a Server 2025 runner compatible with the installer OS minimum, performs these checks, and uploads both Windows artifacts.

## macOS

Install rustup, Git, full Xcode with its macOS/Metal components, and CMake. Xcode command-line tools alone do not provide the Metal shader compiler GPUI uses. Select your Xcode installation as described in [GPUI's upstream macOS build prerequisites](https://zed.dev/docs/development/macos). With Homebrew, CMake is `brew install cmake`.

```bash
xcrun --find clang
xcrun --find metal
xcrun metal --version
bash scripts/bootstrap.sh
bash scripts/dev.sh run
bash scripts/package.sh
```

If `metal --version` reports a missing Metal Toolchain, install the component with `xcodebuild -downloadComponent MetalToolchain`, then retry. `xcrun --find metal` alone can succeed even when this component is missing.

The helpers build for the running architecture: Apple Silicon produces `aarch64-apple-darwin`; Intel produces `x86_64-apple-darwin`. Use native runners for each architecture. The deployment target defaults to macOS 12.0 and can be set with `MACOSX_DEPLOYMENT_TARGET` before **both** bootstrap and Cargo builds. Older macOS versions still need native runtime verification.

Packaging produces `dist/ghostlink-<version>-macos-<arch>.zip`, containing `Ghostlink.app`. Unzip and copy the app to Applications, or open it directly. The binary embeds the brand assets and Metal shaders. The bundle uses an ad-hoc signature, verified during packaging; it is a development artifact, without a Developer ID signature or Apple notarization. A signed public release needs that separate release setup.

## Linux: Ubuntu/Debian

Install rustup, then the development libraries. This list is used by the Ubuntu 24.04 CI builds:

```bash
sudo apt-get update
sudo apt-get install -y --no-install-recommends \
  build-essential git curl xz-utils cmake clang libclang-dev pkg-config libssl-dev \
  libfontconfig1-dev libx11-xcb-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libxkbcommon-x11-dev libwayland-dev libvulkan1 fonts-dejavu-core \
  openssh-client desktop-file-utils
bash scripts/bootstrap.sh
bash scripts/dev.sh run
bash scripts/package.sh
```

Both Wayland and X11 backends are built. A working Vulkan driver/loader is required at runtime; install the appropriate GPU driver for the machine. `xdg-desktop-portal` and the desktop's matching portal backend provide desktop integrations. Real display sessions, clipboard, and IME need native GUI verification beyond the headless tests. CI also runs `bash scripts/smoke-linux.sh` against a virtual X11 display with software Vulkan; it requires `xvfb`, `xauth`, `x11-utils`, and `mesa-vulkan-drivers`, and launches an isolated empty workspace from outside the checkout.

The tarball at `dist/ghostlink-<version>-linux-<arch>.tar.gz` contains `bin/ghostlink`, a desktop launcher, the brand PNG, and a README. Extract and run `bin/ghostlink`, or install into a prefix:

```bash
# From the extracted archive; assumes ~/.local/bin is on your desktop session's PATH.
install -Dm755 bin/ghostlink ~/.local/bin/ghostlink
install -Dm644 share/applications/io.github.last-refuge.ghostlink.desktop \
  ~/.local/share/applications/io.github.last-refuge.ghostlink.desktop
install -Dm644 share/pixmaps/ghostlink.png ~/.local/share/pixmaps/ghostlink.png
update-desktop-database ~/.local/share/applications
```

The executable links system desktop libraries; this is a native archive, not a statically bundled/AppImage distribution. CI's Linux artifacts use Ubuntu 24.04. Building on your own distribution uses its native libraries.

## Arch / Omarchy

Use the same Linux build on Arch/Omarchy, including native Wayland. Install the prerequisites, or use the pacman package recipe below:

```bash
sudo pacman -Syu --needed base-devel git rustup clang cmake pkgconf curl xz \
  fontconfig freetype2 libxcb libx11 libxkbcommon libxkbcommon-x11 wayland \
  vulkan-icd-loader zlib ttf-dejavu openssh desktop-file-utils
bash scripts/bootstrap.sh
bash scripts/dev.sh run
```

To build and install a pacman-managed package:

```bash
cd packaging/arch
makepkg -si
```

Review `PKGBUILD` before running it, as with any Arch package recipe. It follows the committed `main` branch and produces `ghostlink-git-*.pkg.tar.zst`; it is included in the repository, not published to the AUR. `makepkg` runs the tests and installs the executable, launcher, icon, and README under `/usr`. It runs as your regular user; only dependency installation and the final pacman installation require elevated access.

For a specific committed checkout, override the VCS source, for example before entering `packaging/arch`:

```bash
export GHOSTLINK_SOURCE_URL="git+file://$(git rev-parse --show-toplevel)#commit=$(git rev-parse HEAD)"
```

CI uses this override to build the exact checked-out commit, including pull requests. The recipe can also be built on an aarch64 Arch Linux ARM system; the Arch CI job currently uses x86_64. Omarchy/Hyprland graphical behavior remains a native runtime gate.

## Checks, caches, and CI

On Unix:

```bash
bash scripts/dev.sh check
bash scripts/dev.sh test
bash scripts/dev.sh clippy
bash scripts/dev.sh fmt
```

`build` builds the workspace in debug mode; `release` builds the release app; `package.sh` runs the release build and packages it. Build helpers work from any working directory. They reuse an installed stable toolchain only when it exactly matches the pinned Rust version.

Cargo and native build caches live under `~/Library/Caches/Ghostlink/build` on macOS, or `$XDG_CACHE_HOME/ghostlink/build` (default `~/.cache/ghostlink/build`) on Linux. `CARGO_TARGET_DIR` overrides Cargo's location. Zig artifacts install into `.deps/ghostty-<revision>/zig-out/<rust-target>/lib`, isolating architectures and Windows/WSL. `GHOSTTY_DIR` and `GHOSTTY_LIB_DIR` can point Cargo at an explicitly built source/archive directory instead. A documented compatibility patch under `vendor/xattr` preserves GPUI's older dependency API with modern libc; its upstream source, tests, and licenses are retained.

GitHub Actions has two independent workflows. **Build installers** builds and packages Windows, both Mac architectures, Linux x86_64/aarch64, and the Arch package (`makepkg --nocheck`), verifies the packages (Windows install/upgrade/uninstall, Linux startup smoke test), and publishes the **Preview** prerelease after every build succeeds. It does not run the test suite. **Tests** runs `fmt`, `clippy`, `test`, and the Linux installer script tests on the same platforms; a test failure shows there without blocking the Preview. Both run on pushes to `main` and pull requests and can be dispatched manually (`gh workflow run tests.yml`). Release assets have stable platform names and a `SHA256SUMS` manifest. The preview tag moves to the built commit; release assets are replaced on updates. Pull requests run both without publishing. No package is uploaded to the AUR. Building the Arch recipe locally with plain `makepkg` still runs its `check()` test step; pass `--nocheck` to skip it.

For prebuilt Linux / Arch / Omarchy installs, use:

```bash
curl -fsSL https://github.com/last-refuge/ghostlink/releases/download/preview/install.sh | bash
```

The script verifies checksums before installing. Arch/Omarchy x86_64 uses `pacman -U`; other Linux systems use `~/.local/bin/ghostlink` and `$XDG_DATA_HOME/applications` / `pixmaps` (default `~/.local/share`). The desktop launcher uses the full executable path, so it works even when `~/.local/bin` is absent from the desktop's PATH. Rerun to update. To remove a per-user install, delete that binary, `io.github.last-refuge.ghostlink.desktop`, and `pixmaps/ghostlink.png`; for the Arch package, use `sudo pacman -R ghostlink-git`. Saved connections and host trust are retained.

A successful CI build proves compilation and automated behavior, while native rendering, key mappings, clipboard, IME, and mixed-DPI behavior remain separate runtime checks.

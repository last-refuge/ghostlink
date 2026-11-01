[CmdletBinding()]
param([ValidateSet('run', 'build', 'release', 'check', 'test', 'clippy', 'fmt')][string]$Task = 'run')
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
if (!$env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = Join-Path $env:TEMP 'ghostlink-target' }

# Reuse an installed stable toolchain only when it is exactly the pinned version.
# This avoids downloading a second copy of the same compiler under a numbered name.
$toolchain = '+1.93.0'
$installed = & rustup toolchain list
if ($installed -match 'stable-x86_64-pc-windows-msvc') {
    $stableVersion = & rustc +stable --version
    if ($stableVersion -match '^rustc 1\.93\.0 ') { $toolchain = '+stable' }
}
$arguments = switch ($Task) {
    'run' { @('run', '--locked', '-p', 'ghostlink') }
    'build' { @('build', '--locked', '--workspace') }
    'release' { @('build', '--locked', '--release', '-p', 'ghostlink') }
    'check' { @('check', '--locked', '--workspace') }
    'test' { @('test', '--locked', '--workspace') }
    'clippy' { @('clippy', '--locked', '--workspace', '--all-targets', '--', '-D', 'warnings') }
    'fmt' { @('fmt', '--all', '--', '--check') }
}
Push-Location $root
try {
    & cargo $toolchain @arguments
    if ($LASTEXITCODE -ne 0) { throw "cargo $Task failed ($LASTEXITCODE)." }
} finally { Pop-Location }

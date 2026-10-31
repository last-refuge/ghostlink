[CmdletBinding()]
param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$root = Split-Path -Parent $PSScriptRoot
$deps = Join-Path $root '.deps'
$tools = Join-Path $root '.tools'
$revision = 'f64f4aca2c29b554d111b36c3d946a9bddd159ff'
$zigVersion = '0.16.0'
$zigHash = '68659eb5f1e4eb1437a722f1dd889c5a322c9954607f5edcf337bc3684a75a7e'
New-Item -ItemType Directory -Force -Path $deps, $tools | Out-Null

$zig = Join-Path $tools "zig-x86_64-windows-$zigVersion/zig.exe"
if (!(Test-Path -LiteralPath $zig)) {
    $archive = Join-Path $tools "zig-$zigVersion.zip"
    if (!(Test-Path -LiteralPath $archive) -or (Get-Item -LiteralPath $archive).Length -ne 97217739) {
        Write-Host "Downloading Zig $zigVersion..."
        & curl.exe --fail --location --retry 2 --continue-at - --max-time 1200 --output $archive "https://ziglang.org/download/$zigVersion/zig-x86_64-windows-$zigVersion.zip"
        if ($LASTEXITCODE -ne 0) { throw 'Downloading Zig failed. Rerun to resume.' }
    }
    if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $zigHash) {
        throw "Zig archive checksum mismatch: $archive"
    }
    & tar -xf $archive -C $tools
    if ($LASTEXITCODE -ne 0) { throw 'Extracting Zig failed.' }
}

$source = Join-Path $deps "ghostty-$revision"
function Invoke-GitChecked {
    param([string[]]$GitArguments)
    & git @GitArguments
    if ($LASTEXITCODE -ne 0) { throw "Git failed: $($GitArguments -join ' ')" }
}
if (!(Test-Path -LiteralPath (Join-Path $source '.git'))) {
    Invoke-GitChecked -GitArguments @('init', $source)
    Invoke-GitChecked -GitArguments @('-C', $source, 'remote', 'add', 'origin', 'https://github.com/ghostty-org/ghostty.git')
    Invoke-GitChecked -GitArguments @('-C', $source, 'sparse-checkout', 'init', '--no-cone')
    Invoke-GitChecked -GitArguments @('-C', $source, 'sparse-checkout', 'set', '/build.zig', '/build.zig.zon', '/src/', '/include/', '/pkg/')
}
$head = & git -C $source rev-parse --verify HEAD 2>$null
if ($LASTEXITCODE -ne 0 -or $head -ne $revision) {
    Invoke-GitChecked -GitArguments @('-C', $source, 'fetch', '--depth', '1', '--filter=blob:none', 'origin', $revision)
    Invoke-GitChecked -GitArguments @('-C', $source, 'checkout', '--detach', $revision)
}

if (!$SkipBuild) {
    # Rust and Zig create/delete temporary files rapidly. Dropbox locks those files;
    # keep all compilation and Zig package extraction in a local temporary cache.
    $native = Join-Path $env:TEMP "ghostlink-native/$revision"
    New-Item -ItemType Directory -Force -Path $native | Out-Null
    & robocopy $source $native /E /XD .git .zig-cache zig-pkg zig-out /NFL /NDL /NJH /NJS /NP
    if ($LASTEXITCODE -gt 7) { throw 'Copying native build sources failed.' }
    Push-Location $native
    try {
        & $zig build -Demit-lib-vt -Doptimize=ReleaseFast -Dtarget=x86_64-windows-msvc --cache-dir (Join-Path $env:TEMP 'ghostlink-zig-local') --global-cache-dir (Join-Path $env:TEMP 'ghostlink-zig-cache')
        if ($LASTEXITCODE -ne 0) { throw 'Building libghostty-vt failed.' }
    } finally { Pop-Location }
    & robocopy (Join-Path $native 'zig-out') (Join-Path $source 'zig-out') /E /NFL /NDL /NJH /NJS /NP
    if ($LASTEXITCODE -gt 7) { throw 'Copying native library artifacts failed.' }
}
Write-Host "Ghostty source: $source"

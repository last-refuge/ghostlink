[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { throw 'Run this packaging helper on Windows.' }
$root = Split-Path -Parent $PSScriptRoot
$cache = Join-Path $env:TEMP 'ghostlink-packaging'
$dist = Join-Path $root 'dist'
New-Item -ItemType Directory -Force -Path $cache, $dist | Out-Null

$manifest = Get-Content -Raw -LiteralPath (Join-Path $root 'Cargo.toml')
$version = [regex]::Match($manifest, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
$numeric = [regex]::Match($version, '^(\d+)\.(\d+)\.(\d+)')
if (!$numeric.Success) { throw "Unsupported application version: $version" }
$fileVersion = '{0}.{1}.{2}.0' -f $numeric.Groups[1].Value, $numeric.Groups[2].Value, $numeric.Groups[3].Value
$markerSource = Get-Content -Raw -LiteralPath (Join-Path $root 'crates/app/src/windows.rs')
$runningMutex = [regex]::Match($markerSource, 'pub const APP_MUTEX_NAME: &str = r"([^"]+)";').Groups[1].Value
if (!$runningMutex) { throw 'Could not read the application running marker.' }

# GPUI's pinned build script also accepts this override; discover SDK versions
# instead of assuming its one hard-coded fxc.exe path exists on every machine.
if (!$env:GPUI_FXC_PATH) {
    $kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits/10/bin'
    $fxc = Get-ChildItem -LiteralPath $kits -Directory -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' } |
        Sort-Object { [version]$_.Name } -Descending |
        ForEach-Object { Join-Path $_.FullName 'x64/fxc.exe' } |
        Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    if (!$fxc) { throw 'Install the Windows SDK with its x64 fxc shader compiler.' }
    $env:GPUI_FXC_PATH = $fxc
}
if (!(Test-Path -LiteralPath $env:GPUI_FXC_PATH)) { throw "Shader compiler not found: $env:GPUI_FXC_PATH" }
& (Join-Path $PSScriptRoot 'dev.ps1') release
if (!$env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = Join-Path $env:TEMP 'ghostlink-target' }
$target = $env:CARGO_TARGET_DIR
if (![IO.Path]::IsPathRooted($target)) { $target = Join-Path $root $target }
$binary = Join-Path $target 'release/ghostlink.exe'
if (!(Test-Path -LiteralPath $binary)) { throw "Missing release executable: $binary" }

# Keep the compiler private and portable: no file associations, shortcuts, or
# Inno Setup installation/uninstallation registry entries are created.
$innoVersion = '7.1.1'
$innoHash = '8a5a6711c3766a6e5fb74f280ba0e3551e719ce94538d901fdbd1f8f9920b3f0'
$compilerDir = Join-Path $root ".tools/inno-$innoVersion-x64"
$compiler = Join-Path $compilerDir 'ISCC.exe'
if (!(Test-Path -LiteralPath $compiler)) {
    $download = Join-Path $cache "innosetup-$innoVersion-x64.exe"
    if (!(Test-Path -LiteralPath $download) -or (Get-FileHash -LiteralPath $download -Algorithm SHA256).Hash.ToLowerInvariant() -ne $innoHash) {
        Invoke-WebRequest -UseBasicParsing -Uri "https://github.com/jrsoftware/issrc/releases/download/is-7_1_1/innosetup-$innoVersion-x64.exe" -OutFile $download
    }
    if ((Get-FileHash -LiteralPath $download -Algorithm SHA256).Hash.ToLowerInvariant() -ne $innoHash) { throw 'Inno Setup download checksum mismatch.' }
    $signature = Get-AuthenticodeSignature -LiteralPath $download
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'Pyrsys') { throw 'Inno Setup compiler installer signature is not valid.' }
    $arguments = @('/PORTABLE=1', '/CURRENTUSER', '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/DIR=`"$compilerDir`"")
    $install = Start-Process -FilePath $download -ArgumentList $arguments -WindowStyle Hidden -Wait -PassThru
    if ($install.ExitCode -ne 0 -or !(Test-Path -LiteralPath $compiler)) { throw "Preparing the portable compiler failed: $($install.ExitCode)" }
}
$compilerVersion = (& $compiler --version | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $compilerVersion -ne $innoVersion) { throw "Unexpected cached Inno Setup compiler version: $compilerVersion" }

# App-local runtime deployment keeps installation offline and avoids a system
# VC++ redistributable installer/UAC prompt. Only VS's redistributable DLLs ship.
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
if (!(Test-Path -LiteralPath $vswhere)) { throw 'Visual Studio C++ Build Tools and vswhere are required.' }
$visualStudio = (& $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath | Select-Object -First 1)
if (!$visualStudio) { throw 'Could not locate the Visual C++ toolchain.' }
$redistVersion = (Get-Content -LiteralPath (Join-Path $visualStudio 'VC/Auxiliary/Build/Microsoft.VCRedistVersion.default.txt') -Raw).Trim()
$runtime = Get-ChildItem -LiteralPath (Join-Path $visualStudio "VC/Redist/MSVC/$redistVersion/x64") -Directory |
    Where-Object Name -Like 'Microsoft.VC*.CRT' | Select-Object -First 1
if (!$runtime) { throw 'Install the Visual C++ redistributable components in Build Tools.' }
foreach ($required in @('vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll')) {
    if (!(Test-Path -LiteralPath (Join-Path $runtime.FullName $required))) { throw "Missing redistributable: $required" }
}

# Use a fresh, bounded staging folder; do not carry stale DLLs across builds.
$payload = Join-Path $cache ('payload-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path (Join-Path $payload 'docs') -Force | Out-Null
Copy-Item -LiteralPath $binary -Destination (Join-Path $payload 'ghostlink.exe')
Get-ChildItem -LiteralPath $runtime.FullName -Filter '*.dll' -File | Copy-Item -Destination $payload
Copy-Item -LiteralPath (Join-Path $root 'README.md') -Destination $payload
Copy-Item -LiteralPath (Join-Path $root 'docs/building.md') -Destination (Join-Path $payload 'docs/building.md')
Copy-Item -LiteralPath (Join-Path $root 'docs/theme-licenses.md') -Destination (Join-Path $payload 'docs/theme-licenses.md')
$recipe = Join-Path $root 'packaging/windows/ghostlink.iss'
$outputName = "ghostlink-$version-windows-x86_64-setup"
& $compiler --quiet-progress --no-ide-signtools "--define=AppVersion=$version" "--define=FileVersion=$fileVersion" "--define=PayloadDir=$payload" "--define=OutputDir=$dist" "--define=RunningMutex=$runningMutex" "--define=OutputName=$outputName" $recipe
if ($LASTEXITCODE -ne 0) { throw 'Compiling the Windows installer failed.' }
$archive = Join-Path $dist "ghostlink-$version-windows-x86_64.zip"
Compress-Archive -Path (Join-Path $payload '*') -DestinationPath $archive -Force
$result = [ordered]@{
    ProjectRoot = $root; Version = $version; FileVersion = $fileVersion
    Compiler = $compiler; Payload = $payload; Recipe = $recipe; RunningMutex = $runningMutex
    Installer = Join-Path $dist "$outputName.exe"; Archive = $archive
}
$result | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $cache 'installer-build.json') -Encoding utf8
Write-Host "Installer: $($result.Installer)"
Write-Host "Portable archive: $archive"

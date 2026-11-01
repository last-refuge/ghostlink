[CmdletBinding()]
param([switch]$SkipLaunch)
$ErrorActionPreference = 'Stop'
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { throw 'Run this smoke test on Windows.' }
$root = Split-Path -Parent $PSScriptRoot
$cache = Join-Path $env:TEMP 'ghostlink-packaging'
$build = Get-Content -Raw -LiteralPath (Join-Path $cache 'installer-build.json') | ConvertFrom-Json
if ($build.ProjectRoot -ne $root) { throw 'Run scripts/package-windows.ps1 in this checkout first.' }
if ($SkipLaunch) {
    # This fixture never launches the production executable, so its installer
    # marker can be private as well. Existing user sessions stay independent.
    $build.RunningMutex += '.InstallerSmoke.' + [guid]::NewGuid().ToString('N')
}

# Shell links may expose a short path or a different drive/path alias on hosted
# runners. Compare resolved file paths, while still rejecting a different file.
if (!('GhostlinkInstallerSmokePaths' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;
public static class GhostlinkInstallerSmokePaths {
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern uint GetFinalPathNameByHandle(SafeFileHandle handle, StringBuilder path, uint capacity, uint flags);
    public static string Resolve(string path) {
        using (var file = File.OpenRead(path)) {
            var buffer = new StringBuilder(32768);
            uint length = GetFinalPathNameByHandle(file.SafeFileHandle, buffer, (uint)buffer.Capacity, 0);
            if (length == 0) throw new Win32Exception(Marshal.GetLastWin32Error());
            if (length >= buffer.Capacity) throw new IOException("Resolved path exceeds buffer");
            return buffer.ToString();
        }
    }
}
'@
}

function Assert-Test([bool]$Condition, [string]$Message) {
    if (!$Condition) { throw $Message }
}
function Invoke-Setup([string]$File, [string[]]$SetupArguments) {
    # Inno Uninstall relaunches a temporary worker. -Wait tracks descendants,
    # whereas Process.WaitForExit only waits for the initial launcher.
    $process = Start-Process -FilePath $File -ArgumentList $SetupArguments -WindowStyle Hidden -Wait -PassThru
    return $process.ExitCode
}
function Open-RunningMarker {
    $marker = $null
    if ([Threading.Mutex]::TryOpenExisting($build.RunningMutex, [ref]$marker)) { return $marker }
    return $null
}
$existing = Open-RunningMarker
if ($existing) {
    $existing.Dispose()
    throw 'Another Ghostlink instance is running. The installer smoke test will not close it.'
}

# The fixture has its own app ID, uninstall entry, shortcut, and installation
# directory. Production installation metadata and saved connections are untouched.
$id = [guid]::NewGuid().ToString('N')
$appId = "Ghostlink.InstallerSmoke.$id"
$name = "Ghostlink Installer Test $id"
$testRoot = Join-Path $cache "smoke-$id"
$installDir = Join-Path $testRoot 'Installed App'
$userData = Join-Path $testRoot 'Saved Data'
$shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) "$name.lnk"
$registry = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\${appId}_is1"
$uninstaller = Join-Path $installDir 'unins000.exe'
$ownedApp = $null
$heldMarker = $null
$completed = $false
New-Item -ItemType Directory -Path $testRoot, $userData -Force | Out-Null
Set-Content -LiteralPath (Join-Path $userData 'connections.json') -Value '{"sentinel":"saved connections"}' -Encoding utf8
Set-Content -LiteralPath (Join-Path $userData 'known_hosts') -Value 'preserve trust sentinel' -Encoding utf8
$catalogHash = (Get-FileHash -LiteralPath (Join-Path $userData 'connections.json')).Hash
$trustHash = (Get-FileHash -LiteralPath (Join-Path $userData 'known_hosts')).Hash

function Compile-Fixture([string]$Version, [string]$FileVersion, [string]$OutputName) {
    & $build.Compiler --quiet --no-ide-signtools "--define=AppVersion=$Version" "--define=FileVersion=$FileVersion" "--define=PayloadDir=$($build.Payload)" "--define=OutputDir=$testRoot" "--define=RunningMutex=$($build.RunningMutex)" "--define=InstallerAppId=$appId" "--define=InstallerName=$name" "--define=OutputName=$OutputName" $build.Recipe
    if ($LASTEXITCODE -ne 0) { throw 'Compiling the isolated installer fixture failed.' }
    return (Join-Path $testRoot "$OutputName.exe")
}
$silent = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART')
try {
    $setup = Compile-Fixture $build.Version $build.FileVersion 'install'
    $arguments = $silent + @("/DIR=`"$installDir`"", "/LOG=`"$(Join-Path $testRoot 'install.log')`"")
    Assert-Test ((Invoke-Setup $setup $arguments) -eq 0) 'Initial installation failed.'
    $installedExe = Join-Path $installDir 'ghostlink.exe'
    Assert-Test (Test-Path -LiteralPath $installedExe) 'The executable was not installed.'
    Assert-Test (Test-Path -LiteralPath (Join-Path $installDir 'vcruntime140.dll')) 'App-local CRT deployment failed.'
    Assert-Test ((Get-FileHash -LiteralPath $installedExe).Hash -eq (Get-FileHash -LiteralPath (Join-Path $build.Payload 'ghostlink.exe')).Hash) 'Installed executable differs from the release payload.'
    Assert-Test ((Get-ItemProperty -LiteralPath $registry).DisplayVersion -eq $build.Version) 'Uninstall registration has the wrong version.'
    Assert-Test (Test-Path -LiteralPath $shortcut) 'Start menu shortcut was not created.'
    $shell = New-Object -ComObject WScript.Shell
    $link = $shell.CreateShortcut($shortcut)
    Assert-Test (Test-Path -LiteralPath $link.TargetPath) "Shortcut target does not exist: $($link.TargetPath)"
    $actualTarget = [GhostlinkInstallerSmokePaths]::Resolve($link.TargetPath)
    $expectedTarget = [GhostlinkInstallerSmokePaths]::Resolve($installedExe)
    Assert-Test ($actualTarget -eq $expectedTarget) "Shortcut points to '$actualTarget'; expected '$expectedTarget'."
    [void][Runtime.InteropServices.Marshal]::ReleaseComObject($link)
    [void][Runtime.InteropServices.Marshal]::ReleaseComObject($shell)
    Set-Content -LiteralPath (Join-Path $installDir 'user-notes.txt') -Value 'not installer owned' -Encoding utf8

    # Silent updates/uninstalls must fail safely while a session marker exists,
    # rather than closing terminals through Restart Manager.
    $created = $false
    $heldMarker = [Threading.Mutex]::new($false, $build.RunningMutex, [ref]$created)
    Assert-Test $created 'Another Ghostlink instance started during the test.'
    Assert-Test ((Invoke-Setup $setup ($silent + @("/DIR=`"$installDir`""))) -ne 0) 'Installer updated a running application.'
    Assert-Test ((Invoke-Setup $uninstaller $silent) -ne 0) 'Uninstaller removed a running application.'
    Assert-Test (Test-Path -LiteralPath $installedExe) 'Blocked uninstall removed the executable.'
    $heldMarker.Dispose()
    $heldMarker = $null

    $parts = [version]$build.FileVersion
    $nextVersion = '{0}.{1}.{2}' -f $parts.Major, $parts.Minor, ($parts.Build + 1)
    $upgrade = Compile-Fixture $nextVersion "$nextVersion.0" 'upgrade'
    Assert-Test ((Invoke-Setup $upgrade $arguments) -eq 0) 'In-place upgrade failed.'
    Assert-Test ((Get-ItemProperty -LiteralPath $registry).DisplayVersion -eq $nextVersion) 'Upgrade did not update the existing uninstall entry.'
    Assert-Test (@(Get-ChildItem -LiteralPath $installDir -Filter 'unins*.exe').Count -eq 1) 'Upgrade created a second uninstaller.'
    Assert-Test (Test-Path -LiteralPath (Join-Path $installDir 'user-notes.txt')) 'Upgrade removed a user-created file.'

    if (!$SkipLaunch) {
        $ownedApp = Start-Process -FilePath $installedExe -WorkingDirectory $testRoot -WindowStyle Hidden -PassThru
        Start-Sleep -Seconds 3
        $ownedApp.Refresh()
        Assert-Test (!$ownedApp.HasExited) 'Installed release executable exited during startup.'
        $marker = Open-RunningMarker
        Assert-Test ($null -ne $marker) 'Installed executable did not register its running marker.'
        $marker.Dispose()
        # Only this freshly launched, empty fixture is stopped. Never touch other PIDs.
        if (!$ownedApp.CloseMainWindow()) { $ownedApp.Kill() }
        if (!$ownedApp.WaitForExit(5000)) { $ownedApp.Kill(); $ownedApp.WaitForExit() }
        $ownedApp = $null
    }
    Assert-Test ((Invoke-Setup $uninstaller ($silent + @("/LOG=`"$(Join-Path $testRoot 'uninstall.log')`""))) -eq 0) 'Uninstall failed.'
    Assert-Test (!(Test-Path -LiteralPath $installedExe)) 'Uninstall left the application executable.'
    Assert-Test (!(Test-Path -LiteralPath $shortcut)) 'Uninstall left the Start menu shortcut.'
    Assert-Test (!(Test-Path -LiteralPath $registry)) 'Uninstall left its Apps & features entry.'
    Assert-Test (Test-Path -LiteralPath (Join-Path $installDir 'user-notes.txt')) 'Uninstall removed a user-created file.'
    Assert-Test ((Get-FileHash -LiteralPath (Join-Path $userData 'connections.json')).Hash -eq $catalogHash) 'Saved connection sentinel changed.'
    Assert-Test ((Get-FileHash -LiteralPath (Join-Path $userData 'known_hosts')).Hash -eq $trustHash) 'Host trust sentinel changed.'
    $completed = $true
} finally {
    if ($heldMarker) { $heldMarker.Dispose() }
    if ($ownedApp) {
        $ownedApp.Refresh()
        if (!$ownedApp.HasExited) { $ownedApp.Kill(); $ownedApp.WaitForExit() }
    }
    if (Test-Path -LiteralPath $uninstaller) { [void](Invoke-Setup $uninstaller $silent) }
    # Keep failed fixture logs for inspection. Remove only this run's known,
    # resolved cache directory after its uninstall entry is gone.
    $resolvedCache = [IO.Path]::GetFullPath($cache).TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
    $resolvedTest = [IO.Path]::GetFullPath($testRoot)
    if (!$resolvedTest.StartsWith($resolvedCache, [StringComparison]::OrdinalIgnoreCase) -or (Split-Path -Leaf $resolvedTest) -ne "smoke-$id") { throw 'Unsafe test cleanup path.' }
    if (!(Test-Path -LiteralPath $registry) -and $completed) {
        for ($attempt = 0; $attempt -lt 5; $attempt++) {
            try {
                if (Test-Path -LiteralPath $resolvedTest) { Remove-Item -LiteralPath $resolvedTest -Recurse -Force }
                break
            } catch [IO.IOException] {
                if ($attempt -eq 4) { throw }
                Start-Sleep -Milliseconds 200
            }
        }
    }
}
Write-Host 'Windows installer smoke test passed: install, running-app protection, upgrade, shortcuts, uninstall, and data preservation.'

; Build through scripts/package-windows.ps1, which supplies the verified payload.
#ifndef AppVersion
  #error AppVersion is required
#endif
#ifndef FileVersion
  #error FileVersion is required
#endif
#ifndef PayloadDir
  #error PayloadDir is required
#endif
#ifndef OutputDir
  #error OutputDir is required
#endif
#ifndef RunningMutex
  #error RunningMutex is required
#endif
#ifndef InstallerAppId
  #define InstallerAppId "io.github.last-refuge.ghostlink"
#endif
#ifndef InstallerName
  #define InstallerName "Ghostlink"
#endif
#ifndef OutputName
  #define OutputName "ghostlink-" + AppVersion + "-windows-x86_64-setup"
#endif

[Setup]
AppId={#InstallerAppId}
AppName={#InstallerName}
AppVersion={#AppVersion}
AppPublisher=Ghostlink
AppPublisherURL=https://github.com/last-refuge/ghostlink
AppSupportURL=https://github.com/last-refuge/ghostlink/issues
AppUpdatesURL=https://github.com/last-refuge/ghostlink/releases
VersionInfoVersion={#FileVersion}
VersionInfoDescription=Ghostlink Setup
DefaultDirName={localappdata}\Programs\Ghostlink
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.22000
SetupArchitecture=x64
AppMutex={#RunningMutex}
SetupMutex=Local\{#InstallerAppId}.setup
CloseApplications=no
RestartApplications=no
DisableWelcomePage=yes
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableReadyPage=no
WizardStyle=modern dynamic
UsePreviousAppDir=yes
UninstallDisplayIcon={app}\ghostlink.exe
OutputDir={#OutputDir}
OutputBaseFilename={#OutputName}
Compression=lzma2
SolidCompression=yes

[Files]
Source: "{#PayloadDir}\ghostlink.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\*.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\docs\building.md"; DestDir: "{app}\docs"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#InstallerName}"; Filename: "{app}\ghostlink.exe"; WorkingDir: "{app}"; AppUserModelID: "io.github.last-refuge.ghostlink"

[Run]
Filename: "{app}\ghostlink.exe"; Description: "Open Ghostlink"; Flags: nowait postinstall skipifsilent

; No UninstallDelete entries: connection catalogs, known_hosts, and user-created
; files are not installer-owned and must survive upgrades and uninstallation.

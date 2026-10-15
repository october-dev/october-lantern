; October Lantern for Windows: a per-user install (no administrator rights), with a Start menu
; shortcut, optional start at sign-in, and an uninstaller that removes Lantern's agent hooks.
; Built by scripts/package-windows.ps1 from build\windows:
;   ISCC /DAppVersion=0.4.2 /DArch=x86_64 windows\installer.iss
;
; Running agent sessions, their windows and the October Bus run from Lantern's own copies in
; %LOCALAPPDATA%\October Lantern\bin, so installing over an older version doesn't stop them.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef Arch
  #define Arch "x86_64"
#endif

[Setup]
AppId={{6C4C8A0E-5D0B-4F57-9D34-0C2A6F6E7B11}
AppName=October Lantern
AppVersion={#AppVersion}
AppVerName=October Lantern {#AppVersion}
AppPublisher=October
AppPublisherURL=https://october.dev
DefaultDirName={localappdata}\Programs\October Lantern
DisableDirPage=auto
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
#if Arch == "aarch64"
ArchitecturesAllowed=arm64
ArchitecturesInstallIn64BitMode=arm64
#else
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
#endif
MinVersion=10.0.17763
OutputDir=..\build
OutputBaseFilename=October-Lantern-windows-{#Arch}-setup
; The lantern on Setup and its uninstaller (scripts/make-icon-windows.py); Apps shows the icon
; embedded in october-lantern.exe (desktop/build.rs).
SetupIconFile=october-lantern.ico
UninstallDisplayIcon={app}\october-lantern.exe
UninstallDisplayName=October Lantern
LicenseFile=..\LICENSE
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
; Closes a running Lantern (only Lantern's own window and engine use these files).
CloseApplications=yes
RestartApplications=no

[Tasks]
Name: "autostart"; Description: "Start October Lantern when I sign in"; Flags: unchecked

[Files]
Source: "..\build\windows\october-lantern.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\build\windows\lantern-engine.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\build\windows\october-bus.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\build\windows\october-lantern.png"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\build\windows\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\build\windows\OCTOBER-BUS-LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\build\windows\OCTOBER-BUS-COMMIT"; DestDir: "{app}"; Flags: ignoreversion
; Dictation's speech runtime (scripts/build-whisper-windows.ps1); the model is downloaded on first use.
Source: "..\build\windows\whisper-cli.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\build\windows\WHISPER-LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
; The same shortcut the in-app updater points at a new version.
Name: "{userprograms}\October Lantern"; Filename: "{app}\october-lantern.exe"; Comment: "Stay close to your agents"

[Registry]
; The same value Settings › Start when I log in uses. Removed on uninstall however it was set.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "October Lantern"; ValueData: """{app}\october-lantern.exe"""; Tasks: autostart
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "October Lantern"; Flags: uninsdeletevalue

[Run]
Filename: "{app}\october-lantern.exe"; Description: "Open October Lantern"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Takes Lantern out of Claude Code's and Codex's settings (their own settings are kept).
Filename: "{app}\lantern-engine.exe"; Parameters: "hooks uninstall"; Flags: runhidden; RunOnceId: "RemoveHooks"

[UninstallDelete]
; Versions the in-app updater installed.
Type: filesandordirs; Name: "{localappdata}\October Lantern\releases"
; The speech model dictation downloaded on first use.
Type: filesandordirs; Name: "{localappdata}\October Lantern\speech"

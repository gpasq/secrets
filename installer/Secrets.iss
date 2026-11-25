; Inno Setup script for Secrets on Windows. Built by `just installer`, which passes AppVersion,
; Arch (x64 or arm64) and Exe (the release build to package) on the command line.

#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#ifndef Arch
  #define Arch "x64"
#endif
#ifndef Exe
  #define Exe "..\target\release\secrets.exe"
#endif

[Setup]
AppId={{F48190DD-B3D4-465D-9C5E-47043B67F886}
AppName=Secrets
AppVersion={#AppVersion}
AppVerName=Secrets {#AppVersion}
AppPublisher=tecnicl
VersionInfoVersion={#AppVersion}
; Per-user install into %LOCALAPPDATA%\Programs\Secrets: no admin prompt.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=commandline
DefaultDirName={autopf}\Secrets
DefaultGroupName=Secrets
DisableProgramGroupPage=yes
DisableDirPage=auto
ArchitecturesAllowed={#Arch}
ArchitecturesInstallIn64BitMode={#Arch}
MinVersion=10.0.17763
OutputDir=..\target\installer
OutputBaseFilename=Secrets-{#AppVersion}-{#Arch}-setup
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\Secrets.exe
UninstallDisplayName=Secrets
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Options:"; Flags: unchecked

[Files]
Source: "{#Exe}"; DestDir: "{app}"; DestName: "Secrets.exe"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Secrets"; Filename: "{app}\Secrets.exe"
Name: "{autodesktop}\Secrets"; Filename: "{app}\Secrets.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\Secrets.exe"; Description: "Start Secrets"; Flags: nowait postinstall skipifsilent

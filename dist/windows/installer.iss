; Installer for Pro Wave on Windows, for Inno Setup 6 (https://jrsoftware.org/isinfo.php).
;
; Build the program first, from the repo root:
;   cargo build --release -p prowave-app
; Then, from this folder:
;   iscc installer.iss
; The installer comes out in dist\windows\output.

#define AppName "Pro Wave"
#define AppVersion "0.2.1"
#define AppExe "prowave.exe"

[Setup]
AppId={{6F1E2B4C-8A3D-4E57-9C21-7D0B5A6E4F10}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=Saka Audio
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
UninstallDisplayIcon={app}\{#AppExe}
OutputDir=output
OutputBaseFilename=ProWave-{#AppVersion}-setup
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern
ChangesAssociations=yes

[Files]
Source: "..\..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "README.txt"; DestDir: "{app}"; Flags: ignoreversion isreadme

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Registry]
Root: HKA; Subkey: "Software\Classes\.prowave"; ValueType: string; ValueName: ""; ValueData: "ProWave.Project"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\ProWave.Project"; ValueType: string; ValueName: ""; ValueData: "Pro Wave Project"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\ProWave.Project\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExe},0"
Root: HKA; Subkey: "Software\Classes\ProWave.Project\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""

[Run]
Filename: "{app}\{#AppExe}"; Description: "Start {#AppName}"; Flags: nowait postinstall skipifsilent

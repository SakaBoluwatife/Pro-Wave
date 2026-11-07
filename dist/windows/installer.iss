; Installer for Ovrtone on Windows, for Inno Setup 6 (https://jrsoftware.org/isinfo.php).
;
; Build the program first, from the repo root:
;   cargo build --release -p ovrtone-app
; Then, from this folder:
;   iscc installer.iss
; The installer comes out in dist\windows\output.

#define AppName "Ovrtone"
#define AppVersion "0.2.1"
#define AppExe "ovrtone.exe"

[Setup]
AppId={{6F1E2B4C-8A3D-4E57-9C21-7D0B5A6E4F10}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=Bankz Sound Lab
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
UninstallDisplayIcon={app}\{#AppExe}
; Replaces an installed Pro Wave: the AppId above is the same on purpose.
AppPublisherURL=https://github.com/SakaBoluwatife/Pro-Wave
OutputDir=output
OutputBaseFilename=Ovrtone-{#AppVersion}-setup
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern
ChangesAssociations=yes

[Files]
Source: "..\..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "README.txt"; DestDir: "{app}"; Flags: ignoreversion isreadme
; The Stem Splitter's helper, licence and model: see stems-README.md for how to fill this folder.
Source: "stems\*"; DestDir: "{app}\stems"; Flags: ignoreversion

[InstallDelete]
; The program's file from before the rename, when installing over it.
Type: files; Name: "{app}\prowave.exe"

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Registry]
Root: HKA; Subkey: "Software\Classes\.ovrtone"; ValueType: string; ValueName: ""; ValueData: "Ovrtone.Project"; Flags: uninsdeletevalue
; Projects from before the rename open here too.
Root: HKA; Subkey: "Software\Classes\.prowave"; ValueType: string; ValueName: ""; ValueData: "Ovrtone.Project"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Ovrtone.Project"; ValueType: string; ValueName: ""; ValueData: "Ovrtone Project"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Ovrtone.Project\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExe},0"
Root: HKA; Subkey: "Software\Classes\Ovrtone.Project\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""

[Run]
Filename: "{app}\{#AppExe}"; Description: "Start {#AppName}"; Flags: nowait postinstall skipifsilent

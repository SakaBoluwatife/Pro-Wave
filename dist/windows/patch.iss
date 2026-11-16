; Update patch for Ovrtone on Windows, for Inno Setup 6.7 or later. A small installer that
; replaces only what changes between builds (the program and the documents) in an Ovrtone that is
; already installed. It refuses to run where Ovrtone is not installed. The full installer
; (installer.iss, with the Stem Splitter files and the VC++ runtime) is made once, for the final
; build.
;
; Build the program first, from the repo root:
;   cargo build --release -p ovrtone-app --features asio
; Then, from this folder, with the patch's name:
;   iscc /DPatchVersion="1.0.1" /DPatchFileVersion="1.0.1.0" patch.iss
; The patch comes out in dist\windows\output.

#define AppName "Ovrtone"
#ifndef PatchVersion
  #define PatchVersion "1.0.1"
#endif
#ifndef PatchFileVersion
  #define PatchFileVersion "1.0.1.0"
#endif
#define AppExe "ovrtone.exe"
#ifndef ExeDir
  #define ExeDir "..\..\target\release"
#endif
; The full installer's AppId: Windows' entry for the installed Ovrtone is found by it.
#define AppIdText "{6F1E2B4C-8A3D-4E57-9C21-7D0B5A6E4F10}"
; The same in sections where a brace starts a constant.
#define AppIdReg "{{6F1E2B4C-8A3D-4E57-9C21-7D0B5A6E4F10}"

[Setup]
AppId={{6F1E2B4C-8A3D-4E57-9C21-7D0B5A6E4F10}
AppName={#AppName}
AppVersion={#PatchVersion}
AppVerName={#AppName} {#PatchVersion}
VersionInfoVersion={#PatchFileVersion}
VersionInfoProductVersion={#PatchFileVersion}
AppPublisher=Bankz Sound Lab
; Into the folder Ovrtone is installed in, found in [Code] below; never asked.
DefaultDirName={code:InstalledDir}
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableReadyPage=no
; The full installer's uninstaller stays the one Windows knows: the patch adds no entry of its own.
Uninstallable=no
CreateUninstallRegKey=no
OutputDir=output
OutputBaseFilename=Ovrtone-update-{#PatchFileVersion}
SetupIconFile=..\..\crates\ovrtone-app\assets\ovrtone.ico
Compression=lzma2/ultra64
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=yes
RestartApplications=no
InfoBeforeFile=RELEASE-NOTES.txt
WizardStyle=modern dark windows11 hidebevels
WizardBackColor=#16181d

[Messages]
WelcomeLabel1=Update [name]
WelcomeLabel2=This brings the Ovrtone on this computer up to [name/ver]. Your projects, settings and plugins are not touched.%n%nClose Ovrtone before you go on.

[Files]
Source: "{#ExeDir}\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "README.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "RELEASE-NOTES.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "Ovrtone-Manual.html"; DestDir: "{app}"; Flags: ignoreversion
Source: "Ovrtone-Key-Commands.html"; DestDir: "{app}"; Flags: ignoreversion
Source: "Ovrtone-Key-Commands.pdf"; DestDir: "{app}"; Flags: ignoreversion

[Registry]
; Windows' list of programs shows the version now installed.
Root: HKA; Subkey: "Software\Microsoft\Windows\CurrentVersion\Uninstall\{#AppIdReg}_is1"; ValueType: string; ValueName: "DisplayVersion"; ValueData: "{#PatchVersion}"; Flags: dontcreatekey
Root: HKA; Subkey: "Software\Microsoft\Windows\CurrentVersion\Uninstall\{#AppIdReg}_is1"; ValueType: string; ValueName: "DisplayName"; ValueData: "{#AppName} {#PatchVersion}"; Flags: dontcreatekey

[Code]
const
  UninstallKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{#AppIdText}_is1';

{ Where the full installer put Ovrtone, or '' when it is not installed. }
function FindInstalledDir(): String;
var
  Dir: String;
begin
  Result := '';
  if RegQueryStringValue(HKLM64, UninstallKey, 'InstallLocation', Dir) or
     RegQueryStringValue(HKCU, UninstallKey, 'InstallLocation', Dir) then
  begin
    if FileExists(AddBackslash(Dir) + '{#AppExe}') then
      Result := RemoveBackslash(Dir);
  end;
end;

function InstalledDir(Param: String): String;
begin
  Result := FindInstalledDir();
end;

function InitializeSetup(): Boolean;
begin
  Result := FindInstalledDir() <> '';
  if not Result then
    MsgBox('Ovrtone is not installed on this computer. Install it with the full installer first; ' +
      'this update only brings an installed Ovrtone up to date.', mbError, MB_OK);
end;

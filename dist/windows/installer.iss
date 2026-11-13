; Installer for Ovrtone on Windows, for Inno Setup 6.7 or later (https://jrsoftware.org/isinfo.php).
;
; Build the program first, from the repo root:
;   cargo build --release -p ovrtone-app --features asio
; and fill the stems folder (stems-README.md) and put Microsoft's vc_redist.x64.exe, from Visual
; Studio's VC\Redist\MSVC folder, in the runtime folder.
; Then, from this folder:
;   iscc installer.iss
; The installer comes out in dist\windows\output. To package another build of the program (a test
; build, say), pass its folder: iscc /DExeDir=..\..\target\test-build installer.iss
;
; As FL Studio's installer does, it lets the user choose: a Full, Compact or Custom installation,
; which parts to install, where, the file types Ovrtone opens, and a plugin folder of their own.

#define AppName "Ovrtone"
#define AppVersion "0.4.0"
#define AppExe "ovrtone.exe"
#ifndef ExeDir
  #define ExeDir "..\..\target\release"
#endif

[Setup]
AppId={{6F1E2B4C-8A3D-4E57-9C21-7D0B5A6E4F10}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion} (beta)
AppPublisher=Bankz Sound Lab
AppPublisherURL=https://github.com/SakaBoluwatife/Pro-Wave
AppSupportURL=https://github.com/SakaBoluwatife/Pro-Wave/issues
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName} {#AppVersion}
; Replaces an installed Pro Wave: the AppId above is the same on purpose.
OutputDir=output
OutputBaseFilename=Ovrtone-{#AppVersion}-beta-setup
SetupIconFile=..\..\crates\ovrtone-app\assets\ovrtone.ico
Compression=lzma2/ultra64
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
ChangesAssociations=yes
; Upgrading: the same AppId installs over the last version, in its folder, and closes a running
; Ovrtone first. Settings, presets and templates in %APPDATA%\ovrtone are never touched.
UsePreviousAppDir=yes
UsePreviousSetupType=yes
UsePreviousTasks=yes
CloseApplications=yes
RestartApplications=no
InfoBeforeFile=RELEASE-NOTES.txt
LicenseFile=..\..\LICENSE
; The look: Ovrtone's dark, its symbol in the corner, and the pages a choice at a time.
WizardStyle=modern dark windows11 hidebevels
WizardSizePercent=110
WizardBackColor=#16181d
WizardBackImageFile=art\wizard-back.png
WizardImageFile=
WizardSmallImageFile=art\wizard-small.png
DisableWelcomePage=no
DisableDirPage=no
DisableProgramGroupPage=yes
AlwaysShowComponentsList=yes
ShowComponentSizes=yes

[Messages]
WelcomeLabel1=Welcome to [name]
WelcomeLabel2=This installs [name/ver] by Bankz Sound Lab: a DAW for Windows, with a Logic-style arrangement and mixer, FL-style piano roll and channel rack, and spatial mixing.%n%nOvrtone comes with no instruments or sound library: you bring your VST3 and CLAP plugins and your own samples.%n%nClose any running Ovrtone before you go on.
SelectComponentsDesc=Which parts of Ovrtone do you want?
SelectComponentsLabel2=Choose a set-up, or tick exactly the parts you want. Click Next when ready.
SelectTasksDesc=How should Ovrtone fit into Windows?

[Types]
Name: "full"; Description: "Full installation"
Name: "compact"; Description: "Compact installation (the program alone)"
Name: "custom"; Description: "Custom installation"; Flags: iscustom

[Components]
Name: "program"; Description: "Ovrtone (required)"; Types: full compact custom; Flags: fixed
Name: "stems"; Description: "Stem Splitter: splits a song into vocals, drums, bass and the rest (AI model, large)"; Types: full
Name: "docs"; Description: "User manual, read-me and release notes"; Types: full compact

[Tasks]
Name: "desktopicon"; Description: "A desktop shortcut"; GroupDescription: "Shortcuts:"
Name: "startmenu"; Description: "A Start menu entry"; GroupDescription: "Shortcuts:"
Name: "assoc"; Description: "Open .ovrtone projects with Ovrtone"; GroupDescription: "File types:"
Name: "assocother"; Description: "Open other DAWs' sessions with Ovrtone (.dawproject, .rpp, .als, .ardour)"; GroupDescription: "File types:"; Flags: unchecked
Name: "vcredist"; Description: "Install the Microsoft Visual C++ runtime (needed on a fresh Windows)"; GroupDescription: "System:"

[Files]
Source: "{#ExeDir}\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion; Components: program
Source: "README.txt"; DestDir: "{app}"; Flags: ignoreversion isreadme; Components: docs
Source: "RELEASE-NOTES.txt"; DestDir: "{app}"; Flags: ignoreversion; Components: docs
Source: "Ovrtone-Manual.html"; DestDir: "{app}"; Flags: ignoreversion; Components: docs
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion; Components: program
Source: "..\..\crates\ovrtone-app\assets\fonts\*-LICENSE.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion; Components: program
Source: "..\..\crates\ovrtone-app\assets\icons\TABLER-LICENSE.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion; Components: program
Source: "licenses\*-LICENSE.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion; Components: program
; What the program and ONNX Runtime need and a clean Windows may not have. It is installed for the
; whole system, not put beside the program: plugins load into this process, and one built with a
; newer compiler can crash on an older copy of the runtime.
Source: "runtime\vc_redist.x64.exe"; DestDir: "{tmp}"; Flags: deleteafterinstall; Tasks: vcredist
; The Stem Splitter's model, ONNX Runtime and its licences: see stems-README.md for this folder.
Source: "stems\*"; DestDir: "{app}\stems"; Flags: ignoreversion recursesubdirs; Components: stems

[InstallDelete]
; The program's file from before the rename, when installing over it.
Type: files; Name: "{app}\prowave.exe"

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: startmenu
Name: "{autoprograms}\{#AppName} Manual"; Filename: "{app}\Ovrtone-Manual.html"; Tasks: startmenu; Components: docs
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Registry]
Root: HKA; Subkey: "Software\Classes\.ovrtone"; ValueType: string; ValueName: ""; ValueData: "Ovrtone.Project"; Flags: uninsdeletevalue; Tasks: assoc
; Projects from before the rename open here too.
Root: HKA; Subkey: "Software\Classes\.prowave"; ValueType: string; ValueName: ""; ValueData: "Ovrtone.Project"; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\Ovrtone.Project"; ValueType: string; ValueName: ""; ValueData: "Ovrtone Project"; Flags: uninsdeletekey; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\Ovrtone.Project\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExe},0"; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\Ovrtone.Project\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Tasks: assoc
; Other DAWs' sessions: offered in Open With, without taking them from the DAW that owns them.
Root: HKA; Subkey: "Software\Classes\Ovrtone.Session"; ValueType: string; ValueName: ""; ValueData: "DAW Session"; Flags: uninsdeletekey; Tasks: assocother
Root: HKA; Subkey: "Software\Classes\Ovrtone.Session\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Tasks: assocother
Root: HKA; Subkey: "Software\Classes\.dawproject\OpenWithProgids"; ValueType: string; ValueName: "Ovrtone.Session"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assocother
Root: HKA; Subkey: "Software\Classes\.rpp\OpenWithProgids"; ValueType: string; ValueName: "Ovrtone.Session"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assocother
Root: HKA; Subkey: "Software\Classes\.als\OpenWithProgids"; ValueType: string; ValueName: "Ovrtone.Session"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assocother
Root: HKA; Subkey: "Software\Classes\.ardour\OpenWithProgids"; ValueType: string; ValueName: "Ovrtone.Session"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assocother

[Run]
; Does nothing if the same or a newer one is there already.
Filename: "{tmp}\vc_redist.x64.exe"; Parameters: "/install /quiet /norestart"; StatusMsg: "Installing the Microsoft Visual C++ runtime..."; Flags: waituntilterminated; Tasks: vcredist
Filename: "{app}\{#AppExe}"; Description: "Start {#AppName} now"; Flags: nowait postinstall skipifsilent

[Code]
// A page of its own, as FL Studio's installer has for plugins: a folder of the user's plugins for
// Ovrtone to look in, besides the usual VST3 and CLAP folders it always scans.
var
  PluginPage: TInputDirWizardPage;

procedure InitializeWizard;
begin
  PluginPage := CreateInputDirPage(wpSelectTasks,
    'Your plugins',
    'Where are your VST3 and CLAP plugins?',
    'Ovrtone always looks in the usual folders (Common Files\VST3 and Common Files\CLAP). ' +
    'If you keep plugins somewhere else too, choose that folder here; you can add more in Ovrtone ' +
    'later (the Plugins list''s folder button). Leave it empty if you do not need one.',
    False, '');
  PluginPage.Add('Another plugin folder (optional):');
  PluginPage.Values[0] := '';
end;

// Adds the folder chosen to Ovrtone's list of plugin folders (one path a line), once.
procedure AddPluginFolder(Folder: String);
var
  ListFile, Text: String;
  Lines: AnsiString;
begin
  if Trim(Folder) = '' then
    exit;
  ListFile := ExpandConstant('{userappdata}\ovrtone\plugin-folders');
  ForceDirectories(ExtractFileDir(ListFile));
  Text := '';
  if LoadStringFromFile(ListFile, Lines) then
    Text := String(Lines);
  if Pos(Lowercase(Folder), Lowercase(Text)) > 0 then
    exit;
  if (Text <> '') and (Copy(Text, Length(Text), 1) <> #10) then
    Text := Text + #13#10;
  SaveStringToFile(ListFile, Text + Folder + #13#10, False);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    AddPluginFolder(PluginPage.Values[0]);
end;

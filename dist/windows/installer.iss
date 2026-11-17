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
#define AppVersion "1.1.0"
; Windows' file version: 1.0 is 1.0.0; updates to it are 1.0.1, 1.0.2 and on (patch.iss).
#define FileVersion "1.1.0.0"
#define AppExe "ovrtone.exe"
#ifndef ExeDir
  #define ExeDir "..\..\target\release"
#endif

[Setup]
AppId={{6F1E2B4C-8A3D-4E57-9C21-7D0B5A6E4F10}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
VersionInfoVersion={#FileVersion}
VersionInfoProductVersion={#FileVersion}
AppPublisher=Bankz Sound Lab
AppPublisherURL=https://github.com/SakaBoluwatife/Pro-Wave
AppSupportURL=https://github.com/SakaBoluwatife/Pro-Wave/issues
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName} {#AppVersion}
; Replaces an installed Pro Wave: the AppId above is the same on purpose.
OutputDir=output
OutputBaseFilename=Ovrtone-1.1.0-setup
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
; Asks the person's name: Ovrtone shows it in Settings and About ("Licensed to ..."). A label only.
UserInfoPage=yes
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
Name: "analog"; Description: "BSL Classics (35 VST3 analog-hardware plugins by Bankz Sound Lab)"; Types: full

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
Source: "Ovrtone-Key-Commands.html"; DestDir: "{app}"; Flags: ignoreversion; Components: docs
Source: "Ovrtone-Key-Commands.pdf"; DestDir: "{app}"; Flags: ignoreversion; Components: docs
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
; BSL Classics: 35 circuit-informed analog-hardware plugins built in C++/JUCE (repo ..\..\..\analog).
; They install in the system VST3 folder and the reference page lists what each one emulates.
Source: "Ovrtone-BSL-Classics.html"; DestDir: "{app}"; Flags: ignoreversion; Components: analog
Source: "..\..\..\analog\build\plugins\busg\UT_BUSG_artefacts\Release\VST3\BSL BUS-G.vst3\*"; DestDir: "{commonpf64}\VST3\BSL BUS-G.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\chorus1\UT_CHORUS1_artefacts\Release\VST3\BSL CHORUS-1.vst3\*"; DestDir: "{commonpf64}\VST3\BSL CHORUS-1.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\desk500\UT_DESK500_artefacts\Release\VST3\BSL DESK-500.vst3\*"; DestDir: "{commonpf64}\VST3\BSL DESK-500.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\desks\UT_DESKS_artefacts\Release\VST3\BSL DESK-S.vst3\*"; DestDir: "{commonpf64}\VST3\BSL DESK-S.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\deskv\UT_DESKV_artefacts\Release\VST3\BSL DESK-V.vst3\*"; DestDir: "{commonpf64}\VST3\BSL DESK-V.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\digi250\UT_DIGI250_artefacts\Release\VST3\BSL DIGI-250.vst3\*"; DestDir: "{commonpf64}\VST3\BSL DIGI-250.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\dimd\UT_DIMD_artefacts\Release\VST3\BSL DIM-D.vst3\*"; DestDir: "{commonpf64}\VST3\BSL DIM-D.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\echo201\UT_ECHO201_artefacts\Release\VST3\BSL ECHO-201.vst3\*"; DestDir: "{commonpf64}\VST3\BSL ECHO-201.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\echo3\UT_ECHO3_artefacts\Release\VST3\BSL ECHO-3.vst3\*"; DestDir: "{commonpf64}\VST3\BSL ECHO-3.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\eq73\UT_EQ73_artefacts\Release\VST3\BSL EQ-73.vst3\*"; DestDir: "{commonpf64}\VST3\BSL EQ-73.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\eqair\UT_EQAIR_artefacts\Release\VST3\BSL EQ-AIR.vst3\*"; DestDir: "{commonpf64}\VST3\BSL EQ-AIR.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\eqch\UT_EQCH_artefacts\Release\VST3\BSL EQ-CH.vst3\*"; DestDir: "{commonpf64}\VST3\BSL EQ-CH.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\eqpq\UT_EQPQ_artefacts\Release\VST3\BSL EQ-PQ.vst3\*"; DestDir: "{commonpf64}\VST3\BSL EQ-PQ.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\fet1\UT_FET1_artefacts\Release\VST3\BSL FET-1.vst3\*"; DestDir: "{commonpf64}\VST3\BSL FET-1.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\flange126\UT_FLANGE126_artefacts\Release\VST3\BSL FLANGE-126.vst3\*"; DestDir: "{commonpf64}\VST3\BSL FLANGE-126.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\hall224\UT_HALL224_artefacts\Release\VST3\BSL HALL-224.vst3\*"; DestDir: "{commonpf64}\VST3\BSL HALL-224.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\opto2\UT_OPTO2_artefacts\Release\VST3\BSL OPTO-2.vst3\*"; DestDir: "{commonpf64}\VST3\BSL OPTO-2.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\plate1\UT_PLATE1_artefacts\Release\VST3\BSL PLATE-1.vst3\*"; DestDir: "{commonpf64}\VST3\BSL PLATE-1.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\pren\UT_PREN_artefacts\Release\VST3\BSL PRE-N.vst3\*"; DestDir: "{commonpf64}\VST3\BSL PRE-N.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\satd\UT_SATD_artefacts\Release\VST3\BSL SAT-D.vst3\*"; DestDir: "{commonpf64}\VST3\BSL SAT-D.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\satdesk\UT_SATDESK_artefacts\Release\VST3\BSL SAT-DESK.vst3\*"; DestDir: "{commonpf64}\VST3\BSL SAT-DESK.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\sathg\UT_SATHG_artefacts\Release\VST3\BSL SAT-HG.vst3\*"; DestDir: "{commonpf64}\VST3\BSL SAT-HG.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\satone\UT_SATONE_artefacts\Release\VST3\BSL SAT-ONE.vst3\*"; DestDir: "{commonpf64}\VST3\BSL SAT-ONE.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\sattube\UT_SATTUBE_artefacts\Release\VST3\BSL SAT-TUBE.vst3\*"; DestDir: "{commonpf64}\VST3\BSL SAT-TUBE.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\spring2\UT_SPRING2_artefacts\Release\VST3\BSL SPRING-2.vst3\*"; DestDir: "{commonpf64}\VST3\BSL SPRING-2.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\springtank\UT_SPRINGTANK_artefacts\Release\VST3\BSL SPRING-TANK.vst3\*"; DestDir: "{commonpf64}\VST3\BSL SPRING-TANK.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\sumn\UT_SUMN_artefacts\Release\VST3\BSL SUM-N.vst3\*"; DestDir: "{commonpf64}\VST3\BSL SUM-N.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\tape102\UT_TAPE102_artefacts\Release\VST3\BSL TAPE-102.vst3\*"; DestDir: "{commonpf64}\VST3\BSL TAPE-102.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\tape350\UT_TAPE350_artefacts\Release\VST3\BSL TAPE-350.vst3\*"; DestDir: "{commonpf64}\VST3\BSL TAPE-350.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\tape37\UT_TAPE37_artefacts\Release\VST3\BSL TAPE-37.vst3\*"; DestDir: "{commonpf64}\VST3\BSL TAPE-37.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\tape800\UT_TAPE800_artefacts\Release\VST3\BSL TAPE-800.vst3\*"; DestDir: "{commonpf64}\VST3\BSL TAPE-800.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\tapeduo\UT_TAPEDUO_artefacts\Release\VST3\BSL TAPE-DUO.vst3\*"; DestDir: "{commonpf64}\VST3\BSL TAPE-DUO.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\tubeeq\UT_TUBEEQ_artefacts\Release\VST3\BSL TUBE-EQ.vst3\*"; DestDir: "{commonpf64}\VST3\BSL TUBE-EQ.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\vari6\UT_VARI6_artefacts\Release\VST3\BSL VARI-6.vst3\*"; DestDir: "{commonpf64}\VST3\BSL VARI-6.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog
Source: "..\..\..\analog\build\plugins\vca25\UT_VCA25_artefacts\Release\VST3\BSL VCA-25.vst3\*"; DestDir: "{commonpf64}\VST3\BSL VCA-25.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: analog

[InstallDelete]
; The program's file from before the rename, when installing over it.
Type: files; Name: "{app}\prowave.exe"

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: startmenu
Name: "{autoprograms}\{#AppName} Manual"; Filename: "{app}\Ovrtone-Manual.html"; Tasks: startmenu; Components: docs
Name: "{autoprograms}\{#AppName} Key Commands"; Filename: "{app}\Ovrtone-Key-Commands.html"; Tasks: startmenu; Components: docs
Name: "{autoprograms}\{#AppName} BSL Classics"; Filename: "{app}\Ovrtone-BSL-Classics.html"; Tasks: startmenu; Components: analog
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[INI]
; Who this copy is licensed to, for Settings and About. Kept on uninstall, as the other settings are.
Filename: "{userappdata}\ovrtone\license.ini"; Section: "License"; Key: "Name"; String: "{userinfoname}"
; A licence ID of its own, made once: a reinstall keeps it.
Filename: "{userappdata}\ovrtone\license.ini"; Section: "License"; Key: "Id"; String: "{code:LicenseId}"; Flags: createkeyifdoesntexist

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
// A plain text box with a Browse button, not Inno's folder page: that page refuses an empty box,
// and this folder is optional.
var
  PluginPage: TInputQueryWizardPage;

procedure BrowsePluginFolder(Sender: TObject);
var
  Dir: String;
begin
  Dir := Trim(PluginPage.Values[0]);
  if BrowseForFolder('Choose the folder your plugins are in:', Dir, False) then
    PluginPage.Values[0] := Dir;
end;

// A licence ID such as OVR-7K2M-QX9D-4HTB, from letters and digits that cannot be misread.
var
  LicId: String;

function LicenseId(Param: String): String;
var
  I: Integer;
  Chars: String;
begin
  if LicId = '' then
  begin
    Chars := 'ABCDEFGHJKLMNPQRSTUVWXYZ23456789';
    LicId := 'OVR';
    for I := 1 to 12 do
    begin
      if (I - 1) mod 4 = 0 then
        LicId := LicId + '-';
      LicId := LicId + Chars[Random(Length(Chars)) + 1];
    end;
  end;
  Result := LicId;
end;

procedure InitializeWizard;
var
  Browse: TNewButton;
begin
  PluginPage := CreateInputQueryPage(wpSelectTasks,
    'Your plugins',
    'Where are your VST3 and CLAP plugins?',
    'Ovrtone always looks in the usual folders (Common Files\VST3 and Common Files\CLAP). ' +
    'If you keep plugins somewhere else too, choose that folder here; you can add more in Ovrtone ' +
    'later (the Plugins list''s folder button). Leave it empty if you do not need one.');
  PluginPage.Add('Another plugin folder (optional):', False);
  PluginPage.Values[0] := '';
  // The box makes room for a Browse button at its right, as Inno's own folder pages have.
  PluginPage.Edits[0].Width := PluginPage.Edits[0].Width - ScaleX(85);
  Browse := TNewButton.Create(PluginPage);
  Browse.Parent := PluginPage.Surface;
  Browse.Caption := 'Browse...';
  Browse.Left := PluginPage.Edits[0].Left + PluginPage.Edits[0].Width + ScaleX(10);
  Browse.Top := PluginPage.Edits[0].Top - ScaleY(1);
  Browse.Width := ScaleX(75);
  Browse.Height := ScaleY(23);
  Browse.OnClick := @BrowsePluginFolder;
end;

// Empty is fine; a folder typed in must be there.
function NextButtonClick(CurPageID: Integer): Boolean;
var
  Folder: String;
begin
  Result := True;
  if CurPageID = PluginPage.ID then
  begin
    Folder := Trim(PluginPage.Values[0]);
    if (Folder <> '') and not DirExists(Folder) then
    begin
      MsgBox('That folder could not be found. Choose it with Browse, or leave the box empty.',
        mbError, MB_OK);
      Result := False;
    end;
  end;
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

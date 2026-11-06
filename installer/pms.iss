; PimpMyShell's Windows installer, for Inno Setup 6 (https://jrsoftware.org/isinfo.php). `cargo xtask installer`, from
; the folder above, builds the release exes and compiles this into target\installer\pms-setup-VERSION.exe.
;
; It installs for the user alone, with no administrator: into %LOCALAPPDATA%\Programs\PimpMyShell, where Windows keeps
; per-user programs, with a Start menu shortcut to the settings and an entry in Settings > Apps. Starting at sign-in
; is a task, ticked to begin with; the settings' General page can switch it later, and an update leaves that as it found
; it. The config, in
; %APPDATA%\PimpMyShell (roaming), is the user's and is left on uninstall, as are banners.txt and the icons in
; %LOCALAPPDATA%\PimpMyShell (below). INSTALLER.md, beside this file, says how to build it.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef Bin
  #define Bin "..\target\release"
#endif

[Setup]
; The id names the entry in Settings > Apps across versions: never change it.
AppId={{7C3E2F4A-9B61-4D8E-A2C5-5E1F0B7D9A36}
AppName=PimpMyShell
AppVersion={#AppVersion}
AppVerName=PimpMyShell {#AppVersion}
VersionInfoVersion={#AppVersion}
PrivilegesRequired=lowest
; {autopf} is %LOCALAPPDATA%\Programs for a per-user install.
DefaultDirName={autopf}\PimpMyShell
DisableProgramGroupPage=yes
; The box's moment: without this, the wizard opens on a question and the box appears only at the end.
DisableWelcomePage=no
DisableDirPage=auto
DisableReadyPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.22000
UninstallDisplayIcon={app}\pms-gui.exe
UninstallDisplayName=PimpMyShell
OutputDir=..\target\installer
OutputBaseFilename=pms-setup-{#AppVersion}
Compression=lzma2/max
SolidCompression=yes
; Dark whatever Windows is set to: the bills are made for it, and the window is seen once.
WizardStyle=modern dark
WizardBackColor=#0F1411
WizardBackImageFile=art\backdrop-*.png
; pms-art draws these (cargo xtask art): the box as Setup's icon, the settings' tile of bills behind every page, the box on
; it for the welcome and finish pages and in the other pages' corner - each at the sizes display scales want.
SetupIconFile=art\setup.ico
WizardImageFile=art\wizard-*.png
WizardSmallImageFile=art\corner-*.png
AppPublisher=PimpMyShell
AppComments=Exclusively for Windows 11.
; The daemon is stopped by the code below (which also restores the stock taskbar), not by the Restart Manager.
CloseApplications=no

[Messages]
; Without the stock advice to close every other application: Setup closes PimpMyShell itself, and nothing else matters to it.
WelcomeLabel2=This will install [name/ver] on your computer.

[Tasks]
Name: "autostart"; Description: "Start PimpMyShell when I sign in"
Name: "desktopicon"; Description: "Add a PimpMyShell shortcut to the desktop"; Flags: unchecked

[Files]
Source: "{#Bin}\pmsd.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Bin}\pms.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Bin}\pms-gui.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Bin}\pms_tap.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\pms.example.toml"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
; The notice the background needs, as in the settings: read by the code below, and not installed.
Source: "..\pms-gui\src\legal.txt"; Flags: dontcopy

[Icons]
Name: "{autoprograms}\PimpMyShell"; Filename: "{app}\pms-gui.exe"; Comment: "PimpMyShell's settings"
Name: "{autodesktop}\PimpMyShell"; Filename: "{app}\pms-gui.exe"; Tasks: desktopicon

[Registry]
; The same value (PimpMyShell) the settings' and tray's "start at sign-in" writes; the uninstaller removes it whichever wrote it (see below).
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "PimpMyShell"; \
  ValueData: """{app}\pmsd.exe"""; Flags: uninsdeletevalue; Tasks: autostart; Check: AsksAboutSignIn

[UninstallDelete]
; What the updater downloads: the last installer it fetched, its log, tried.txt; then the folder, if that emptied it.
Type: files; Name: "{%TEMP}\pms-update\pms-setup-*.exe"
Type: files; Name: "{%TEMP}\pms-update\pms-setup-*.exe.part"
Type: files; Name: "{%TEMP}\pms-update\pms-setup-*.log"
Type: files; Name: "{%TEMP}\pms-update\tried.txt"
Type: dirifempty; Name: "{%TEMP}\pms-update"
; The logs and what's noted beside them, each by its exact name (pms_tap::logfile, update.rs, the TAP's tree dump).
; The config (pms.toml) and banners.txt stay: the config is yours, and banners.txt names the notification banners
; PimpMyShell switched off, should they need switching back on.
Type: files; Name: "{localappdata}\PimpMyShell\daemon.log"
Type: files; Name: "{localappdata}\PimpMyShell\daemon.1.log"
Type: files; Name: "{localappdata}\PimpMyShell\daemon.2.log"
Type: files; Name: "{localappdata}\PimpMyShell\tap.log"
Type: files; Name: "{localappdata}\PimpMyShell\tap.1.log"
Type: files; Name: "{localappdata}\PimpMyShell\tap.2.log"
; Where the logs are from 0.1.4 on; the ones above are older.
Type: files; Name: "{userappdata}\PimpMyShell\logs\daemon.log"
Type: files; Name: "{userappdata}\PimpMyShell\logs\daemon.1.log"
Type: files; Name: "{userappdata}\PimpMyShell\logs\daemon.2.log"
Type: files; Name: "{userappdata}\PimpMyShell\logs\tap.log"
Type: files; Name: "{userappdata}\PimpMyShell\logs\tap.1.log"
Type: files; Name: "{userappdata}\PimpMyShell\logs\tap.2.log"
Type: dirifempty; Name: "{userappdata}\PimpMyShell\logs"
Type: files; Name: "{localappdata}\PimpMyShell\tree.txt"
Type: files; Name: "{localappdata}\PimpMyShell\update-checked.txt"
Type: files; Name: "{localappdata}\PimpMyShell\taskbar-told.txt"
Type: files; Name: "{localappdata}\PimpMyShell\welcomed.txt"

[Run]
Filename: "{app}\pmsd.exe"; Description: "Start PimpMyShell now"; Flags: nowait postinstall skipifsilent
Filename: "{app}\pms-gui.exe"; Description: "Open PimpMyShell settings"; Flags: nowait postinstall skipifsilent unchecked

[Code]
const
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';

var
  WasRunning: Boolean;
  SettingsWereOpen: Boolean;
  SignInShown: Boolean;

// The notice, in fine print to the left of the buttons, on every page. It's long: it scrolls.
procedure InitializeWizard();
var
  Notice: TNewMemo;
  Text: AnsiString;
begin
  ExtractTemporaryFile('legal.txt');
  if not LoadStringFromFile(ExpandConstant('{tmp}\legal.txt'), Text) then
    Exit;
  Notice := TNewMemo.Create(WizardForm);
  Notice.Parent := WizardForm;
  Notice.Left := ScaleX(10);
  Notice.Width := WizardForm.BackButton.Left - ScaleX(22);
  Notice.Height := ScaleY(45);
  Notice.Top := WizardForm.ClientHeight - Notice.Height - ScaleY(5);
  Notice.Anchors := [akLeft, akRight, akBottom];
  Notice.BorderStyle := bsNone;
  Notice.ReadOnly := True;
  Notice.TabStop := False;
  Notice.ScrollBars := ssVertical;
  // Its own colors, not the style's; the style keeps the scroll bar.
  Notice.StyleElements := [seBorder];
  Notice.Color := $0A0D0B;
  Notice.Font.Name := 'Segoe UI';
  Notice.Font.Size := 6;
  Notice.Font.Color := $8FA697;
  Notice.Text := String(Text);
end;

// Whether this goes over an installed copy.
function Upgrading(): Boolean;
begin
  Result := WizardForm.PrevAppDir <> '';
end;

// A silent update has no way to ask, and the task it remembers is from the first install: start at sign-in may have
// been switched off since, and that's for its owner to undo.
function AsksAboutSignIn(): Boolean;
begin
  Result := not (WizardSilent and Upgrading);
end;

// Over an installed copy the task says how things are now, not how they were at the first install.
procedure CurPageChanged(CurPageID: Integer);
begin
  if (CurPageID = wpSelectTasks) and Upgrading and not SignInShown then
  begin
    SignInShown := True;
    if RegValueExists(HKCU, RunKey, 'PimpMyShell') then
      WizardSelectTasks('autostart')
    else
      WizardSelectTasks('!autostart');
  end;
end;

// Stops a running PimpMyShell: the daemon is asked to stop, which puts the stock taskbar (and notification banners)
// back; the settings window is closed. Notes what ran, so that a silent update starts it again.
procedure StopPimpMyShell();
var
  Code: Integer;
begin
  if FileExists(ExpandConstant('{app}\pms.exe')) then
  begin
    if Exec(ExpandConstant('{app}\pms.exe'), 'daemon stop', '', SW_HIDE, ewWaitUntilTerminated, Code) and (Code = 0) then
      WasRunning := True;
    Sleep(1500);
  end;
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/f /im pmsd.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
  if Exec(ExpandConstant('{sys}\taskkill.exe'), '/f /im pms-gui.exe', '', SW_HIDE, ewWaitUntilTerminated, Code) and (Code = 0) then
    SettingsWereOpen := True;
  Sleep(300);
end;

// A silent install (an update) skips the finish page: what it stopped is started again, the new one. So are settings
// that closed themselves to install this, and said so (/SETTINGS=1). With a wizard, an unticked task is switched off.
procedure CurStepChanged(CurStep: TSetupStep);
var
  Code: Integer;
  Settings: Boolean;
begin
  if CurStep <> ssPostInstall then
    Exit;
  if not WizardSilent then
  begin
    if Upgrading and not WizardIsTaskSelected('autostart') then
      RegDeleteValue(HKCU, RunKey, 'PimpMyShell');
    Exit;
  end;
  Settings := SettingsWereOpen or (ExpandConstant('{param:settings|0}') = '1');
  if WasRunning then
  begin
    ExecAsOriginalUser(ExpandConstant('{app}\pmsd.exe'), '', '', SW_SHOWNORMAL, ewNoWait, Code);
    // The daemon first: with none running, the settings would convert an old config too, at the same moment.
    if Settings then
      Sleep(1500);
  end;
  if Settings then
    ExecAsOriginalUser(ExpandConstant('{app}\pms-gui.exe'), '', '', SW_SHOWNORMAL, ewNoWait, Code);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopPimpMyShell();
  Result := '';
end;

function InitializeUninstall(): Boolean;
begin
  StopPimpMyShell();
  Result := True;
end;

// The settings can turn starting at sign-in on after the install, and Task Manager can mark it off: both go with the app.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
  begin
    RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'PimpMyShell');
    RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run', 'PimpMyShell');
    // The name and logo PimpMyShell's notifications go out under (tray.rs).
    RegDeleteKeyIncludingSubkeys(HKCU, 'Software\Classes\AppUserModelId\PimpMyShell');
  end;
end;

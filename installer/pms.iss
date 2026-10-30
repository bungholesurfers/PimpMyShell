; PimpMyShell's Windows installer, for Inno Setup 6 (https://jrsoftware.org/isinfo.php). `cargo xtask installer`, from
; the folder above, builds the release exes and compiles this into target\installer\pms-setup-VERSION.exe.
;
; It installs for the user alone, with no administrator: into %LOCALAPPDATA%\Programs\PimpMyShell, where Windows keeps
; per-user programs, with a Start menu shortcut to the settings and an entry in Settings > Apps. Starting at sign-in
; is a task, ticked to begin with; the settings' General page can switch it later. The config, in
; %LOCALAPPDATA%\PimpMyShell, is the user's and is left on uninstall. INSTALLER.md, beside this file, says how to build it.

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
WizardStyle=modern dynamic
; pms-art draws these (cargo xtask art): the box as Setup's icon, the box on the dark of the sea for the welcome and
; finish pages, the box in the other pages' corner - each at the sizes display scales want.
SetupIconFile=art\setup.ico
WizardImageFile=art\wizard-*.png
WizardSmallImageFile=art\corner-*.png
AppPublisher=PimpMyShell
AppComments=Exclusively for Windows 11; The daemon is stopped by the code below (which also restores the stock taskbar), not by the Restart Manager.
CloseApplications=no

[Tasks]
Name: "autostart"; Description: "Start PimpMyShell when I sign in"
Name: "desktopicon"; Description: "Create a desktop shortcut to PimpMyShell's settings"; Flags: unchecked

[Files]
Source: "{#Bin}\pmsd.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Bin}\pms.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Bin}\pms-gui.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Bin}\pms_tap.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\pms.example.toml"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\PimpMyShell"; Filename: "{app}\pms-gui.exe"; Comment: "PimpMyShell's settings"
Name: "{autodesktop}\PimpMyShell"; Filename: "{app}\pms-gui.exe"; Tasks: desktopicon

[Registry]
; The same value the settings' "Start at logon" writes; the uninstaller removes it whichever wrote it (see below).
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "PimpMyShell"; \
  ValueData: """{app}\pmsd.exe"""; Flags: uninsdeletevalue; Tasks: autostart

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
Type: files; Name: "{localappdata}\PimpMyShell\tree.txt"
Type: files; Name: "{localappdata}\PimpMyShell\update-checked.txt"

[Run]
Filename: "{app}\pmsd.exe"; Description: "Start PimpMyShell now"; Flags: nowait postinstall skipifsilent
Filename: "{app}\pms-gui.exe"; Description: "Open PimpMyShell's settings"; Flags: nowait postinstall skipifsilent unchecked

[Code]
var
  WasRunning: Boolean;

// Stops a running PimpMyShell: the daemon is asked to stop, which puts the stock taskbar (and notification banners)
// back; the settings window is closed. Notes whether the daemon ran, so that a silent update starts it again.
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
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/f /im pms-gui.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Sleep(300);
end;

// A silent install (an update) skips the finish page: a daemon it stopped is started again, the new one.
procedure CurStepChanged(CurStep: TSetupStep);
var
  Code: Integer;
begin
  if (CurStep = ssPostInstall) and WizardSilent and WasRunning then
    ExecAsOriginalUser(ExpandConstant('{app}\pmsd.exe'), '', '', SW_SHOWNORMAL, ewNoWait, Code);
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

; Savepoint installer (Inno Setup 6).
;
; Built by scripts/package.ps1, which passes the version and the folder with the
; release files:  ISCC /DAppVersion=0.1.0 /DSourceDir=..\dist\Savepoint savepoint.iss
;
; Installs for the current user only (no admin prompt), into
; %LOCALAPPDATA%\Programs\Savepoint. Uninstalling never touches the user's
; settings or backups.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\dist\Savepoint"
#endif

#define AppName "Savepoint"
#define AppExe "savepointd.exe"
#define RepoUrl "https://github.com/eneskocamaan/savepoint"

[Setup]
; Never change AppId: it is how Windows recognizes later versions as updates.
AppId={{8058CA50-1E80-4B52-8DC8-41A7867FB348}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=eneskocamaan
AppPublisherURL={#RepoUrl}
AppSupportURL={#RepoUrl}/issues
AppUpdatesURL={#RepoUrl}/releases
AppCopyright=Copyright (c) 2026 eneskocamaan
VersionInfoVersion={#AppVersion}
PrivilegesRequired=lowest
DefaultDirName={autopf}\{#AppName}
DisableProgramGroupPage=yes
DisableDirPage=auto
DisableReadyPage=yes
LicenseFile=..\LICENSE
OutputDir=..\dist
OutputBaseFilename=Savepoint-{#AppVersion}-windows-x64-setup
SetupIconFile=..\ui\src-tauri\icons\icon.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
CloseApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "turkish"; MessagesFile: "compiler:Languages\Turkish.isl"
Name: "spanish"; MessagesFile: "compiler:Languages\Spanish.isl"

[CustomMessages]
english.NeedWebView2=Savepoint's window uses Microsoft Edge WebView2, which was not found on this PC. It comes with Windows 11 and most Windows 10 updates.%n%nOpen the download page now? (Savepoint's backups work without it; only the window needs it.)
turkish.NeedWebView2=Savepoint penceresi Microsoft Edge WebView2 kullanır ve bu bilgisayarda bulunamadı. Windows 11 ile ve çoğu Windows 10 güncellemesiyle birlikte gelir.%n%nİndirme sayfası şimdi açılsın mı? (Yedekleme onsuz da çalışır; yalnızca pencere için gerekir.)
spanish.NeedWebView2=La ventana de Savepoint usa Microsoft Edge WebView2, que no se encontró en este PC. Viene con Windows 11 y con la mayoría de las actualizaciones de Windows 10.%n%n¿Abrir ahora la página de descarga? (Las copias funcionan sin él; solo la ventana lo necesita.)

[Tasks]
Name: "autostart"; Description: "{cm:AutoStartProgram,{#AppName}}"; GroupDescription: "{cm:AutoStartProgramGroupDescription}"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\savepointd.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\savepoint-ui.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\WebView2Loader.dll"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "{#SourceDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\THIRD-PARTY-LICENSES.txt"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist

[Icons]
; "--show" opens the window; a second start only brings up the running copy.
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"; Parameters: "--show"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Parameters: "--show"; Tasks: desktopicon

[Registry]
; The same value the app's own "Start with Windows" switch writes, so both stay in sync.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "{#AppName}"; ValueData: """{app}\{#AppExe}"""; Tasks: autostart

[Run]
Filename: "{app}\{#AppExe}"; Parameters: "--show"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[Code]
const
  WebView2Key = '\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';

function WebView2Installed: Boolean;
var
  Version: String;
begin
  Result :=
    (RegQueryStringValue(HKLM, 'SOFTWARE\WOW6432Node' + WebView2Key, 'pv', Version) and (Version <> '') and (Version <> '0.0.0.0')) or
    (RegQueryStringValue(HKCU, 'Software' + WebView2Key, 'pv', Version) and (Version <> '') and (Version <> '0.0.0.0'));
end;

{ Savepoint runs in the background, so it is stopped before its files are replaced or removed. }
procedure StopSavepoint;
var
  Code: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM savepointd.exe /IM savepoint-ui.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Sleep(400);
end;

function InitializeSetup: Boolean;
var
  Code: Integer;
begin
  Result := True;
  if not WebView2Installed and not WizardSilent then
    if MsgBox(CustomMessage('NeedWebView2'), mbConfirmation, MB_YESNO) = IDYES then
      ShellExec('open', 'https://developer.microsoft.com/microsoft-edge/webview2/', '', '', SW_SHOWNORMAL, ewNoWait, Code);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopSavepoint;
  Result := '';
end;

function InitializeUninstall: Boolean;
begin
  StopSavepoint;
  Result := True;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  { The app itself may have set this, so it is removed even if the installer did not add it.
    Settings and backups are left alone on purpose. }
  if CurUninstallStep = usPostUninstall then
    RegDeleteValue(HKCU, RunKey, '{#AppName}');
end;

#ifndef AppVersion
  #error AppVersion must be supplied by build-installer.ps1
#endif
#ifndef BuildDir
  #error BuildDir must be supplied by build-installer.ps1
#endif
#ifndef OutputDir
  #error OutputDir must be supplied by build-installer.ps1
#endif
#ifndef VcRuntimePath
  #error VcRuntimePath must point to the x64 Visual C++ redistributable DLL
#endif
#ifndef NoticesPath
  #error NoticesPath must point to generated dependency license notices
#endif

[Setup]
; Keep this identity stable so updates share the same installation and uninstaller.
AppId={{707A222C-758B-4E20-A86E-3A577E428B95}
AppName=Cash Ledger
AppVersion={#AppVersion}
AppVerName=Cash Ledger {#AppVersion}
; Preserve the legacy directory so existing installations upgrade in place.
DefaultDirName={localappdata}\Programs\Till
DefaultGroupName=Cash Ledger
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; The release imports Windows ICU, available from Windows 10 version 1703.
MinVersion=10.0.15063
OutputDir={#OutputDir}
OutputBaseFilename=CashLedger-{#AppVersion}-windows-x64-setup
SetupIconFile=..\..\assets\till.ico
UninstallDisplayIcon={app}\cash-counter.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no
; Every GUI instance holds this mutex; unattended updates abort rather than
; replacing an executable while another session is using it.
AppMutex=Local\CashLedger-707A222C-758B-4E20-A86E-3A577E428B95

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[Files]
Source: "{#BuildDir}\cash-counter.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#VcRuntimePath}"; DestDir: "{app}"
Source: "..\..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\THIRD_PARTY_NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#NoticesPath}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\Cash Ledger"; Filename: "{app}\cash-counter.exe"; WorkingDir: "{app}"
Name: "{userdesktop}\Cash Ledger"; Filename: "{app}\cash-counter.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[InstallDelete]
; Remove only shortcuts belonging to the previous product name.
Type: files; Name: "{userprograms}\Till.lnk"
Type: files; Name: "{userdesktop}\Till.lnk"

[INI]
Filename: "{app}\cash-ledger-installation.ini"; Section: "Installation"; Key: "AppId"; String: "707A222C-758B-4E20-A86E-3A577E428B95"; Flags: uninsdeleteentry

[UninstallDelete]
Type: files; Name: "{app}\cash-ledger-installation.ini"

[Run]
Filename: "{app}\cash-counter.exe"; Description: "Open Cash Ledger"; Flags: nowait postinstall skipifsilent; Check: IsManualInstall
; Update relaunch occurs exactly once after successful file replacement. Files
; deliberately omit restartreplace: a locked executable must fail, not queue an update.
; postinstall also prevents launch if Setup requires a system restart. Omit
; skipifsilent so app-initiated silent updates still launch the new version.
Filename: "{app}\cash-counter.exe"; Flags: nowait postinstall runasoriginaluser; Check: IsAppUpdate

[Code]
function IsAppUpdate: Boolean;
var
  Index: Integer;
begin
  Result := False;
  for Index := 1 to ParamCount do
    if CompareText(ParamStr(Index), '/UPDATEFROMAPP') = 0 then
      Result := True;
end;

function IsManualInstall: Boolean;
begin
  Result := not IsAppUpdate;
end;

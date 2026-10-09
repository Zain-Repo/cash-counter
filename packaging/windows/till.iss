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
AppName=Till
AppVersion={#AppVersion}
AppVerName=Till {#AppVersion}
DefaultDirName={localappdata}\Programs\Till
DefaultGroupName=Till
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; The release imports Windows ICU, available from Windows 10 version 1703.
MinVersion=10.0.15063
OutputDir={#OutputDir}
OutputBaseFilename=Till-{#AppVersion}-windows-x64-setup
SetupIconFile=..\..\assets\till.ico
UninstallDisplayIcon={app}\cash-counter.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[Files]
Source: "{#BuildDir}\cash-counter.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#VcRuntimePath}"; DestDir: "{app}"
Source: "..\..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\THIRD_PARTY_NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#NoticesPath}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\Till"; Filename: "{app}\cash-counter.exe"; WorkingDir: "{app}"
Name: "{userdesktop}\Till"; Filename: "{app}\cash-counter.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\cash-counter.exe"; Description: "Open Till"; Flags: nowait postinstall skipifsilent

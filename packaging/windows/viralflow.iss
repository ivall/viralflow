; Windows installer for ViralFlow (Inno Setup 6). Built by scripts/package.ps1,
; which passes AppVersion, SourceDir (the staged app folder) and OutputDir.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{6E4C7F0B-6C1B-4E7E-9E1A-5B7A3F2D9C41}
AppName=ViralFlow
AppVersion={#AppVersion}
AppPublisher=ivall
AppPublisherURL=https://github.com/ivall/viralflow
DefaultDirName={autopf}\ViralFlow
DefaultGroupName=ViralFlow
DisableProgramGroupPage=yes
PrivilegesRequiredOverridesAllowed=dialog
OutputDir={#OutputDir}
OutputBaseFilename=ViralFlow-{#AppVersion}-setup
SetupIconFile=..\..\assets\icons\icon.ico
UninstallDisplayIcon={app}\ViralFlow.exe
LicenseFile={#SourceDir}\LICENSE.txt
Compression=lzma2/max
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs

[Icons]
Name: "{autoprograms}\ViralFlow"; Filename: "{app}\ViralFlow.exe"
Name: "{autodesktop}\ViralFlow"; Filename: "{app}\ViralFlow.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\ViralFlow.exe"; Description: "{cm:LaunchProgram,ViralFlow}"; Flags: nowait postinstall skipifsilent

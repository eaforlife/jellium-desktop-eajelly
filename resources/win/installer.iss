#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef AppArch
  #define AppArch "x64"
#endif
#ifndef SourceDir
  #define SourceDir "..\..\build\install"
#endif
#ifndef OutputDir
  #define OutputDir "..\..\dist"
#endif

[Setup]
AppId={{6F46FC31-79CF-49CF-91C1-87710731168B}
AppName=Jellium Desktop eajelly
AppVersion={#AppVersion}
AppPublisher=eajelly
AppPublisherURL=https://github.com/eaforlife/jellium-desktop-eajelly
AppSupportURL=https://github.com/eaforlife/jellium-desktop-eajelly/issues
DefaultDirName={autopf}\Jellium Desktop eajelly
DefaultGroupName=Jellium Desktop eajelly
DisableProgramGroupPage=yes
OutputDir={#OutputDir}
OutputBaseFilename=JelliumDesktop-{#AppVersion}-windows-{#AppArch}-setup
SetupIconFile=jellyfin.ico
UninstallDisplayIcon={app}\jellium-desktop.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
PrivilegesRequired=admin
#if AppArch == "arm64"
ArchitecturesAllowed=arm64
ArchitecturesInstallIn64BitMode=arm64
#else
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
#endif

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional icons:"; Flags: unchecked

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Jellium Desktop eajelly"; Filename: "{app}\jellium-desktop.exe"
Name: "{autodesktop}\Jellium Desktop eajelly"; Filename: "{app}\jellium-desktop.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\jellium-desktop.exe"; Description: "Launch Jellium Desktop eajelly"; Flags: nowait postinstall skipifsilent

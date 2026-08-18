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
AppName=Jellium Desktop - EAJelly
AppVersion={#AppVersion}
AppPublisher=eajelly
AppPublisherURL=https://github.com/eaforlife/jellium-desktop-eajelly
AppSupportURL=https://github.com/eaforlife/jellium-desktop-eajelly/issues
DefaultDirName={autopf}\Jellium Desktop - EAJelly
DefaultGroupName=Jellium Desktop - EAJelly
DisableProgramGroupPage=yes
OutputDir={#OutputDir}
OutputBaseFilename=JelliumDesktop-{#AppVersion}-windows-{#AppArch}-setup
SetupIconFile=jellyfin.ico
UninstallDisplayIcon={app}\jellium-desktop.exe
Compression=lzma2/fast
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
Name: "{autoprograms}\Jellium Desktop - EAJelly"; Filename: "{app}\jellium-desktop.exe"
Name: "{autodesktop}\Jellium Desktop - EAJelly"; Filename: "{app}\jellium-desktop.exe"; Tasks: desktopicon

[InstallDelete]
Type: files; Name: "{autoprograms}\Jellium Desktop eajelly.lnk"
Type: files; Name: "{autodesktop}\Jellium Desktop eajelly.lnk"

[Run]
Filename: "{app}\jellium-desktop.exe"; Description: "Launch Jellium Desktop - EAJelly"; Flags: nowait postinstall skipifsilent

[Code]
const
  UninstallRegistryPath = 'Software\Microsoft\Windows\CurrentVersion\Uninstall';
  LegacyJellyfinAppId = '{a78bea4a-5bd0-4aa3-bdf3-579b4f58a921}_is1';

function LegacyJellyfinDesktopInRegistry(RootKey: Integer): Boolean;
var
  DisplayName: String;
  I: Integer;
  Subkeys: TArrayOfString;
begin
  Result := RegKeyExists(
    RootKey,
    UninstallRegistryPath + '\' + LegacyJellyfinAppId
  );
  if Result then
    Exit;

  { Also cover older MSI/WiX releases whose product key changed by version. }
  if not RegGetSubkeyNames(RootKey, UninstallRegistryPath, Subkeys) then
    Exit;

  for I := 0 to GetArrayLength(Subkeys) - 1 do
  begin
    if RegQueryStringValue(
      RootKey,
      UninstallRegistryPath + '\' + Subkeys[I],
      'DisplayName',
      DisplayName
    ) and (CompareText(Trim(DisplayName), 'Jellyfin Desktop') = 0) then
    begin
      Result := True;
      Exit;
    end;
  end;
end;

function LegacyJellyfinDesktopInstalled: Boolean;
begin
  Result :=
    LegacyJellyfinDesktopInRegistry(HKCU32) or
    LegacyJellyfinDesktopInRegistry(HKCU64) or
    LegacyJellyfinDesktopInRegistry(HKLM32) or
    LegacyJellyfinDesktopInRegistry(HKLM64);
end;

function InitializeSetup: Boolean;
begin
  Result := not LegacyJellyfinDesktopInstalled;
  if not Result then
    MsgBox(
      'The old Jellyfin Desktop app is still installed.' + #13#10 + #13#10 +
      'Uninstall Jellyfin Desktop from Windows Settings > Apps before ' +
      'installing Jellium Desktop - EAJelly.',
      mbError,
      MB_OK
    );
end;

; Cuprita's Windows installer (Inno Setup 6), after `cargo build --release -p cuprita`, the
; page Share shows in target\web (`web.sh` builds it, on any system: the workflow's `web` job)
; and KiCad's libraries in target\libraries (`bash packaging/libraries.sh` fetches them):
;   iscc /DVersion=0.0.1 packaging\windows\cuprita.iss
; writes target\package\cuprita-<version>-windows-x64-setup.exe. It installs for the user, or for
; everyone if they choose so (then with an administrator's OK), in the app's languages (Inno Setup
; picks the system's), and claims no file types: KiCad files still open where they did.
#ifndef Version
  #define Version "0.0.0"
#endif

[Setup]
AppId={{E34ECF1B-FC09-4798-87EB-127E18A5B731}
AppName=Cuprita
AppVersion={#Version}
AppPublisher=Cuprita's developers
AppPublisherURL=https://github.com/rafaelcapeloo/cuprita
DefaultDirName={autopf}\Cuprita
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
LicenseFile=..\..\LICENSE
SetupIconFile=cuprita.ico
UninstallDisplayIcon={app}\cuprita.ico
OutputDir=..\..\target\package
OutputBaseFilename=cuprita-{#Version}-windows-x64-setup
Compression=lzma2
SolidCompression=yes
; KiCad's 3D models are 3 GB to compress: in blocks, on every core.
LZMANumBlockThreads=4
WizardStyle=modern

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "pt_BR"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\..\target\release\cuprita.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "cuprita.ico"; DestDir: "{app}"
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"
Source: "..\NOTICE.txt"; DestDir: "{app}"
Source: "..\CRATES.txt"; DestDir: "{app}"
Source: "..\..\crates\app\assets\LICENSE-*.txt"; DestDir: "{app}\licenses"
Source: "..\..\target\web\index.html"; DestDir: "{app}\web"; Flags: ignoreversion
Source: "..\..\target\web\*.js"; DestDir: "{app}\web"; Flags: ignoreversion
Source: "..\..\target\web\*.wasm"; DestDir: "{app}\web"; Flags: ignoreversion
Source: "..\..\target\web\*.svg"; DestDir: "{app}\web"; Flags: ignoreversion
Source: "..\..\target\libraries\*"; DestDir: "{app}\kicad"; Flags: ignoreversion recursesubdirs createallsubdirs

[InstallDelete]
Type: filesandordirs; Name: "{app}\web"
Type: filesandordirs; Name: "{app}\kicad"

[Icons]
Name: "{autoprograms}\Cuprita"; Filename: "{app}\cuprita.exe"; IconFilename: "{app}\cuprita.ico"
Name: "{autodesktop}\Cuprita"; Filename: "{app}\cuprita.exe"; IconFilename: "{app}\cuprita.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\cuprita.exe"; Description: "{cm:LaunchProgram,Cuprita}"; Flags: nowait postinstall skipifsilent

; Inno Setup 6 script for Rust Photos. Build with installer\build-installer.ps1 (or compile this file in the Inno Setup IDE
; after `cargo build --release -p rust-photos --target-dir target-dist`).
#ifndef AppVersion
  #define AppVersion "3.0.0"
#endif
#define AppName "Rust Photos"
#define AppExe "rust-photos.exe"

[Setup]
; keep this GUID forever: it is how Windows recognises an upgrade of the same app
AppId={{F50877CA-86B6-4225-94F5-948D859CCC82}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=minhtrong67
DefaultDirName={autopf}\Rust Photos
DefaultGroupName=Rust Photos
DisableProgramGroupPage=yes
OutputDir=..\dist
OutputBaseFilename=RustPhotos-Setup-{#AppVersion}
SetupIconFile=..\crates\app\assets\icon.ico
UninstallDisplayIcon={app}\icon.ico
UninstallDisplayName={#AppName}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; installs for the current user without an admin prompt; the dialog lets people choose "all users"
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
MinVersion=10.0
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "assoc"; Description: "Show Rust Photos in ""Open with"" for pictures (jpg, png, webp, gif, bmp, ico)"; GroupDescription: "File types:"; Flags: unchecked

[Files]
Source: "..\target-dist\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\crates\app\assets\icon.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"; IconFilename: "{app}\icon.ico"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; IconFilename: "{app}\icon.ico"; Tasks: desktopicon

[Registry]
; a picture "class" for the app, then one OpenWithProgids entry per extension. This adds the app to "Open with" and to
; Settings > Default apps without taking over any file type by force.
Root: HKA; Subkey: "Software\Classes\RustPhotos.Image"; ValueType: string; ValueData: "Picture"; Flags: uninsdeletekey; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\RustPhotos.Image\DefaultIcon"; ValueType: string; ValueData: "{app}\icon.ico"; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\RustPhotos.Image\shell\open\command"; ValueType: string; ValueData: """{app}\{#AppExe}"" ""%1"""; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\.jpg\OpenWithProgids"; ValueType: string; ValueName: "RustPhotos.Image"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\.jpeg\OpenWithProgids"; ValueType: string; ValueName: "RustPhotos.Image"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\.jfif\OpenWithProgids"; ValueType: string; ValueName: "RustPhotos.Image"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\.png\OpenWithProgids"; ValueType: string; ValueName: "RustPhotos.Image"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\.gif\OpenWithProgids"; ValueType: string; ValueName: "RustPhotos.Image"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\.webp\OpenWithProgids"; ValueType: string; ValueName: "RustPhotos.Image"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\.bmp\OpenWithProgids"; ValueType: string; ValueName: "RustPhotos.Image"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\.ico\OpenWithProgids"; ValueType: string; ValueName: "RustPhotos.Image"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc

[Run]
Filename: "{app}\{#AppExe}"; Description: "Launch {#AppName}"; Flags: nowait postinstall skipifsilent

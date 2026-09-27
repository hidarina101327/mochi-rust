; 正式 Windows 安装包。构建变量由 package-native.ps1 注入，避免把开发机路径
; 或用户名写入发布产物。
#define AppVersion GetEnv("MOCHI_PACKAGE_VERSION")
#define AppSource GetEnv("MOCHI_PACKAGE_STAGE")
#define AppOutput GetEnv("MOCHI_PACKAGE_OUTPUT")

[Setup]
AppId={{B7972D74-8E6F-4C22-82A4-B92B7439C1A5}
AppName=墨池
AppVersion={#AppVersion}
AppVerName=墨池 {#AppVersion}
AppPublisher=墨池
DefaultDirName={localappdata}\Programs\墨池
DefaultGroupName=墨池
DisableProgramGroupPage=yes
DisableWelcomePage=no
DisableDirPage=no
UsePreviousAppDir=yes
OutputDir={#AppOutput}
OutputBaseFilename=Mochi-{#AppVersion}-x64-Setup
SetupIconFile={#AppSource}\Mochi.ico
UninstallDisplayIcon={app}\Mochi.exe
VersionInfoVersion={#AppVersion}
VersionInfoCompany=墨池
VersionInfoProductName=墨池
VersionInfoDescription=墨池个人工作台
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
WizardStyle=modern
Compression=lzma2/ultra64
SolidCompression=yes
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "chinesesimplified"; MessagesFile: "lang\ChineseSimplified.isl"

[Tasks]
Name: "desktopicon"; Description: "在桌面创建快捷方式"; GroupDescription: "附加图标:"; Flags: checkedonce

[Files]
Source: "{#AppSource}\Mochi.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#AppSource}\mochi-workflow.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#AppSource}\mochi-clipper-host.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#AppSource}\mochi-community.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#AppSource}\Mochi.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#AppSource}\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#AppSource}\runtime\python\*"; DestDir: "{app}\runtime\python"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\墨池"; Filename: "{app}\Mochi.exe"; WorkingDir: "{app}"; IconFilename: "{app}\Mochi.ico"
Name: "{autodesktop}\墨池"; Filename: "{app}\Mochi.exe"; WorkingDir: "{app}"; IconFilename: "{app}\Mochi.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\Mochi.exe"; Parameters: "--install-bundled-guides"; Flags: runhidden waituntilterminated
Filename: "{app}\Mochi.exe"; Parameters: "--register-web-clipper"; Flags: runhidden waituntilterminated
Filename: "{app}\Mochi.exe"; Description: "启动墨池"; Flags: nowait postinstall skipifsilent
Filename: "{app}\Mochi.exe"; Flags: nowait skipifnotsilent; Check: RestartAfterUpdate

[UninstallDelete]
Type: files; Name: "{localappdata}\Mochi\NativeMessaging\com.mochi.web_clipper.json"

[Code]
function RestartAfterUpdate: Boolean;
begin
  Result := ExpandConstant('{param:MOCHIRESTART|0}') = '1';
end;
// 此处的每用户启动项由应用管理。卸载时清除旧命令，
// 但保留 StartupApproved 中的设置，
// 让用户在 Windows 中关闭自启动的选择在重新安装后仍然生效。
procedure CurUninstallStepChanged(UninstallStep: TUninstallStep);
begin
  if UninstallStep = usUninstall then
  begin
    RegDeleteKeyIncludingSubkeys(HKEY_CURRENT_USER, 'Software\Google\Chrome\NativeMessagingHosts\com.mochi.web_clipper');
    RegDeleteKeyIncludingSubkeys(HKEY_CURRENT_USER, 'Software\Microsoft\Edge\NativeMessagingHosts\com.mochi.web_clipper');
    RegDeleteKeyIncludingSubkeys(HKEY_CURRENT_USER, 'Software\Classes\mochi-clipper');
    RegDeleteValue(
      HKEY_CURRENT_USER,
      'Software\Microsoft\Windows\CurrentVersion\Run',
      'Mochi');
  end;
end;

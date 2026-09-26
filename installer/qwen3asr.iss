#ifndef StageDir
#define StageDir GetEnv("QWEN3ASR_PACKAGING_STAGE_DIR")
#if StageDir == ""
#define StageDir AddBackslash(SourcePath) + "..\.ag-artifacts\packaging-stage\default"
#endif
#endif

[Setup]
AppId={{BC30E4CA-27B1-4A3F-9C4F-278F8D7E6CA0}
AppName=Qwen3ASR
AppVersion=0.2.0
AppVerName=Qwen3ASR 0.2.0
AppPublisher=Qwen3ASR
DefaultDirName={autopf}\Qwen3ASR
DefaultGroupName=Qwen3ASR
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
UsePreviousPrivileges=yes
ArchitecturesAllowed=x64os
ArchitecturesInstallIn64BitMode=x64os
ChangesEnvironment=yes
SetupLogging=yes
OutputDir=..\dist
OutputBaseFilename=qwen3asr-0.2.0-setup
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
UninstallDisplayName=Qwen3ASR 0.2.0
UninstallLogging=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "chinesetraditional"; MessagesFile: "compiler:Languages\ChineseTraditional.isl"

[CustomMessages]
english.AddToPathTask=Add Qwen3ASR to this install scope's PATH
english.CodexTask=Configure the Codex integration
english.AgyTask=Configure the AGY integration
english.ClaudeTask=Configure the Claude integration
english.PathAddFailed=Qwen3ASR could not update PATH. The install folder is still available directly.
english.PathRemoveFailed=Qwen3ASR could not remove its PATH entry. Other PATH entries were preserved.
english.IntegrationStartFailed=Could not start the %1 integration. Setup log: %2
english.IntegrationFailed=The %1 integration failed with exit code %2. Setup log: %3
english.IntegrationOwnershipRecordFailed=The %1 integration completed, but its uninstall ownership marker could not be saved. Run agents uninstall --target %1 before removing this install. Setup log: %2
english.UninstallIntegrationStartFailed=Could not start removal of the %1 integration (error code %2). Its client configuration may still refer to this install. Uninstall log: %3
english.UninstallIntegrationFailed=Removal of the %1 integration returned exit code %2. Changed or foreign client entries were preserved. Uninstall log: %3
english.UninstallIntegrationNotOwned=Skipped removal of the %1 integration because this install's current-user ownership marker did not match its install path. Client settings were left untouched. Uninstall log: %2
english.UninstallIntegrationMarkerFailed=The %1 integration was removed, but its current-user ownership marker could not be cleared. Uninstall log: %2
chinesetraditional.AddToPathTask=將 Qwen3ASR 加入目前安裝範圍的 PATH
chinesetraditional.CodexTask=設定 Codex 整合
chinesetraditional.AgyTask=設定 AGY 整合
chinesetraditional.ClaudeTask=設定 Claude 整合
chinesetraditional.PathAddFailed=無法更新 PATH；仍可直接從安裝資料夾執行 Qwen3ASR。
chinesetraditional.PathRemoveFailed=無法移除 Qwen3ASR 的 PATH 項目；其他 PATH 項目已保留。
chinesetraditional.IntegrationStartFailed=無法啟動 %1 整合。安裝記錄：%2
chinesetraditional.IntegrationFailed=%1 整合失敗，結束碼為 %2。安裝記錄：%3
chinesetraditional.IntegrationOwnershipRecordFailed=%1 整合已完成，但無法記錄此安裝的解除安裝所有權。移除此安裝前請先手動執行 agents uninstall --target %1。安裝記錄：%2
chinesetraditional.UninstallIntegrationStartFailed=無法啟動 %1 整合的移除程序（錯誤碼 %2）。用戶端設定可能仍指向此安裝。解除安裝記錄：%3
chinesetraditional.UninstallIntegrationFailed=%1 整合移除程序回傳結束碼 %2。已變更或屬於其他安裝的用戶端設定會保留。解除安裝記錄：%3
chinesetraditional.UninstallIntegrationNotOwned=因目前使用者的所有權標記與此安裝路徑不符，已略過移除 %1 整合。用戶端設定保持不變。解除安裝記錄：%2
chinesetraditional.UninstallIntegrationMarkerFailed=%1 整合已移除，但無法清除目前使用者的所有權標記。解除安裝記錄：%2

[Tasks]
Name: "addtopath"; Description: "{cm:AddToPathTask}"; Flags: checkedonce
Name: "codex"; Description: "{cm:CodexTask}"; Flags: unchecked; Check: not IsAdminInstallMode
Name: "agy"; Description: "{cm:AgyTask}"; Flags: unchecked; Check: not IsAdminInstallMode
Name: "claude"; Description: "{cm:ClaudeTask}"; Flags: unchecked; Check: not IsAdminInstallMode

[Files]
Source: "{#StageDir}\qwen3asr.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\native\cpu\*"; DestDir: "{app}\native\cpu"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#StageDir}\native\cuda\*"; DestDir: "{app}\native\cuda"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#StageDir}\native\ffmpeg.exe"; DestDir: "{app}\native"; Flags: ignoreversion
Source: "{#StageDir}\native\ffprobe.exe"; DestDir: "{app}\native"; Flags: ignoreversion
Source: "{#StageDir}\native\licenses\*"; DestDir: "{app}\native\licenses"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#StageDir}\skills\qwen3asr\SKILL.md"; DestDir: "{app}\skills\qwen3asr"; Flags: ignoreversion
Source: "{#StageDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\THIRD_PARTY_NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\docs\*"; DestDir: "{app}\docs"; Flags: ignoreversion recursesubdirs createallsubdirs

[Code]
const
  PathSubkeyUser = 'Environment';
  PathSubkeySystem = 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment';

var
  RequestedActionsFailed: Boolean;

function InitializeSetup: Boolean;
begin
  RequestedActionsFailed := False;
  Result := True;
end;

function GetCustomSetupExitCode: Integer;
begin
  if RequestedActionsFailed then
  begin
    Log('[setup failure] A requested PATH or integration action failed; returning exit code 10.');
    Result := 10;
  end
  else
    Result := 0;
end;

function PathRootKey: Integer;
begin
  if IsAdminInstallMode then
    Result := HKLM
  else
    Result := HKCU;
end;

function PathSubkey: String;
begin
  if IsAdminInstallMode then
    Result := PathSubkeySystem
  else
    Result := PathSubkeyUser;
end;

function PathMarkerSubkey: String;
begin
  Result := 'Software\Qwen3ASR\InstallerState';
end;

function PathMarkerExists: Boolean;
begin
  Result := RegValueExists(PathRootKey, PathMarkerSubkey, 'PathAddedByInstaller');
end;

function PathWasPresentBeforeInstall: Boolean;
var
  Value: String;
begin
  Value := '';
  Result := RegQueryStringValue(PathRootKey, PathMarkerSubkey, 'PathWasPresent', Value) and (Value = '1');
end;

function NormalizePathEntry(const Value: String): String;
begin
  Result := Trim(Value);
  if (Length(Result) >= 2) and (Result[1] = '"') and (Result[Length(Result)] = '"') then
    Result := Copy(Result, 2, Length(Result) - 2);
  while (Length(Result) > 3) and (Result[Length(Result)] = '\') do
    Delete(Result, Length(Result), 1);
end;

function IntegrationMarkerSubkey: String;
var
  InstallDir: String;
begin
  InstallDir := UpperCase(NormalizePathEntry(ExpandConstant('{app}')));
  Result := 'Software\Qwen3ASR\InstallerState\Integrations\' +
    GetSHA256OfUnicodeString(InstallDir);
end;

function RecordIntegrationOwnership(const Target: String): Boolean;
var
  InstallDir, ExistingInstallDir: String;
begin
  InstallDir := NormalizePathEntry(ExpandConstant('{app}'));
  ExistingInstallDir := '';
  if RegQueryStringValue(HKCU, IntegrationMarkerSubkey, 'InstallPath', ExistingInstallDir) and
    (CompareText(NormalizePathEntry(ExistingInstallDir), InstallDir) <> 0) then
  begin
    Result := False;
    Exit;
  end;
  if not RegWriteStringValue(HKCU, IntegrationMarkerSubkey, 'InstallPath', InstallDir) then
  begin
    Result := False;
    Exit;
  end;
  Result := RegWriteStringValue(HKCU, IntegrationMarkerSubkey, Target, '1');
end;

function ClearIntegrationOwnership(const Target: String): Boolean;
begin
  Result := RegDeleteValue(HKCU, IntegrationMarkerSubkey, Target);
  if not Result then
    Exit;
  if not RegValueExists(HKCU, IntegrationMarkerSubkey, 'codex') and
    not RegValueExists(HKCU, IntegrationMarkerSubkey, 'agy') and
    not RegValueExists(HKCU, IntegrationMarkerSubkey, 'claude') then
  begin
    if RegValueExists(HKCU, IntegrationMarkerSubkey, 'InstallPath') and
      not RegDeleteValue(HKCU, IntegrationMarkerSubkey, 'InstallPath') then
    begin
      Result := False;
      Exit;
    end;
    RegDeleteKeyIfEmpty(HKCU, IntegrationMarkerSubkey);
  end;
end;

function PathContainsEntry(const PathValue, Entry: String): Boolean;
var
  I, StartAt: Integer;
  Item: String;
  AtDelimiter: Boolean;
begin
  Result := False;
  StartAt := 1;
  for I := 1 to Length(PathValue) + 1 do
  begin
    if I > Length(PathValue) then
      AtDelimiter := True
    else
      AtDelimiter := PathValue[I] = ';';
    if AtDelimiter then
    begin
      Item := Copy(PathValue, StartAt, I - StartAt);
      if CompareText(NormalizePathEntry(Item), NormalizePathEntry(Entry)) = 0 then
      begin
        Result := True;
        Exit;
      end;
      StartAt := I + 1;
    end;
  end;
end;

function AddInstallDirToPath: Boolean;
var
  Existing, InstallDir, Updated, WasPresentValue: String;
  ExistingPathPresent: Boolean;
begin
  InstallDir := ExpandConstant('{app}');
  Existing := '';
  ExistingPathPresent := RegQueryStringValue(PathRootKey, PathSubkey, 'Path', Existing);
  if PathContainsEntry(Existing, InstallDir) then
  begin
    Result := True;
    Exit;
  end;
  if Existing = '' then
    Updated := InstallDir
  else if Existing[Length(Existing)] = ';' then
    Updated := Existing + InstallDir
  else
    Updated := Existing + ';' + InstallDir;
  if not RegWriteExpandStringValue(PathRootKey, PathSubkey, 'Path', Updated) then
  begin
    Result := False;
    Exit;
  end;
  if ExistingPathPresent then
    WasPresentValue := '1'
  else
    WasPresentValue := '0';
  if not RegWriteStringValue(PathRootKey, PathMarkerSubkey, 'PathWasPresent', WasPresentValue) or
    not RegWriteStringValue(PathRootKey, PathMarkerSubkey, 'PathAddedByInstaller', '1') then
  begin
    if ExistingPathPresent then
      RegWriteExpandStringValue(PathRootKey, PathSubkey, 'Path', Existing)
    else
      RegDeleteValue(PathRootKey, PathSubkey, 'Path');
    RegDeleteValue(PathRootKey, PathMarkerSubkey, 'PathWasPresent');
    RegDeleteValue(PathRootKey, PathMarkerSubkey, 'PathAddedByInstaller');
    Result := False;
    Exit;
  end;
  Result := True;
end;

function RemoveInstallDirFromPath: Boolean;
var
  Existing, InstallDir, Updated, Item: String;
  I, StartAt: Integer;
  FirstItem, Found, OtherItems, WasPresent: Boolean;
  AtDelimiter: Boolean;
begin
  Result := True;
  if not PathMarkerExists then
    Exit;
  Existing := '';
  if not RegQueryStringValue(PathRootKey, PathSubkey, 'Path', Existing) then
  begin
    RegDeleteValue(PathRootKey, PathMarkerSubkey, 'PathAddedByInstaller');
    Exit;
  end;
  InstallDir := ExpandConstant('{app}');
  WasPresent := PathWasPresentBeforeInstall;
  Updated := '';
  StartAt := 1;
  FirstItem := True;
  Found := False;
  OtherItems := False;
  for I := 1 to Length(Existing) + 1 do
  begin
    if I > Length(Existing) then
      AtDelimiter := True
    else
      AtDelimiter := Existing[I] = ';';
    if AtDelimiter then
    begin
      Item := Copy(Existing, StartAt, I - StartAt);
      if CompareText(NormalizePathEntry(Item), NormalizePathEntry(InstallDir)) = 0 then
        Found := True
      else
      begin
        if not FirstItem then
          Updated := Updated + ';';
        Updated := Updated + Item;
        FirstItem := False;
        OtherItems := True;
      end;
      StartAt := I + 1;
    end;
  end;
  if not Found then
  begin
    RegDeleteValue(PathRootKey, PathMarkerSubkey, 'PathAddedByInstaller');
    Exit;
  end;
  if not OtherItems then
  begin
    if WasPresent then
      Result := RegWriteExpandStringValue(PathRootKey, PathSubkey, 'Path', '')
    else
      Result := RegDeleteValue(PathRootKey, PathSubkey, 'Path');
  end
  else
    Result := RegWriteExpandStringValue(PathRootKey, PathSubkey, 'Path', Updated);
  if Result then
  begin
    RegDeleteValue(PathRootKey, PathMarkerSubkey, 'PathAddedByInstaller');
    RegDeleteValue(PathRootKey, PathMarkerSubkey, 'PathWasPresent');
  end;
end;

procedure RunIntegration(const Target: String);
var
  ResultCode: Integer;
  Started: Boolean;
begin
  Log('Starting optional integration target ' + Target);
  ResultCode := 0;
  Started := ExecAsOriginalUser(ExpandConstant('{app}\qwen3asr.exe'),
    'agents install --target ' + Target, ExpandConstant('{app}'), SW_HIDE,
    ewWaitUntilTerminated, ResultCode);
  if not Started then
  begin
    RequestedActionsFailed := True;
    Log('[integration failure] Could not start target ' + Target + '; error code ' + IntToStr(ResultCode));
    SuppressibleMsgBox(FmtMessage(CustomMessage('IntegrationStartFailed'), [Target, ExpandConstant('{log}')]), mbError, MB_OK, IDOK);
    Exit;
  end;
  if ResultCode <> 0 then
  begin
    RequestedActionsFailed := True;
    Log('[integration failure] Target ' + Target + ' returned exit code ' + IntToStr(ResultCode));
    SuppressibleMsgBox(FmtMessage(CustomMessage('IntegrationFailed'), [Target, IntToStr(ResultCode), ExpandConstant('{log}')]), mbError, MB_OK, IDOK);
    Exit;
  end;
  if not RecordIntegrationOwnership(Target) then
  begin
    RequestedActionsFailed := True;
    Log('[integration ownership failure] Target ' + Target + ' succeeded, but its HKCU ownership marker could not be recorded for ' + ExpandConstant('{app}'));
    SuppressibleMsgBox(FmtMessage(CustomMessage('IntegrationOwnershipRecordFailed'), [Target, ExpandConstant('{log}')]), mbError, MB_OK, IDOK);
    Exit;
  end;
  Log('[integration success] Target ' + Target);
end;

procedure RunOwnedIntegrationRemoval(const Target: String);
var
  InstallDir, RecordedInstallDir, Owned: String;
  ResultCode: Integer;
  Started: Boolean;
begin
  if not RegValueExists(HKCU, IntegrationMarkerSubkey, Target) then
    Exit;

  InstallDir := NormalizePathEntry(ExpandConstant('{app}'));
  RecordedInstallDir := '';
  Owned := '';
  if not RegQueryStringValue(HKCU, IntegrationMarkerSubkey, 'InstallPath', RecordedInstallDir) or
    (CompareText(NormalizePathEntry(RecordedInstallDir), InstallDir) <> 0) or
    not RegQueryStringValue(HKCU, IntegrationMarkerSubkey, Target, Owned) or
    (Owned <> '1') then
  begin
    Log('[integration cleanup skipped] Target ' + Target + ' has no matching current-user ownership marker for ' + InstallDir + '; client settings were preserved.');
    SuppressibleMsgBox(FmtMessage(CustomMessage('UninstallIntegrationNotOwned'), [Target, ExpandConstant('{log}')]), mbError, MB_OK, IDOK);
    Exit;
  end;

  Log('[integration cleanup start] Removing target ' + Target + ' owned by ' + InstallDir);
  ResultCode := 0;
  Started := Exec(ExpandConstant('{app}\qwen3asr.exe'),
    'agents uninstall --target ' + Target, ExpandConstant('{app}'), SW_HIDE,
    ewWaitUntilTerminated, ResultCode);
  if not Started then
  begin
    Log('[integration cleanup failure] Could not start target ' + Target + '; error code ' + IntToStr(ResultCode));
    SuppressibleMsgBox(FmtMessage(CustomMessage('UninstallIntegrationStartFailed'), [Target, IntToStr(ResultCode), ExpandConstant('{log}')]), mbError, MB_OK, IDOK);
    Exit;
  end;
  if ResultCode <> 0 then
  begin
    Log('[integration cleanup failure] Target ' + Target + ' returned exit code ' + IntToStr(ResultCode) + '; ownership marker and client settings were preserved.');
    SuppressibleMsgBox(FmtMessage(CustomMessage('UninstallIntegrationFailed'), [Target, IntToStr(ResultCode), ExpandConstant('{log}')]), mbError, MB_OK, IDOK);
    Exit;
  end;

  if not ClearIntegrationOwnership(Target) then
  begin
    Log('[integration cleanup warning] Target ' + Target + ' was removed, but its ownership marker could not be cleared.');
    SuppressibleMsgBox(FmtMessage(CustomMessage('UninstallIntegrationMarkerFailed'), [Target, ExpandConstant('{log}')]), mbError, MB_OK, IDOK);
    Exit;
  end;
  Log('[integration cleanup success] Target ' + Target + ' removed.');
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and WizardIsTaskSelected('addtopath') then
  begin
    if not AddInstallDirToPath then
    begin
      RequestedActionsFailed := True;
      Log('[PATH failure] Could not add install directory to PATH.');
      SuppressibleMsgBox(CustomMessage('PathAddFailed'), mbError, MB_OK, IDOK);
    end;
  end;
  if (CurStep = ssPostInstall) and not IsAdminInstallMode and WizardIsTaskSelected('codex') then
    RunIntegration('codex');
  if (CurStep = ssPostInstall) and not IsAdminInstallMode and WizardIsTaskSelected('agy') then
    RunIntegration('agy');
  if (CurStep = ssPostInstall) and not IsAdminInstallMode and WizardIsTaskSelected('claude') then
    RunIntegration('claude');
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
  begin
    RunOwnedIntegrationRemoval('codex');
    RunOwnedIntegrationRemoval('agy');
    RunOwnedIntegrationRemoval('claude');
    if not RemoveInstallDirFromPath then
    begin
      Log('[PATH failure] Could not remove the installer-owned PATH entry.');
      SuppressibleMsgBox(CustomMessage('PathRemoveFailed'), mbError, MB_OK, IDOK);
    end;
  end;
end;

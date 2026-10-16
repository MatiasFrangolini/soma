#requires -Version 7.0
param([Parameter(Mandatory)][string]$BinaryDirectory,[Parameter(Mandatory)][string]$Workspace,[Parameter(Mandatory)][string]$Jdk)
$ErrorActionPreference='Stop'
$PSNativeCommandUseErrorActionPreference=$false
$cli=Join-Path $BinaryDirectory 'soma-cli.exe'
$data=Join-Path $Workspace 'data'
$otherData=Join-Path $Workspace 'other-data'
$main=Join-Path $Workspace 'main checkout'
$linked=Join-Path $Workspace 'linked checkout'
New-Item -ItemType Directory -Force -Path $main,$data,$otherData | Out-Null
$checks=[Collections.Generic.List[string]]::new()
function Assert([bool]$Condition,[string]$Description){if(-not $Condition){throw "FAIL: $Description"};$checks.Add($Description);Write-Output "PASS: $Description"}
function FixtureGit([string[]]$Arguments){& git -C $main @Arguments | Out-Null;if($LASTEXITCODE -ne 0){throw ('Git fixture failed: '+($Arguments -join ' '))}}
function Invoke-TargetRunner([string[]]$Arguments,[string]$HomeDirectory=$data){
    $raw=& $cli @Arguments --data-dir $HomeDirectory
    $code=$LASTEXITCODE
    try{$value=$raw | ConvertFrom-Json}catch{throw "CLI JSON invalid: $raw"}
    [pscustomobject]@{Code=$code;Value=$value}
}
function State{(Invoke-TargetRunner @('status')).Value.status}
function Wait-State([string[]]$Wanted,[int]$Seconds=25){
    $deadline=[DateTime]::UtcNow.AddSeconds($Seconds)
    do{$state=State;if($state.state -in $Wanted){return $state};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow -lt $deadline)
    throw ('State timeout: '+($state|ConvertTo-Json -Compress))
}
function Stop-Owned([string]$Operation){$result=Invoke-TargetRunner @('stop','--operation',$Operation);Assert $result.Value.ok 'Matching operation can request Stop';$null=Wait-State @('stopped','exited','failed')}
function Port{ $listener=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0);$listener.Start();try{$listener.LocalEndpoint.Port}finally{$listener.Stop()} }
$port=Port;$downPort=Port;$foreignPort=Port;$wildcardPort=Port
Copy-Item (Join-Path $PSScriptRoot 'Probe.java') (Join-Path $main 'Probe.java')
@'
param([string]$ProjectDir,[string]$Jdk)
& (Join-Path $Jdk 'bin/javac.exe') '-J-Xmx64m' '--release' '21' '-encoding' 'UTF-8' (Join-Path $ProjectDir 'Probe.java')
if($LASTEXITCODE -ne 0){exit $LASTEXITCODE}
& (Join-Path $Jdk 'bin/jar.exe') '--create' '--file' (Join-Path $ProjectDir 'probe.jar') '--main-class' 'Probe' '-C' $ProjectDir 'Probe.class'
exit $LASTEXITCODE
'@ | Set-Content (Join-Path $main 'build.ps1')
Set-Content (Join-Path $main '.gitignore') -Value @('.env','*.class','probe.jar','stopped-*.txt')
Set-Content (Join-Path $main 'marker.txt') -Value 'main-marker'
Set-Content (Join-Path $main '.env') -Value 'SOMA_MODE=main-env'
FixtureGit @('init','-b','fixture-main');FixtureGit @('config','user.name','Soma fixture');FixtureGit @('config','user.email','fixture@example.invalid')
FixtureGit @('add','.');FixtureGit @('commit','-m','Fixture main');FixtureGit @('branch','fixture-free');FixtureGit @('worktree','add','-b','fixture-feature',$linked)
Set-Content (Join-Path $linked 'marker.txt') -Value 'worktree-marker'
& git -C $linked add marker.txt | Out-Null;& git -C $linked commit -m 'Fixture feature' | Out-Null
if($LASTEXITCODE -ne 0){throw 'Feature fixture commit failed'}
Set-Content (Join-Path $linked '.env') -Value 'SOMA_MODE=worktree-env'
$configs=foreach($entry in @(@{id='http';mode='http';port=$port;delay=1200;ready=$true},@{id='wildcard';mode='http-wildcard';port=$wildcardPort;delay=0;ready=$true},@{id='down';mode='down';port=$downPort;delay=0;ready=$true},@{id='foreign';mode='idle';port=$foreignPort;delay=0;ready=$true},@{id='normal';mode='idle';port=0;delay=0;ready=$false},@{id='exit';mode='exit';port=0;delay=0;ready=$true})){
    $config=[ordered]@{id=$entry.id;name=$entry.id;env_file='.env';env=@{};jdk=$Jdk;profiles=@();config_files=@();
        build=@{executable='pwsh';args=@('-NoProfile','-File',(Join-Path $main 'build.ps1'),'-ProjectDir',$main,'-Jdk',$Jdk)};
        run=@{kind='java_jar';artifact='probe.jar';jvm_args=@('-Xmx32m');args=@($entry.mode,[string]$entry.port,[string]$entry.delay)};
        inputs=@('Probe.java','build.ps1','marker.txt');input_excludes=@()}
    if($entry.ready){$config.readiness=@{url="http://127.0.0.1:$port/health";timeout_seconds=2;json_field='status';expected='UP'};if($entry.port){$config.readiness.url="http://127.0.0.1:$($entry.port)/health"}else{$config.readiness.url="http://127.0.0.1:$port/health"};if($entry.id -eq 'http'){$config.readiness.timeout_seconds=30}}
    $config
}
$settings=@{schema_version=1;projects=@(@{id='probe';name='Target fixture';directory=$main;configs=@($configs)})}
foreach($path in @($data,$otherData)){$settings|ConvertTo-Json -Depth 15|Set-Content (Join-Path $path 'projects.json')}
$foreign=$null
try{
    Assert ((Invoke-TargetRunner @('validate')).Value.ok) 'Existing configuration shape remains valid with optional readiness'
    $trees=(Invoke-TargetRunner @('worktrees','--path',$main)).Value
    Assert (@($trees.worktrees).Count -eq 2 -and @($trees.branches).Contains('fixture-feature')) 'Git discovers both actual worktrees and selectable local branches'
    $built=Invoke-TargetRunner @('build','--path',$main,'--config','http','--wait')
    Assert ($built.Code -eq 0 -and $built.Value.status.directory -eq $main) 'Build resolves the explicit directory to its registered project'
    $mainHash=(Get-FileHash (Join-Path $main 'probe.jar')).Hash
    $stale=Invoke-TargetRunner @('run','--path',$linked,'--config','http','--reuse','--wait')
    Assert ($stale.Code -eq 4) 'A build recorded in another worktree cannot be reused'
    $started=Invoke-TargetRunner @('run','--path',$linked,'--config','http')
    $operation=$started.Value.operation_id
    $ready=Invoke-TargetRunner @('wait','--operation',$operation,'--ready','--timeout','45')
    if($ready.Code -ne 0){throw ('Readiness failed: '+($ready.Value|ConvertTo-Json -Depth 8 -Compress)+' logs: '+(Invoke-TargetRunner @('logs','--tail','100')).Value.text)}
    Assert ($ready.Code -eq 0 -and $ready.Value.status.readiness -eq 'ready' -and $ready.Value.status.state -eq 'running') 'Readiness waits for HTTP JSON UP and returns while Java remains running'
    Assert ($ready.Value.status.directory -eq $linked -and $ready.Value.status.branch -eq 'fixture-feature' -and $ready.Value.status.head -eq (& git -C $linked rev-parse HEAD)) 'Status identifies the actual checkout, branch and commit'
    $logs=(Invoke-TargetRunner @('logs','--tail','200')).Value.text
    Assert ($logs.Contains('MODE=worktree-env') -and $logs.Contains('MARKER=worktree-marker') -and $logs.Contains("CWD=$linked")) 'Build paths and repository .env are resolved against the selected worktree'
    Assert ((Get-FileHash (Join-Path $main 'probe.jar')).Hash -eq $mainHash) 'Executing a linked worktree leaves the primary artifact unchanged'
    $wrong=Invoke-TargetRunner @('stop','--operation','some-other-operation')
    Assert ($wrong.Code -eq 7 -and (State).operation_id -eq $operation -and (State).state -eq 'running') 'Wrong operation ID cannot stop an active application'
    $blocked=Invoke-TargetRunner @('switch','--path',$linked,'--branch','fixture-free')
    Assert ($blocked.Code -eq 3 -and (& git -C $linked branch --show-current) -eq 'fixture-feature') 'Branch changes are rejected while the owner is running'
    $leased=Invoke-TargetRunner @('run','--path',$linked,'--config','normal','--wait') $otherData
    Assert ($leased.Code -eq 3 -and $leased.Value.status.error -like 'checkout_busy:*') 'Another Soma owner cannot build or run the same active checkout'
    $leasedSwitch=Invoke-TargetRunner @('switch','--path',$linked,'--branch','fixture-free') $otherData
    Assert (-not $leasedSwitch.Value.ok -and $leasedSwitch.Value.error -like 'checkout_busy:*') 'Another Soma window cannot switch the branch of an active checkout'
    Stop-Owned $operation
    $switched=Invoke-TargetRunner @('switch','--path',$linked,'--branch','fixture-free')
    Assert ($switched.Value.ok -and (& git -C $linked branch --show-current) -eq 'fixture-free') 'A stopped clean worktree can switch to an available local branch'
    $occupied=Invoke-TargetRunner @('switch','--path',$linked,'--branch','fixture-main')
    Assert (-not $occupied.Value.ok -and (& git -C $linked branch --show-current) -eq 'fixture-free') 'Git refuses a branch occupied by another worktree without forcing it'
    Set-Content (Join-Path $linked 'marker.txt') -Value 'uncommitted'
    $dirty=Invoke-TargetRunner @('switch','--path',$linked,'--branch','fixture-feature')
    Assert (-not $dirty.Value.ok -and (Get-Content (Join-Path $linked 'marker.txt') -Raw).Trim() -eq 'uncommitted') 'Switch preserves dirty files and refuses to stash or discard them'
    & git -C $linked restore marker.txt
    $null=Invoke-TargetRunner @('switch','--path',$linked,'--branch','fixture-feature')
    $again=Invoke-TargetRunner @('run','--path',$linked,'--config','http','--wait-ready','--timeout','30')
    Assert ($again.Code -eq 0 -and $again.Value.status.readiness -eq 'ready') 'Run --wait-ready provides the same readiness contract'
    $newOperation=$again.Value.status.operation_id
    Assert ((Invoke-TargetRunner @('stop','--operation',$operation)).Code -eq 7 -and (State).operation_id -eq $newOperation) 'A stale ID cannot stop a later execution of the same project'
    Stop-Owned $newOperation
    $down=Invoke-TargetRunner @('run','--path',$linked,'--config','down','--wait-ready','--timeout','20')
    Assert ($down.Code -eq 9 -and $down.Value.status.state -eq 'running') 'HTTP 200 with status DOWN times out without claiming readiness or stopping Java'
    Stop-Owned $down.Value.status.operation_id
    $wildcard=Invoke-TargetRunner @('run','--path',$linked,'--config','wildcard','--wait-ready','--timeout','20')
    Assert ($wildcard.Code -eq 0 -and $wildcard.Value.status.readiness -eq 'ready') 'Readiness recognizes an owned dual-stack wildcard listener through IPv4 loopback'
    Stop-Owned $wildcard.Value.status.operation_id
    $normal=Invoke-TargetRunner @('run','--path',$linked,'--config','normal','--wait-ready','--timeout','20')
    Assert ($normal.Code -eq 8 -and $normal.Value.status.state -eq 'running') 'Readiness without a configured endpoint returns an explicit error'
    Stop-Owned $normal.Value.status.operation_id
    $exited=Invoke-TargetRunner @('run','--path',$linked,'--config','exit','--wait-ready','--timeout','20')
    Assert ($exited.Code -eq 5) 'An exit code zero before readiness is not mistaken for a ready server'
    $info=[Diagnostics.ProcessStartInfo]::new((Join-Path $Jdk 'bin/java.exe'))
    $info.UseShellExecute=$false;$info.CreateNoWindow=$true;$info.RedirectStandardOutput=$true;$info.RedirectStandardError=$true;$info.WorkingDirectory=$main
    foreach($argument in @('-Xmx32m','-cp',$main,'Probe','http',[string]$foreignPort,'0')){$info.ArgumentList.Add($argument)}
    $foreign=[Diagnostics.Process]::Start($info)
    $deadline=[DateTime]::UtcNow.AddSeconds(10)
    do{try{$health=Invoke-RestMethod "http://127.0.0.1:$foreignPort/health" -TimeoutSec 1;if($health.status -eq 'UP'){break}}catch{};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow -lt $deadline)
    Assert ($health.status -eq 'UP') 'An independent healthy server exists for the ownership negative control'
    $notOwned=Invoke-TargetRunner @('run','--path',$linked,'--config','foreign','--wait-ready','--timeout','20')
    Assert ($notOwned.Code -eq 9 -and $notOwned.Value.status.readiness -eq 'timed_out') 'A healthy listener owned by another process cannot satisfy readiness'
    Stop-Owned $notOwned.Value.status.operation_id
    $foreign.Refresh();Assert (-not $foreign.HasExited) 'Owned Stop leaves the independent HTTP server alive'
    $otherProject=Join-Path $Workspace 'unrelated folder';New-Item -ItemType Directory -Force -Path $otherProject|Out-Null
    $wrongPath=Invoke-TargetRunner @('run','--project','probe','--path',$otherProject,'--config','normal')
    Assert (-not $wrongPath.Value.ok) 'An explicit project cannot redirect its configuration into an unrelated directory'
    Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class SomaTargetUi {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr window, int id);
    [DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr window, uint message, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll",CharSet=CharSet.Unicode,EntryPoint="SendMessageW")] public static extern IntPtr ReadText(IntPtr window, uint message, IntPtr wParam, StringBuilder text);
    [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr window);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out RECT rect);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr window, out RECT rect);
    [DllImport("user32.dll")] public static extern int MapWindowPoints(IntPtr from, IntPtr to, ref RECT rect, uint points);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr window, IntPtr after, int x, int y, int width, int height, uint flags);
}
'@
    $owner=Get-CimInstance Win32_Process -Filter "Name = 'Soma.exe'" | Where-Object {$_.CommandLine.Contains('"'+$data+'"')} | Select-Object -First 1
    if(-not $owner){throw 'Fixture GUI owner not found'}
    $window=(Get-Process -Id $owner.ProcessId).MainWindowHandle
    function Select-Combo([int]$Id,[int]$Index){
        $control=[SomaTargetUi]::GetDlgItem($window,$Id)
        $null=[SomaTargetUi]::SendMessageW($control,0x14E,[IntPtr]$Index,[IntPtr]::Zero)
        $null=[SomaTargetUi]::SendMessageW($window,0x111,[IntPtr]($Id -bor (1 -shl 16)),$control)
    }
    Select-Combo 106 0
    $branchControl=[SomaTargetUi]::GetDlgItem($window,132)
    $branchCount=[SomaTargetUi]::SendMessageW($branchControl,0x146,[IntPtr]::Zero,[IntPtr]::Zero).ToInt32()
    $branchIndex=-1
    for($index=0;$index -lt $branchCount;$index++){$text=[Text.StringBuilder]::new(1024);$null=[SomaTargetUi]::ReadText($branchControl,0x148,[IntPtr]$index,$text);if($text.ToString() -eq 'fixture-free'){$branchIndex=$index}}
    Assert ($branchIndex -ge 0) 'Native branch selector lists the available branch'
    Select-Combo 132 $branchIndex
    Assert ((& git -C $main branch --show-current) -eq 'fixture-free') 'Selecting a branch in the native window performs Git switch on that checkout'
    Select-Combo 106 1
    $saved=Get-Content (Join-Path $data 'projects.json') -Raw|ConvertFrom-Json
    Assert ($saved.projects[0].selected_worktree -eq $linked) 'Native worktree selection is remembered without rewriting the base project directory'
    Select-Combo 102 0
    $null=[SomaTargetUi]::SendMessageW($window,0x111,[IntPtr]111,[IntPtr]::Zero)
    $state=Wait-State @('running','failed')
    Assert ($state.state -eq 'running' -and $state.directory -eq $linked) 'Native Run executes the selected worktree'
    $ready=Invoke-TargetRunner @('wait','--operation',$state.operation_id,'--ready','--timeout','30')
    Assert ($ready.Code -eq 0) 'Native Run uses the same readiness contract as the CLI'
    $deadline=[DateTime]::UtcNow.AddSeconds(3)
    do{$disabled=-not [SomaTargetUi]::IsWindowEnabled([SomaTargetUi]::GetDlgItem($window,106));if($disabled){break};Start-Sleep -Milliseconds 50}while([DateTime]::UtcNow -lt $deadline)
    Assert ($disabled -and -not [SomaTargetUi]::IsWindowEnabled([SomaTargetUi]::GetDlgItem($window,132))) 'Native checkout selectors are disabled while an application is active'
    Stop-Owned $state.operation_id
    foreach($width in @(780,1080,1440,1920)){
        $null=[SomaTargetUi]::SetWindowPos($window,[IntPtr]::Zero,30,30,$width,740,0x14)
        $client=[SomaTargetUi+RECT]::new();$null=[SomaTargetUi]::GetClientRect($window,[ref]$client)
        foreach($id in @(101,102,103,104,105,106,110,111,112,113,114,115,120,121,122,123,124,132)){
            $control=[SomaTargetUi]::GetDlgItem($window,$id);$rect=[SomaTargetUi+RECT]::new();$null=[SomaTargetUi]::GetWindowRect($control,[ref]$rect);$null=[SomaTargetUi]::MapWindowPoints([IntPtr]::Zero,$window,[ref]$rect,2)
            if($rect.Left -lt 0 -or $rect.Top -lt 0 -or $rect.Right -gt $client.Right -or $rect.Bottom -gt $client.Bottom){throw "Control $id overflows at $width"}
        }
    }
    Assert $true 'Native controls fit at 780, 1080, 1440 and 1920 pixels'
    $explicit=Invoke-TargetRunner @('run','--path',$main,'--config','http','--wait-ready','--timeout','30')
    Assert ($explicit.Code -eq 0 -and $explicit.Value.status.directory -eq $main) 'Explicit CLI path overrides the worktree remembered by the window'
    Stop-Owned $explicit.Value.status.operation_id
    $profile=Invoke-TargetRunner @('run','--path',$main,'--config','normal','--wait-ready','--timeout','20')
    Assert ($profile.Code -eq 8) 'CLI can start another profile in the same checkout'
    $configControl=[SomaTargetUi]::GetDlgItem($window,102)
    $deadline=[DateTime]::UtcNow.AddSeconds(3)
    do{
        $selected=[SomaTargetUi]::SendMessageW($configControl,0x147,[IntPtr]::Zero,[IntPtr]::Zero)
        $text=[Text.StringBuilder]::new(1024)
        $null=[SomaTargetUi]::ReadText($configControl,0x148,$selected,$text)
        if($text.ToString() -eq 'normal'){break}
        Start-Sleep -Milliseconds 50
    }while([DateTime]::UtcNow -lt $deadline)
    Assert ($text.ToString() -eq 'normal') 'Native profile selector follows a CLI execution even when its checkout stays the same'
    Stop-Owned $profile.Value.status.operation_id
    [ordered]@{Result='PASS';Checks=@($checks.ToArray());Main=$main;Linked=$linked;Data=$data;Jdk=$Jdk}|ConvertTo-Json -Depth 6|Set-Content (Join-Path $Workspace 'targets-verification.json')
}catch{
    $failure=$_
    try{
        [ordered]@{Error=$failure.Exception.Message;Runtime=(Invoke-TargetRunner @('status')).Value;Listeners=@(Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction SilentlyContinue|Select-Object LocalAddress,LocalPort,OwningProcess)}|ConvertTo-Json -Depth 10|Set-Content (Join-Path $Workspace 'targets-failure.json')
        (Invoke-TargetRunner @('logs','--tail','100')).Value.text|Set-Content (Join-Path $Workspace 'targets-failure.log')
    }catch{}
    throw $failure
}finally{
    $null=Invoke-TargetRunner @('close');$null=Invoke-TargetRunner @('close') $otherData
    if($null -ne $foreign){if(-not $foreign.HasExited){$foreign.Kill();$foreign.WaitForExit()};$foreign.Dispose()}
}



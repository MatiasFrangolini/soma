#requires -Version 7.0
param([Parameter(Mandatory)][string]$BinaryDirectory,[Parameter(Mandatory)][string]$Workspace)
$ErrorActionPreference='Stop'
$PSNativeCommandUseErrorActionPreference=$false
if(Test-Path -LiteralPath $Workspace){throw 'Use a fresh disposable workspace'}
$data=Join-Path $Workspace 'data'
New-Item -ItemType Directory -Path $data -Force|Out-Null
$cli=Join-Path $BinaryDirectory 'soma-cli.exe'
$checks=[Collections.Generic.List[string]]::new()
function Assert([bool]$Condition,[string]$Description){if(-not $Condition){throw ('FAIL: '+$Description)};$checks.Add($Description);Write-Output ('PASS: '+$Description)}
function Invoke-Cli([string[]]$Arguments){
    $raw=& $cli @Arguments --data-dir $data
    $code=$LASTEXITCODE
    try{$value=$raw|ConvertFrom-Json}catch{throw "Invalid CLI JSON: $raw"}
    [pscustomobject]@{Code=$code;Value=$value}
}
function Owners { @(Get-CimInstance Win32_Process -Filter "Name='Soma.exe'"|Where-Object {$_.CommandLine.Contains($data)}) }
function Wait-Closed {
    $deadline=[DateTime]::UtcNow.AddSeconds(15)
    do { if((Invoke-Cli @('status')).Value.status.state -eq 'closed' -and (Owners).Count -eq 0){return};Start-Sleep -Milliseconds 100 } while([DateTime]::UtcNow -lt $deadline)
    throw 'Temporary owner did not exit'
}
@'
param([string]$Mode)
Write-Output ('LIFECYCLE_'+$Mode)
if($Mode -eq 'held'){while(-not (Test-Path -LiteralPath (Join-Path $PSScriptRoot 'release.flag'))){Start-Sleep -Milliseconds 100}}
if($Mode -eq 'fail'){exit 23}
'@|Set-Content -LiteralPath (Join-Path $Workspace 'probe.ps1')
$configs=foreach($mode in @('held','fast','fail')){
    @{id=$mode;name=$mode;inputs=@('probe.ps1');build=@{executable='cmd.exe';args=@('/d','/c','exit','0')};run=@{kind='command';executable='pwsh';args=@('-NoProfile','-File',(Join-Path $Workspace 'probe.ps1'),$mode)}}
}
@{schema_version=1;projects=@(@{id='probe';name='Lifecycle';directory=$Workspace;configs=@($configs)})}|ConvertTo-Json -Depth 12|Set-Content -LiteralPath (Join-Path $data 'projects.json')
try {
    $started=Invoke-Cli @('run','--project','probe','--config','held')
    Assert ($started.Value.ok -and $started.Value.operation_id) 'CLI starts an application without an existing owner'
    Start-Sleep -Milliseconds 800
    $owners=Owners
    Assert ($owners.Count -eq 1) 'One owner serves this data directory'
    $ownerPid=$owners[0].ProcessId
    $process=Get-Process -Id $ownerPid
    Assert ($process.MainWindowHandle -eq 0) 'CLI execution opens no application window'
    $status=(Invoke-Cli @('status')).Value
    Assert ($status.owner.pid -eq $ownerPid -and $status.owner.temporary) 'CLI reports the temporary owner identity'
    $busy=Invoke-Cli @('run','--project','probe','--config','fast')
    Assert ($busy.Value.code -eq 'busy' -and (Owners).Count -eq 1 -and (Owners)[0].ProcessId -eq $ownerPid) 'Another command reuses the owner and preserves its active application'
    Start-Sleep -Seconds 3
    Assert ((Invoke-Cli @('status')).Value.status.operation_id -eq $started.Value.operation_id) 'Temporary owner stays alive while the application runs'
    Set-Content -LiteralPath (Join-Path $Workspace 'release.flag') -Value 'release'
    Wait-Closed
    $finished=Invoke-Cli @('wait','--operation',$started.Value.operation_id,'--timeout','1')
    Assert ($finished.Code -eq 0 -and $finished.Value.status.state -eq 'exited' -and $finished.Value.source -eq 'disk') 'Delayed wait retrieves the completed result after automatic owner exit'
    Assert ((Invoke-Cli @('logs')).Value.text.Contains('LIFECYCLE_held')) 'Logs remain readable after automatic owner exit'
    $failed=Invoke-Cli @('run','--project','probe','--config','fail','--wait')
    Assert ($failed.Code -eq 5 -and $failed.Value.status.exit_code -eq 23) 'A failed process keeps its real outcome'
    Wait-Closed
    $failedAgain=Invoke-Cli @('wait','--operation',$failed.Value.status.operation_id,'--timeout','1')
    Assert ($failedAgain.Code -eq 5 -and $failedAgain.Value.status.exit_code -eq 23) 'Delayed wait preserves failure after automatic owner exit'
    $build=Invoke-Cli @('build','--project','probe','--config','fast','--wait')
    Assert ($build.Code -eq 0) 'A finite build also starts without a GUI'
    Wait-Closed
    $unknown=Invoke-Cli @('wait','--operation','../untrusted','--timeout','1')
    Assert ($unknown.Code -ne 0 -and $unknown.Value.code -eq 'stopped') 'Offline operation lookup rejects an invalid identity'
    Remove-Item -LiteralPath (Join-Path $Workspace 'release.flag')
    $clients=@()
    try {
        foreach($index in 1..2){
            $info=[Diagnostics.ProcessStartInfo]::new($cli)
            $info.UseShellExecute=$false;$info.CreateNoWindow=$true;$info.RedirectStandardOutput=$true;$info.RedirectStandardError=$true
            foreach($argument in @('run','--project','probe','--config','held','--data-dir',$data)){$info.ArgumentList.Add($argument)}
            $clients+=,[Diagnostics.Process]::Start($info)
        }
        $results=@(foreach($client in $clients){$result=$client.StandardOutput.ReadToEnd()|ConvertFrom-Json;$client.WaitForExit();$result})
        Assert (@($results|Where-Object ok -eq $true).Count -eq 1 -and @($results|Where-Object code -eq 'busy').Count -eq 1 -and (Owners).Count -eq 1) 'Simultaneous CLI launches share one owner without replacing the accepted operation'
        Set-Content -LiteralPath (Join-Path $Workspace 'release.flag') -Value 'release'
        Wait-Closed
    } finally {foreach($client in $clients){$client.Dispose()}}
    Remove-Item -LiteralPath (Join-Path $Workspace 'release.flag')
    $held=Invoke-Cli @('run','--project','probe','--config','held')
    $promotedPid=(Owners)[0].ProcessId
    Assert ((Invoke-Cli @('show')).Value.ok) 'Explicit show opens the interface of the existing owner'
    Start-Sleep -Milliseconds 800
    Assert ((Owners).Count -eq 1 -and (Owners)[0].ProcessId -eq $promotedPid -and (Get-Process -Id $promotedPid).MainWindowHandle -ne 0) 'Explicit show reuses the process and makes its interface visible'
    Set-Content -LiteralPath (Join-Path $Workspace 'release.flag') -Value 'release'
    $null=Invoke-Cli @('wait','--operation',$held.Value.operation_id,'--timeout','10')
    Start-Sleep -Seconds 3
    Assert ((Invoke-Cli @('status')).Value.owner.temporary -eq $false) 'An explicitly opened interface remains available after completion'
    $null=Invoke-Cli @('close')
    Wait-Closed
    Assert $true 'Explicit close shuts down only this disposable owner'
    $checks|ConvertTo-Json|Set-Content -LiteralPath (Join-Path $Workspace 'lifecycle-verification.json')
} finally { $null=Invoke-Cli @('close') }

param([Parameter(Mandatory)][string]$BinaryDirectory,[Parameter(Mandatory)][string]$Workspace,[Parameter(Mandatory)][string]$Jdk)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$cli = Join-Path $BinaryDirectory 'soma-cli.exe'
$data = Join-Path $Workspace 'data'
$fixture = Join-Path $Workspace 'fixture'
New-Item -ItemType Directory -Path $fixture -Force | Out-Null
& git -C $fixture init -b soma-qa-a | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'Could not create disposable Git fixture' }
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'Probe.java') -Destination (Join-Path $fixture 'Probe.java')
Set-Content -LiteralPath (Join-Path $fixture '.env') -Value 'SOMA_MODE=from-dotenv'
$compileScript = @'
param([string]$Jdk)
$ErrorActionPreference = 'Stop'
if (Test-Path -LiteralPath (Join-Path $PSScriptRoot 'fail.flag')) { Write-Error 'Deliberate build failure' -ErrorAction Continue; exit 23 }
& (Join-Path $Jdk 'bin\javac.exe') '-J-Xmx64m' '--release' '21' '-encoding' 'UTF-8' (Join-Path $PSScriptRoot 'Probe.java')
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& (Join-Path $Jdk 'bin\jar.exe') '--create' '--file' (Join-Path $PSScriptRoot 'probe.jar') '--main-class' 'Probe' '-C' $PSScriptRoot 'Probe.class'
exit $LASTEXITCODE
'@
Set-Content -LiteralPath (Join-Path $fixture 'build.ps1') -Value $compileScript
$configs = foreach ($mode in @('normal','spam','paced','hang')) {
    [ordered]@{id=$mode;name=$mode;env_file='.env';env=if($mode -eq 'hang'){@{SOMA_MODE='explicit-override'}}else{@{}};jdk=$Jdk;profiles=@();config_files=@();build=@{executable='pwsh';args=@('-NoProfile','-File',(Join-Path $fixture 'build.ps1'),'-Jdk',$Jdk)};run=@{kind='java_jar';artifact='probe.jar';jvm_args=@('-Xmx32m');args=@($mode)};inputs=@('Probe.java','build.ps1');input_excludes=@()}
}
function Invoke-Runner([string[]]$Arguments) {
    $raw = & $cli @Arguments --data-dir $data
    $code = $LASTEXITCODE
    try { $result = $raw | ConvertFrom-Json } catch { throw ('Invalid JSON from CLI: ' + ($raw -join ' ')) }
    return [pscustomobject]@{Code=$code;Value=$result}
}
$script:CheckCount=0
function Assert([bool]$Condition,[string]$Description) { if (-not $Condition) { throw ('FAIL: ' + $Description) }; $script:CheckCount++; Write-Output ('PASS: ' + $Description) }
function State { return (Invoke-Runner @('status')).Value.status }
function Wait-State([string[]]$Wanted,[int]$Seconds=40) {
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    do { $value = State; if ($value.state -in $Wanted) { return $value }; Start-Sleep -Milliseconds 100 } while ([DateTime]::UtcNow -lt $deadline)
    throw ('Timed out waiting for ' + ($Wanted -join ',') + ': ' + ($value | ConvertTo-Json -Compress))
}
function Wait-Log([string]$Needle,[int]$Seconds=30) {
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    do { $text=(Invoke-Runner @('logs','--tail','20000')).Value.text; if ($text.Contains($Needle)) { return $text }; Start-Sleep -Milliseconds 100 } while ([DateTime]::UtcNow -lt $deadline)
    throw ('Missing log: ' + $Needle)
}
$null=Invoke-Runner @('projects')
@{schema_version=1;projects=@(@{id='probe';name='Java QA';directory=$fixture;configs=@($configs)})} | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $data 'projects.json')
$report=[ordered]@{}
$unrelated=$null
try {
    Assert ((Invoke-Runner @('validate')).Value.ok) 'Configuration validates without disclosing values'
    $show=Invoke-Runner @('show'); Assert $show.Value.ok 'First launch creates window and working IPC'
    Start-Sleep -Milliseconds 700
    $gui=Get-Process -Name Soma | Where-Object { $_.Path -eq (Join-Path $BinaryDirectory 'Soma.exe') } | Select-Object -First 1
    $report.IdleWorkingSetMiB=[Math]::Round($gui.WorkingSet64/1MB,2);$report.IdlePrivateMiB=[Math]::Round($gui.PrivateMemorySize64/1MB,2);$report.GuiPid=$gui.Id
    Assert ($gui.WorkingSet64 -lt 50MB) 'Idle wrapper working set is below 50 MiB'
    $reuse=Invoke-Runner @('run','--project','probe','--config','normal','--reuse');Assert $reuse.Value.ok 'Run returns an operation ID'
    $s=Wait-State @('failed');Assert ($s.error -like 'build_stale:*') 'Unknown old build is rejected'
    $null=Invoke-Runner @('run','--project','probe','--config','normal');$text=Wait-Log 'CHILD_PID=';
    $s=Wait-State @('running');Assert ($text.Contains('MODE=from-dotenv')) '.env overrides inherited environment'
    Assert ($text.Contains('UNICODE=áéíóú ñ ✓')) 'Unicode output survives Java and pipes'
    $parentIds=@([regex]::Matches($text,'PARENT_PID=(\d+)') | ForEach-Object { [int]$_.Groups[1].Value })
    $childId=[int]([regex]::Match($text,'CHILD_PID=(\d+)').Groups[1].Value)
    Assert ((Invoke-Runner @('run','--project','probe','--config','normal')).Code -eq 3) 'Second Run returns busy without replacing active application'
    $null=Invoke-Runner @('stop');$s=Wait-State @('stopped');
    Assert (-not $s.forced) 'Java stops through Ctrl+C without forced termination'
    Assert (Test-Path -LiteralPath (Join-Path $fixture 'stopped-normal.txt')) 'Java shutdown hook actually ran'
    Assert ($null -eq (Get-Process -Id $childId -ErrorAction SilentlyContinue)) 'Owned child process is gone'
    $unrelatedInfo=[System.Diagnostics.ProcessStartInfo]::new((Join-Path $Jdk 'bin\java.exe'))
    $unrelatedInfo.UseShellExecute=$false;$unrelatedInfo.CreateNoWindow=$true;$unrelatedInfo.RedirectStandardOutput=$true;$unrelatedInfo.RedirectStandardError=$true;$unrelatedInfo.WorkingDirectory=$fixture
    foreach ($argument in @('-Xmx32m','-cp',$fixture,'Probe','unrelated')) { $unrelatedInfo.ArgumentList.Add($argument) }
    $unrelated=[System.Diagnostics.Process]::Start($unrelatedInfo)
    $null=Invoke-Runner @('run','--project','probe','--config','normal','--reuse');$null=Wait-State @('running');$null=Invoke-Runner @('stop');$null=Wait-State @('stopped');
    Assert $true 'Successful build can be reused and stopped again'
    Add-Content -LiteralPath (Join-Path $fixture 'Probe.java') -Value '// input changed'
    $null=Invoke-Runner @('run','--project','probe','--config','normal','--reuse');$s=Wait-State @('failed');Assert ($s.error -like 'build_stale:*') 'Source changes invalidate cached build'
    $completedBuild=Invoke-Runner @('build','--project','probe','--config','normal','--wait');Assert ($completedBuild.Code -eq 0 -and $completedBuild.Value.status.state -eq 'exited') 'Agent can wait for actual build completion with a successful exit code'
    & git -C $fixture symbolic-ref HEAD refs/heads/soma-qa-b
    if ($LASTEXITCODE -ne 0) { throw 'Could not change disposable Git branch' }
    $stale=Invoke-Runner @('run','--project','probe','--config','normal','--reuse','--wait');Assert ($stale.Code -eq 4 -and $stale.Value.code -eq 'build_stale') 'Branch changes invalidate a build and return CLI code 4 with --wait'
    Set-Content -LiteralPath (Join-Path $fixture 'fail.flag') -Value 'fail'
    $null=Invoke-Runner @('run','--project','probe','--config','normal');$s=Wait-State @('failed');Assert ($s.exit_code -eq 23) 'Failed build returns its exit code and does not launch old JAR'
    $null=Invoke-Runner @('run','--project','probe','--config','normal','--reuse');$s=Wait-State @('failed');Assert ($s.error -like 'build_stale:*') 'Failed compilation invalidates the previous stamp'
    Remove-Item -LiteralPath (Join-Path $fixture 'fail.flag')
    $null=Invoke-Runner @('run','--project','probe','--config','spam');$s=Wait-State @('exited','failed') 60;Assert ($s.state -eq 'exited') 'Large output process completed without pipe deadlock'
    $logs=(Invoke-Runner @('logs','--tail','20000')).Value
    Assert ($logs.visible_bytes -le 2MB) 'Visible log buffer remains at most 2 MiB'
    $disk=0L
    foreach($logFile in (Get-ChildItem -LiteralPath (Join-Path $data 'logs') -Filter '*.log')){
        $logHandle=[System.IO.File]::Open($logFile.FullName,[System.IO.FileMode]::Open,[System.IO.FileAccess]::Read,([System.IO.FileShare]::ReadWrite -bor [System.IO.FileShare]::Delete))
        try{$disk+=$logHandle.Length}finally{$logHandle.Dispose()}
    }
    Assert ($disk -le 30MB) 'Rotating disk logs remain at most 30 MiB'
    Assert ($logs.text.Contains('SPAM_COMPLETE')) 'Tail still contains the newest output after rotation'
    Assert (-not $logs.text.Contains('�')) 'UTF-8 characters spanning pipe reads are preserved'
    $gui.Refresh();$report.AfterFirstLogBurstWorkingSetMiB=[Math]::Round($gui.WorkingSet64/1MB,2);$report.AfterFirstLogBurstPrivateMiB=[Math]::Round($gui.PrivateMemorySize64/1MB,2);$report.LogBytesOnDisk=$disk;$report.LogBurstPayloadBytes=54006000
    Start-Sleep -Milliseconds 700;$gui.Refresh();$firstLogPrivate=$gui.PrivateMemorySize64
    $null=Invoke-Runner @('run','--project','probe','--config','spam','--reuse');$s=Wait-State @('exited','failed') 60
    Assert ($s.state -eq 'exited') 'Repeated large output reuses the build and completes'
    Start-Sleep -Milliseconds 700;$gui.Refresh();$report.AfterSecondLogBurstWorkingSetMiB=[Math]::Round($gui.WorkingSet64/1MB,2);$report.AfterSecondLogBurstPrivateMiB=[Math]::Round($gui.PrivateMemorySize64/1MB,2)
    Assert (($gui.PrivateMemorySize64-$firstLogPrivate) -lt 20MB) 'Second large-output cycle does not retain another entire log stream'
    $null=Invoke-Runner @('run','--project','probe','--config','paced');$s=Wait-State @('exited','failed') 100
    $pacedLogs=(Invoke-Runner @('logs','--tail','20000')).Value
    Assert ($s.state -eq 'exited' -and $pacedLogs.text.Contains('PACED_COMPLETE')) 'Continuous paced output keeps native GUI and IPC alive through log updates'
    $null=Invoke-Runner @('run','--project','probe','--config','hang');$text=Wait-Log 'MODE=explicit-override';Assert ($text.Contains('MODE=explicit-override')) 'Explicit configuration overrides .env'
    $null=Invoke-Runner @('stop');$s=Wait-State @('stopped') 25;Assert $s.forced 'Hung shutdown is forcibly completed after the grace period'
    Assert (Test-Path -LiteralPath (Join-Path $fixture 'stopped-hang.txt')) 'Forced path first attempted a real Java shutdown hook'
    $unrelated.Refresh();Assert (-not $unrelated.HasExited) 'Forced Stop leaves an unrelated process alive'
    $null=Invoke-Runner @('run','--project','probe','--config','normal');$null=Wait-State @('running')
    $deadline=[DateTime]::UtcNow.AddSeconds(20)
    do {$latest=(Invoke-Runner @('logs','--tail','20000')).Value.text;$offset=$latest.LastIndexOf('[Soma] Java QA / normal · run');if($offset -ge 0 -and $latest.Substring($offset).Contains('CHILD_PID=')){break};Start-Sleep -Milliseconds 100} while([DateTime]::UtcNow -lt $deadline)
    Assert ($offset -ge 0 -and $latest.Substring($offset).Contains('CHILD_PID=')) 'Final close scenario has a live Java parent and child'
    $closingChild=[int]([regex]::Match($latest.Substring($offset),'CHILD_PID=(\d+)').Groups[1].Value)
    $null=Invoke-Runner @('close');
    $deadline=[DateTime]::UtcNow.AddSeconds(25);do {Start-Sleep -Milliseconds 100;$s=State} while ($s.state -ne 'closed' -and [DateTime]::UtcNow -lt $deadline)
    Assert ($s.state -eq 'closed') 'Closing the GUI also closes its active execution'
    Assert ($null -eq (Get-Process -Id $closingChild -ErrorAction SilentlyContinue)) 'GUI close leaves no owned child process'
    Assert ((Invoke-Runner @('logs')).Value.source -eq 'disk') 'Agent can read logs after the window closes'
    $report.Result='PASS';$report.Checks=$script:CheckCount
    $report | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $Workspace 'verification.json')
    Write-Output ($report | ConvertTo-Json)
} finally { $null=Invoke-Runner @('close');if($null -ne $unrelated -and -not $unrelated.HasExited){$unrelated.Kill();$unrelated.WaitForExit()};if($null -ne $unrelated){$unrelated.Dispose()} }

#requires -Version 7.0
param([Parameter(Mandatory)][string]$BinaryDirectory,[Parameter(Mandatory)][string]$Workspace,[switch]$Verify,[switch]$LegacyWorkflow)
$ErrorActionPreference='Stop'
$PSNativeCommandUseErrorActionPreference=$false
if(Test-Path -LiteralPath $Workspace){throw 'Use a new workspace for each variant'}
$cli=Join-Path $BinaryDirectory 'soma-cli.exe'
$env:SOMA_HOME=Join-Path $Workspace 'data'
New-Item -ItemType Directory $env:SOMA_HOME -Force|Out-Null
$projects=foreach($name in 'a','b','c'){
    $dir=Join-Path $Workspace $name
    New-Item -ItemType Directory $dir|Out-Null
    New-Item -ItemType Directory "$dir/inputs"|Out-Null
    Set-Content "$dir/pom.xml" '<project><modelVersion>4.0.0</modelVersion><groupId>soma</groupId><artifactId>bench</artifactId><version>1</version></project>'
    Set-Content "$dir/mvnw.cmd" @'
@echo off
if exist fail.flag exit /b 1
if not exist hold.flag exit /b 0
:wait
ping -n 2 127.0.0.1 >nul
if exist hold.flag goto wait
exit /b 0
'@
    $runtime=@{id='observer-probe';name='Observer probe';inputs=@('pom.xml');run=@{kind='command';executable='ping.exe';args=@('-n','60','127.0.0.1')}}
    @{id=$name;name=$name;directory=$dir;configs=@($runtime);test_configs=@(@{id='tests';name='tests';env=@{};inputs=@('pom.xml','mvnw.cmd','inputs')})}
}
@{schema_version=1;job_concurrency=2;projects=@($projects)}|ConvertTo-Json -Depth 9|Set-Content "$env:SOMA_HOME/projects.json"
function Call([string[]]$Argv){ (& $cli @Argv)|ConvertFrom-Json }
function Submit([string]$Name,[string[]]$Goals=@('compile')){
    Call (@('jobs','submit','--path',(Join-Path $Workspace $Name),'--kind','build','--')+$Goals)
}
function Wait-Result($Job){Call @('jobs','wait','--job',$Job.job_id,'--timeout','45','--summary')}
function Measure-Flow([bool]$Fail){
    $flag=Join-Path $Workspace 'c/fail.flag'
    if($Fail){Set-Content $flag 'fail'}
    $timer=[Diagnostics.Stopwatch]::StartNew()
    $submit=& $cli jobs submit --path (Join-Path $Workspace c) --kind build -- compile
    $id=($submit|ConvertFrom-Json).job_id
    if(!$id){throw 'No job ID for workflow measurement'}
    $output=@($submit)
    if($LegacyWorkflow){
        $output+=(& $cli jobs status --job $id)
        $output+=(& $cli jobs logs --job $id --tail 200)
        $final=& $cli jobs wait --job $id --timeout 45
    }else{$final=& $cli jobs wait --job $id --timeout 45 --summary}
    $output+=@($final)
    $wall=$timer.ElapsedMilliseconds
    if([bool](($final|ConvertFrom-Json).ok) -eq $Fail){throw 'Workflow verdict mismatch'}
    if($Fail){Remove-Item -LiteralPath $flag}
    @{wall_ms=$wall;cli_calls=if($LegacyWorkflow){4}else{2};stdout_bytes=[Text.Encoding]::UTF8.GetByteCount(($output -join "`n"))}
}
function Ms([scriptblock]$Action){$sw=[Diagnostics.Stopwatch]::StartNew();$null=& $Action;$sw.Elapsed.TotalMilliseconds}
$receipt=[ordered]@{}
$locks=@()
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class SomaBenchIO {
    [StructLayout(LayoutKind.Sequential)]
    public struct Counters { public ulong Reads, Writes, Other, ReadBytes, WriteBytes, OtherBytes; }
    [DllImport("kernel32.dll", SetLastError=true)]
    public static extern bool GetProcessIoCounters(IntPtr handle, out Counters counters);
}
'@
try {
    $receipt.cold_submit_ms=Ms {$script:first=Submit c}
    if(!(Wait-Result $first).ok){throw 'Warmup build failed'}
    $receipt.flow_success=Measure-Flow $false
    $receipt.flow_failure=Measure-Flow $true
    foreach($name in 'a','b'){
        $dir=(Join-Path $Workspace $name).ToLowerInvariant()
        $hash=[Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($dir))).ToLowerInvariant()
        $mutex=[Threading.Mutex]::new($false,"Local\SomaCheckout-$hash")
        if(!$mutex.WaitOne(0)){throw 'Fixture lock unexpectedly occupied'}
        $locks+=@($mutex)
    }
    $blocked=@((Submit a),(Submit b))
    $sw=[Diagnostics.Stopwatch]::StartNew()
    $free=Submit c
    $check=Call @('jobs','wait','--job',$free.job_id,'--timeout','2','--summary')
    $receipt.free_checkout_before_unlock=$check.verdict -eq 'built'
    $receipt.free_observed_ms=$sw.ElapsedMilliseconds
    # Count distinct published receipt timestamps after both checkouts are blocked.
    $paths=@($blocked|ForEach-Object {Join-Path $env:SOMA_HOME "jobs/$($_.job_id)/result.json"})
    $stamps=@{}
    foreach($path in $paths){$stamps[$path]=[IO.File]::GetLastWriteTimeUtc($path)}
    $rewrites=0
    $sw.Restart()
    while($sw.ElapsedMilliseconds -lt 1500){
        foreach($path in $paths){
            $stamp=[IO.File]::GetLastWriteTimeUtc($path)
            if($stamp -ne $stamps[$path]){$rewrites++;$stamps[$path]=$stamp}
        }
        Start-Sleep -Milliseconds 20
    }
    $receipt.blocked_rewrites_observed=$rewrites
    foreach($mutex in $locks){$mutex.ReleaseMutex();$mutex.Dispose()};$locks=@()
    foreach($job in @($blocked)+@($free)){if(!(Wait-Result $job).ok){throw 'Checkout failed after release'}}
    foreach($observers in 1,10){
        $flag=Join-Path $Workspace 'c/hold.flag'
        Set-Content $flag 'hold'
        $job=Submit c
        $same=Submit c
        if($same.job_id -ne $job.job_id){throw 'Requests did not join'}
        $released=Call @('jobs','release','--job',$same.job_id,'--participant',$same.participant_id)
        if(!$released.ok -or $released.participants -ne 1){throw 'Shared execution lost its remaining participant'}
        $runtime=Call @('run','--path',"$Workspace/a",'--config','observer-probe')
        if(!$runtime.ok -or !$runtime.operation_id){throw 'Disposable runtime failed to start'}
        $clients=foreach($i in 1..$observers){
            $info=[Diagnostics.ProcessStartInfo]::new($cli)
            $info.UseShellExecute=$false;$info.CreateNoWindow=$true;$info.RedirectStandardOutput=$true
            foreach($arg in @('jobs','wait','--job',$job.job_id,'--timeout','45','--summary')){$info.ArgumentList.Add($arg)}
            $client=[Diagnostics.Process]::Start($info)
            [pscustomobject]@{process=$client;output=$client.StandardOutput.ReadToEndAsync()}
        }
        $timeout=Call @('jobs','wait','--job',$job.job_id,'--timeout','1','--summary')
        if($timeout.code -ne 'timeout'){throw 'Timed wait lost pending outcome'}
        $latencies=@(1..10|ForEach-Object {Ms {Call @('jobs','status','--job',$job.job_id,'--summary')}})
        $stopMs=Ms {$script:stopped=Call @('stop','--operation',$runtime.operation_id)}
        if(!$stopped.ok){throw 'Stop request failed while clients were waiting'}
        $sw.Restart()
        Remove-Item -LiteralPath $flag
        $bytes=0;$cpu=0;$writes=0;$readBytes=0
        foreach($client in $clients){
            if(!$client.process.WaitForExit(45000)){throw 'Observer did not finish'}
            $raw=$client.output.Result
            if(($raw|ConvertFrom-Json).verdict -ne 'built'){throw 'Inconsistent observer verdict'}
            $bytes+=[Text.Encoding]::UTF8.GetByteCount($raw)
            $cpu+=$client.process.TotalProcessorTime.TotalMilliseconds
            $io=[SomaBenchIO+Counters]::new()
            if(![SomaBenchIO]::GetProcessIoCounters($client.process.Handle,[ref]$io)){throw 'Missing waiter I/O accounting'}
            $writes+=$io.Writes
            $readBytes+=$io.ReadBytes
            $client.process.Dispose()
        }
        $receipt["waiters_$observers"]=@{status_ms=$latencies;release_to_last_verdict_ms=$sw.ElapsedMilliseconds;output_bytes=$bytes;client_cpu_ms=$cpu;client_write_operations=$writes;client_read_bytes=$readBytes}
        $receipt["waiters_$observers"].stop_ack_ms=$stopMs
        $null=Call @('wait','--operation',$runtime.operation_id,'--timeout','10')
    }
    Set-Content "$Workspace/c/fail.flag" 'fail'
    $failed=Submit c
    if((Wait-Result $failed).verdict -ne 'failed'){throw 'Failed build reported success'}
    $receipt.split_property_ms=Ms {$script:split=Submit c @('-Dmaven','.test.skip=true','package')}
    $receipt.split_created_job=[bool]$split.job_id
    if($split.job_id){$null=Wait-Result $split}
    $receipt.split_code=$split.code
    $null=Call @('close')
    $deadline=[DateTime]::UtcNow.AddSeconds(20)
    while((Call @('status')).owner.state -ne 'absent'){if([DateTime]::UtcNow -gt $deadline){throw 'Owner did not retire'};Start-Sleep -Milliseconds 100}
    $receipt.offline_id_ms=@(1..10|ForEach-Object {Ms {Call @('jobs','status','--job',$first.job_id,'--summary')}})
    $template=Get-Content "$env:SOMA_HOME/jobs/$($first.job_id)/result.json" -Raw|ConvertFrom-Json
    $reference=[Datetime]::UtcNow.AddDays(-1)
    $archive=New-Item -ItemType Directory "$env:SOMA_HOME/job-history" -Force
    foreach($i in 1..717){
        $template.job_id='job-arch'+$i
        $template|ConvertTo-Json -Compress -Depth 15|Set-Content "$($archive.FullName)/$($template.job_id).json"
    }
    foreach($i in 1..90){
        $template.job_id='job-live'+$i
        $dir=New-Item -ItemType Directory "$env:SOMA_HOME/jobs/$($template.job_id)" -Force
        $path=Join-Path $dir.FullName result.json
        $template|ConvertTo-Json -Compress -Depth 15|Set-Content $path
        [IO.File]::SetLastWriteTimeUtc($path,$reference)
    }
    foreach($i in 1..368){
        $dir=New-Item -ItemType Directory "$env:SOMA_HOME/jobs/job-residue$i/logs" -Force
        '{"stream":"old","base":0,"operations":[]}'|Set-Content "$($dir.FullName)/observations.json"
    }
    $receipt.populated_submit_ms=Ms {$script:open=Submit a}
    if(!(Wait-Result $open).ok){throw 'Recovery benchmark failed'}
    $receipt.history_rewrites=@(Get-ChildItem "$env:SOMA_HOME/jobs/job-live*/result.json"|Where-Object LastWriteTimeUtc -gt $reference.AddMinutes(1)).Count
    $jobsDirectory=Join-Path $env:SOMA_HOME jobs
    $receipt.residue_directories=[IO.Directory]::GetDirectories($jobsDirectory,'job-residue*').Length
    $receipt.filtered_list_ms=@(1..10|ForEach-Object {Ms {Call @('jobs','list','--path',"$Workspace/c",'--state','completed','--limit','1','--summary')}})
    if($Verify){
        $deadline=[DateTime]::UtcNow.AddSeconds(5)
        while([IO.Directory]::GetDirectories($jobsDirectory,'job-residue*').Length -and [DateTime]::UtcNow -lt $deadline){Start-Sleep -Milliseconds 100}
        $receipt.residues_after_idle=[IO.Directory]::GetDirectories($jobsDirectory,'job-residue*').Length
    }
    $receipt|ConvertTo-Json -Depth 8|Set-Content "$Workspace/measurement.json"
    $receipt|ConvertTo-Json -Depth 8
    if($Verify -and (!$receipt.free_checkout_before_unlock -or $rewrites -gt 0 -or $receipt.history_rewrites -gt 0 -or $receipt.residues_after_idle -gt 0 -or $receipt.split_created_job -or $receipt.split_code -ne 'split_maven_property')){
        throw 'Checkout scheduling, history recovery or malformed request contract failed'
    }
    if($Verify){
        $dir=Join-Path $Workspace c
        Remove-Item -LiteralPath "$dir/fail.flag"
        $inputs=Join-Path $dir inputs
        Set-Content "$inputs/original" 'one'
        foreach($mutation in 'add','rename','delete','same-size-and-time'){
            $hold=Join-Path $dir hold.flag
            Set-Content $hold 'hold'
            $active=Submit c
            $deadline=[DateTime]::UtcNow.AddSeconds(20)
            do {
                $state=Call @('jobs','status','--job',$active.job_id,'--summary')
                if($state.state -eq 'running'){break}
                if([DateTime]::UtcNow -ge $deadline){throw 'Fixture never reached running state'}
                Start-Sleep -Milliseconds 50
            }while($true)
            switch($mutation){
                'add' {Set-Content "$inputs/added" 'two'}
                'rename' {Rename-Item -LiteralPath "$inputs/added" -NewName renamed}
                'delete' {Remove-Item -LiteralPath "$inputs/renamed"}
                'same-size-and-time' {
                    $at=[IO.File]::GetLastWriteTimeUtc("$inputs/original")
                    Set-Content "$inputs/original" 'two'
                    [IO.File]::SetLastWriteTimeUtc("$inputs/original",$at)
                }
            }
            Remove-Item -LiteralPath $hold
            if((Wait-Result $active).verdict -ne 'invalidated'){throw "Input change escaped verification: $mutation"}
        }
    }
}finally{
    foreach($mutex in $locks){$mutex.ReleaseMutex();$mutex.Dispose()}
    $null=Call @('close')
}

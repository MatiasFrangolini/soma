#requires -Version 7.0
param([Parameter(Mandatory)][string]$BinaryDirectory,[Parameter(Mandatory)][string]$Workspace,[Parameter(Mandatory)][string]$Jdk,[string]$MavenWrapper)
$ErrorActionPreference='Stop'
$PSNativeCommandUseErrorActionPreference=$false
$cli=Join-Path $BinaryDirectory 'soma-cli.exe'
$data=Join-Path $Workspace 'data'
$env:SOMA_HOME=$data
$fixture=Join-Path $Workspace 'maven fixture'
New-Item -ItemType Directory -Path $data,(Join-Path $fixture 'src/test/java') -Force|Out-Null
$checks=[Collections.Generic.List[string]]::new()
function Assert([bool]$Condition,[string]$Description){if(-not $Condition){throw ('FAIL: '+$Description)};$checks.Add($Description);Write-Output ('PASS: '+$Description)}
function Invoke-JobCli([string[]]$Arguments){
    $raw=& $cli @Arguments
    $code=$LASTEXITCODE
    try{$value=$raw|ConvertFrom-Json}catch{throw "Invalid CLI JSON: $raw"}
    [pscustomobject]@{Code=$code;Value=$value}
}
function Submit([string]$Mode='pass',[string]$Kind='test',[string[]]$Extra=@()){
    $maven=@('-B','-ntp','-T1','-DforkCount=1','-DreuseForks=true',"-Dprobe.mode=$Mode")+$Extra
    if($Kind -eq 'test'){$maven+=@('test')}else{$maven+=@('package','-DskipTests')}
    (Invoke-JobCli (@('jobs','submit','--path',$fixture,'--config','tests','--kind',$Kind,'--')+$maven)).Value
}
function Job([string]$Id){(Invoke-JobCli @('jobs','status','--job',$Id)).Value}
function Wait-Job([string]$Id){Invoke-JobCli @('jobs','wait','--job',$Id,'--timeout','180')}
function Wait-Started([string]$Id){
    $deadline=[DateTime]::UtcNow.AddSeconds(90)
    do{if(Test-Path -LiteralPath (Join-Path $fixture 'started.txt')){return};$s=Job $Id;if($s.job.state -in @('failed','invalidated','cancelled')){throw ($s|ConvertTo-Json -Depth 12)};Start-Sleep -Milliseconds 150}while([DateTime]::UtcNow -lt $deadline)
    throw 'JUnit fixture did not start'
}
function Clear-Started{Remove-Item -LiteralPath (Join-Path $fixture 'started.txt') -ErrorAction SilentlyContinue}
@'
<project xmlns="http://maven.apache.org/POM/4.0.0"><modelVersion>4.0.0</modelVersion>
<groupId>dev.soma.fixture</groupId><artifactId>jobs</artifactId><version>1</version>
<properties><maven.compiler.release>21</maven.compiler.release><project.build.sourceEncoding>UTF-8</project.build.sourceEncoding></properties>
<dependencies><dependency><groupId>org.junit.jupiter</groupId><artifactId>junit-jupiter</artifactId><version>5.12.2</version><scope>test</scope></dependency></dependencies>
<build><plugins><plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-compiler-plugin</artifactId><version>3.14.0</version></plugin>
<plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-surefire-plugin</artifactId><version>3.5.3</version></plugin></plugins></build></project>
'@|Set-Content -LiteralPath (Join-Path $fixture 'pom.xml')
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'JobProbeTest.java') -Destination (Join-Path $fixture 'src/test/java/JobProbeTest.java')
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'Probe.java') -Destination (Join-Path $fixture 'Probe.java')
Set-Content -LiteralPath (Join-Path $fixture '.env') -Value 'SOMA_MODE=runtime-only'
Set-Content -LiteralPath (Join-Path $fixture 'input.txt') -Value 'initial'
if($MavenWrapper){
    Copy-Item -LiteralPath $MavenWrapper -Destination (Join-Path $fixture 'mvnw.cmd')
    $wrapperHome=Join-Path $fixture '.mvn/wrapper';New-Item -ItemType Directory -Path $wrapperHome -Force|Out-Null
    Copy-Item -LiteralPath (Join-Path (Split-Path $MavenWrapper -Parent) '.mvn/wrapper/maven-wrapper.properties') -Destination $wrapperHome
}
& (Join-Path $Jdk 'bin/javac.exe') '-J-Xmx64m' '--release' '21' '-encoding' 'UTF-8' (Join-Path $fixture 'Probe.java')
if($LASTEXITCODE -ne 0){throw 'Fixture compilation failed'}
& (Join-Path $Jdk 'bin/jar.exe') '--create' '--file' (Join-Path $fixture 'probe.jar') '--main-class' 'Probe' '-C' $fixture 'Probe.class'
if($LASTEXITCODE -ne 0){throw 'Fixture packaging failed'}
$settings=@{schema_version=1;projects=@(@{id='probe';name='Jobs fixture';directory=$fixture;
    configs=@(@{id='runtime';name='runtime';jdk=$Jdk;env_file='.env';env=@{};inputs=@('Probe.java');
        build=@{executable='cmd.exe';args=@('/d','/c','exit','0')};
        run=@{kind='java_jar';snapshot=$true;artifact='probe.jar';jvm_args=@('-Xmx32m');args=@('idle')}});
    test_configs=@(@{id='tests';name='Independent tests';jdk=$Jdk;env_file=$null;env=@{SOMA_TEST_MODE='test-only'};inputs=@('pom.xml','src','input.txt')})})}
$settings|ConvertTo-Json -Depth 15|Set-Content -LiteralPath (Join-Path $data 'projects.json')
try{
    Assert ((Invoke-JobCli @('validate')).Value.ok) 'Separate runtime and test configurations validate'
    $first=Submit 'sleep' 'test' @('--define=private.canary=soma-private-argument-fixture')
    Assert ($first.ok -and $first.job_id -and $first.participant_id) 'Submit returns a job and a participant'
    $owner=Get-CimInstance Win32_Process -Filter "Name='Soma.exe'"|Where-Object CommandLine -like "*$data*"|Select-Object -First 1
    Assert ($null -ne $owner -and (Get-Process -Id $owner.ProcessId).MainWindowHandle -eq 0) 'Maven submit starts a temporary owner without a window'
    Wait-Started $first.job_id
    $same=Submit 'sleep' 'test' @('--define=private.canary=soma-private-argument-fixture')
    Assert ($same.ok -and $same.deduplicated -and $same.job_id -eq $first.job_id -and $same.participant_id -ne $first.participant_id) 'Two agents share one active Maven execution with independent participation'
    $queued=Submit 'pass'
    Assert ((Job $queued.job_id).job.state -eq 'queued') 'Another Maven request queues behind the active job'
    $released=Invoke-JobCli @('jobs','release','--job',$queued.job_id,'--participant',$queued.participant_id)
    Assert ($released.Value.ok -and (Job $queued.job_id).job.state -eq 'cancelled') 'Releasing the final queued participant cancels before Maven starts'
    $released=Invoke-JobCli @('jobs','release','--job',$first.job_id,'--participant',$first.participant_id)
    Assert ($released.Value.participants -eq 1) 'Releasing one participant preserves another agent execution'
    $done=Wait-Job $same.job_id
    Assert ($done.Code -eq 0 -and $done.Value.job.evidence.tests -eq 3 -and $done.Value.job.evidence.executed -eq 2 -and $done.Value.job.evidence.skipped -eq 1) 'Real Surefire evidence reports executed and explicitly skipped tests'
    Assert (@(Get-Content -LiteralPath (Join-Path $fixture 'invocations.txt')).Count -eq 1) 'Joined and cancelled requests did not execute JUnit again'
    $timing=$done.Value.job.timings
    Assert (-not ($done.Value.job|ConvertTo-Json -Depth 12 -Compress).Contains('soma-private-argument-fixture')) 'Long-form Maven property values stay out of requested and effective JSON metadata'
    Assert ($null -ne $timing.resource_wait_ms -and $timing.maven_ms -gt 12000 -and $timing.total_ms -ge $timing.maven_ms) 'JSON separates measured preparation, queue, adapter waiting and Maven wall time'
    $deadline=[DateTime]::UtcNow.AddSeconds(15)
    do { $closed=(Invoke-JobCli @('status')).Value.status.state -eq 'closed';if($closed){break};Start-Sleep -Milliseconds 100 } while([DateTime]::UtcNow -lt $deadline)
    Assert ($closed -and -not (Get-Process -Id $owner.ProcessId -ErrorAction SilentlyContinue)) 'Temporary Maven owner exits after the queue drains even while status is polled'
    Assert ((Wait-Job $same.job_id).Code -eq 0) 'Maven wait retains its evidence after automatic owner exit'
    $null=Invoke-JobCli @('show')
    $owner=Get-CimInstance Win32_Process -Filter "Name='Soma.exe'"|Where-Object CommandLine -like "*$data*"|Select-Object -First 1
    $again=Submit 'pass';$againDone=Wait-Job $again.job_id
    Assert ($again.job_id -ne $same.job_id -and $againDone.Code -eq 0 -and @(Get-Content -LiteralPath (Join-Path $fixture 'invocations.txt')).Count -eq 2) 'A finished green result is not automatically reused'
    $failed=Submit 'fail';$failedDone=Wait-Job $failed.job_id
    $failureDetail=@($failedDone.Value.job.evidence.details|Where-Object outcome -eq 'failure')
    Assert ($failedDone.Code -eq 5 -and $failedDone.Value.job.evidence.failures -eq 1 -and $failureDetail[0].message.Contains('Deliberate assertion failure')) 'Assertion failure produces nonzero JSON outcome and bounded failure evidence'
    $empty=Submit 'pass' 'test' @('-Dtest=MissingTest','-Dsurefire.failIfNoSpecifiedTests=false');$emptyDone=Wait-Job $empty.job_id
    Assert ($emptyDone.Code -eq 11 -and $emptyDone.Value.job.exit_code -eq 0 -and $emptyDone.Value.job.evidence.status -eq 'no_tests') 'Zero tests cannot reuse previous XML or produce a green validation'
    $mutated=Submit 'mutate';$mutatedDone=Wait-Job $mutated.job_id
    Assert ($mutatedDone.Code -eq 10 -and $mutatedDone.Value.job.state -eq 'invalidated') 'Changes during a passing test invalidate its result'
    $build=Submit 'pass' 'build';$buildDone=Wait-Job $build.job_id
    Assert ($buildDone.Code -eq 0 -and $buildDone.Value.job.evidence.status -eq 'not_requested') 'A build can succeed without claiming test validation'
    $outside=(Invoke-JobCli @('jobs','submit','--path',$fixture,'--config','tests','--','-f','../other/pom.xml','test')).Value
    Assert ((Job $outside.job_id).job.error -like '*-f/--file*') 'Maven cannot redirect outside the checkout lease'
    $run=Invoke-JobCli @('run','--path',$fixture,'--config','runtime')
    $deadline=[DateTime]::UtcNow.AddSeconds(20)
    do{$runtime=(Invoke-JobCli @('status')).Value.status;if($runtime.state -eq 'running'){break};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow -lt $deadline)
    Assert ($runtime.state -eq 'running') 'Self-contained JAR starts from an immutable runtime copy'
    Remove-Item -LiteralPath (Join-Path $fixture 'probe.jar')
    $coexist=Submit 'pass';$coexistDone=Wait-Job $coexist.job_id
    Assert ($coexistDone.Code -eq 0 -and (Invoke-JobCli @('status')).Value.status.operation_id -eq $run.Value.operation_id -and (Invoke-JobCli @('status')).Value.status.state -eq 'running') 'Maven tests coexist with the original backend after its source JAR is removed'
    $null=Invoke-JobCli @('stop','--operation',$run.Value.operation_id)
    $spam=Submit 'spam';$spamDone=Wait-Job $spam.job_id
    $bytes=(Get-ChildItem -LiteralPath (Join-Path $data "jobs/$($spam.job_id)/logs") -File|Measure-Object Length -Sum).Sum
    Assert ($spamDone.Code -eq 0 -and $bytes -le 3MB -and (Invoke-JobCli @('jobs','logs','--job',$spam.job_id)).Value.text.Contains('BUILD SUCCESS')) 'Large job output rotates within three MiB and remains readable from disk'
    $loadedIdle=(Get-Process -Id $owner.ProcessId).WorkingSet64
    Assert ($loadedIdle -lt 50MB) 'Native owner stays below 50 MiB after retaining results and rotating large job logs'
    Clear-Started
    $cancel=Submit 'hang';Wait-Started $cancel.job_id
    $cancelPid=[int](Get-Content -LiteralPath (Join-Path $fixture 'started.txt'))
    $null=Invoke-JobCli @('jobs','release','--job',$cancel.job_id,'--participant',$cancel.participant_id)
    $cancelDone=Wait-Job $cancel.job_id
    Assert ($cancelDone.Code -eq 130 -and -not (Get-Process -Id $cancelPid -ErrorAction SilentlyContinue)) 'Releasing the last active participant stops only its owned test process tree'
    Clear-Started
    $hang=Submit 'hang';Wait-Started $hang.job_id
    $testPid=[int](Get-Content -LiteralPath (Join-Path $fixture 'started.txt'))
    $queuedClose=Submit 'pass'
    $null=Invoke-JobCli @('close')
    $deadline=[DateTime]::UtcNow.AddSeconds(25)
    do{$closed=(Invoke-JobCli @('status')).Value.status.state -eq 'closed';if($closed){break};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow -lt $deadline)
    Assert ($closed -and -not (Get-Process -Id $testPid -ErrorAction SilentlyContinue)) 'Closing the window drains and stops its owned Maven test process tree'
    Assert ((Job $hang.job_id).job.state -eq 'cancelled' -and (Job $queuedClose.job_id).job.state -eq 'cancelled') 'Cancelled active and queued results remain readable without reopening Soma'
    Assert (-not (Get-ChildItem -LiteralPath (Join-Path $data 'jobs') -Filter request.json -Recurse)) 'Transient private Maven requests are removed after completion'
    $null=Invoke-JobCli @('show')
    $newOwner=Get-CimInstance Win32_Process -Filter "Name='Soma.exe'"|Where-Object CommandLine -like "*$data*"|Select-Object -First 1
    $idle=(Get-Process -Id $newOwner.ProcessId).WorkingSet64
    Assert ($idle -lt 50MB) 'Native owner remains below the initial 50 MiB idle target after jobs'
    Clear-Started
    $crash=Submit 'hang';Wait-Started $crash.job_id
    $crashPid=[int](Get-Content -LiteralPath (Join-Path $fixture 'started.txt'))
    $crashQueued=Submit 'pass'
    # This is the disposable owner created by this test, never a pre-existing app.
    $ownedProcess=Get-Process -Id $newOwner.ProcessId
    $ownedProcess.Kill();$ownedProcess.WaitForExit();$ownedProcess.Dispose()
    $deadline=[DateTime]::UtcNow.AddSeconds(10)
    do{if(-not (Get-Process -Id $crashPid -ErrorAction SilentlyContinue)){break};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow -lt $deadline)
    Assert (-not (Get-Process -Id $crashPid -ErrorAction SilentlyContinue) -and (Job $crash.job_id).owner_unavailable) 'Unexpected owner exit releases its process tree and offline status refuses to invent completion'
    $null=Invoke-JobCli @('show')
    Assert ((Job $crash.job_id).job.state -eq 'interrupted' -and (Job $crashQueued.job_id).job.state -eq 'interrupted') 'Reopening records interrupted active and queued jobs without reexecuting them'
    for($i=0;$i -lt 105;$i++){$null=Invoke-JobCli @('jobs','submit','--path',$fixture,'--config','tests','--kind','invalid')}
    $retained=(Invoke-JobCli @('jobs','list')).Value.jobs
    $allLogs=@(Get-ChildItem -LiteralPath (Join-Path $data 'jobs') -Filter '*.log' -File -Recurse)
    Assert (@($retained).Count -eq 100 -and ($allLogs|Measure-Object Length -Sum).Sum -le 30MB) 'Completed history prunes to 100 jobs and the total job log budget stays bounded'
    [ordered]@{Result='PASS';Checks=@($checks.ToArray());IdleWorkingSetMiB=[Math]::Round($loadedIdle/1MB,2);ReopenedWorkingSetMiB=[Math]::Round($idle/1MB,2);SampleTimings=$timing;Data=$data}|ConvertTo-Json -Depth 10|Set-Content -LiteralPath (Join-Path $Workspace 'jobs-verification.json')
}finally{$null=Invoke-JobCli @('close')}

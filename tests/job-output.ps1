#requires -Version 7.0
param([Parameter(Mandatory)][string]$BinaryDirectory,[Parameter(Mandatory)][string]$Workspace,[Parameter(Mandatory)][string]$Jdk)
$ErrorActionPreference='Stop'
$PSNativeCommandUseErrorActionPreference=$false
if(Test-Path -LiteralPath $Workspace){throw 'Use a fresh workspace for persisted-job checks'}
$cli=Join-Path $BinaryDirectory 'soma-cli.exe'
$data=Join-Path $Workspace 'data'
$fixture=Join-Path $Workspace 'fixture space'
$env:SOMA_HOME=$data
New-Item -ItemType Directory -Path (Join-Path $data 'jobs'),$fixture,(Join-Path $fixture '.mvn/wrapper') -Force | Out-Null
$checks=[Collections.Generic.List[string]]::new()
function Assert([bool]$Condition,[string]$Description){if(-not $Condition){throw "FAIL: $Description"};$checks.Add($Description);Write-Output "PASS: $Description"}
function Invoke-Cli([string[]]$Arguments){
    $raw=& $cli @Arguments
    $code=$LASTEXITCODE
    [pscustomobject]@{Code=$code;Value=($raw|ConvertFrom-Json)}
}
Set-Content (Join-Path $fixture 'pom.xml') '<project><modelVersion>4.0.0</modelVersion><groupId>soma</groupId><artifactId>output</artifactId><version>1</version></project>'
Set-Content (Join-Path $fixture 'mvnw.cmd') '@exit /b 90'
Set-Content (Join-Path $fixture '.mvn/wrapper/maven-wrapper.properties') 'distributionUrl=https://example.invalid/maven.zip'
$adapter=Join-Path $Workspace 'adapter.ps1'
Set-Content $adapter "throw 'The preflight must never execute this adapter'"
$settings=@{schema_version=1;job_concurrency=2;job_adapter=@{executable='pwsh.exe';args=@('-NoProfile','-File',$adapter)};
    projects=@(@{id='fixture';name='Keep this project';directory=$fixture;configs=@();test_configs=@(
        @{id='tests';name='Source tests';jdk=$Jdk;env=@{SOMA_CANARY='secret-not-for-cli-output'};inputs=@('pom.xml')},
        @{id='other';name='Keep this test profile';jdk=$Jdk;env=@{};inputs=@('pom.xml')})})}
$settings|ConvertTo-Json -Depth 12|Set-Content (Join-Path $data 'projects.json')
$stamp=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()-10000
function Record([string]$Id,[string]$State='completed',[string]$Evidence='passed',[object]$ExitCode=0,[string]$Kind='test',[long]$Created=$stamp){
    # Original 0.7.0 storage shape: new optional instrumentation is deliberately absent.
    @{job_id=$Id;canonical_job_id=$null;state=$State;kind=$Kind;project='fixture';config='tests';directory=$fixture;
      branch='fixture-branch';head='fixture-head';fingerprint='fixture-digest';adapter_digest='fixture-adapter';
      requested_maven_args=@('-pl','module','-Dtest=LegacyContractTest','test');effective_maven_args=$null;
      created_at_ms=$Created;finished_at_ms=if($State -eq 'running'){$null}else{$Created+1};exit_code=$ExitCode;forced=$false;error=$null;
      timings=@{preparation_ms=1;soma_queue_ms=2;resource_wait_ms=3;adapter_and_maven_ms=14;maven_ms=11;evidence_ms=1;total_ms=18};
      evidence=@{status=$Evidence;reports=1;tests=2;executed=2;failures=0;errors=0;skipped=0;test_reported_ms=7;details=@();diagnostics=@();details_truncated=$false}}
}
function Save-Record([hashtable]$Record){
    $directory=Join-Path $data "jobs/$($Record.job_id)"
    New-Item -ItemType Directory -Path $directory -Force|Out-Null
    $Record|ConvertTo-Json -Depth 12 -Compress|Set-Content (Join-Path $directory 'result.json')
}
$cases=@(
    @{State='completed';Evidence='passed';Exit=0;Kind='test';Code=0;Verdict='passed'},
    @{State='completed';Evidence='not_requested';Exit=0;Kind='build';Code=0;Verdict='built'},
    @{State='failed';Evidence='passed';Exit=1;Kind='test';Code=5;Verdict='failed'},
    @{State='completed';Evidence='failed';Exit=0;Kind='test';Code=5;Verdict='failed'},
    @{State='invalidated';Evidence='passed';Exit=0;Kind='test';Code=10;Verdict='invalidated'},
    @{State='completed';Evidence='no_tests';Exit=0;Kind='test';Code=11;Verdict='incomplete'},
    @{State='completed';Evidence='not_requested';Exit=0;Kind='test';Code=11;Verdict='incomplete'},
    @{State='interrupted';Evidence='';Exit=$null;Kind='test';Code=12;Verdict='interrupted'},
    @{State='cancelled';Evidence='';Exit=130;Kind='test';Code=130;Verdict='cancelled'},
    @{State='completed';Evidence='passed';Exit=$null;Kind='test';Code=11;Verdict='incomplete'},
    @{State='completed';Evidence='passed_with_skips';Exit=0;Kind='test';Code=0;Verdict='passed_with_skips'},
    @{State='running';Evidence='';Exit=$null;Kind='test';Code=12;Verdict='interrupted'}
)
$index=0
foreach($case in $cases){
    $id='job-'+($index++).ToString('x8')
    $record=Record $id $case.State $case.Evidence $case.Exit $case.Kind ($stamp+$index)
    if($case.Evidence -eq 'passed_with_skips' -or $case.Evidence -eq 'failed'){
        $record.evidence.tests=3;$record.evidence.skipped=1
        $record.evidence.details=@(@{class='Skipped';test='omitted';outcome='skipped';message='deliberately omitted'})
        if($case.Evidence -eq 'failed'){
            $record.evidence.failures=1
            $record.evidence.details+=@{class='RealFailure';test='fails';outcome='failure';message='real-failure-canary'}
        }
    }
    Save-Record $record
    $full=Invoke-Cli @('jobs','wait','--job',$id,'--timeout','1')
    $short=Invoke-Cli @('jobs','wait','--job',$id,'--timeout','1','--summary')
    Assert ($full.Code -eq $case.Code -and $short.Code -eq $case.Code -and $short.Value.verdict -eq $case.Verdict -and -not $short.Value.PSObject.Properties['job']) "Full and compact persisted result agree: $($case.State)/$($case.Evidence)/$($case.Exit)/$($case.Kind)"
    if($case.Evidence -eq 'failed'){Assert ($short.Value.first_failure.class -eq 'RealFailure') 'A skipped detail cannot hide the first actual failure'}
    if($case.Evidence -eq 'passed_with_skips'){Assert ($null -eq $short.Value.first_failure) 'Passing with skips does not present an omitted test as a failure'}
}
$lookup=Invoke-Cli @('jobs','status','--job','job-00000002','--summary')
Assert ($lookup.Code -eq 0 -and $lookup.Value.ok -and $lookup.Value.verdict -eq 'failed') 'Lookup success remains distinct from failed validation in compact output'
$unreachable=Invoke-Cli @('jobs','status','--job','job-0000000b','--summary')
Assert ($unreachable.Value.verdict -eq 'unknown' -and $unreachable.Value.historical) 'Offline running snapshot cannot claim current progress'
$largeResult=Record 'job-large-result'
$largeResult.requested_maven_args=@(1..12|ForEach-Object {'x'*4000})
$largeResult.effective_maven_args=@(1..14|ForEach-Object {'x'*4000})
$largeResult.evidence.details=@(1..20|ForEach-Object {@{class='Failure';test="test$_";outcome='failure';message=('f'*2048)}})
$largeResult.state='failed';$largeResult.exit_code=1;$largeResult.evidence.status='failed'
Save-Record $largeResult
$largeLookup=Invoke-Cli @('jobs','status','--job','job-large-result','--summary')
Assert ($largeLookup.Value.verdict -eq 'failed' -and $largeLookup.Value.first_failure.message.Length -eq 240) 'Maximum requested/effective arguments and bounded failure details remain readable as one compact result'
$filtered=Invoke-Cli @('jobs','list','--summary','--path',$fixture,'--state','failed,invalidated','--limit','1')
Assert ($filtered.Value.jobs.Count -eq 1 -and $filtered.Value.jobs[0].state -eq 'invalidated' -and $filtered.Value.matched -eq 3 -and $filtered.Value.has_more) 'List filters checkout and states, orders newest first and reports truncation'
$none=Invoke-Cli @('jobs','list','--path',(Join-Path $Workspace 'another checkout'),'--summary')
Assert ($none.Value.jobs.Count -eq 0) 'Historical path filtering does not substitute a different checkout'
$doctor=Invoke-Cli @('jobs','doctor','--path',$fixture)
Assert ($doctor.Code -eq 0 -and ($doctor.Value|ConvertTo-Json -Depth 12) -notmatch 'secret-not-for-cli-output') 'Preflight checks selected files and adapter without printing environment values or running Maven'
$sourceBefore=Get-FileHash (Join-Path $data 'projects.json')
$isolated=Join-Path $Workspace 'isolated data'
$clone=Invoke-Cli @('jobs','config-clone','--path',$fixture,'--from','tests','--to','isolated','--to-data-dir',$isolated)
$target=Get-Content (Join-Path $isolated 'projects.json') -Raw|ConvertFrom-Json
Assert ($clone.Value.ok -and $clone.Value.isolated_store -and $target.job_concurrency -eq 2 -and $target.job_adapter.args[2] -eq $adapter -and $target.projects[0].test_configs[0].id -eq 'isolated' -and (Get-FileHash (Join-Path $data 'projects.json')).Hash -eq $sourceBefore.Hash) 'Isolated profile clone preserves resource adapter and concurrency while leaving source settings unchanged'
$clone=Invoke-Cli @('jobs','config-clone','--path',$fixture,'--from','tests','--to','local-copy')
$beforeDuplicate=(Get-FileHash (Join-Path $data 'projects.json')).Hash
$duplicate=Invoke-Cli @('jobs','config-clone','--path',$fixture,'--from','tests','--to','local-copy')
Assert ($clone.Value.ok -and $duplicate.Code -ne 0 -and (Get-FileHash (Join-Path $data 'projects.json')).Hash -eq $beforeDuplicate) 'Clone refuses an existing destination without changing other configuration'
$badEnv=Join-Path $fixture 'invalid.env'
Set-Content $badEnv 'PASSWORD="secret-canary-with-unclosed-quote'
$bad=Invoke-Cli @('jobs','config-clone','--path',$fixture,'--from','tests','--to','broken','--env-file',$badEnv)
Assert ($bad.Code -ne 0 -and ($bad.Value|ConvertTo-Json) -notmatch 'secret-canary' -and (Get-FileHash (Join-Path $data 'projects.json')).Hash -eq $beforeDuplicate) 'Malformed environment cannot partially write a profile or expose its contents'
$processes=@()
foreach($name in @('concurrent-a','concurrent-b')){
    $processes+=Start-Process -FilePath $cli -ArgumentList @('jobs','config-clone','--path',('"'+$fixture+'"'),'--from','tests','--to',$name) -WindowStyle Hidden -RedirectStandardOutput (Join-Path $Workspace "$name.json") -PassThru
}
foreach($process in $processes){$process.WaitForExit();if($process.ExitCode -ne 0){throw 'Concurrent profile creation failed'};$process.Dispose()}
$after=Get-Content (Join-Path $data 'projects.json') -Raw|ConvertFrom-Json
Assert (@($after.projects[0].test_configs|Where-Object id -in @('concurrent-a','concurrent-b')).Count -eq 2 -and $after.projects[0].name -eq 'Keep this project' -and @($after.projects[0].test_configs|Where-Object id -eq 'other')[0].name -eq 'Keep this test profile') 'Concurrent profile transactions preserve both additions and unrelated settings'
Assert ((Invoke-Cli @('status')).Value.owner.state -eq 'absent') 'Result queries, preflight and profile cloning never start an owner'
$configurationPath=Join-Path $data 'projects.json'
$validConfiguration=[IO.File]::ReadAllBytes($configurationPath)
try{
    $invalid=Get-Content $configurationPath -Raw|ConvertFrom-Json -AsHashtable
    $invalid.projects[0].test_configs[0].env='json-secret-canary'
    $invalid|ConvertTo-Json -Depth 12|Set-Content $configurationPath
    $badDoctor=Invoke-Cli @('jobs','doctor','--path',$fixture)
    $badClone=Invoke-Cli @('jobs','config-clone','--path',$fixture,'--from','tests','--to','invalid-json')
    Assert ($badDoctor.Code -ne 0 -and $badClone.Code -ne 0 -and ($badDoctor.Value|ConvertTo-Json) -notmatch 'json-secret-canary' -and ($badClone.Value|ConvertTo-Json) -notmatch 'json-secret-canary') 'Configuration type errors identify their location without echoing environment contents'
}finally{[IO.File]::WriteAllBytes($configurationPath,$validConfiguration)}

# Absolute in-project paths remain portable between real Git worktrees.
& git -C $fixture init -q
Set-Content (Join-Path $fixture 'profile.env') 'SOMA_CHECKOUT_MARKER=base'
& git -C $fixture add pom.xml profile.env
& git -C $fixture -c user.name='Soma fixture' -c user.email='fixture@example.invalid' commit -qm fixture
if($LASTEXITCODE -ne 0){throw 'Could not create disposable Git fixture'}
$linked=Join-Path $Workspace 'linked checkout'
& git -C $fixture worktree add --detach -q $linked HEAD
if($LASTEXITCODE -ne 0){throw 'Could not create disposable linked checkout'}
Set-Content (Join-Path $linked 'profile.env') 'SOMA_CHECKOUT_MARKER=linked'
$settings=Get-Content (Join-Path $data 'projects.json') -Raw|ConvertFrom-Json -AsHashtable
$settings.projects[0].test_configs[0].env_file=Join-Path $fixture 'profile.env'
$settings.projects[0].test_configs[0].inputs=@((Join-Path $fixture 'pom.xml'))
$settings|ConvertTo-Json -Depth 12|Set-Content (Join-Path $data 'projects.json')
$clone=Invoke-Cli @('jobs','config-clone','--path',$linked,'--from','tests','--to','worktree-copy')
$settings=Get-Content (Join-Path $data 'projects.json') -Raw|ConvertFrom-Json
$copied=@($settings.projects[0].test_configs|Where-Object id -eq 'worktree-copy')[0]
$baseDoctor=Invoke-Cli @('jobs','doctor','--path',$fixture,'--config','worktree-copy')
$linkedDoctor=Invoke-Cli @('jobs','doctor','--path',$linked,'--config','worktree-copy')
Assert ($clone.Value.ok -and $copied.inputs[0] -eq (Join-Path $fixture 'pom.xml') -and $baseDoctor.Value.env_file -eq (Join-Path $fixture 'profile.env') -and $linkedDoctor.Value.env_file -eq (Join-Path $linked 'profile.env')) 'A shared clone from a linked worktree rebinds inputs and environment to each selected checkout'

# Exercise metadata retirement through a real temporary owner, without invoking Maven.
for($n=20;$n -lt 125;$n++){Save-Record (Record ('job-'+$n.ToString('x8')) 'completed' 'passed' 0 'test' ($stamp+$n))}
try{
    $submitted=Invoke-Cli @('jobs','submit','--path',$fixture,'--kind','invalid','--','test')
    $null=Invoke-Cli @('jobs','wait','--job',$submitted.Value.job_id,'--timeout','30')
    $archived=Invoke-Cli @('jobs','wait','--job','job-00000000','--summary')
    Assert ($archived.Code -eq 0 -and $archived.Value.archived -and $archived.Value.verdict -eq 'passed' -and -not $archived.Value.details_retained) 'Retiring detailed history keeps the original verdict, checkout and timing metadata queryable'
    $logs=Invoke-Cli @('jobs','logs','--job','job-00000000')
    Assert ($logs.Value.code -eq 'job_logs_unavailable') 'Expired job logs are reported explicitly while metadata remains readable'
    $history=Invoke-Cli @('jobs','list','--history','--summary','--limit','1000')
    Assert ($history.Value.jobs.Count -gt 100 -and @($history.Value.jobs|Where-Object archived).Count -gt 0) 'History list and JSON export include compact archive without reopening old executions'
    $null=Invoke-Cli @('close')
    function Wait-Closed {
        $deadline=[DateTime]::UtcNow.AddSeconds(15)
        do{if((Invoke-Cli @('status')).Value.owner.state -eq 'absent'){return};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow -lt $deadline)
        throw 'Owned temporary fixture owner did not close'
    }
    Wait-Closed
    $archive=Join-Path $data 'job-history'
    # Transient archive read failures must not become destructive cleanup.
    $lockedId='job-locked-metadata'
    $lockedPath=Join-Path $archive "$lockedId.json"
    Record $lockedId|ConvertTo-Json -Depth 12 -Compress|Set-Content $lockedPath
    $lockedHash=(Get-FileHash $lockedPath).Hash
    $handle=[IO.File]::Open($lockedPath,[IO.FileMode]::Open,[IO.FileAccess]::Write,[IO.FileShare]::Delete)
    try{
        $blocked=Invoke-Cli @('jobs','submit','--path',$fixture,'--kind','invalid','--','test')
        if($blocked.Value.job_id){$null=Invoke-Cli @('jobs','wait','--job',$blocked.Value.job_id,'--timeout','30')}
        $incompleteHistory=Invoke-Cli @('jobs','list','--history','--summary')
    }finally{$handle.Dispose()}
    Assert ((Test-Path $lockedPath) -and (Get-FileHash $lockedPath).Hash -eq $lockedHash) 'A sharing violation during archive inspection preserves valid metadata byte for byte'
    Assert ($blocked.Value.code -eq 'history_unavailable') 'Storage inspection failure is communicated before accepting another job'
    Assert ($incompleteHistory.Value.code -eq 'history_unavailable') 'History export does not silently omit an unreadable metadata record'
    $recovered=Invoke-Cli @('jobs','submit','--path',$fixture,'--kind','invalid','--','test')
    if($recovered.Value.job_id){$null=Invoke-Cli @('jobs','wait','--job',$recovered.Value.job_id,'--timeout','30')}
    Assert ($recovered.Value.ok -and $recovered.Value.job_id) 'Releasing an owned storage lock restores admission without changing permissions'
    $null=Invoke-Cli @('close');Wait-Closed
    # Archive count and age are independent from storage error recovery.
    for($n=0;$n -lt 1030;$n++){
        $id='job-archive'+$n.ToString('x8')
        Record $id 'completed' 'passed' 0 'test' ($stamp-20000+$n)|ConvertTo-Json -Depth 12 -Compress|Set-Content (Join-Path $archive "$id.json")
    }
    $expiredId='job-expired'
    Record $expiredId 'completed' 'passed' 0 'test' ($stamp-31L*24*60*60*1000)|ConvertTo-Json -Depth 12 -Compress|Set-Content (Join-Path $archive "$expiredId.json")
    $submitted=Invoke-Cli @('jobs','submit','--path',$fixture,'--kind','invalid','--','test')
    $null=Invoke-Cli @('jobs','wait','--job',$submitted.Value.job_id,'--timeout','30')
    Assert (@(Get-ChildItem $archive -Filter '*.json' -File).Count -le 1000 -and -not (Test-Path (Join-Path $archive "$expiredId.json"))) 'Archive independently enforces record count and age without retaining expired metadata'
    $null=Invoke-Cli @('close');Wait-Closed
    $oldest=Get-ChildItem (Join-Path $data 'jobs') -Filter 'result.json' -File -Recurse|ForEach-Object {Get-Content $_.FullName -Raw|ConvertFrom-Json}|Sort-Object created_at_ms|Select-Object -First 1
    $protected=@(Get-ChildItem $archive -Filter '*.json' -File)
    $handles=[Collections.Generic.List[IO.FileStream]]::new()
    try{
        foreach($file in $protected){$handles.Add([IO.File]::Open($file.FullName,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::ReadWrite))}
        $accepted=Invoke-Cli @('jobs','submit','--path',$fixture,'--kind','invalid','--','test')
        if($accepted.Value.job_id){$null=Invoke-Cli @('jobs','wait','--job',$accepted.Value.job_id,'--timeout','30')}
        $blocked=Invoke-Cli @('jobs','submit','--path',$fixture,'--kind','invalid','--','test')
        [ordered]@{Oldest=$oldest.job_id;DetailedRetained=(Test-Path (Join-Path $data "jobs/$($oldest.job_id)/result.json"));Accepted=$accepted.Value;Blocked=$blocked.Value;ProtectedCount=$handles.Count}|ConvertTo-Json -Depth 8|Set-Content (Join-Path $Workspace 'eviction-diagnostic.json')
        Assert ((Test-Path (Join-Path $data "jobs/$($oldest.job_id)/result.json")) -and $blocked.Value.code -eq 'history_unavailable') 'Failed archive eviction retains detailed evidence and blocks growth instead of dropping the newly archived result'
    }finally{
        foreach($handle in $handles){$handle.Dispose()}
    }
    $recovered=Invoke-Cli @('jobs','submit','--path',$fixture,'--kind','invalid','--','test')
    if($recovered.Value.job_id){$null=Invoke-Cli @('jobs','wait','--job',$recovered.Value.job_id,'--timeout','30')}
    Assert ($recovered.Value.ok -and (Invoke-Cli @('jobs','status','--job',$oldest.job_id)).Value.archived) 'After storage recovery the preserved detailed result moves into bounded metadata normally'
    $null=Invoke-Cli @('close');Wait-Closed
    for($n=0;$n -lt 350;$n++){
        $id='job-large'+$n.ToString('x8')
        $large=Record $id 'completed' 'passed' 0 'test' ($stamp-10000+$n)
        $large.requested_maven_args=@(('x'*48000))
        $large|ConvertTo-Json -Depth 12 -Compress|Set-Content (Join-Path $archive "$id.json")
    }
    $submitted=Invoke-Cli @('jobs','submit','--path',$fixture,'--kind','invalid','--','test')
    $null=Invoke-Cli @('jobs','wait','--job',$submitted.Value.job_id,'--timeout','30')
    $archiveBytes=(Get-ChildItem $archive -Filter '*.json' -File|Measure-Object Length -Sum).Sum
    Assert ($archiveBytes -le 16MB) 'Archive byte budget remains bounded independently from rotating logs'
}finally{$null=Invoke-Cli @('close')}
[ordered]@{Result='PASS';Checks=@($checks.ToArray())}|ConvertTo-Json -Depth 10|Set-Content (Join-Path $Workspace 'job-output-verification.json')

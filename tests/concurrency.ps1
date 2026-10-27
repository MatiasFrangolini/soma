#requires -Version 7.0
param([Parameter(Mandatory)][string]$BinaryDirectory,[Parameter(Mandatory)][string]$Workspace,[Parameter(Mandatory)][string]$Jdk)
$ErrorActionPreference='Stop'
$PSNativeCommandUseErrorActionPreference=$false
$cli=Join-Path $BinaryDirectory 'soma-cli.exe'
$data=Join-Path $Workspace 'data'
$env:SOMA_HOME=$data
New-Item -ItemType Directory -Path $data -Force|Out-Null
$projects=@()
foreach($name in @('a','b')){
    $fixture=Join-Path $Workspace $name
    New-Item -ItemType Directory -Path (Join-Path $fixture 'src/test/java') -Force|Out-Null
    @'
<project xmlns="http://maven.apache.org/POM/4.0.0"><modelVersion>4.0.0</modelVersion>
<groupId>dev.soma.fixture</groupId><artifactId>concurrency</artifactId><version>1</version>
<properties><maven.compiler.release>21</maven.compiler.release></properties>
<dependencies><dependency><groupId>org.junit.jupiter</groupId><artifactId>junit-jupiter</artifactId><version>5.12.2</version><scope>test</scope></dependency></dependencies>
<build><plugins><plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-compiler-plugin</artifactId><version>3.14.0</version></plugin>
<plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-surefire-plugin</artifactId><version>3.5.3</version></plugin></plugins></build></project>
'@|Set-Content -LiteralPath (Join-Path $fixture 'pom.xml')
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'JobProbeTest.java') -Destination (Join-Path $fixture 'src/test/java/JobProbeTest.java')
    $projects+=@{id=$name;name=$name;directory=$fixture;configs=@();test_configs=@(@{id='tests';name='tests';jdk=$Jdk;env=@{SOMA_TEST_MODE='test-only'};inputs=@('pom.xml','src')})}
}
@{schema_version=1;job_concurrency=2;projects=$projects}|ConvertTo-Json -Depth 12|Set-Content -LiteralPath (Join-Path $data 'projects.json')
function Invoke-Cli([string[]]$Arguments){
    $raw=& $cli @Arguments
    try{$raw|ConvertFrom-Json}catch{throw "Invalid CLI JSON: $raw"}
}
function Assert([bool]$Condition,[string]$Description){if(-not $Condition){throw "FAIL: $Description"};Write-Output "PASS: $Description"}
function Submit([string]$Name,[string]$Mode){
    Invoke-Cli @('jobs','submit','--path',(Join-Path $Workspace $Name),'--config','tests','--kind','test','--','-B','-ntp','-T1','-DforkCount=1','-DreuseForks=true',"-Dprobe.mode=$Mode",'test')
}
function Wait-Started([string]$Name,[string]$Id){
    $deadline=[DateTime]::UtcNow.AddSeconds(150)
    do{
        if(Test-Path -LiteralPath (Join-Path $Workspace "$Name/started.txt")){return}
        $job=Invoke-Cli @('jobs','status','--job',$Id)
        if($job.job.state -in @('failed','invalidated','cancelled')){throw ($job|ConvertTo-Json -Depth 12)}
        Start-Sleep -Milliseconds 200
    }while([DateTime]::UtcNow -lt $deadline)
    throw "Fixture $Name did not start"
}
try{
    $a=Submit 'a' 'hang'; Assert ([bool]$a.ok) 'First checkout accepted'; Wait-Started 'a' $a.job_id
    $same=Submit 'a' 'pass'; Assert ([bool]$same.ok) 'Second selection on same checkout accepted'
    $b=Submit 'b' 'hang'; Assert ([bool]$b.ok) 'Different checkout accepted'; Wait-Started 'b' $b.job_id
    $aStatus=Invoke-Cli @('jobs','status','--job',$a.job_id)
    $sameStatus=Invoke-Cli @('jobs','status','--job',$same.job_id)
    $bStatus=Invoke-Cli @('jobs','status','--job',$b.job_id)
    Assert ($aStatus.job.state -eq 'running' -and $bStatus.job.state -eq 'running') 'Different checkouts run concurrently'
    Assert ($sameStatus.job.state -eq 'queued' -and $sameStatus.wait_reason -eq 'checkout_in_use') 'Same checkout queues without blocking another checkout'
    $list=Invoke-Cli @('jobs','list'); Assert ($list.concurrency -eq 2) 'Owner reports configured concurrency'
    $null=Invoke-Cli @('jobs','release','--job',$a.job_id,'--participant',$a.participant_id)
    $null=Invoke-Cli @('jobs','wait','--job',$a.job_id,'--timeout','120')
    $bStatus=Invoke-Cli @('jobs','status','--job',$b.job_id)
    Assert ($bStatus.job.state -eq 'running') 'Cancelling one checkout preserves the other'
    $sameDone=Invoke-Cli @('jobs','wait','--job',$same.job_id,'--timeout','150')
    Assert ($sameDone.ok -and $sameDone.job.evidence.executed -gt 0) 'Queued checkout resumes with its own evidence'
    $null=Invoke-Cli @('jobs','release','--job',$b.job_id,'--participant',$b.participant_id)
    $null=Invoke-Cli @('jobs','wait','--job',$b.job_id,'--timeout','120')
}finally{$null=Invoke-Cli @('close')}

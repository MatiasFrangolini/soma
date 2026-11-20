#requires -Version 7.0
param([Parameter(Mandatory)][string]$BinaryDirectory,[Parameter(Mandatory)][string]$Workspace)
$ErrorActionPreference='Stop'
$PSNativeCommandUseErrorActionPreference=$false
if(Test-Path -LiteralPath $Workspace){throw 'Expected a fresh fingerprint fixture'}
$cli=Join-Path $BinaryDirectory 'soma-cli.exe'
$toolDir=New-Item -ItemType Directory (Join-Path $Workspace 'tools') -Force
# The owner invokes Git after choosing its input names. Use that boundary to
# place a filesystem transition at the same point on every machine.
@'
use std::{env, fs, path::PathBuf};
fn main() {
    let args: Vec<_> = env::args_os().collect();
    if args.len() == 5 && args[3] == "branch" && args[4] == "--show-current" {
        if let Ok(mode) = fs::read_to_string(env::var_os("SOMA_QA_ARM").unwrap()) {
            let root = PathBuf::from(&args[2]).join("inputs");
            if mode.trim() == "excluded" {
                fs::create_dir_all(root.join("target")).unwrap();
                fs::File::create_new(root.join("target").join(std::process::id().to_string())).unwrap();
            } else if mode.trim() == "remove" {
                let _ = fs::remove_file(root.join("original"));
            } else if mode.trim() == "case" {
                fs::write(root.join("generated/added"), "new").unwrap();
            } else {
                fs::write(root.join("added"), "new").unwrap();
            }
        }
        println!("fixture");
    } else if args.len() == 5 && args[3] == "rev-parse" && args[4] == "HEAD" {
        println!("0123456789abcdef0123456789abcdef01234567");
    } else { std::process::exit(1); }
}
'@|Set-Content (Join-Path $toolDir.FullName 'fixture.rs')
& rustc --edition=2021 (Join-Path $toolDir.FullName 'fixture.rs') -o (Join-Path $toolDir.FullName 'git.exe')
if($LASTEXITCODE){throw 'Failed to compile fixture tool'}
$previousPath=$env:PATH
$previousStore=$env:SOMA_HOME
$previousArm=$env:SOMA_QA_ARM
try{
    $env:PATH=$toolDir.FullName+';'+$env:PATH
    foreach($case in 'preparation','final','excluded','remove','case'){
        $caseDir=Join-Path $Workspace $case
        $fixture=Join-Path $caseDir 'checkout'
        $env:SOMA_HOME=Join-Path $caseDir 'data'
        $env:SOMA_QA_ARM=Join-Path $caseDir 'armed'
        foreach($dir in @((Join-Path $fixture 'inputs'),$env:SOMA_HOME)){New-Item -ItemType Directory $dir -Force|Out-Null}
        Set-Content (Join-Path $fixture 'pom.xml') '<project/>'
        Set-Content (Join-Path $fixture 'mvnw.cmd') ('@echo '+$case+'>"%SOMA_QA_ARM%"')
        if($case -eq 'remove'){Set-Content (Join-Path $fixture 'inputs/original') 'original'}
        if($case -in @('preparation','remove')){Set-Content $env:SOMA_QA_ARM $case}
        $project=@{id='fixture';name='fixture';directory=$fixture;configs=@();test_configs=@(@{id='tests';name='tests';inputs=@('pom.xml','inputs')})}
        if($case -eq 'case'){
            New-Item -ItemType Directory (Join-Path $fixture 'inputs/generated')|Out-Null
            $project.test_configs[0].inputs=@('pom.xml','Inputs')
            $project.test_configs[0].input_excludes=@('inputs/generated')
        }
        @{schema_version=1;projects=@($project)}|ConvertTo-Json -Depth 9|Set-Content (Join-Path $env:SOMA_HOME 'projects.json')
        try{
            $job=& $cli jobs submit --path $fixture --kind build -- compile|ConvertFrom-Json
            if(!$job.job_id){throw ($job|ConvertTo-Json -Compress)}
            $result=& $cli jobs wait --job $job.job_id --timeout 45 --summary|ConvertFrom-Json
            if($case -eq 'excluded'){
                if($result.verdict -ne 'built' -or !(Get-ChildItem (Join-Path $fixture 'inputs/target') -File)){throw 'Excluded Maven output invalidated a build'}
            }elseif($case -eq 'remove'){
                if($result.verdict -ne 'invalidated' -or (Test-Path (Join-Path $fixture 'inputs/original'))){throw 'Removal during preparation must invalidate the job'}
            }elseif($case -eq 'case'){
                if($result.verdict -ne 'invalidated' -or !(Test-Path (Join-Path $fixture 'inputs/generated/added'))){throw 'Configured spelling must match traversal semantics'}
            }elseif($result.verdict -ne 'invalidated' -or !(Test-Path (Join-Path $fixture 'inputs/added'))){
                throw "Missing invalidation during $case. Verdict $($result.verdict)"
            }
            Write-Output "PASS $case fingerprint transition ($($result.verdict))"
        }finally{ & $cli close|Out-Null }
    }
}finally{$env:PATH=$previousPath;$env:SOMA_HOME=$previousStore;$env:SOMA_QA_ARM=$previousArm}

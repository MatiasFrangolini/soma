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
    if args.len() == 5 && args[1] == "noise" {
        let root = PathBuf::from(&args[2]);
        for n in 0..1024 { fs::write(root.join("inputs").join(n.to_string()), "").unwrap(); }
        fs::write(&args[3], "ready").unwrap();
        let started = std::time::Instant::now();
        while !PathBuf::from(&args[4]).exists() && started.elapsed().as_secs() < 30 {
            fs::write(root.join("scratch.txt"), "").unwrap();
            fs::remove_file(root.join("scratch.txt")).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        return;
    }
    if args.len() == 5 && args[3] == "branch" && args[4] == "--show-current" {
        if let Ok(mode) = fs::read_to_string(env::var_os("SOMA_QA_ARM").unwrap()) {
            let root = PathBuf::from(&args[2]).join("inputs");
            if mode.trim() == "excluded" {
                fs::create_dir_all(root.join("target")).unwrap();
                fs::File::create_new(root.join("target").join(std::process::id().to_string())).unwrap();
            } else if mode.trim() == "outside" {
                fs::File::create_new(PathBuf::from(&args[2]).join(format!("scratch-{}", std::process::id()))).unwrap();
            } else if mode.trim() == "remove" {
                let _ = fs::remove_file(root.join("original"));
            } else if mode.trim() == "remove-directory" {
                let _ = fs::remove_dir(root.join("../empty"));
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
    foreach($case in 'preparation','final','excluded','remove','case','outside','missing-excluded','remove-directory','queued-remove','missing-required','missing-busy'){
        $caseDir=Join-Path $Workspace $case
        $fixture=Join-Path $caseDir 'checkout'
        $env:SOMA_HOME=Join-Path $caseDir 'data'
        $env:SOMA_QA_ARM=Join-Path $caseDir 'armed'
        foreach($dir in @((Join-Path $fixture 'inputs'),$env:SOMA_HOME)){New-Item -ItemType Directory $dir -Force|Out-Null}
        Set-Content (Join-Path $fixture 'pom.xml') '<project/>'
        Set-Content (Join-Path $fixture 'mvnw.cmd') ('@echo '+$case+'>"%SOMA_QA_ARM%"')
        if($case -eq 'remove'){Set-Content (Join-Path $fixture 'inputs/original') 'original'}
        if($case -in @('preparation','remove','remove-directory')){Set-Content $env:SOMA_QA_ARM $case}
        $project=@{id='fixture';name='fixture';directory=$fixture;configs=@();test_configs=@(@{id='tests';name='tests';inputs=@('pom.xml','inputs')})}
        if($case -eq 'case'){
            New-Item -ItemType Directory (Join-Path $fixture 'inputs/generated')|Out-Null
            $project.test_configs[0].inputs=@('pom.xml','Inputs')
            $project.test_configs[0].input_excludes=@('inputs/generated')
        }
        if($case -eq 'missing-excluded'){
            $project.test_configs[0].inputs+='generated'
            $project.test_configs[0].input_excludes=@('generated')
        }
        if($case -in @('remove-directory','queued-remove','missing-required','missing-busy')){
            $project.test_configs[0].inputs+='empty'
            if($case -in @('remove-directory','queued-remove')){New-Item -ItemType Directory (Join-Path $fixture 'empty')|Out-Null}
        }
        @{schema_version=1;projects=@($project)}|ConvertTo-Json -Depth 9|Set-Content (Join-Path $env:SOMA_HOME 'projects.json')
        $lock=$null
        $noise=$null
        try{
            if($case -eq 'queued-remove'){
                $canonical=(Resolve-Path -LiteralPath $fixture).Path.ToLowerInvariant()
                $key=[Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($canonical))).ToLowerInvariant()
                $lock=[Threading.Mutex]::new($false,"Local\SomaCheckout-$key")
                if(!$lock.WaitOne(0)){throw 'Fixture checkout unexpectedly busy'}
            }
            if($case -eq 'missing-busy'){
                $info=[Diagnostics.ProcessStartInfo]::new((Join-Path $toolDir.FullName 'git.exe'))
                $info.UseShellExecute=$false
                $info.CreateNoWindow=$true
                foreach($arg in @('noise',$fixture,"$caseDir/ready","$caseDir/stop")){$info.ArgumentList.Add($arg)}
                $noise=[Diagnostics.Process]::Start($info)
                $limit=[datetime]::UtcNow.AddSeconds(20)
                while(!(Test-Path "$caseDir/ready")){
                    if($noise.HasExited -or [datetime]::UtcNow -gt $limit){throw 'Noise fixture failed to start'}
                    Start-Sleep -Milliseconds 15
                }
            }
            $job=& $cli jobs submit --path $fixture --kind build -- compile|ConvertFrom-Json
            if(!$job.job_id){throw ($job|ConvertTo-Json -Compress)}
            if($lock){
                $limit=[datetime]::UtcNow.AddSeconds(20)
                do{
                    $status=& $cli jobs status --job $job.job_id --summary|ConvertFrom-Json
                    if($status.state -in @('queued','waiting_checkout')){break}
                    if($status.verdict -ne 'pending' -or [datetime]::UtcNow -gt $limit){throw 'Preparation did not reach the checkout queue'}
                    Start-Sleep -Milliseconds 25
                }while($true)
                [IO.Directory]::Delete((Join-Path $fixture 'empty'))
                $lock.ReleaseMutex();$lock.Dispose();$lock=$null
            }
            $result=& $cli jobs wait --job $job.job_id --timeout 45 --summary|ConvertFrom-Json
            if($case -eq 'excluded'){
                if($result.verdict -ne 'built' -or !(Get-ChildItem (Join-Path $fixture 'inputs/target') -File)){throw 'Excluded Maven output invalidated a build'}
            }elseif($case -eq 'outside'){
                if($result.verdict -ne 'built' -or !(Get-ChildItem $fixture -Filter 'scratch-*' -File)){throw 'Unconfigured top-level file invalidated a build'}
            }elseif($case -eq 'remove'){
                if($result.verdict -ne 'invalidated' -or (Test-Path (Join-Path $fixture 'inputs/original'))){throw 'Removal during preparation must invalidate the job'}
            }elseif($case -in @('remove-directory','queued-remove')){
                if($result.verdict -ne 'invalidated' -or (Test-Path (Join-Path $fixture 'empty'))){throw 'Rescan must preserve the observed directory removal'}
            }elseif($case -in @('missing-required','missing-busy')){
                if($result.verdict -ne 'failed'){throw 'An initially absent required input must fail preparation'}
            }elseif($case -eq 'case'){
                if($result.verdict -ne 'invalidated' -or !(Test-Path (Join-Path $fixture 'inputs/generated/added'))){throw 'Configured spelling must match traversal semantics'}
            }elseif($result.verdict -ne 'invalidated' -or !(Test-Path (Join-Path $fixture 'inputs/added'))){
                throw "Missing invalidation during $case. Verdict $($result.verdict)"
            }
            Write-Output "PASS $case fingerprint transition ($($result.verdict))"
        }finally{
            if($lock){$lock.ReleaseMutex();$lock.Dispose()}
            if($noise){Set-Content "$caseDir/stop" 'stop';$noise.WaitForExit();$noise.Dispose()}
            & $cli close|Out-Null
        }
    }
}finally{$env:PATH=$previousPath;$env:SOMA_HOME=$previousStore;$env:SOMA_QA_ARM=$previousArm}

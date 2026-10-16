#requires -Version 7.0
param([Parameter(Mandatory)][string]$BinaryDirectory,[Parameter(Mandatory)][string]$Workspace,[Parameter(Mandatory)][string]$Package)
$ErrorActionPreference='Stop'
$root=Split-Path -Parent $PSScriptRoot
New-Item -ItemType Directory -Path $Workspace -Force|Out-Null
$checks=[Collections.Generic.List[string]]::new()
function Assert([bool]$Condition,[string]$Description){if(-not $Condition){throw ('FAIL: '+$Description)};$checks.Add($Description);Write-Output ('PASS: '+$Description)}
function Invoke-SomaCli([string]$Exe,[string[]]$Arguments){
    $info=[Diagnostics.ProcessStartInfo]::new($Exe);$info.UseShellExecute=$false;$info.CreateNoWindow=$true;$info.RedirectStandardOutput=$true;$info.RedirectStandardError=$true
    $info.Environment['LOCALAPPDATA']=Join-Path $Workspace 'profile';$null=$info.Environment.Remove('SOMA_HOME');$null=$info.Environment.Remove('RUNLOCAL_HOME')
    foreach($argument in $Arguments){$info.ArgumentList.Add($argument)}
    $p=[Diagnostics.Process]::Start($info)
    try{$raw=$p.StandardOutput.ReadToEnd();$errorText=$p.StandardError.ReadToEnd();$p.WaitForExit();if($p.ExitCode -ne 0){throw ($raw+$errorText)};return $raw|ConvertFrom-Json}finally{$p.Dispose()}
}
$one=Join-Path $Workspace 'version-one';$two=Join-Path $Workspace 'version-two'
foreach($path in @($one,$two)){Expand-Archive -LiteralPath $Package -DestinationPath $path -Force}
$first=Join-Path $one 'soma-cli.exe';$second=Join-Path $two 'soma-cli.exe'
$expected=Join-Path $Workspace 'profile/Soma/projects.json'
Assert ((Invoke-SomaCli $first @('config-path')).path -eq $expected) 'Fresh installations use per-user LocalAppData, not the version folder'
$project=Join-Path $Workspace 'a project';New-Item -ItemType Directory -Path $project -Force|Out-Null
$added=Invoke-SomaCli $first @('add','--path',$project)
$listed=Invoke-SomaCli $second @('projects')
Assert ($listed.projects[0].id -eq $added.project) 'Another installed version sees existing personal projects'
Assert (-not (Test-Path -LiteralPath (Join-Path $one 'Soma-data'))) 'The executable folder stays free of personal data'
$custom=Join-Path $Workspace 'custom'
Assert ((Invoke-SomaCli $second @('config-path','--data-dir',$custom)).path -eq (Join-Path $custom 'projects.json')) 'Explicit data-dir still overrides the installed default'
$legacy=Join-Path $one 'RunLocal-data';New-Item -ItemType Directory -Path $legacy|Out-Null
Set-Content -LiteralPath (Join-Path $legacy 'projects.json') -Value '{"schema_version":1,"projects":[]}'
Assert ((Invoke-SomaCli $first @('config-path')).path -eq (Join-Path $legacy 'projects.json')) 'Existing legacy portable configuration is preserved'
$portable=Join-Path $one 'Soma-data';New-Item -ItemType Directory -Path $portable|Out-Null
Assert ((Invoke-SomaCli $first @('config-path')).path -eq (Join-Path $portable 'projects.json')) 'An explicit portable data folder has priority over legacy and installed defaults'
$maven=Join-Path $Workspace 'maven project & fixture';New-Item -ItemType Directory -Path $maven|Out-Null
Set-Content -LiteralPath (Join-Path $maven 'pom.xml') -Value '<project><artifactId>demo</artifactId><version>1</version></project>'
@'
@echo off
echo MAVEN_ARGS=%*
if not exist target mkdir target
echo fixture-artifact>target\demo-1.jar
exit /b 0
'@|Set-Content -LiteralPath (Join-Path $maven 'mvnw.cmd') -Encoding ascii
$mavenData=Join-Path $Workspace 'maven-data'
$mavenProject=Invoke-SomaCli $second @('add','--path',$maven,'--data-dir',$mavenData)
try {
    $built=Invoke-SomaCli $second @('build','--project',$mavenProject.project,'--wait','--data-dir',$mavenData)
    $logs=Invoke-SomaCli $second @('logs','--data-dir',$mavenData)
    Assert ($built.status.state -eq 'exited' -and $logs.text.Contains('MAVEN_ARGS=package')) 'New Maven presets execute the repo wrapper without a personal adapter, including paths with spaces and ampersands'
}finally{$null=Invoke-SomaCli $second @('close','--data-dir',$mavenData)}
$archive=[IO.Compression.ZipFile]::OpenRead($Package)
try{
    $names=@($archive.Entries|ForEach-Object FullName|Sort-Object)
    $allowed=@('AGENTS.md','LICENSE','README.md','Soma.exe','build-info.json','soma-cli.exe')|Sort-Object
    Assert (($names -join '|') -eq ($allowed -join '|')) 'Public ZIP contains exactly the declared files and no private configuration or logs'
    $entry=$archive.GetEntry('build-info.json');$reader=[IO.StreamReader]::new($entry.Open())
    try{$info=$reader.ReadToEnd()|ConvertFrom-Json}finally{$reader.Dispose()}
    foreach($file in $info.files){Assert ((Get-FileHash -LiteralPath (Join-Path $two $file.name)).Hash.ToLowerInvariant() -eq $file.sha256) ('Packaged hash matches '+$file.name)}
}finally{$archive.Dispose()}
@{Result='PASS';Checks=@($checks.ToArray())}|ConvertTo-Json -Depth 6|Set-Content -LiteralPath (Join-Path $Workspace 'distribution-verification.json') -Encoding utf8NoBOM

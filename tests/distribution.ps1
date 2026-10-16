#requires -Version 7.0
param([Parameter(Mandatory)][string]$BinaryDirectory,[Parameter(Mandatory)][string]$Workspace,[Parameter(Mandatory)][string]$Package)
$ErrorActionPreference='Stop'
$root=Split-Path -Parent $PSScriptRoot
New-Item -ItemType Directory -Path $Workspace -Force|Out-Null
$checks=[Collections.Generic.List[string]]::new()
function Assert([bool]$Condition,[string]$Description){if(-not $Condition){throw ('FAIL: '+$Description)};$checks.Add($Description);Write-Output ('PASS: '+$Description)}
function Invoke-SomaCli([string]$Exe,[string[]]$Arguments,[string]$ProfileDirectory=(Join-Path $Workspace 'profile')){
    $info=[Diagnostics.ProcessStartInfo]::new($Exe);$info.UseShellExecute=$false;$info.CreateNoWindow=$true;$info.RedirectStandardOutput=$true;$info.RedirectStandardError=$true
    $info.Environment['USERPROFILE']=$ProfileDirectory;$info.Environment['LOCALAPPDATA']=Join-Path $ProfileDirectory 'AppData/Local';$null=$info.Environment.Remove('SOMA_HOME');$null=$info.Environment.Remove('RUNLOCAL_HOME')
    foreach($argument in $Arguments){$info.ArgumentList.Add($argument)}
    $p=[Diagnostics.Process]::Start($info)
    try{$raw=$p.StandardOutput.ReadToEnd();$errorText=$p.StandardError.ReadToEnd();$p.WaitForExit();if($p.ExitCode -ne 0){throw ($raw+$errorText)};return $raw|ConvertFrom-Json}finally{$p.Dispose()}
}
$one=Join-Path $Workspace 'version-one';$two=Join-Path $Workspace 'version-two'
foreach($path in @($one,$two)){Expand-Archive -LiteralPath $Package -DestinationPath $path -Force}
$first=Join-Path $one 'soma-cli.exe';$second=Join-Path $two 'soma-cli.exe'
$expected=Join-Path $Workspace 'profile/.soma/projects.json'
Assert ((Invoke-SomaCli $first @('config-path')).path -eq $expected) 'Fresh installations use the stable user profile directory, outside version folders and AppData package redirection'
$project=Join-Path $Workspace 'a project';New-Item -ItemType Directory -Path $project -Force|Out-Null
$added=Invoke-SomaCli $first @('add','--path',$project)
$listed=Invoke-SomaCli $second @('projects')
Assert ($listed.projects[0].id -eq $added.project) 'Another installed version sees existing personal projects'
Assert (-not (Test-Path -LiteralPath (Join-Path $one 'Soma-data'))) 'The executable folder stays free of personal data'
$migratedProfile=Join-Path $Workspace 'migrated-profile'
$legacyAppData=Join-Path $migratedProfile 'AppData/Local/Soma'
New-Item -ItemType Directory -Path (Join-Path $legacyAppData 'logs') -Force|Out-Null
Copy-Item -LiteralPath $expected -Destination (Join-Path $legacyAppData 'projects.json')
$legacyAdapter=Join-Path $legacyAppData 'build-maven.ps1'
Set-Content -LiteralPath $legacyAdapter -Value 'Write-Output migration-fixture'
$legacySettings=Get-Content -LiteralPath (Join-Path $legacyAppData 'projects.json') -Raw|ConvertFrom-Json
$legacySettings|Add-Member -NotePropertyName maven_adapter -NotePropertyValue $legacyAdapter -Force
$legacySettings|ConvertTo-Json -Depth 20|Set-Content -LiteralPath (Join-Path $legacyAppData 'projects.json')
Set-Content -LiteralPath (Join-Path $legacyAppData 'logs/current.log') -Value 'LEGACY HISTORY'
$legacyHash=(Get-FileHash -LiteralPath (Join-Path $legacyAppData 'projects.json')).Hash
$migrated=Invoke-SomaCli $second @('projects') $migratedProfile
Assert ($migrated.projects[0].id -eq $added.project) 'The 0.3.0 AppData configuration migrates to the stable profile on first use'
$migratedSettings=Get-Content -LiteralPath (Join-Path $migratedProfile '.soma/projects.json') -Raw|ConvertFrom-Json
Assert ($migratedSettings.maven_adapter -eq (Join-Path $migratedProfile '.soma/build-maven.ps1') -and (Test-Path -LiteralPath $migratedSettings.maven_adapter)) 'A private Maven adapter is copied and repointed outside AppData redirection'
$migratedLogs=Invoke-SomaCli $second @('logs') $migratedProfile
Assert ($migratedLogs.text.Contains('LEGACY HISTORY')) 'Existing rotating logs are preserved during AppData migration'
Assert ((Get-FileHash -LiteralPath (Join-Path $legacyAppData 'projects.json')).Hash -eq $legacyHash) 'Migration keeps the original AppData configuration intact'
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

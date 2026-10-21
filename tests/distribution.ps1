#requires -Version 7.0
param([Parameter(Mandatory)][string]$BinaryDirectory,[Parameter(Mandatory)][string]$Workspace,[Parameter(Mandatory)][string]$Package,[string]$Jdk=$env:JAVA_HOME_21_X64)
$ErrorActionPreference='Stop'
$root=Split-Path -Parent $PSScriptRoot
if(-not $Jdk){$Jdk=$env:JAVA_HOME}
if(-not $Jdk -or -not (Test-Path -LiteralPath (Join-Path $Jdk 'bin/javac.exe'))){throw 'A JDK is required to verify the portable Java preset'}
$script:PortableJdk=$Jdk
New-Item -ItemType Directory -Path $Workspace -Force|Out-Null
$checks=[Collections.Generic.List[string]]::new()
function Assert([bool]$Condition,[string]$Description){if(-not $Condition){throw ('FAIL: '+$Description)};$checks.Add($Description);Write-Output ('PASS: '+$Description)}
function Invoke-SomaCli([string]$Exe,[string[]]$Arguments,[string]$ProfileDirectory=(Join-Path $Workspace 'profile'),[string]$JavaHome=$script:PortableJdk){
    $info=[Diagnostics.ProcessStartInfo]::new($Exe);$info.UseShellExecute=$false;$info.CreateNoWindow=$true;$info.RedirectStandardOutput=$true;$info.RedirectStandardError=$true
    $info.Environment['USERPROFILE']=$ProfileDirectory;$info.Environment['LOCALAPPDATA']=Join-Path $ProfileDirectory 'AppData/Local';$null=$info.Environment.Remove('SOMA_HOME');$null=$info.Environment.Remove('RUNLOCAL_HOME')
    if($JavaHome){$info.Environment['JAVA_HOME']=$JavaHome}else{$null=$info.Environment.Remove('JAVA_HOME')}
    $info.Environment['PATH']=(Join-Path $script:PortableJdk 'bin')+';'+$info.Environment['PATH']
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
$script:PortableJdk=Join-Path $Workspace 'JDK vendor & portable'
New-Item -ItemType Junction -Path $script:PortableJdk -Target $Jdk|Out-Null
Set-Content -LiteralPath (Join-Path $maven 'pom.xml') -Value '<project><artifactId>demo</artifactId><version>1</version></project>'
@'
import java.lang.management.ManagementFactory;
public class JvmProbe {
    public static void main(String[] args) {
        System.out.println("VM_ARGS="+String.join(";",ManagementFactory.getRuntimeMXBean().getInputArguments()));
        System.out.println("MAX_HEAP="+Runtime.getRuntime().maxMemory());
        System.out.println("JAVA_HOME="+System.getProperty("java.home"));
    }
}
'@|Set-Content -LiteralPath (Join-Path $maven 'JvmProbe.java')
& (Join-Path $Jdk 'bin/javac.exe') '-J-Xmx64m' '--release' '21' (Join-Path $maven 'JvmProbe.java')
if($LASTEXITCODE -ne 0){throw 'Could not compile the disposable JVM probe'}
@'
@echo off
echo MAVEN_ARGS=%*
if not exist target mkdir target
jar.exe --create --file target\demo-1.jar --main-class JvmProbe -C . JvmProbe.class
exit /b %ERRORLEVEL%
'@|Set-Content -LiteralPath (Join-Path $maven 'mvnw.cmd') -Encoding ascii
$mavenData=Join-Path $Workspace 'maven-data'
$mavenProject=Invoke-SomaCli $second @('add','--path',$maven,'--data-dir',$mavenData)
$mavenSettings=Get-Content -LiteralPath (Join-Path $mavenData 'projects.json') -Raw|ConvertFrom-Json
$javaProject=$mavenSettings.projects|Where-Object id -eq $mavenProject.project
Assert (@($javaProject.configs|Where-Object {$_.id -eq 'jvm-optimized' -and $_.name -eq 'JVM optimizada'}).Count -eq 1) 'Fresh Maven projects expose the portable JVM optimizada configuration alongside Local'
Assert (@($javaProject.configs|Where-Object {$_.jdk -ne $script:PortableJdk}).Count -eq 0 -and @($javaProject.configs|Where-Object id -eq 'local')[0].run.jvm_args.Count -eq 0) 'JDK discovery uses this users JAVA_HOME and leaves Local JVM defaults unchanged'
$publicPreset=Get-Content -LiteralPath (Join-Path $two 'presets/jvm-optimized.json') -Raw|ConvertFrom-Json
$embeddedPreset=(Invoke-SomaCli $second @('presets')).presets[0]
Assert ($publicPreset.id -eq $embeddedPreset.id -and $publicPreset.name -eq $embeddedPreset.name -and ($publicPreset.jvm_args -join '|') -eq ($embeddedPreset.jvm_args -join '|')) 'The distributed preset JSON matches the presets embedded in the executable'
$decoy=Join-Path $Workspace 'profile/.jdks/ms-21-decoy';New-Item -ItemType Directory -Path $decoy -Force|Out-Null
$pathData=Join-Path $Workspace 'path-java-data'
$null=Invoke-SomaCli $second @('add','--path',$maven,'--data-dir',$pathData) -JavaHome ''
$pathConfig=Get-Content -LiteralPath (Join-Path $pathData 'projects.json') -Raw|ConvertFrom-Json
Assert (@($pathConfig.projects[0].configs|Where-Object {$null -ne $_.jdk}).Count -eq 0) 'Without JAVA_HOME a vendor-named user folder is ignored and Java can resolve through PATH'
try {
    $built=Invoke-SomaCli $second @('build','--project',$mavenProject.project,'--wait','--data-dir',$mavenData)
    $logs=Invoke-SomaCli $second @('logs','--data-dir',$mavenData)
    Assert ($built.status.state -eq 'exited' -and $logs.text.Contains('MAVEN_ARGS=package')) 'New Maven presets execute the repo wrapper without a personal adapter, including paths with spaces and ampersands'
    $pathRun=Invoke-SomaCli $second @('run','--project',$pathConfig.projects[0].id,'--config','jvm-optimized','--wait','--data-dir',$pathData) -JavaHome ''
    $pathLogs=(Invoke-SomaCli $second @('logs','--data-dir',$pathData) -JavaHome '').text
    Assert ($pathRun.status.state -eq 'exited' -and $pathLogs.Contains('MAX_HEAP=805306368')) 'Without JAVA_HOME the packaged application builds and executes Java from the users PATH'
    $javaProject.configs=@($javaProject.configs|Where-Object id -eq 'local')
    $javaProject.configs[0].name='My existing Java configuration'
    $javaProject.configs[0].run.jvm_args=@('-Xmx128m','-Dportability.fixture=kept')
    $mavenSettings|Add-Member -NotePropertyName job_adapter -NotePropertyValue @{executable='cmd.exe';args=@('/d','/c','exit','0')}
    $mavenSettings|ConvertTo-Json -Depth 20|Set-Content -LiteralPath (Join-Path $mavenData 'projects.json')
    $original=$javaProject.configs[0]|ConvertTo-Json -Depth 12 -Compress
    $applied=Invoke-SomaCli $second @('apply-preset','--project',$mavenProject.project,'--preset','jvm-optimized','--data-dir',$mavenData)
    $updated=Get-Content -LiteralPath (Join-Path $mavenData 'projects.json') -Raw|ConvertFrom-Json
    $preserved=$updated.projects[0].configs|Where-Object id -eq 'local'
    Assert ($applied.added -and ($preserved|ConvertTo-Json -Depth 12 -Compress) -eq $original -and $updated.job_adapter.executable -eq 'cmd.exe') 'Applying a preset adds it to an existing project without changing its original configuration or private job adapter'
    $userPreset=$updated.projects[0].configs|Where-Object id -eq 'jvm-optimized'
    $userPreset.name='My customized JVM'
    $userPreset.run.jvm_args+=@('-Dportability.user.custom=retained')
    $updated|ConvertTo-Json -Depth 20|Set-Content -LiteralPath (Join-Path $mavenData 'projects.json')
    $hashBefore=(Get-FileHash -LiteralPath (Join-Path $mavenData 'projects.json')).Hash
    $repeated=Invoke-SomaCli $second @('apply-preset','--project',$mavenProject.project,'--data-dir',$mavenData)
    Assert (-not $repeated.added -and (Get-FileHash -LiteralPath (Join-Path $mavenData 'projects.json')).Hash -eq $hashBefore) 'Reapplying the preset preserves a users customized configuration byte for byte'
    $optimized=Invoke-SomaCli $second @('run','--project',$mavenProject.project,'--config','jvm-optimized','--wait','--data-dir',$mavenData)
    $output=(Invoke-SomaCli $second @('logs','--data-dir',$mavenData)).text
    Assert ($optimized.status.state -eq 'exited' -and $output.Contains('-Xms64m') -and $output.Contains('-Xmx768m') -and $output.Contains('-XX:+UseG1GC') -and $output.Contains('-Dportability.fixture=kept') -and $output.Contains('-Dportability.user.custom=retained') -and -not $output.Contains('-Xmx128m') -and $output.Contains('MAX_HEAP=805306368')) 'The installed optimized configuration runs a real Java 21 JAR with the expected heap, collector and preserved application JVM arguments'
}finally{$null=Invoke-SomaCli $second @('close','--data-dir',$mavenData);$null=Invoke-SomaCli $second @('close','--data-dir',$pathData)}
$archive=[IO.Compression.ZipFile]::OpenRead($Package)
try{
    $names=@($archive.Entries|Where-Object {$_.Name -ne ''}|ForEach-Object {$_.FullName.Replace('\','/')}|Sort-Object)
    $allowed=@('AGENTS.md','CLI.md','LICENSE','README.md','PORTABILITY.md','Soma.exe','Update-Soma.ps1','build-info.json','soma-cli.exe','presets/jvm-optimized.json')|Sort-Object
    Assert (($names -join '|') -eq ($allowed -join '|')) 'Public ZIP contains exactly the declared files and no private configuration or logs'
    $entry=$archive.GetEntry('build-info.json');$reader=[IO.StreamReader]::new($entry.Open())
    try{$info=$reader.ReadToEnd()|ConvertFrom-Json}finally{$reader.Dispose()}
    foreach($file in $info.files){Assert ((Get-FileHash -LiteralPath (Join-Path $two $file.name)).Hash.ToLowerInvariant() -eq $file.sha256) ('Packaged hash matches '+$file.name)}
}finally{$archive.Dispose()}
@{Result='PASS';Checks=@($checks.ToArray())}|ConvertTo-Json -Depth 6|Set-Content -LiteralPath (Join-Path $Workspace 'distribution-verification.json') -Encoding utf8NoBOM

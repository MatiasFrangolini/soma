#requires -Version 7.0
param([Parameter(Mandatory)][string]$BinaryDirectory,[string]$OutputDirectory=(Join-Path $PSScriptRoot '../dist'),[string]$RepositoryUrl='https://github.com/MatiasFrangolini/soma')
$ErrorActionPreference='Stop'
$root=Split-Path -Parent $PSScriptRoot
if($RepositoryUrl -notmatch '^https://github\.com/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$'){throw 'Expected a GitHub repository URL without a trailing slash'}
$version=[regex]::Match((Get-Content -LiteralPath (Join-Path $root 'Cargo.toml') -Raw),'(?m)^version = "(\d+\.\d+\.\d+)"').Groups[1].Value
if(-not $version){throw 'Expected a stable semantic version in Cargo.toml'}
$output=[IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Path $output -Force|Out-Null
$stage=Join-Path $output ('.stage-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $stage|Out-Null
foreach($name in @('Soma.exe','soma-cli.exe')){Copy-Item -LiteralPath (Join-Path $BinaryDirectory $name) -Destination (Join-Path $stage $name)}
foreach($name in @('README.md','CLI.md','AGENTS.md','LICENSE')){Copy-Item -LiteralPath (Join-Path $root $name) -Destination (Join-Path $stage $name)}
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'Update-Soma.ps1') -Destination (Join-Path $stage 'Update-Soma.ps1')
New-Item -ItemType Directory -Path (Join-Path $stage 'presets')|Out-Null
Copy-Item -LiteralPath (Join-Path $root 'presets/jvm-optimized.json') -Destination (Join-Path $stage 'presets/jvm-optimized.json')
Copy-Item -LiteralPath (Join-Path $root 'docs/portability.md') -Destination (Join-Path $stage 'PORTABILITY.md')
$files=@(foreach($name in @('Soma.exe','soma-cli.exe')){$path=Join-Path $stage $name;[ordered]@{name=$name;sha256=(Get-FileHash -LiteralPath $path).Hash.ToLowerInvariant()}})
@{version=$version;architecture='x64';files=$files}|ConvertTo-Json -Depth 5|Set-Content -LiteralPath (Join-Path $stage 'build-info.json') -Encoding utf8NoBOM
$filename="Soma-$version-windows-x64.zip"
$zip=Join-Path $output $filename
Compress-Archive -LiteralPath @((Get-ChildItem -LiteralPath $stage).FullName) -DestinationPath $zip -Force
$hash=(Get-FileHash -LiteralPath $zip).Hash.ToLowerInvariant()
Set-Content -LiteralPath (Join-Path $output 'SHA256SUMS') -Value "$hash  $filename" -Encoding ascii
$guard=@(
    '$oldCli = Join-Path (appdir $app $global) ''current\soma-cli.exe''',
    'if (Test-Path -LiteralPath $oldCli) {',
    '    $state = (& $oldCli status | ConvertFrom-Json).status.state',
    '    if ($state -ne ''closed'') { throw ''Close Soma before updating or uninstalling. Use soma-cli close for an orderly shutdown.'' }',
    '}'
)
$manifest=[ordered]@{version=$version;description='Lightweight native Windows runner with an optional JSON CLI';homepage=$RepositoryUrl;license='MIT';architecture=@{'64bit'=@{url="$RepositoryUrl/releases/download/v$version/$filename";hash=$hash}};bin=@('soma-cli.exe','Soma.exe',@('Update-Soma.ps1','soma-update'));shortcuts=@(,@('Soma.exe','Soma'));pre_install=$guard;pre_uninstall=$guard;notes='Personal configuration and logs stay in USERPROFILE\.soma. Close Soma and run soma-update to update. Java is required only for Java projects. No AI tools are required.';checkver=@{github=$RepositoryUrl};autoupdate=@{architecture=@{'64bit'=@{url=($RepositoryUrl+'/releases/download/v$version/Soma-$version-windows-x64.zip')}}}}
$manifest|ConvertTo-Json -Depth 9|Set-Content -LiteralPath (Join-Path $output 'soma.json') -Encoding utf8NoBOM
if(-not $stage.StartsWith($output+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)){throw 'Unsafe stage cleanup'}
Remove-Item -LiteralPath $stage -Recurse -Force
[pscustomobject]@{Version=$version;Zip=$zip;Bytes=(Get-Item -LiteralPath $zip).Length;SHA256=$hash}|ConvertTo-Json

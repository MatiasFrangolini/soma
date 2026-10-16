#requires -Version 7.0
$ErrorActionPreference='Stop'
$root=Split-Path -Parent $PSScriptRoot
if((git -C $root branch --show-current) -ne 'main'){throw 'Release from main'}
if(git -C $root status --porcelain){throw 'Commit your changes before releasing'}
$version=[regex]::Match((Get-Content -LiteralPath (Join-Path $root 'Cargo.toml') -Raw),'(?m)^version = "(\d+\.\d+\.\d+)"').Groups[1].Value
if(-not $version){throw 'Expected a stable semantic version'}
$tag='v'+$version
git -C $root push origin main
if($LASTEXITCODE -ne 0){exit $LASTEXITCODE}
git -C $root tag -a $tag -m "Soma $version"
if($LASTEXITCODE -ne 0){exit $LASTEXITCODE}
git -C $root push origin $tag
exit $LASTEXITCODE

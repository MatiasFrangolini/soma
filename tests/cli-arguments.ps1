#requires -Version 7.0
param([Parameter(Mandatory)][string]$BinaryDirectory,[Parameter(Mandatory)][string]$Workspace)
$ErrorActionPreference='Stop'
$PSNativeCommandUseErrorActionPreference=$false
$cli=Join-Path $BinaryDirectory 'soma-cli.exe'
$data=Join-Path $Workspace 'never-created'
if(Test-Path -LiteralPath $data){throw 'Use a fresh workspace for CLI argument checks'}
$env:SOMA_HOME=$data
$checks=@(
    @{Args=@('jobs','submit','--path','unused','--help'); Help=$true},
    @{Args=@('jobs','submit','-h'); Help=$true},
    @{Args=@('add','--help'); Help=$true},
    @{Args=@('jobs','submit','--path','unused','--unknown','--','test'); Help=$false},
    @{Args=@('jobs','submit','--path','unused'); Help=$false},
    @{Args=@('jobs','submit','--path','--kind','test','--','test'); Help=$false},
    @{Args=@('jobs','wait','--job','unused','--timeout','wrong'); Help=$false}
)
foreach($check in $checks){
    $output=& $cli @($check.Args) 2>&1 | Out-String
    $code=$LASTEXITCODE
    if($check.Help){
        if($code -ne 0 -or $output -notmatch 'Soma '){throw "Expected help without execution: $($check.Args -join ' ')"}
    }else{
        if($code -eq 0){throw "Expected invalid arguments to fail: $($check.Args -join ' ')"}
        $json=$output | ConvertFrom-Json
        if($json.ok -ne $false){throw 'Invalid arguments must return a JSON error'}
    }
    if(Test-Path -LiteralPath $data){throw 'Argument handling unexpectedly initialized the data store'}
}
Write-Output 'PASS: help and invalid Soma arguments cause no data or owner side effects'
exit 0

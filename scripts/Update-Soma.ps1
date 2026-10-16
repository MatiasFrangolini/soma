$ErrorActionPreference='Stop'
$scoop=(Get-Command scoop -ErrorAction Stop).Source
& $scoop update
if($LASTEXITCODE -ne 0){exit $LASTEXITCODE}
& $scoop update soma
exit $LASTEXITCODE

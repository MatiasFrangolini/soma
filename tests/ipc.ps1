#requires -Version 7.0
param([Parameter(Mandatory)][string]$BinaryDirectory,[Parameter(Mandatory)][string]$Workspace)
$ErrorActionPreference='Stop'
$PSNativeCommandUseErrorActionPreference=$false
if(Test-Path -LiteralPath $Workspace){throw 'Use a fresh disposable workspace'}
$data=Join-Path $Workspace 'data'
$clientDirectory=Join-Path $Workspace 'client-only'
New-Item -ItemType Directory -Path $data,$clientDirectory,(Join-Path $data 'logs'),(Join-Path $data 'builds') -Force|Out-Null
# A missing sibling GUI makes an accidental launch observable without opening a dialog.
$cli=Join-Path $clientDirectory 'soma-cli.exe'
Copy-Item -LiteralPath (Join-Path $BinaryDirectory 'soma-cli.exe') -Destination $cli
$fullCli=Join-Path $BinaryDirectory 'soma-cli.exe'
$settings=@{schema_version=1;projects=@(@{id='probe';name='IPC fixture';directory=$Workspace;configs=@(@{id='local';name='local';env=@{};run=@{kind='command';executable='cmd.exe';args=@('/d','/c','exit','0')}})})}
$settings|ConvertTo-Json -Depth 10|Set-Content -LiteralPath (Join-Path $data 'projects.json')
$configurationHash=(Get-FileHash -LiteralPath (Join-Path $data 'projects.json')).Hash
$digest=[Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($data.ToLowerInvariant()))).ToLowerInvariant()
$pipeName=('\\.\pipe\Soma-'+$digest).Substring(0,41)
$mutexName='Local\'+$pipeName.Replace('\','-')
Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
public static class SomaIpcFixture {
    [StructLayout(LayoutKind.Sequential)] struct SecurityAttributes { public int Length; public IntPtr Descriptor; public int Inherit; }
    [DllImport("advapi32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern bool ConvertStringSecurityDescriptorToSecurityDescriptorW(string sddl,uint revision,out IntPtr descriptor,IntPtr size);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern IntPtr CreateNamedPipeW(string name,uint access,uint mode,uint instances,uint output,uint input,uint timeout,ref SecurityAttributes security);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern IntPtr CreateMutexW(ref SecurityAttributes security,bool owner,string name);
    [DllImport("kernel32.dll")] static extern IntPtr LocalFree(IntPtr memory);
    [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr handle);
    public static IntPtr Create(string name, bool pipe) {
        IntPtr descriptor;
        if(!ConvertStringSecurityDescriptorToSecurityDescriptorW("D:P(A;;GA;;;SY)",1,out descriptor,IntPtr.Zero)) throw new Win32Exception();
        try {
            var security=new SecurityAttributes {Length=Marshal.SizeOf<SecurityAttributes>(),Descriptor=descriptor};
            var handle=pipe ? CreateNamedPipeW(name,3,8,1,4096,4096,0,ref security) : CreateMutexW(ref security,false,name);
            if(handle==IntPtr.Zero || handle==new IntPtr(-1)) throw new Win32Exception();
            return handle;
        } finally {LocalFree(descriptor);}
    }
}
'@
$checks=[Collections.Generic.List[string]]::new()
function Assert([bool]$Condition,[string]$Description){if(-not $Condition){throw ('FAIL: '+$Description)};$checks.Add($Description);Write-Output ('PASS: '+$Description)}
function Invoke-Cli([string[]]$Arguments,[string]$Executable=$cli){
    $raw=& $Executable @Arguments --data-dir $data
    $code=$LASTEXITCODE
    try{$value=$raw|ConvertFrom-Json}catch{throw "Invalid CLI JSON: $raw"}
    [pscustomobject]@{Code=$code;Value=$value}
}
$pipe=[IntPtr]::Zero
$mutex=[IntPtr]::Zero
$opened=$false
try {
    $pipe=[SomaIpcFixture]::Create($pipeName,$true)
    $denied=$false
    try {$probe=[IO.File]::Open($pipeName,[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite);$probe.Dispose()}
    catch { $errorValue=$_.Exception;while($errorValue.InnerException){$errorValue=$errorValue.InnerException};$denied=($errorValue.HResult -band 65535) -eq 5 }
    Assert $denied 'The disposable pipe really denies this Windows identity with error 5'
    $commands=@(
        @('status'),@('stop'),@('close'),@('logs'),@('wait','--operation','fixture','--timeout','1'),
        @('show'),@('build','--project','probe'),@('run','--project','probe'),@('restart'),
        @('switch','--project','probe','--branch','unused'),@('jobs','list'),
        @('jobs','status','--job','fixture'),@('jobs','wait','--job','fixture','--timeout','1'),
        @('jobs','logs','--job','fixture'),@('jobs','release','--job','fixture','--participant','fixture'),
        @('jobs','submit','--path',$Workspace,'--config','tests','--kind','test','--','test')
    )
    foreach($arguments in $commands){
        $result=Invoke-Cli $arguments
        Assert ($result.Code -ne 0 -and $result.Value.ok -eq $false -and $result.Value.code -eq 'access_denied' -and $result.Value.os_error -eq 5 -and $result.Value.next_action -eq 'request_authorized_execution') (($arguments -join ' ')+' preserves permission denial and identifies the authorized recovery without launching a GUI or reporting a stopped owner; received '+($result.Value|ConvertTo-Json -Compress -Depth 5))
    }
    $mutex=[SomaIpcFixture]::Create($mutexName,$false)
    $gui=Start-Process -FilePath (Join-Path $BinaryDirectory 'Soma.exe') -ArgumentList @('--data-dir',('"'+$data+'"')) -WindowStyle Hidden -PassThru
    try {
        Assert ($gui.WaitForExit(5000)) 'Direct GUI invocation exits on denied ownership without a modal dialog'
    } finally {
        if(-not $gui.HasExited){$null=$gui.CloseMainWindow()}
        $gui.Dispose()
    }
    Assert ((Get-FileHash -LiteralPath (Join-Path $data 'projects.json')).Hash -eq $configurationHash -and -not (Test-Path (Join-Path $data 'jobs'))) 'Denied ownership does not initialize or rewrite job history'
    $null=[SomaIpcFixture]::CloseHandle($mutex);$mutex=[IntPtr]::Zero
    $null=[SomaIpcFixture]::CloseHandle($pipe);$pipe=[IntPtr]::Zero
    $closed=Invoke-Cli @('status')
    Assert ($closed.Code -eq 0 -and $closed.Value.status.state -eq 'closed') 'A genuinely absent pipe still reports closed'
    Assert ((Invoke-Cli @('logs')).Value.source -eq 'disk') 'Absent owner still permits historical disk logs'
    Assert ((Invoke-Cli @('wait','--operation','fixture','--timeout','1')).Value.code -eq 'stopped') 'Wait reports stopped only for an absent owner'
    $busyServer=[IO.Pipes.NamedPipeServerStream]::new($pipeName.Substring(9),[IO.Pipes.PipeDirection]::InOut,1,[IO.Pipes.PipeTransmissionMode]::Byte,[IO.Pipes.PipeOptions]::Asynchronous)
    $busyClient=[IO.Pipes.NamedPipeClientStream]::new('.',$pipeName.Substring(9),[IO.Pipes.PipeDirection]::InOut)
    try {
        $connection=$busyServer.WaitForConnectionAsync()
        $busyClient.Connect(1000)
        $connection.GetAwaiter().GetResult()
        $busy=Invoke-Cli @('show')
        Assert ($busy.Code -ne 0 -and $busy.Value.code -eq 'ipc_unavailable' -and $busy.Value.os_error -eq 231) 'A busy owner reports unavailable without trying to open another GUI'
    } finally {$busyClient.Dispose();$busyServer.Dispose()}
    $started=Invoke-Cli @('show') $fullCli
    $opened=$started.Value.ok -eq $true
    Assert $opened 'A genuinely closed owner can still be launched'
    Assert ((Invoke-Cli @('status') $fullCli).Value.status.state -eq 'idle') 'The newly launched disposable owner accepts IPC'
    Assert ((Invoke-Cli @('close') $fullCli).Value.ok) 'Only the disposable owner opened by this test is closed'
    $opened=$false
    $checks|ConvertTo-Json|Set-Content -LiteralPath (Join-Path $Workspace 'ipc-verification.json')
} finally {
    if($mutex -ne [IntPtr]::Zero){$null=[SomaIpcFixture]::CloseHandle($mutex)}
    if($pipe -ne [IntPtr]::Zero){$null=[SomaIpcFixture]::CloseHandle($pipe)}
    if($opened){$null=Invoke-Cli @('close') $fullCli}
}

[CmdletBinding()]
param(
    [string]$BinariesDirectory,
    [string]$DeviceId,
    [string]$RunnerBundleId = '',
    [ValidateRange(1,65535)][int]$Port = 18087,
    [switch]$AllowServiceRestart,
    [switch]$AllowUsbInterfaceRestart
)
$ErrorActionPreference='Stop'
if(-not $BinariesDirectory) {$BinariesDirectory=if(Test-Path -LiteralPath (Join-Path $PSScriptRoot 'rust-ios-device-daemon.exe')){$PSScriptRoot}else{Join-Path $PSScriptRoot '..\target\release'}}
$daemonPath=Join-Path $BinariesDirectory 'rust-ios-device-daemon.exe'
$guiPath=Join-Path $BinariesDirectory 'TetherDeck.exe'
if(-not (Test-Path -LiteralPath $guiPath)){ $guiPath=Join-Path $BinariesDirectory 'tetherdeck-desktop.exe' }
if(-not (Test-Path -LiteralPath $guiPath)){ $guiPath=Join-Path $BinariesDirectory 'rust-ios-device-gui.exe' }
foreach($programPath in @($daemonPath,$guiPath)) {if(-not (Test-Path -LiteralPath $programPath -PathType Leaf)){throw "Compilacao ausente: $programPath. Execute cargo build --release --workspace ou informe -BinariesDirectory."}}
if($RunnerBundleId -and $RunnerBundleId -notmatch '^[A-Za-z0-9][A-Za-z0-9.-]{1,254}$'){throw 'RunnerBundleId invalido.'}
if(-not $DeviceId) {
    $reportText=(& $daemonPath --doctor | Out-String)
    if($LASTEXITCODE -ne 0){throw 'O diagnostico local falhou.'}
    $report=$reportText|ConvertFrom-Json
    $candidates=@($report.devices|Where-Object {$_.udid -and $_.identity_status -eq 'available'}|Group-Object udid|ForEach-Object {$_.Group[0]})
    if($candidates.Count -eq 0){
        $savedToken=$env:RUST_IOS_DEVICE_TOKEN;$savedReport=$env:RUST_IOS_DEVICE_REPORT
        try {$env:RUST_IOS_DEVICE_TOKEN=$null;$env:RUST_IOS_DEVICE_REPORT=$null;$diagnosticGui=Start-Process -FilePath $guiPath -WindowStyle Normal -PassThru;$diagnosticGui.WaitForExit()} finally {$env:RUST_IOS_DEVICE_TOKEN=$savedToken;$env:RUST_IOS_DEVICE_REPORT=$savedReport}
        return
    }
    if($candidates.Count -ne 1){throw 'Ha mais de um iPhone. Informe -DeviceId com o UDID correto.'}
    $DeviceId=$candidates[0].udid
}
if($DeviceId -notmatch '^[A-Za-z0-9-]{1,128}$'){throw 'DeviceId invalido.'}
$listener=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,$Port)
try {$listener.Start()} catch {throw "A porta local $Port ja esta ocupada. Feche a instancia anterior ou escolha -Port."} finally {$listener.Stop()}
$previousToken=$env:RUST_IOS_DEVICE_TOKEN
$previousUrl=$env:RUST_IOS_DEVICE_URL
$previousReport=$env:RUST_IOS_DEVICE_REPORT
$previousLog=$env:RUST_LOG
$daemon=$null
try {
    $env:RUST_IOS_DEVICE_TOKEN=[Guid]::NewGuid().ToString('N')+[Guid]::NewGuid().ToString('N')
    $env:RUST_IOS_DEVICE_URL="http://127.0.0.1:$Port"
    $env:RUST_IOS_DEVICE_REPORT=$null
    $env:RUST_LOG='info'
    $logDirectory=Join-Path $env:LOCALAPPDATA 'RustIosDevice\logs'
    New-Item -ItemType Directory -Path $logDirectory -Force|Out-Null
    $runId=[Guid]::NewGuid().ToString('N')
    $arguments=@('--listen',"127.0.0.1:$Port",'--device',$DeviceId)
    if($RunnerBundleId){$arguments+=@('--wda-runner',$RunnerBundleId,'--require-wda')}
    if($AllowServiceRestart){$arguments+='--allow-service-restart'}
    if($AllowUsbInterfaceRestart){$arguments+='--allow-usb-interface-restart'}
    $daemon=Start-Process -FilePath $daemonPath -ArgumentList $arguments -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $logDirectory "$runId.stdout.log") -RedirectStandardError (Join-Path $logDirectory "$runId.stderr.log")
    $deadline=[DateTime]::UtcNow.AddSeconds(150)
    $ready=$false
    while([DateTime]::UtcNow -lt $deadline) {
        if($daemon.HasExited){throw "O daemon encerrou. Consulte os logs em $logDirectory."}
        try {$health=Invoke-RestMethod ($env:RUST_IOS_DEVICE_URL+'/v1/health') -TimeoutSec 1; if($health.api_version -eq 1){$ready=$true;break}} catch {}
        Start-Sleep -Milliseconds 250
    }
    if(-not $ready){throw "O daemon nao iniciou dentro do prazo. Consulte $logDirectory."}
    $gui=Start-Process -FilePath $guiPath -WindowStyle Normal -PassThru
    $gui.WaitForExit()
} finally {
    # This process object is only the child created above; existing instances are never stopped.
    if($null -ne $daemon -and -not $daemon.HasExited){$daemon.Kill();$daemon.WaitForExit()}
    $env:RUST_IOS_DEVICE_TOKEN=$previousToken
    $env:RUST_IOS_DEVICE_URL=$previousUrl
    $env:RUST_IOS_DEVICE_REPORT=$previousReport
    $env:RUST_LOG=$previousLog
}

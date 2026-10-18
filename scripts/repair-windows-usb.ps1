# Optional Windows support utility. No driver installation, removal, or pairing reset.
[CmdletBinding()]
param(
    [string]$InstanceId,
    [switch]$RestartAppleInterface
)
$ErrorActionPreference = 'Stop'
$appleInterfaces = @(Get-PnpDevice -PresentOnly | Where-Object {
    $_.Class -eq 'USBDevice' -and $_.InstanceId -match '^USB\\VID_05AC&PID_[0-9A-F]{4}&MI_[0-9A-F]{2}\\'
})
if (-not $RestartAppleInterface) {
    [pscustomobject]@{ action = 'inventory'; interfaces = @($appleInterfaces | Select-Object Status, Class, FriendlyName, InstanceId) } | ConvertTo-Json -Depth 4
    exit 0
}
if ([string]::IsNullOrWhiteSpace($InstanceId)) { throw 'Specify an exact InstanceId from the inventory; no wildcard or automatic selection is allowed.' }
$target = @($appleInterfaces | Where-Object { $_.InstanceId -eq $InstanceId })
if ($target.Count -ne 1) { throw 'The selected instance is not one currently present Apple USB interface.' }
$principal = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Administrator privileges are required to restart this USB interface.' }
$result = & "$env:SystemRoot\System32\pnputil.exe" /restart-device $target[0].InstanceId 2>&1
$resultCode = $LASTEXITCODE
[pscustomobject]@{
    action = 'restart_apple_usb_interface'
    instance_id = $target[0].InstanceId
    exit_code = $resultCode
    outcome = if ($resultCode -eq 0) { 'completed' } elseif ($resultCode -eq 3010) { 'reboot_required' } else { 'failed' }
    message = ($result -join [Environment]::NewLine)
    next_step = 'Run rust-ios-device-daemon --doctor. A successful PnP restart does not prove phone connectivity.'
} | ConvertTo-Json -Depth 4
if ($resultCode -ne 0) { exit 1 }

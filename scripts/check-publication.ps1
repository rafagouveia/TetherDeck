[CmdletBinding()]
param([switch]$IncludeHistory, [switch]$IncludeVendor)
$ErrorActionPreference = 'Stop'
$taskRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$taskFindings = [System.Collections.Generic.List[object]]::new()
$taskDevicePattern = '\b[0-9A-Fa-f]{8}-[0-9A-Fa-f]{16}\b'
$taskPrivateIpPattern = '\b(?:10(?:\.[0-9]{1,3}){3}|192\.168(?:\.[0-9]{1,3}){2}|172\.(?:1[6-9]|2[0-9]|3[01])(?:\.[0-9]{1,3}){2})\b'
$taskSecretExtensions = @('.key','.pem','.p8','.p12','.pfx','.cer','.crt','.der','.mobileprovision','.provisionprofile','.ipa')
$taskTextExtensions = @('.rs','.toml','.md','.txt','.json','.yaml','.yml','.ps1','.sh','.svg','.py')
$taskPatterns = [ordered]@{
    'private-key-header' = '-----BEGIN (?:[A-Z0-9 ]+ )?PRIVATE KEY-----'
    'credential-token-prefix' = '\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{25,}|sk-(?:proj-)?[A-Za-z0-9_-]{24,})\b'
    'device-identifier' = $taskDevicePattern
    'personal-windows-path' = '(?i)C:[\\/]+Users[\\/]+(?!Public\b|USERNAME\b|User\b|you\b|<)[^\\/\s"'']+'
}
function Redact-TaskPath([string]$Path) {
    $taskSafe = [regex]::Replace($Path, $taskDevicePattern, '[device-id]')
    $taskSafe = [regex]::Replace($taskSafe, '\b[0-9A-Fa-f]{40}\b', '[identifier]')
    $taskSafe = [regex]::Replace($taskSafe, $taskPatterns['credential-token-prefix'], '[token]')
    [regex]::Replace($taskSafe, $taskPrivateIpPattern, '[private-ip]')
}
function Add-TaskFinding([string]$Path,[string]$Kind,[string]$Revision='working-tree') {
    $taskFindings.Add([pscustomobject]@{Revision=$Revision;Path=(Redact-TaskPath $Path);Kind=$Kind})
}
function Test-TaskExcluded([string]$Path,[string]$Revision) {
    if (($Path -match '(^|/)(work|outputs|target|\.zsign_cache|\.pymobiledevice3|pair-records)/') -or
        (($Path -match '(^|/)\.env($|\.)') -and ($Path -notmatch '\.env\.example$')) -or
        ($Path -match '(^|/)\.cargo/credentials(\.toml)?$') -or
        ($taskSecretExtensions -contains [IO.Path]::GetExtension($Path).ToLowerInvariant())) {
        Add-TaskFinding $Path 'private-or-generated-file' $Revision
        return $true
    }
    return $false
}
function Test-TaskText([string]$Path) {
    ($taskTextExtensions -contains [IO.Path]::GetExtension($Path).ToLowerInvariant()) -or
        ([IO.Path]::GetFileName($Path) -in @('LICENSE','Cargo.lock','.gitignore'))
}
function Inspect-TaskText([string]$Path,[string]$Text,[string]$Revision) {
    foreach ($taskKind in $taskPatterns.Keys) {
        if ([regex]::IsMatch($Text,$taskPatterns[$taskKind])) { Add-TaskFinding $Path $taskKind $Revision }
    }
}

$taskFiles = @(& git -C $taskRoot -c core.quotepath=false ls-files --cached --others --exclude-standard | Sort-Object -Unique)
if ($LASTEXITCODE -ne 0) { throw 'Could not enumerate the repository source candidates.' }
foreach ($taskRelative in $taskFiles) {
    if (Test-TaskExcluded $taskRelative 'working-tree') { continue }
    if (-not $IncludeVendor -and $taskRelative.StartsWith('vendor/')) { continue }
    if (-not (Test-TaskText $taskRelative)) { continue }
    $taskPath = Join-Path $taskRoot $taskRelative
    if (-not (Test-Path -LiteralPath $taskPath -PathType Leaf)) { Add-TaskFinding $taskRelative 'unresolved-file-name'; continue }
    if ((Get-Item -LiteralPath $taskPath).Length -gt 4MB) { Add-TaskFinding $taskRelative 'oversized-text-needs-review'; continue }
    # Certificate/key/profile files were excluded above; never read their content.
    Inspect-TaskText $taskRelative ([IO.File]::ReadAllText($taskPath)) 'working-tree'
}
if ($IncludeHistory) {
    $taskCommits = @(& git -C $taskRoot rev-list --all)
    if ($LASTEXITCODE -ne 0) { throw 'Could not enumerate history.' }
    foreach ($taskCommit in $taskCommits) {
        $taskHistoryFiles = @(& git -C $taskRoot -c core.quotepath=false ls-tree -r --name-only $taskCommit)
        if ($LASTEXITCODE -ne 0) { throw 'Could not enumerate a historical source tree.' }
        foreach ($taskRelative in $taskHistoryFiles) {
            if (Test-TaskExcluded $taskRelative $taskCommit.Substring(0,8)) { continue }
            if (-not $IncludeVendor -and $taskRelative.StartsWith('vendor/')) { continue }
            if (-not (Test-TaskText $taskRelative)) { continue }
            $taskText = (& git -C $taskRoot show "${taskCommit}:$taskRelative" | Out-String)
            if ($LASTEXITCODE -ne 0) { throw 'Could not inspect a historical source file.' }
            Inspect-TaskText $taskRelative $taskText $taskCommit.Substring(0,8)
        }
    }
    $taskEmails = @(& git -C $taskRoot log --all '--format=%aE%n%cE' | Sort-Object -Unique)
    $taskPersonalCount = @($taskEmails | Where-Object { $_ -and $_ -notmatch '(?i)noreply|example\.|localhost|^codex@' }).Count
    if ($taskPersonalCount -gt 0) { Write-Warning "History contains $taskPersonalCount non-placeholder email identity/identities. Review public identity exposure; values are intentionally omitted." }
}
if ($taskFindings.Count -gt 0) {
    $taskFindings | Sort-Object Revision,Path,Kind -Unique | Format-Table -AutoSize
    Write-Error 'Publication review required. Only file paths and finding types were printed; matched values were not printed.'
    exit 1
}
Write-Output "Publication pattern check: no blocking findings in $($taskFiles.Count) source candidates. This bounded pattern check is not a full secret or license audit."
if (-not $IncludeVendor) { Write-Output 'Vendor content was excluded from pattern checks; review its licenses, provenance and unchanged upstream fixtures separately.' }

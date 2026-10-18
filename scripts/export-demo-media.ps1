[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$FramesDirectory,
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '../docs/media'),
    [string]$Ffmpeg = 'ffmpeg'
)
$ErrorActionPreference = 'Stop'
$taskFrames = (Resolve-Path -LiteralPath $FramesDirectory).Path
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$taskOutput = (Resolve-Path -LiteralPath $OutputDirectory).Path
$taskJobs = @(
    @{ Prefix='workflow'; Name='workflow.gif'; Scale='scale=1000:-2:flags=lanczos' },
    @{ Prefix='remote'; Name='remote-control.gif'; Scale='scale=420:780:force_original_aspect_ratio=decrease:flags=lanczos,pad=420:780:(ow-iw)/2:(oh-ih)/2:color=0x282c33' }
)
foreach ($taskJob in $taskJobs) {
    if (-not (Test-Path -LiteralPath (Join-Path $taskFrames ($taskJob.Prefix+'-0000.jpg')))) {
        throw "Missing reviewed frame sequence: $($taskJob.Prefix)-0000.jpg"
    }
    # Documentation playback is sampled, with pauses shortened. It is not a benchmark.
    $taskFilter = $taskJob.Scale+',setsar=1,tpad=start_mode=clone:start_duration=0.6:stop_mode=clone:stop_duration=1,split[a][b];[a]palettegen=stats_mode=diff:reserve_transparent=0[p];[b][p]paletteuse=dither=sierra2_4a:diff_mode=rectangle'
    & $Ffmpeg -hide_banner -loglevel error -y -framerate 8 -i (Join-Path $taskFrames ($taskJob.Prefix+'-%04d.jpg')) -filter_complex $taskFilter -map_metadata -1 -loop 0 (Join-Path $taskOutput $taskJob.Name)
    if ($LASTEXITCODE -ne 0) { throw "Media export failed: $($taskJob.Name)" }
    Get-Item -LiteralPath (Join-Path $taskOutput $taskJob.Name) | Select-Object Name,Length
}

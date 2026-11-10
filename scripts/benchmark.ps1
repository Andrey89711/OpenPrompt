[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $Executable,

    [Parameter(Mandatory = $true)]
    [string] $Path,

    [ValidateRange(1, 200)]
    [int] $Iterations = 20
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$executablePath = (Resolve-Path -LiteralPath $Executable).ProviderPath
$workingPath = (Resolve-Path -LiteralPath $Path).ProviderPath
if (-not (Test-Path -LiteralPath $workingPath -PathType Container)) {
    throw '-Path must name a directory.'
}

function Measure-Render {
    param([string[]] $RenderArguments)
    $watch = [Diagnostics.Stopwatch]::StartNew()
    & $executablePath @RenderArguments | Out-Null
    $exitCode = $LASTEXITCODE
    $watch.Stop()
    if ($exitCode -ne 0) { throw "Render exited with code $exitCode." }
    return $watch.Elapsed.TotalMilliseconds
}

function Get-Summary {
    param([string] $Name, [double[]] $Samples)
    $sorted = @($Samples | Sort-Object)
    $middle = [int][Math]::Floor($sorted.Count / 2)
    $median = if ($sorted.Count % 2) { $sorted[$middle] } else { ($sorted[$middle - 1] + $sorted[$middle]) / 2 }
    $p95 = $sorted[[int][Math]::Ceiling($sorted.Count * 0.95) - 1]
    [PSCustomObject]@{
        Scenario = $Name
        Samples = $sorted.Count
        MedianMs = [Math]::Round($median, 3)
        P95Ms = [Math]::Round($p95, 3)
    }
}

$temporaryPath = Join-Path ([IO.Path]::GetTempPath()) ('openprompt-benchmark-' + [Guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($temporaryPath) | Out-Null
try {
    $baselineConfig = Join-Path $temporaryPath 'baseline.toml'
    [IO.File]::WriteAllText($baselineConfig, @'
[git]
enabled = false
[runtime]
enabled = false
[environment]
enabled = false
[session]
enabled = false
'@)
    $render = @('render', '--path', $workingPath, '--no-color')
    $baseline = @('--config', $baselineConfig) + $render

    Write-Host 'Full CLI process wall-clock times, including startup and configuration loading.'
    Write-Host 'Fresh bypasses the Git cache; warm uses the normal configuration and cache.'
    Write-Host 'Cold means first invocation observed here, not a flushed OS/filesystem cache.'
    $cold = Measure-Render -RenderArguments $render
    Get-Summary -Name 'Default first invocation (cold)' -Samples @($cold)

    # Warm each scenario once outside its measured sample set.
    Measure-Render -RenderArguments $baseline | Out-Null
    $baselineSamples = @(for ($i = 0; $i -lt $Iterations; $i++) {
        Measure-Render -RenderArguments $baseline
    })
    Get-Summary -Name 'Baseline (context modules disabled)' -Samples $baselineSamples

    $fresh = $render + @('--fresh')
    Measure-Render -RenderArguments $fresh | Out-Null
    $freshSamples = @(for ($i = 0; $i -lt $Iterations; $i++) {
        Measure-Render -RenderArguments $fresh
    })
    Get-Summary -Name 'Default fresh (Git cache bypassed)' -Samples $freshSamples

    Measure-Render -RenderArguments $render | Out-Null
    $warmSamples = @(for ($i = 0; $i -lt $Iterations; $i++) {
        Measure-Render -RenderArguments $render
    })
    Get-Summary -Name 'Default warm (normal cache policy)' -Samples $warmSamples
} finally {
    # No user configuration is written or changed.
    [IO.Directory]::Delete($temporaryPath, $true)
}

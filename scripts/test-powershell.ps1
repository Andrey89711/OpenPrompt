[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Executable
)

$ErrorActionPreference = 'Stop'
$resolvedExecutable = (Resolve-Path -LiteralPath $Executable).ProviderPath
$shellCandidates = @('powershell.exe', 'pwsh.exe', 'pwsh')
$shellPaths = @()
foreach ($candidate in $shellCandidates) {
    $shellCommand = Get-Command $candidate -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($null -ne $shellCommand -and $shellPaths -notcontains $shellCommand.Source) {
        $shellPaths += $shellCommand.Source
    }
}
if ($shellPaths.Count -eq 0) {
    throw 'No PowerShell executable was found.'
}

$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('openprompt-powershell-' + [Guid]::NewGuid().ToString('N'))
$unicodeMarker = ([string][char]0x4E2D) + [char]0x6587 + '-' + [char]0x00E9
$markerPath = Join-Path $testRoot ("openprompt-marker's-directory-" + $unicodeMarker)
$null = New-Item -ItemType Directory -Path $markerPath -Force

# Pass paths as encoded literals, not command-line interpolations. Every test
# runs in a fresh shell and no profile is read or written.
$testBody = @'
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
function Assert-True($Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}
$originalConsoleEncoding = [Console]::OutputEncoding
try {
# Force a legacy decoder so a missing UTF-8 override cannot pass accidentally.
[Console]::OutputEncoding = [Text.Encoding]::GetEncoding(437)
$expectedConsoleEncoding = [Console]::OutputEncoding
function global:prompt { 'original-prompt-marker> ' }
$originalPromptText = (Get-Item Function:\prompt).ScriptBlock.ToString()
$initLines = @(& $executable init powershell)
Assert-True ($LASTEXITCODE -eq 0) 'init powershell failed.'
$initText = [string]::Join([Environment]::NewLine, [string[]]$initLines)
$parseTokens = $null
$parseErrors = $null
$null = [System.Management.Automation.Language.Parser]::ParseInput($initText, [ref]$parseTokens, [ref]$parseErrors)
Assert-True ($parseErrors.Count -eq 0) 'The initialization script does not parse.'
Assert-True (-not $initText.Contains('__OPENPROMPT_EXECUTABLE__')) 'Executable placeholder was not replaced.'
Assert-True ($initText -notmatch '[^\x00-\x7F]') 'Initialization must contain ASCII only.'
Invoke-Expression $initText
Assert-True ($global:OpenPromptExecutable -eq $executable) 'Executable literal did not round-trip.'
Set-Location -LiteralPath $markerPath
$global:LASTEXITCODE = 37
$rendered = prompt
Assert-True ($global:LASTEXITCODE -eq 37) 'Rendering changed LASTEXITCODE.'
Assert-True ([Console]::OutputEncoding.Equals($expectedConsoleEncoding)) 'Rendering changed the console output encoding.'
$renderedText = [string]$rendered
$expectedMarker = Split-Path -Leaf $markerPath
Assert-True ($renderedText.Contains($expectedMarker)) 'Rendered prompt does not include the exact Unicode marker directory.'
Assert-True ($renderedText -ne ('PS ' + $PWD.Path + '> ')) 'Renderer unexpectedly used the fallback.'
Assert-True ($rendered -is [string]) 'Prompt must return one string, not an array.'

# A native failure must be visible, but a later successful cmdlet must not
# inherit the stale failure merely because LASTEXITCODE remains nonzero.
if ($env:OS -eq 'Windows_NT') {
    & $env:ComSpec /d /c exit 17
    $failedPrompt = prompt
    Assert-True ($failedPrompt.Contains('exit:17')) 'Native command failure was not rendered.'
    Assert-True ($global:LASTEXITCODE -eq 17) 'Native command exit code was not preserved.'
    $null = Get-Item -LiteralPath $markerPath
    $successfulPrompt = prompt
    Assert-True (-not $successfulPrompt.Contains('exit:')) 'Successful cmdlet inherited a stale native failure.'
}

$customConfig = Join-Path $markerPath 'custom.toml'
[IO.File]::WriteAllText($customConfig, "theme = 'minimal'`n[git]`nenabled = false`n")
$customInit = [string]::Join([Environment]::NewLine, [string[]]@(& $executable --config $customConfig init powershell))
Invoke-Expression $customInit
Assert-True ($global:OpenPromptConfig -eq $customConfig) 'Custom configuration path was not preserved.'
$customPrompt = prompt
Assert-True (-not $customPrompt.Contains("`n")) 'Custom one-line theme was not applied.'

Invoke-Expression $initText
Disable-OpenPrompt
Assert-True ((prompt) -eq 'original-prompt-marker> ') 'Repeated init overwrote the original prompt.'
Assert-True ((Get-Item Function:\prompt).ScriptBlock.ToString() -eq $originalPromptText) 'Original prompt script block was not restored.'

Invoke-Expression $initText
$global:OpenPromptExecutable = Join-Path $markerPath 'missing-openprompt-executable.exe'
$global:LASTEXITCODE = 43
$fallbackOutputOne = @(prompt 2>&1)
$fallbackOutputTwo = @(prompt 2>&1)
Assert-True ($fallbackOutputOne.Count -eq 1 -and $fallbackOutputTwo.Count -eq 1) 'Missing executable emitted extra output or errors.'
$fallbackOne = $fallbackOutputOne[0]
$fallbackTwo = $fallbackOutputTwo[0]
Assert-True ($global:LASTEXITCODE -eq 43) 'Fallback changed LASTEXITCODE.'
Assert-True ([Console]::OutputEncoding.Equals($expectedConsoleEncoding)) 'Fallback changed the console output encoding.'
Assert-True ($fallbackOne -eq ('PS ' + $PWD.Path + '> ')) 'Missing executable did not return a plain fallback.'
Assert-True ($fallbackTwo -eq $fallbackOne) 'Repeated fallback is inconsistent.'
Assert-True (-not $fallbackOne.Contains([string][char]27)) 'Fallback contains ANSI escapes.'

# A non-filesystem location must never be passed to the renderer.
Set-Location Env:
$providerFallback = prompt
Assert-True ($providerFallback.StartsWith('PS ')) 'Non-filesystem provider did not use fallback.'
Assert-True ($global:LASTEXITCODE -eq 43) 'Provider fallback changed LASTEXITCODE.'
Assert-True ([Console]::OutputEncoding.Equals($expectedConsoleEncoding)) 'Provider fallback changed the console output encoding.'
Disable-OpenPrompt
Write-Output 'PASS: PowerShell integration'
} finally {
    [Console]::OutputEncoding = $originalConsoleEncoding
}
'@

try {
    foreach ($shellPath in $shellPaths) {
        $prefix = '$executable = ' + "'" + $resolvedExecutable.Replace("'", "''") + "'" + [Environment]::NewLine
        $prefix += '$markerPath = ' + "'" + $markerPath.Replace("'", "''") + "'" + [Environment]::NewLine
        $prefix += '$env:XDG_CONFIG_HOME = ' + "'" + $testRoot.Replace("'", "''") + "'" + [Environment]::NewLine
        $encodedCommand = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($prefix + $testBody))
        $testOutput = @(& $shellPath -NoLogo -NoProfile -NonInteractive -OutputFormat Text -EncodedCommand $encodedCommand 2>&1)
        if ($LASTEXITCODE -ne 0) {
            throw ('PowerShell integration failed in ' + $shellPath + ':' + [Environment]::NewLine + ($testOutput -join [Environment]::NewLine))
        }
        Write-Output ($shellPath + ': ' + ($testOutput -join [Environment]::NewLine))
    }
} finally {
    # Only remove the unique directory created by this test run.
    Remove-Item -LiteralPath $testRoot -Recurse -Force -ErrorAction SilentlyContinue
}

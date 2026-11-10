# Evaluate with: Invoke-Expression (& openprompt init powershell | Out-String)
# This script never writes to a PowerShell profile.
$global:OpenPromptExecutable = __OPENPROMPT_EXECUTABLE__
$global:OpenPromptConfig = __OPENPROMPT_CONFIG__

# A display hint only: never use this environment marker for authorization.
# Check the actual Windows token once, rather than spawning whoami per prompt.
try {
    if ($env:OS -eq 'Windows_NT') {
        $openPromptIdentity = [Security.Principal.WindowsIdentity]::GetCurrent()
        try {
            $openPromptPrincipal = New-Object Security.Principal.WindowsPrincipal($openPromptIdentity)
            $env:OPENPROMPT_ADMIN = $(if ($openPromptPrincipal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { '1' } else { '0' })
        } finally { $openPromptIdentity.Dispose() }
    }
} catch {
    $env:OPENPROMPT_ADMIN = '0'
}

# Keep the first prompt even when initialization is evaluated more than once.
if ($null -eq (Get-Variable -Name OpenPromptOriginalPrompt -Scope Global -ErrorAction SilentlyContinue)) {
    $openPromptOriginal = Get-Item -Path Function:\prompt -ErrorAction SilentlyContinue
    $global:OpenPromptOriginalPrompt = @{
        Exists = ($null -ne $openPromptOriginal)
        ScriptBlock = $(if ($null -ne $openPromptOriginal) { $openPromptOriginal.ScriptBlock } else { $null })
    }
}

function global:Disable-OpenPrompt {
    if ($null -ne (Get-Variable -Name OpenPromptOriginalPrompt -Scope Global -ErrorAction SilentlyContinue)) {
        if ($global:OpenPromptOriginalPrompt.Exists) {
            Set-Item -Path Function:\global:prompt -Value $global:OpenPromptOriginalPrompt.ScriptBlock
        } else {
            Remove-Item -Path Function:\global:prompt -ErrorAction SilentlyContinue
        }
    }
}

function global:prompt {
    $openPromptPreviousSuccess = $?
    $openPromptExitVariable = Get-Variable -Name LASTEXITCODE -Scope Global -ErrorAction SilentlyContinue
    $openPromptSavedExitCode = $(if ($null -ne $openPromptExitVariable) { $openPromptExitVariable.Value } else { $null })
    $openPromptFallback = 'PS > '
    $openPromptSavedEncoding = $null
    $openPromptRestoreEncoding = $false

    try {
        # Keep command-discovery and host/provider failures quiet, including when
        # the caller uses a terminating error preference.
        $ErrorActionPreference = 'Stop'
        $openPromptSafePath = [regex]::Replace($PWD.Path, '[\p{Cc}\u061C\u200B-\u200F\u2028-\u202E\u2060-\u206F\uFEFF]', '')
        $openPromptFallback = 'PS ' + $openPromptSafePath + '> '
        if ($PWD.Provider.Name -ne 'FileSystem') {
            return $openPromptFallback
        }

        $openPromptStatus = 0
        if (-not $openPromptPreviousSuccess) {
            $openPromptStatus = 1
            if ($null -ne $openPromptSavedExitCode -and $openPromptSavedExitCode -ne 0) {
                $openPromptStatus = $openPromptSavedExitCode
            }
        }

        $openPromptDuration = [long]0
        try {
            $openPromptHistory = Get-History -Count 1 -ErrorAction Stop
            if ($null -ne $openPromptHistory) {
                $openPromptMilliseconds = ($openPromptHistory.EndExecutionTime - $openPromptHistory.StartExecutionTime).TotalMilliseconds
                if ($openPromptMilliseconds -gt 0) {
                    $openPromptDuration = [long][Math]::Floor($openPromptMilliseconds)
                }
            }
        } catch {
            $openPromptDuration = [long]0
        }

        $openPromptWidth = 80
        try {
            $openPromptHostWidth = $Host.UI.RawUI.WindowSize.Width
            if ($openPromptHostWidth -gt 0) {
                $openPromptWidth = $openPromptHostWidth
            }
        } catch {
            $openPromptWidth = 80
        }

        # Windows PowerShell decodes native stdout using the console encoding.
        # Some hosts have no console; encoding access must not break the prompt.
        try {
            $openPromptSavedEncoding = [Console]::OutputEncoding
            $openPromptRestoreEncoding = $true
            [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
        } catch {
            # Continue with the host's decoding when console access is unavailable.
        }

        $openPromptConfigArgs = @()
        if ($null -ne $global:OpenPromptConfig) {
            $openPromptConfigArgs = @('--config', $global:OpenPromptConfig)
        }
        $openPromptLines = @(& $global:OpenPromptExecutable @openPromptConfigArgs render --shell powershell --path $PWD.Path --status $openPromptStatus --duration $openPromptDuration --width $openPromptWidth 2>$null)
        if ($LASTEXITCODE -ne 0 -or $openPromptLines.Count -eq 0) {
            return $openPromptFallback
        }
        return [string]::Join([Environment]::NewLine, [string[]]$openPromptLines)
    } catch {
        return $openPromptFallback
    } finally {
        if ($openPromptRestoreEncoding) {
            try {
                [Console]::OutputEncoding = $openPromptSavedEncoding
            } catch {
                # A detached or unavailable console must not emit prompt errors.
            }
        }
        # A renderer is a native process; it must not replace the user's code.
        if ($null -ne $openPromptExitVariable) {
            $global:LASTEXITCODE = $openPromptSavedExitCode
        } else {
            Remove-Variable -Name LASTEXITCODE -Scope Global -ErrorAction SilentlyContinue
        }
    }
}

# Installs only a local, independently verified Windows release archive.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $Archive,

    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9a-fA-F]{64}$')]
    [string] $Sha256,

    [string] $Destination = (Join-Path $HOME '.local/bin'),

    [switch] $Force
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression

$archivePath = (Resolve-Path -LiteralPath $Archive).ProviderPath
if ([IO.Path]::GetExtension($archivePath) -ine '.zip') {
    throw 'Archive must be a local .zip file.'
}
$destinationPath = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Destination)
$executablePath = Join-Path $destinationPath 'openprompt.exe'
$licensePath = Join-Path $destinationPath 'openprompt-LICENSE.txt'
$noticesPath = Join-Path $destinationPath 'openprompt-THIRD_PARTY_NOTICES.txt'
foreach ($installedFile in @($executablePath, $licensePath, $noticesPath)) {
    if ((Test-Path -LiteralPath $installedFile) -and -not $Force) {
        throw "Installation file already exists: $installedFile. Use -Force to replace it."
    }
}

$stream = $null
$zip = $null
$temporaryPath = $null
try {
    # Keep this same file open read-only through hashing and extraction, so the
    # bytes extracted are the bytes that were verified.
    $stream = [IO.File]::Open($archivePath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    $hasher = [Security.Cryptography.SHA256]::Create()
    try {
        $actualHash = [BitConverter]::ToString($hasher.ComputeHash($stream)).Replace('-', '')
    } finally {
        $hasher.Dispose()
    }
    if ($actualHash -ine $Sha256) {
        throw "SHA256 mismatch. Expected $Sha256; got $actualHash. Nothing was installed."
    }

    $stream.Position = 0
    $zip = [IO.Compression.ZipArchive]::new($stream, [IO.Compression.ZipArchiveMode]::Read, $true)
    $expected = @('openprompt.exe', 'LICENSE', 'README.md', 'THIRD_PARTY_NOTICES.txt')
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($entry in $zip.Entries) {
        # Exact top-level names exclude traversal, absolute paths, directories,
        # alternate separators, alternate data streams, and unexpected files.
        if ($expected -cnotcontains $entry.FullName -or -not $seen.Add($entry.FullName)) {
            throw "Unexpected or duplicate ZIP entry: $($entry.FullName)"
        }
        $unixType = ($entry.ExternalAttributes -shr 16) -band 0xF000
        if (($unixType -ne 0 -and $unixType -ne 0x8000) -or (($entry.ExternalAttributes -band 0x400) -ne 0)) {
            throw "Links and special files are not allowed: $($entry.FullName)"
        }
        if ($entry.Length -gt 100MB) {
            throw "ZIP entry is too large: $($entry.FullName)"
        }
    }
    if ($seen.Count -ne $expected.Count) {
        throw 'ZIP must contain exactly openprompt.exe, LICENSE, README.md, and THIRD_PARTY_NOTICES.txt.'
    }

    $temporaryPath = Join-Path ([IO.Path]::GetTempPath()) ('openprompt-install-' + [Guid]::NewGuid().ToString('N'))
    [IO.Directory]::CreateDirectory($temporaryPath) | Out-Null
    foreach ($entry in $zip.Entries) {
        $inputStream = $entry.Open()
        $outputStream = $null
        try {
            $outputStream = [IO.File]::Open((Join-Path $temporaryPath $entry.FullName), [IO.FileMode]::CreateNew)
            $inputStream.CopyTo($outputStream)
        } finally {
            if ($null -ne $outputStream) { $outputStream.Dispose() }
            $inputStream.Dispose()
        }
    }
    [IO.Directory]::CreateDirectory($destinationPath) | Out-Null
    [IO.File]::Copy((Join-Path $temporaryPath 'LICENSE'), $licensePath, $Force.IsPresent)
    [IO.File]::Copy((Join-Path $temporaryPath 'THIRD_PARTY_NOTICES.txt'), $noticesPath, $Force.IsPresent)
    [IO.File]::Copy((Join-Path $temporaryPath 'openprompt.exe'), $executablePath, $Force.IsPresent)
    Write-Output "Installed $executablePath"
    Write-Output 'PATH and shell profiles were not changed.'
} finally {
    if ($null -ne $zip) { $zip.Dispose() }
    if ($null -ne $stream) { $stream.Dispose() }
    if ($null -ne $temporaryPath -and [IO.Directory]::Exists($temporaryPath)) {
        # Only this invocation's randomly named staging directory is removed.
        [IO.Directory]::Delete($temporaryPath, $true)
    }
}

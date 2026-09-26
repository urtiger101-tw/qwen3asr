[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $scriptDir ".."))
$manifest = Join-Path $repoRoot "Cargo.toml"
$binary = Join-Path $repoRoot "target\release\qwen3asr.exe"

try {
    if (-not (Test-Path -LiteralPath $manifest -PathType Leaf)) {
        throw "Cargo.toml was not found at $manifest"
    }

    $cargo = Get-Command cargo -ErrorAction Stop
    Push-Location -LiteralPath $repoRoot
    try {
        & $cargo.Source build --release --locked --manifest-path $manifest
        $buildExitCode = $LASTEXITCODE
        if ($buildExitCode -ne 0) {
            throw "cargo build failed with exit code $buildExitCode"
        }
    }
    finally {
        Pop-Location
    }

    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
        throw "The release binary was not created at $binary"
    }

    $manifestText = Get-Content -LiteralPath $manifest -Raw
    $versionMatch = [regex]::Match($manifestText, '(?m)^version\s*=\s*"([^"]+)"')
    if (-not $versionMatch.Success) {
        throw "Could not read the package version from Cargo.toml"
    }
    $expectedVersion = $versionMatch.Groups[1].Value
    $reportedVersion = & $binary --version
    $versionExitCode = $LASTEXITCODE
    if ($versionExitCode -ne 0 -or $reportedVersion.Trim() -ne "qwen3asr $expectedVersion") {
        throw "The release CLI version check failed. Expected qwen3asr $expectedVersion; got '$($reportedVersion.Trim())'."
    }

    Write-Output "Built native CLI $reportedVersion at $binary"
}
catch {
    [Console]::Error.WriteLine("Build failed: {0}", $_.Exception.Message)
    exit 1
}

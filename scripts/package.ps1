[CmdletBinding()]
param(
    [string]$IsccPath,
    [string]$NativeRoot,
    [string]$OutputDir,
    [switch]$SkipBuild
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $scriptDir ".."))
$distDir = Join-Path $repoRoot "dist"
$stageRoot = Join-Path $repoRoot ".ag-artifacts\packaging-stage"
$stageName = "native-0.2.1-online-{0}-{1}" -f (Get-Date -Format "yyyyMMdd-HHmmss"), ([Guid]::NewGuid().ToString("N"))
$stageDir = Join-Path $stageRoot $stageName
$setupName = "qwen3asr-0.2.1-setup.exe"
$buildScript = Join-Path $scriptDir "build-launcher.ps1"
$installerScript = Join-Path $repoRoot "installer\qwen3asr.iss"

function Resolve-IsccCandidate([string]$Candidate) {
    if ([string]::IsNullOrWhiteSpace($Candidate)) {
        return $null
    }

    $candidatePath = [System.IO.Path]::GetFullPath($Candidate)
    if (Test-Path -LiteralPath $candidatePath -PathType Container) {
        $candidatePath = Join-Path $candidatePath "ISCC.exe"
    }
    if (Test-Path -LiteralPath $candidatePath -PathType Leaf) {
        return $candidatePath
    }
    return $null
}

function Find-IsccPath([string]$ExplicitPath) {
    if (-not [string]::IsNullOrWhiteSpace($ExplicitPath)) {
        $explicitResolved = Resolve-IsccCandidate $ExplicitPath
        if (-not $explicitResolved) {
            throw "The specified ISCC path does not exist: $ExplicitPath"
        }
        return $explicitResolved
    }

    $candidates = @()
    $pathCommand = Get-Command "ISCC.exe" -ErrorAction SilentlyContinue
    if ($pathCommand) {
        $candidates += $pathCommand.Source
    }
    foreach ($name in @("ISCC_PATH", "ISCC_EXE", "INNO_SETUP_HOME", "INNO_SETUP_ROOT")) {
        $value = [Environment]::GetEnvironmentVariable($name)
        if (-not [string]::IsNullOrWhiteSpace($value)) {
            $candidates += $value
        }
    }

    $programFiles = [Environment]::GetEnvironmentVariable("ProgramFiles")
    $programFilesX86 = [Environment]::GetEnvironmentVariable("ProgramFiles(x86)")
    $localAppData = [Environment]::GetEnvironmentVariable("LOCALAPPDATA")
    foreach ($basePath in @($programFiles, $programFilesX86, $localAppData)) {
        if ([string]::IsNullOrWhiteSpace($basePath)) {
            continue
        }
        if ($basePath -eq $localAppData) {
            $candidates += Join-Path $basePath "Programs\Inno Setup 7\ISCC.exe"
        }
        else {
            $candidates += Join-Path $basePath "Inno Setup 7\ISCC.exe"
        }
    }

    foreach ($candidate in $candidates) {
        $resolved = Resolve-IsccCandidate $candidate
        if ($resolved) {
            return $resolved
        }
    }
    throw "Inno Setup compiler was not found. Add ISCC.exe to PATH, set ISCC_PATH/INNO_SETUP_HOME, install under Program Files or LOCALAPPDATA, or pass -IsccPath."
}

function Resolve-RepoOrAbsolutePath([string]$Value, [string]$DefaultPath) {
    if ([string]::IsNullOrWhiteSpace($Value)) {
        return [System.IO.Path]::GetFullPath($DefaultPath)
    }
    if ([System.IO.Path]::IsPathRooted($Value)) {
        return [System.IO.Path]::GetFullPath($Value)
    }
    return [System.IO.Path]::GetFullPath((Join-Path $repoRoot $Value))
}

function Require-File([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Required packaging input is missing: $Path"
    }
}

function Copy-Tree([string]$Source, [string]$Destination, [string]$Description) {
    $null = New-Item -ItemType Directory -Path $Destination -Force
    $robocopy = Get-Command robocopy.exe -ErrorAction Stop
    & $robocopy.Source $Source $Destination /E /COPY:DAT /DCOPY:DAT /R:1 /W:1 /XJ /NFL /NDL /NJH /NJS /NP | Out-Null
    $copyExitCode = $LASTEXITCODE
    if ($copyExitCode -gt 7) {
        throw "Copying $Description failed with Robocopy exit code $copyExitCode"
    }
}

try {
    if ([string]::IsNullOrWhiteSpace($NativeRoot)) {
        $NativeRoot = Join-Path $repoRoot "native\bin"
    }
    $NativeRoot = Resolve-RepoOrAbsolutePath $NativeRoot (Join-Path $repoRoot "native\bin")
    $IsccPath = Find-IsccPath $IsccPath
    if ([string]::IsNullOrWhiteSpace($OutputDir)) {
        $OutputDir = $distDir
    }
    $OutputDir = [System.IO.Path]::GetFullPath($OutputDir)

    if (-not $SkipBuild) {
        $buildOutput = & $buildScript 2>&1
        $buildExitCode = $LASTEXITCODE
        if ($buildExitCode -and $buildExitCode -ne 0) {
            throw "Rust release build failed with exit code $buildExitCode. $($buildOutput -join [Environment]::NewLine)"
        }
    }

    $requiredFiles = @(
        (Join-Path $repoRoot "target\release\qwen3asr.exe"),
        (Join-Path $repoRoot "README.md"),
        (Join-Path $repoRoot "LICENSE"),
        (Join-Path $repoRoot "THIRD_PARTY_NOTICES.md"),
        (Join-Path $repoRoot "native\notices"),
        (Join-Path $repoRoot "docs\installation.md"),
        (Join-Path $repoRoot "skills\qwen3asr\SKILL.md"),
        (Join-Path $NativeRoot "licenses"),
        (Join-Path $NativeRoot "cuda\licenses\CUDA-EULA.txt"),
        $buildScript,
        $installerScript,
        $IsccPath
    )
    foreach ($requiredFile in $requiredFiles) {
        if (Test-Path -LiteralPath $requiredFile -PathType Container) {
            continue
        }
        Require-File $requiredFile
    }
    # -SkipBuild reuses a binary but never bypasses the release identity gate.
    $cliVersion = & (Join-Path $repoRoot "target\release\qwen3asr.exe") --version
    if ($LASTEXITCODE -ne 0 -or ($cliVersion -join "`n").Trim() -ne 'qwen3asr 0.2.1') {
        throw "The CLI binary does not match installer version 0.2.1. Build the current release first."
    }

    $expectedPatchHash = (Get-FileHash -LiteralPath (Join-Path $repoRoot "native\patches\audio-cpp-c7dbd4a.patch") -Algorithm SHA256).Hash
    $expectedWorkerHash = (Get-FileHash -LiteralPath (Join-Path $repoRoot "native\worker.cpp") -Algorithm SHA256).Hash
    $expectedCommit = 'c7dbd4a481db6aa8f6235ace03cdccfb07961f59'
    foreach ($backend in @("cpu", "cuda")) {
        $backendRoot = Join-Path $NativeRoot $backend
        if (-not (Test-Path -LiteralPath $backendRoot -PathType Container)) {
            throw "Native $backend stage is missing at $backendRoot. Build it with scripts/build-native.ps1 first."
        }
        Require-File (Join-Path $backendRoot "qwen3asr-worker.exe")
        $backendDlls = Get-ChildItem -LiteralPath $backendRoot -Filter "*.dll" -File -Recurse
        if (-not $backendDlls) {
            throw "Native $backend stage has no runtime DLLs: $backendRoot"
        }
        $provenancePath = Join-Path $backendRoot "provenance.json"
        Require-File $provenancePath
        $provenance = Get-Content -LiteralPath $provenancePath -Raw | ConvertFrom-Json
        if ($provenance.source_commit -ne $expectedCommit -or
            $provenance.backend -ne $backend -or
            $provenance.compatibility_patch_sha256 -ne $expectedPatchHash -or
            $provenance.worker_source_sha256 -ne $expectedWorkerHash) {
            throw "Native $backend stage is stale or from another source revision. Rebuild both backends."
        }
        $probeOutput = & (Join-Path $backendRoot "qwen3asr-worker.exe") --probe
        if ($LASTEXITCODE -ne 0) { throw "Native $backend probe failed" }
        $probe = ($probeOutput -join "`n") | ConvertFrom-Json
        if ($probe.ok -ne $true -or $probe.source_commit -ne $expectedCommit -or
            $probe.cpu_available -ne $true -or $probe.cuda_compiled -ne ($backend -eq 'cuda')) {
            throw "Native $backend probe is inconsistent with its package metadata"
        }
    }

    $stageFull = [System.IO.Path]::GetFullPath($stageDir)
    if (Test-Path -LiteralPath $stageFull) {
        throw "Refusing to reuse an existing staging directory: $stageFull"
    }
    $null = New-Item -ItemType Directory -Path $stageFull -Force
    $nativeStage = Join-Path $stageFull "native"
    Copy-Tree (Join-Path $NativeRoot "cpu") (Join-Path $nativeStage "cpu") "native CPU runtime"
    Copy-Tree (Join-Path $NativeRoot "cuda") (Join-Path $nativeStage "cuda") "native CUDA runtime"
    Copy-Tree (Join-Path $NativeRoot "licenses") (Join-Path $nativeStage "licenses") "native runtime license files"
    Copy-Tree (Join-Path $repoRoot "native\notices") (Join-Path $nativeStage "licenses") "project dependency license notices"

    # The verified upstream archive supplies the exact FFmpeg license at install time.
    $stagedFfmpegLicense = Join-Path $nativeStage "licenses\FFmpeg-LGPL-3.0.txt"
    if (Test-Path -LiteralPath $stagedFfmpegLicense) {
        Remove-Item -LiteralPath $stagedFfmpegLicense
    }

    Copy-Item -LiteralPath (Join-Path $repoRoot "target\release\qwen3asr.exe") -Destination (Join-Path $stageFull "qwen3asr.exe")
    Copy-Item -LiteralPath (Join-Path $repoRoot "README.md") -Destination (Join-Path $stageFull "README.md")
    Copy-Item -LiteralPath (Join-Path $repoRoot "LICENSE") -Destination (Join-Path $stageFull "LICENSE")
    Copy-Item -LiteralPath (Join-Path $repoRoot "THIRD_PARTY_NOTICES.md") -Destination (Join-Path $stageFull "THIRD_PARTY_NOTICES.md")

    $docsRoot = Join-Path $repoRoot "docs"
    $docsStage = Join-Path $stageFull "docs"
    Copy-Tree $docsRoot $docsStage "documentation"
    $skillStage = Join-Path $stageFull "skills\qwen3asr"
    $null = New-Item -ItemType Directory -Path $skillStage -Force
    Copy-Item -LiteralPath (Join-Path $repoRoot "skills\qwen3asr\SKILL.md") -Destination (Join-Path $skillStage "SKILL.md")

    $forbiddenPaths = @(
        (Join-Path $stageFull "python"),
        (Join-Path $stageFull "uv.exe"),
        (Join-Path $stageFull "requirements-runtime.txt"),
        (Join-Path $stageFull "qwen3asr")
    )
    foreach ($forbiddenPath in $forbiddenPaths) {
        if (Test-Path -LiteralPath $forbiddenPath) {
            throw "Unexpected legacy Python runtime content entered the native package stage: $forbiddenPath"
        }
    }

    $unexpectedMediaPayload = Get-ChildItem -LiteralPath $stageFull -Recurse -File | Where-Object {
        $_.Name -in @("ffmpeg.exe", "ffprobe.exe", "ffmpeg-n8.1.3-win64-lgpl-8.1.zip")
    }
    if ($unexpectedMediaPayload) {
        throw "Public online installer stage contains an FFmpeg binary/archive: $($unexpectedMediaPayload.FullName -join ', ')"
    }

    foreach ($backend in @("cpu", "cuda")) {
        $worker = Join-Path $nativeStage "$backend\qwen3asr-worker.exe"
        & $worker --probe
        $workerExitCode = $LASTEXITCODE
        if ($workerExitCode -ne 0) {
            throw "The staged $backend worker probe failed with exit code $workerExitCode"
        }
    }
    $null = New-Item -ItemType Directory -Path $OutputDir -Force
    $previousStageEnvironment = [Environment]::GetEnvironmentVariable("QWEN3ASR_PACKAGING_STAGE_DIR", "Process")
    try {
        $env:QWEN3ASR_PACKAGING_STAGE_DIR = $stageFull
        & $IsccPath "--output-dir=$OutputDir" $installerScript
        $isccExitCode = $LASTEXITCODE
    }
    finally {
        if ($null -eq $previousStageEnvironment) {
            Remove-Item Env:\QWEN3ASR_PACKAGING_STAGE_DIR -ErrorAction SilentlyContinue
        }
        else {
            $env:QWEN3ASR_PACKAGING_STAGE_DIR = $previousStageEnvironment
        }
    }

    if ($isccExitCode -ne 0) {
        throw "Inno Setup compilation failed with exit code $isccExitCode"
    }

    $setupPath = Join-Path $OutputDir $setupName
    Require-File $setupPath
    $hash = (Get-FileHash -LiteralPath $setupPath -Algorithm SHA256).Hash.ToLowerInvariant()
    Set-Content -LiteralPath (Join-Path $OutputDir "SHA256SUMS.txt") -Value "$hash  $setupName" -Encoding ascii

    Write-Output "Created $setupPath"
    Write-Output "SHA256 $hash"
    Write-Output "Staging retained at $stageFull"
}
catch {
    [Console]::Error.WriteLine("Packaging failed: {0}", $_.Exception.Message)
    exit 1
}

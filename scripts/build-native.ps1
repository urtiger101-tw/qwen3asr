[CmdletBinding()]
param(
    [ValidateSet('cpu', 'cuda', 'both')][string]$Backend = 'both',
    [string]$BuildRoot = '',
    [string]$SourceDirectory = '',
    [string]$AudioBuildDirectory = '',
    [string]$VsInstall = '',
    [string]$CudaMsvcToolset = '',
    [int]$Jobs = 8,
    [string]$CudaArchitectures = '86-real;86-virtual'
)

$ErrorActionPreference = 'Stop'
$sourceCommit = 'c7dbd4a481db6aa8f6235ace03cdccfb07961f59'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if ($BuildRoot -eq '') { $BuildRoot = Join-Path $repo 'native\.build' }
if ($SourceDirectory -eq '') { $SourceDirectory = Join-Path $BuildRoot 'audio.cpp' }
$patch = Join-Path $repo 'native\patches\audio-cpp-c7dbd4a.patch'

if (-not (Test-Path (Join-Path $SourceDirectory '.git'))) {
    New-Item -ItemType Directory -Force $SourceDirectory | Out-Null
    & git -C $SourceDirectory init
    if ($LASTEXITCODE -ne 0) { throw 'Cannot initialize audio.cpp checkout' }
    & git -C $SourceDirectory remote add origin 'https://github.com/0xShug0/audio.cpp.git'
    if ($LASTEXITCODE -ne 0) { throw 'Cannot set audio.cpp remote' }
    & git -C $SourceDirectory fetch --depth 1 origin $sourceCommit
    if ($LASTEXITCODE -ne 0) { throw 'Cannot fetch pinned audio.cpp commit' }
    & git -C $SourceDirectory checkout --detach FETCH_HEAD
    if ($LASTEXITCODE -ne 0) { throw 'Cannot checkout pinned audio.cpp commit' }
}
$actualCommit = (& git -C $SourceDirectory rev-parse HEAD).Trim()
if ($actualCommit -ne $sourceCommit) { throw "audio.cpp commit mismatch: $actualCommit" }

& git -C $SourceDirectory apply --reverse --check $patch 2>$null
if ($LASTEXITCODE -ne 0) {
    & git -C $SourceDirectory apply --check $patch
    if ($LASTEXITCODE -ne 0) { throw 'audio.cpp compatibility patch does not apply' }
    & git -C $SourceDirectory apply $patch
    if ($LASTEXITCODE -ne 0) { throw 'Cannot apply audio.cpp compatibility patch' }
}
New-Item -ItemType Directory -Force $BuildRoot | Out-Null
$actualPatch = Join-Path (Resolve-Path $BuildRoot).Path "audio.cpp-current-$PID.patch"
& git -C $SourceDirectory diff --binary --no-ext-diff HEAD "--output=$actualPatch"
if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect audio.cpp source diff' }
$expectedPatchHash = (Get-FileHash $patch -Algorithm SHA256).Hash
$actualPatchHash = (Get-FileHash $actualPatch -Algorithm SHA256).Hash
if ($actualPatchHash -ne $expectedPatchHash) {
    throw 'audio.cpp worktree differs from the checked-in compatibility patch'
}
Remove-Item -LiteralPath $actualPatch -Force
$untrackedSource = @(& git -C $SourceDirectory ls-files --others --exclude-standard)
if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect audio.cpp untracked files' }
if ($untrackedSource.Count -ne 0) {
    throw "audio.cpp source has unexpected untracked files: $($untrackedSource -join ', ')"
}

if ($VsInstall -eq '') {
    $vswhere = 'C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path $vswhere)) { throw 'Visual Studio C++ Build Tools are missing' }
    $VsInstall = (& $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath | Select-Object -First 1)
    if (-not $VsInstall) { throw 'Visual Studio C++ Build Tools are missing' }
}
$requestedVsInstall = $VsInstall

$targets = if ($Backend -eq 'both') { @('cpu', 'cuda') } else { @($Backend) }
if ($Backend -eq 'both' -and $AudioBuildDirectory -ne '') {
    throw '-AudioBuildDirectory can only be used with one backend'
}
$cudaToolkitRoot = if ($env:CUDA_PATH) { $env:CUDA_PATH } else {
    Join-Path $env:ProgramFiles 'NVIDIA GPU Computing Toolkit\CUDA\v12.8'
}
foreach ($targetBackend in $targets) {
    $preset = if ($targetBackend -eq 'cuda') { 'windows-cuda-release' } else { 'windows-cpu-release' }
    $selectedVsInstall = $requestedVsInstall
    $selectedToolset = ''
    $cudaMinCc = 0
    if ($targetBackend -eq 'cuda') {
        if (-not (Test-Path (Join-Path $cudaToolkitRoot 'bin\nvcc.exe'))) {
            throw 'Install CUDA Toolkit 12.8 or set CUDA_PATH to its root'
        }
        $nvccVersion = & (Join-Path $cudaToolkitRoot 'bin\nvcc.exe') --version
        if (-not ($nvccVersion -match 'release 12\.8')) {
            throw 'This pinned native build requires CUDA Toolkit 12.8'
        }
        foreach ($architecture in ($CudaArchitectures -split ';')) {
            if ($architecture -notmatch '^([0-9]+)(?:-(?:real|virtual))?$') {
                throw "Unsupported CUDA architecture specification: $architecture"
            }
            $candidateCc = [int]$Matches[1]
            if ($cudaMinCc -eq 0 -or $candidateCc -lt $cudaMinCc) { $cudaMinCc = $candidateCc }
        }
        $candidates = @($requestedVsInstall)
        if ($PSBoundParameters.ContainsKey('VsInstall') -eq $false) {
            $vswhere = 'C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe'
            $candidates = @(& $vswhere -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)
        }
        foreach ($candidate in $candidates) {
            $toolsetRoot = Join-Path $candidate 'VC\Tools\MSVC'
            foreach ($item in (Get-ChildItem $toolsetRoot -Directory -ErrorAction SilentlyContinue | Sort-Object Name -Descending)) {
                $compilerCandidate = Join-Path $item.FullName 'bin\Hostx64\x64\cl.exe'
                if (([version]$item.Name) -lt [version]'14.50' -and (Test-Path $compilerCandidate) -and
                    ($CudaMsvcToolset -eq '' -or $item.Name -eq $CudaMsvcToolset)) {
                    $selectedVsInstall = $candidate
                    $selectedToolset = $item.Name
                    break
                }
            }
            if ($selectedToolset -ne '') { break }
        }
        if ($selectedToolset -eq '') { throw 'CUDA 12.x requires an installed MSVC toolset earlier than 14.50' }
    }
    $audioBuild = if ($AudioBuildDirectory -ne '') { $AudioBuildDirectory } else { Join-Path $BuildRoot "audio.cpp-build-$targetBackend" }
    $workerBuild = Join-Path $BuildRoot "worker-build-$targetBackend"
    $stage = Join-Path $repo "native\bin\$targetBackend"
    $buildArgs = @{
        Preset = $preset; Target = 'audiocpp'; Jobs = $Jobs
        ModelSet = 'custom'; Models = 'qwen3_asr,qwen3_forced_aligner'
        VsInstall = $selectedVsInstall; BuildDirectory = $audioBuild
        BuildCAPI = $true; DeploymentBuild = $true
    }
    $buildArgs.CpuArch = 'baseline'
    if ($targetBackend -eq 'cuda') {
        $buildArgs.CudaArchitectures = $CudaArchitectures
        $buildArgs.MsvcToolset = $selectedToolset
    }
    & (Join-Path $SourceDirectory 'scripts\build_windows.ps1') @buildArgs
    if ($LASTEXITCODE -ne 0) { throw "audio.cpp $targetBackend build failed" }

    $compiler = (Get-Command cl.exe -ErrorAction Stop).Source
    $cmakeArgs = @(
        '-S', (Join-Path $repo 'native'), '-B', $workerBuild, '-G', 'Ninja',
        "-DCMAKE_C_COMPILER=$compiler", "-DCMAKE_CXX_COMPILER=$compiler",
        '-DCMAKE_BUILD_TYPE=Release',
        "-DAUDIOCPP_SOURCE_DIR=$SourceDirectory", "-DAUDIOCPP_BINARY_DIR=$audioBuild",
        "-DQWEN_AUDIOCPP_COMMIT=$sourceCommit",
        "-DNATIVE_CUDA_COMPILED=$(if ($targetBackend -eq 'cuda') { 'ON' } else { 'OFF' })",
        "-DNATIVE_CUDA_MIN_CC=$cudaMinCc"
    )
    & cmake @cmakeArgs
    if ($LASTEXITCODE -ne 0) { throw "Worker $targetBackend configure failed" }
    & cmake --build $workerBuild --config Release --parallel $Jobs
    if ($LASTEXITCODE -ne 0) { throw "Worker $targetBackend build failed" }

    New-Item -ItemType Directory -Force $stage | Out-Null
    Copy-Item -LiteralPath (Join-Path $workerBuild 'bin\qwen3asr-worker.exe') -Destination $stage -Force
    Get-ChildItem (Join-Path $audioBuild 'bin') -Filter '*.dll' | ForEach-Object {
        Copy-Item -LiteralPath $_.FullName -Destination $stage -Force
    }
    $redistRoot = Join-Path $selectedVsInstall 'VC\Redist\MSVC'
    $redistVersion = Get-ChildItem $redistRoot -Directory |
        Where-Object { $_.Name -match '^\d+\.\d+\.\d+$' } |
        Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
    if (-not $redistVersion) { throw "MSVC redistributable files are missing under $redistRoot" }
    foreach ($name in @('msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll', 'vcomp140.dll')) {
        $runtime = Get-ChildItem (Join-Path $redistVersion.FullName 'x64') -Recurse -File -Filter $name |
            Select-Object -First 1
        if (-not $runtime) { throw "MSVC redistributable is missing $name" }
        Copy-Item -LiteralPath $runtime.FullName -Destination $stage -Force
    }
    if ($targetBackend -eq 'cuda') {
        $cudaBin = Join-Path $cudaToolkitRoot 'bin'
        foreach ($name in @('cudart64_12.dll', 'cublas64_12.dll', 'cublasLt64_12.dll')) {
            $runtime = Join-Path $cudaBin $name
            if (-not (Test-Path $runtime)) { throw "CUDA 12.8 redistributable is missing $runtime" }
            Copy-Item -LiteralPath $runtime -Destination $stage -Force
        }
    }
    $licenses = Join-Path $stage 'licenses'
    New-Item -ItemType Directory -Force $licenses | Out-Null
    $msvcRedistNotice = Get-ChildItem (Join-Path $selectedVsInstall 'Licenses') -Recurse -File -Filter 'Redist.txt' |
        Select-Object -First 1
    if ($msvcRedistNotice) {
        Copy-Item -LiteralPath $msvcRedistNotice.FullName -Destination (Join-Path $licenses 'MSVC-Redist.txt') -Force
    }
    if ($targetBackend -eq 'cuda') {
        Copy-Item -LiteralPath (Join-Path $cudaToolkitRoot 'EULA.txt') -Destination (Join-Path $licenses 'CUDA-EULA.txt') -Force
    }
    $notices = @{
        'audio.cpp-APACHE-2.0.txt' = 'LICENSE'
        'ggml-MIT.txt' = 'external\ggml\LICENSE'
        'cJSON-MIT.txt' = 'external\cJSON\LICENSE'
        'sentencepiece-APACHE-2.0.txt' = 'external\sentencepiece\LICENSE'
        'sentencepiece-protobuf-lite.txt' = 'external\sentencepiece\third_party\protobuf-lite\LICENSE'
        'sentencepiece-esaxx.txt' = 'external\sentencepiece\third_party\esaxx\LICENSE'
        'sentencepiece-darts-clone.txt' = 'external\sentencepiece\third_party\darts_clone\LICENSE'
        'sentencepiece-absl.txt' = 'external\sentencepiece\third_party\absl\LICENSE'
    }
    foreach ($entry in $notices.GetEnumerator()) {
        Copy-Item -LiteralPath (Join-Path $SourceDirectory $entry.Value) -Destination (Join-Path $licenses $entry.Key) -Force
    }
    @{
        source_url = 'https://github.com/0xShug0/audio.cpp'
        source_commit = $sourceCommit
        compatibility_patch_sha256 = (Get-FileHash $patch -Algorithm SHA256).Hash.ToLowerInvariant()
        worker_source_sha256 = (Get-FileHash (Join-Path $repo 'native\worker.cpp') -Algorithm SHA256).Hash.ToLowerInvariant()
        backend = $targetBackend
        cuda_architectures = if ($targetBackend -eq 'cuda') { $CudaArchitectures } else { $null }
        cuda_min_compute_capability = $cudaMinCc
        cpu_architecture = 'baseline'
        msvc_redist_version = $redistVersion.Name
        model_weights_included = $false
    } | ConvertTo-Json | Set-Content -Encoding utf8NoBOM (Join-Path $stage 'provenance.json')
    & (Join-Path $stage 'qwen3asr-worker.exe') --probe
    if ($LASTEXITCODE -ne 0) { throw "Worker $targetBackend probe failed" }
    Write-Host "Native worker staged: $stage"
}

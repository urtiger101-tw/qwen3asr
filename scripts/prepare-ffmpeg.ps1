#requires -Version 7.4
[CmdletBinding()]
param([string]$Archive)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$filename = 'ffmpeg-n8.1.3-win64-lgpl-8.1.zip'
$source = "https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-09-26-13-03/$filename"
$expected = '933b9625fb4b0dc2e1e96cf20fb54b94ed24ba561858418de29531fb7c88ad74'
$buildCache = Join-Path $repo '.ag-artifacts/ffmpeg-download'
New-Item -ItemType Directory -Force $buildCache | Out-Null
if (-not $Archive) {
    $Archive = Join-Path $buildCache $filename
    if (-not (Test-Path -LiteralPath $Archive)) {
        Invoke-WebRequest -Uri $source -OutFile "$Archive.partial" -ConnectionTimeoutSeconds 30 -OperationTimeoutSeconds 300
        if ((Get-FileHash -LiteralPath "$Archive.partial" -Algorithm SHA256).Hash -ne $expected) { throw 'FFmpeg archive checksum mismatch' }
        Move-Item -LiteralPath "$Archive.partial" -Destination $Archive
    }
}
$Archive = (Resolve-Path -LiteralPath $Archive).Path
if ((Get-FileHash -LiteralPath $Archive -Algorithm SHA256).Hash -ne $expected) { throw 'FFmpeg archive checksum mismatch' }
$stage = Join-Path $repo 'native/bin'
$licenses = Join-Path $stage 'licenses'
New-Item -ItemType Directory -Force $stage,$licenses | Out-Null
$root = 'ffmpeg-n8.1.3-win64-lgpl-8.1/'
$files = [ordered]@{
    'bin/ffmpeg.exe' = (Join-Path $stage 'ffmpeg.exe')
    'bin/ffprobe.exe' = (Join-Path $stage 'ffprobe.exe')
    'LICENSE.txt' = (Join-Path $licenses 'FFmpeg-LGPL-3.0.txt')
}
$zip = [IO.Compression.ZipFile]::OpenRead($Archive)
try {
    foreach ($entryPath in $files.Keys) {
        $entry = $zip.GetEntry($root + $entryPath)
        if (-not $entry -or $entry.Length -gt 160MB) { throw "Missing or excessive archive entry: $entryPath" }
        $inputStream = $entry.Open()
        $outputStream = [IO.File]::Create($files[$entryPath])
        try { $inputStream.CopyTo($outputStream) }
        finally { $outputStream.Dispose(); $inputStream.Dispose() }
    }
}
finally { $zip.Dispose() }
@{
    binary_archive = $source
    binary_archive_sha256 = $expected
    ffmpeg_version = 'n8.1.3'
    ffmpeg_source_commit = '1041abdc962f4cc4f394aa8de9dc5236c0c3b9e7'
    ffmpeg_source = 'https://github.com/FFmpeg/FFmpeg/archive/refs/tags/n8.1.3.tar.gz'
    builder_source = 'https://github.com/BtbN/FFmpeg-Builds/archive/refs/tags/autobuild-2026-09-26-13-03.tar.gz'
    redistribution_note = 'Static third-party dependencies require their own corresponding sources and notices before public redistribution. See docs/ffmpeg-distribution.md.'
} | ConvertTo-Json | Set-Content (Join-Path $licenses 'FFmpeg-provenance.json') -Encoding utf8NoBOM
& (Join-Path $stage 'ffmpeg.exe') -version
if ($LASTEXITCODE -ne 0) { throw 'FFmpeg smoke check failed' }
& (Join-Path $stage 'ffprobe.exe') -version
if ($LASTEXITCODE -ne 0) { throw 'FFprobe smoke check failed' }
Write-Output "Verified native media tools staged at $stage"

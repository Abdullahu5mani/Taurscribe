# Downloads LibriSpeech test-clean (OpenSLR SLR12, CC BY 4.0).
# Set LIBRISPEECH_ROOT to the parent folder where "LibriSpeech" should appear
# (default: repo-root\taurscribe-runtime\librispeech).
#
# Usage:
#   .\scripts\download_librispeech_test_clean.ps1
#   $env:LIBRISPEECH_ROOT = "D:\data"; .\scripts\download_librispeech_test_clean.ps1

$ErrorActionPreference = "Stop"
$Tarball = "test-clean.tar.gz"
# From https://www.openslr.org/resources/12/md5sum.txt
$ExpectedMd5 = "32fa31d27d2e1cad72775fee3f4849a9"
$Url = "https://www.openslr.org/resources/12/$Tarball"

$RepoRoot = Split-Path -Parent $PSScriptRoot
$DefaultRoot = Join-Path $RepoRoot "taurscribe-runtime\librispeech"
$DestRoot = if ($env:LIBRISPEECH_ROOT) { $env:LIBRISPEECH_ROOT } else { $DefaultRoot }

New-Item -ItemType Directory -Force -Path $DestRoot | Out-Null
$ArchivePath = Join-Path $DestRoot $Tarball

if (-not (Test-Path $ArchivePath)) {
    Write-Host "Downloading $Url ..."
    Invoke-WebRequest -Uri $Url -OutFile $ArchivePath -UseBasicParsing
} else {
    Write-Host "Archive already present: $ArchivePath"
}

Write-Host "Verifying MD5..."
$hash = (Get-FileHash -Path $ArchivePath -Algorithm MD5).Hash.ToLowerInvariant()
if ($hash -ne $ExpectedMd5) {
    throw "MD5 mismatch: got $hash expected $ExpectedMd5 (delete $ArchivePath and retry)"
}

$ExtractMarker = Join-Path $DestRoot "LibriSpeech\test-clean"
if (Test-Path $ExtractMarker) {
    Write-Host "Corpus already extracted: $ExtractMarker"
    exit 0
}

Write-Host "Extracting (tar)..."
Push-Location $DestRoot
try {
    tar -xzf $Tarball
} finally {
    Pop-Location
}

if (-not (Test-Path $ExtractMarker)) {
    throw "Extraction failed: expected $ExtractMarker"
}
Write-Host "Done. test-clean at: $ExtractMarker"
Write-Host "Build manifest: cargo run --manifest-path src-tauri/Cargo.toml --bin librispeech_manifest -- --root `"$ExtractMarker`" --out `"$DestRoot\eval_manifest.jsonl`""

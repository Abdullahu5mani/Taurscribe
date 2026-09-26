param(
    [Parameter(Mandatory = $true)][string]$TargetTriple
)

$ErrorActionPreference = 'Stop'
$sevenZip = Get-Command 7z.exe -ErrorAction SilentlyContinue
$sevenZipPath = if ($sevenZip) { $sevenZip.Source } else { Join-Path $env:ProgramFiles '7-Zip\7z.exe' }
if (-not (Test-Path -LiteralPath $sevenZipPath)) {
    throw '7-Zip is required to inspect the NSIS installer payload.'
}

$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'src-tauri/target' }
$setupDir = Join-Path $targetDir "$TargetTriple/release/bundle/nsis"
$installers = @(Get-ChildItem -LiteralPath $setupDir -Filter '*.exe' -File)
if ($installers.Count -ne 1) {
    throw "Expected one NSIS installer in $setupDir, found $($installers.Count)."
}

$portable = 'dist-windows-portable/taurscribe-portable.zip'
if (-not (Test-Path -LiteralPath $portable)) {
    throw "Portable archive missing: $portable"
}

$required = @('taurscribe.exe', 'llama.dll', 'ggml.dll', 'ggml-base.dll', 'ggml-cpu.dll')
foreach ($artifact in @($installers[0].FullName, $portable)) {
    $listing = & $sevenZipPath l -slt $artifact
    if ($LASTEXITCODE -ne 0) {
        throw "Could not inspect $artifact with 7-Zip."
    }
    $names = @($listing | ForEach-Object {
        if ($_ -match '^Path = (.+)$') { [IO.Path]::GetFileName($Matches[1]) }
    })
    foreach ($name in $required) {
        if ($names -notcontains $name) {
            throw "$artifact is missing $name."
        }
    }
    Write-Host "Verified $artifact contains $($required -join ', ')"
}

if ($TargetTriple -eq 'x86_64-pc-windows-msvc') {
    $readObj = Get-Command llvm-readobj.exe -ErrorAction SilentlyContinue
    $readObjPath = if ($readObj) { $readObj.Source } else { Join-Path $env:ProgramFiles 'LLVM\bin\llvm-readobj.exe' }
    if (-not (Test-Path -LiteralPath $readObjPath)) {
        throw 'llvm-readobj is required to check Windows startup DLL imports.'
    }
    $appExe = Join-Path $targetDir "$TargetTriple/release/taurscribe.exe"
    $imports = & $readObjPath --coff-imports $appExe
    if ($LASTEXITCODE -ne 0) {
        throw "Could not inspect PE imports in $appExe."
    }
    if ($imports -match '(?i)\b(nvcuda|cublas\w*|cudart\w*|vulkan-1)\.dll\b') {
        throw 'The standard Windows installer requires NVIDIA CUDA or Vulkan at process startup.'
    }
    Write-Host 'Verified the standard x64 executable has no mandatory CUDA or Vulkan imports.'
}

param(
    [Parameter(Mandatory = $true)][string]$TargetTriple,
    # GPU flavor of this build: cpu (standard), nvidia, vulkan or adreno.
    [string]$Flavor = 'cpu'
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
if ($Flavor -eq 'nvidia') { $required += @('ggml-cuda.dll') }
if ($Flavor -eq 'vulkan') { $required += @('ggml-vulkan.dll') }
if ($Flavor -eq 'adreno') { $required += @('ggml-opencl.dll') }
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

# The NVIDIA build must carry the CUDA runtime it imports at startup.
if ($Flavor -eq 'nvidia') {
    foreach ($artifact in @($installers[0].FullName, $portable)) {
        $listing = & $sevenZipPath l -slt $artifact
        foreach ($pattern in @('^cudart64_\d+\.dll$', '^cublas64_\d+\.dll$', '^cublasLt64_\d+\.dll$')) {
            $hit = $listing | Where-Object { $_ -match '^Path = (.+)$' -and [IO.Path]::GetFileName($Matches[1]) -match $pattern }
            if (-not $hit) { throw "$artifact is missing a DLL matching $pattern (the CUDA runtime)." }
        }
    }
    Write-Host 'Verified the NVIDIA build bundles the CUDA runtime.'
}

# Only the standard (CPU + DirectML) build must start on any PC; GPU flavors
# deliberately import their vendor runtime.
if ($TargetTriple -eq 'x86_64-pc-windows-msvc' -and $Flavor -eq 'cpu') {
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

# nvcuda.dll ships only with the NVIDIA driver. The NVIDIA build delay-loads it
# so the app still starts on a PC without the driver (and falls back to the CPU).
if ($Flavor -eq 'nvidia') {
    $readObj = Get-Command llvm-readobj.exe -ErrorAction SilentlyContinue
    $readObjPath = if ($readObj) { $readObj.Source } else { Join-Path $env:ProgramFiles 'LLVM\bin\llvm-readobj.exe' }
    if (-not (Test-Path -LiteralPath $readObjPath)) {
        throw 'llvm-readobj is required to check Windows startup DLL imports.'
    }
    foreach ($bin in @('taurscribe.exe', 'ggml-cuda.dll')) {
        $path = Join-Path $targetDir "$TargetTriple/release/$bin"
        $imports = (& $readObjPath --coff-imports $path) -join "`n"
        if ($LASTEXITCODE -ne 0) { throw "Could not inspect PE imports in $path." }
        if ($imports -match '(?m)^\s*Import \{\s*\n\s*Name: nvcuda\.dll') {
            throw "$bin imports nvcuda.dll at startup; it must be delay-loaded."
        }
    }
    Write-Host 'Verified nvcuda.dll is delay-loaded, so the NVIDIA build starts without the driver.'
}

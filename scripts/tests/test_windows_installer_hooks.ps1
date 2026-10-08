param(
    [string]$MakeNsis = "$env:LOCALAPPDATA\tauri\NSIS\makensis.exe"
)

# Windows integration test: compile the real hooks, install using verified
# local DLLs, verify compression preserves bytes, and remove downloaded DLLs.
# Also ensure a failed download aborts before any app files are installed.
$ErrorActionPreference = 'Stop'
$repo = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$root = Join-Path $env:TEMP ("Taurscribe hook test " + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root | Out-Null
try {
    Copy-Item "$repo\src-tauri\windows\*" $root
    Copy-Item "$env:WINDIR\System32\where.exe" "$root\fixture.exe"
    $fixtureHash = (Get-FileHash "$root\fixture.exe").Hash
    $manifest = Join-Path $root 'cuda-redist-13.3.json'
    $spec = Get-Content $manifest -Raw | ConvertFrom-Json
    $deletes = ($spec.files | ForEach-Object { '  Delete "$INSTDIR\' + $_.name + '"' }) -join "`n"
    @"
!define TAURSCRIBE_ONLINE_CUDA
!define TAURSCRIBE_CUDA_MANIFEST "$manifest"
!macro TAURSCRIBE_DELETE_CUDA_DLLS
$deletes
!macroend
"@ | Set-Content -Encoding utf8 "$root\runtime-config.nsh"
    @'
Unicode true
RequestExecutionLevel user
SilentInstall silent
SilentUnInstall silent
Name "Taurscribe hook fixture"
OutFile "fixture-setup.exe"
!include "install-hooks.nsh"
Section
  SetOutPath "$INSTDIR"
  !insertmacro NSIS_HOOK_PREINSTALL
  File "fixture.exe"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  !insertmacro NSIS_HOOK_POSTINSTALL
SectionEnd
Section "Uninstall"
  ; Emulate a running app's DLL handle during the preuninstall phase.
  FileOpen $1 "$INSTDIR\cublas64_13.dll" r
  !ifmacrodef NSIS_HOOK_PREUNINSTALL
    !insertmacro NSIS_HOOK_PREUNINSTALL
  !endif
  ; Tauri's process check closes the app before ordinary resource removal.
  FileClose $1
  Delete "$INSTDIR\fixture.exe"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  !insertmacro NSIS_HOOK_POSTUNINSTALL
SectionEnd
'@ | Set-Content -Encoding utf8 "$root\fixture.nsi"
    Push-Location $root
    try {
        & $MakeNsis /V2 fixture.nsi
        if ($LASTEXITCODE -ne 0) { throw 'NSIS hook compilation failed' }
        $installed = Join-Path $root 'installed app'
        $setup = Start-Process "$root\fixture-setup.exe" -ArgumentList "/S /D=$installed" -Wait -PassThru
        if ($setup.ExitCode -ne 0) { throw "Installation failed: $($setup.ExitCode)" }
        foreach ($file in $spec.files) {
            $path = Join-Path $installed $file.name
            if ((Get-Item $path).Length -ne $file.bytes -or (Get-FileHash $path).Hash -ne $file.sha256) {
                throw "Installed runtime differs: $($file.name)"
            }
        }
        if ((Get-FileHash "$installed\fixture.exe").Hash -ne $fixtureHash) { throw 'Compression changed app bytes' }
        # _?= disables NSIS's asynchronous self-copy so -Wait covers removal.
        $uninstall = Start-Process "$installed\uninstall.exe" -ArgumentList "/S _?=$installed" -Wait -PassThru
        if ($uninstall.ExitCode -ne 0 -or (Test-Path "$installed\cublas64_13.dll") -or (Test-Path "$installed\cublasLt64_13.dll")) {
            throw 'Uninstall left downloaded CUDA DLLs'
        }

        # Valid source domain, nonexistent resource: fail deterministically.
        $spec.url = 'https://developer.download.nvidia.com/compute/cuda/redist/taurscribe-nonexistent-test.zip'
        $spec | ConvertTo-Json -Depth 5 | Set-Content -Encoding utf8 $manifest
        & $MakeNsis /V2 fixture.nsi
        if ($LASTEXITCODE -ne 0) { throw 'Failure fixture compilation failed' }
        $savedCudaPath = $env:CUDA_PATH
        try {
            Remove-Item Env:CUDA_PATH -ErrorAction SilentlyContinue
            $failed = Join-Path $root 'failed install'
            $setup = Start-Process "$root\fixture-setup.exe" -ArgumentList "/S /D=$failed" -Wait -PassThru
            if ($setup.ExitCode -eq 0 -or (Test-Path "$failed\fixture.exe")) { throw 'Download failure did not abort before app installation' }
        } finally { $env:CUDA_PATH = $savedCudaPath }
        Write-Output 'PASS: NSIS install, pinned DLLs, byte-preserving compression, uninstall, and download failure.'
    } finally { Pop-Location }
} finally { Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue }

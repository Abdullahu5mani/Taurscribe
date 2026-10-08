param(
    [Parameter(Mandatory=$true)][string]$BuildDirectory,
    [ValidateSet('x64','arm64')][string]$Architecture = 'x64'
)
$ErrorActionPreference = 'Stop'
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vs) { throw 'Visual Studio C++ build tools are required for the Windows repair launcher' }
$vcvars = Join-Path $vs 'VC\Auxiliary\Build\vcvarsall.bat'
$vcarch = if ($Architecture -eq 'arm64') { 'amd64_arm64' } else { 'amd64' }
cmd /c "call `"$vcvars`" $vcarch >nul && set" | ForEach-Object {
    if ($_ -match '^([^=]+)=(.*)$') { Set-Item -Path "env:$($Matches[1])" -Value $Matches[2] }
}
Push-Location $BuildDirectory
try {
    & rc.exe /nologo launcher.rc
    if ($LASTEXITCODE -ne 0) { throw 'Launcher resource compilation failed' }
    # Static CRT: the repair message itself must not depend on a VC runtime DLL.
    & cl.exe /nologo /O1 /MT /W4 /Fe:taurscribe.exe launcher.c launcher.res user32.lib /link /SUBSYSTEM:WINDOWS
    if ($LASTEXITCODE -ne 0) { throw 'Repair launcher compilation failed' }
} finally { Pop-Location }

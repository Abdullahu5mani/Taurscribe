param([string]$AppDirectory)
$ErrorActionPreference = 'Stop'
$repo = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$root = if ($AppDirectory) { $AppDirectory } else { Join-Path $env:TEMP ('Taurscribe repair test ' + [Guid]::NewGuid().ToString('N')) }
if (-not $AppDirectory) { New-Item -ItemType Directory -Path $root | Out-Null }
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class RepairDialog {
  public delegate bool Callback(IntPtr hwnd, IntPtr data);
  [DllImport("user32.dll")] public static extern bool EnumWindows(Callback cb, IntPtr data);
  [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr h, Callback cb, IntPtr data);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
  public static string ReadAndClose(uint pid) {
    string text = null;
    EnumWindows((h,d) => {
      uint p; GetWindowThreadProcessId(h,out p); if(p != pid) return true;
      var title = new StringBuilder(2048); GetWindowText(h,title,2048);
      if(title.ToString() != "Taurscribe needs repair") return true;
      text = title.ToString();
      EnumChildWindows(h,(c,x) => { var s = new StringBuilder(2048); GetWindowText(c,s,2048); text += "\n"+s; return true; },IntPtr.Zero);
      // Close the inactive dialog on the SSH test desktop too.
      PostMessage(h,0x10,IntPtr.Zero,IntPtr.Zero);
      return false;
    },IntPtr.Zero);
    return text;
  }
}
'@
function Assert-Repair([string]$Arguments, [string]$Expected) {
    $p = Start-Process "$root\taurscribe.exe" -ArgumentList $Arguments -PassThru
    try {
        $text = $null
        $deadline = [DateTime]::UtcNow.AddSeconds(15)
        while (-not $text -and [DateTime]::UtcNow -lt $deadline) {
            $text = [RepairDialog]::ReadAndClose($p.Id)
            if (-not $text) { Start-Sleep -Milliseconds 100 }
        }
        if (-not $text -or $text -notlike "*$Expected*" -or $text -notlike '*Run the Taurscribe installer again*') {
            throw "Missing actionable repair dialog: $text"
        }
        if (-not $p.WaitForExit(10000)) { throw 'Repair launcher did not exit after OK' }
        if ($p.ExitCode -ne 1) { throw "Repair launcher returned $($p.ExitCode)" }
        Write-Output "PASS repair dialog: $Expected"
    } finally { if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force } }
}
if ($AppDirectory) {
    # Only pass a disposable extracted app folder, never a user's installation.
    foreach ($name in @('cublas64_13.dll', 'llama.dll')) {
        $path = Join-Path $root $name
        $backup = "$path.repair-test-backup"
        if (Test-Path $backup) { throw 'A previous test backup already exists' }
        Move-Item $path $backup
        try { Assert-Repair '--version' $name } finally { Move-Item $backup $path }
    }
    Write-Output 'Packaged app repair tests passed.'
    return
}
try {
    Copy-Item "$repo\src-tauri\windows\launcher.c" $root
    Copy-Item "$repo\src-tauri\icons\icon.ico" $root
    '1 ICON "icon.ico"' | Set-Content -Encoding ascii "$root\launcher.rc"
    # Compile a probe through the same static-CRT toolchain, then keep it as core.
    @'
#define UNICODE
#include <windows.h>
#include <stdio.h>
#include <wchar.h>
int WINAPI wWinMain(HINSTANCE a,HINSTANCE b,PWSTR args,int c) {
    if(wcsstr(args,L"--loader-error")) return (int)0xC0000135;
    if(!wcsstr(args,L"\"space value\"")) return 2;
    char input[32]; if(!fgets(input,32,stdin)) return 3;
    printf("args-ok;stdin=%s",input); fflush(stdout);
    fprintf(stderr,"stderr-ok\n"); fflush(stderr);
    return 37;
}
'@ | Set-Content -Encoding ascii "$root\launcher.c"
    & "$repo\src-tauri\windows\build-launcher.ps1" -BuildDirectory $root
    Move-Item "$root\taurscribe.exe" "$root\taurscribe-core.exe"
    Copy-Item "$repo\src-tauri\windows\launcher.c" "$root\launcher.c" -Force
    [IO.File]::WriteAllBytes("$root\fixture.dll", [byte[]](1,2,3,4,5,6))
    $size = (Get-Item "$root\taurscribe-core.exe").Length
    @"
static const struct { const wchar_t *name; ULONGLONG bytes; } required_files[] = {
 { L"taurscribe-core.exe", ${size}ULL }, { L"fixture.dll", 6ULL }
};
"@ | Set-Content -Encoding ascii "$root\launcher-files.h"
    & "$repo\src-tauri\windows\build-launcher.ps1" -BuildDirectory $root
    'ping' | Set-Content -Encoding ascii "$root\stdin.txt"
    $p = Start-Process "$root\taurscribe.exe" -ArgumentList '"space value"' -WorkingDirectory $root -Wait -PassThru `
        -RedirectStandardInput "$root\stdin.txt" -RedirectStandardOutput "$root\stdout.txt" -RedirectStandardError "$root\stderr.txt"
    if ($p.ExitCode -ne 37 -or (Get-Content "$root\stdout.txt" -Raw) -notlike '*args-ok;stdin=ping*' -or
        (Get-Content "$root\stderr.txt" -Raw) -notlike '*stderr-ok*') { throw 'Argument/pipe/exit-code forwarding failed' }
    Write-Output 'PASS arguments, stdin/stdout/stderr, and exit-code forwarding'
    Remove-Item "$root\fixture.dll"
    Assert-Repair '--test' 'fixture.dll'
    [IO.File]::WriteAllBytes("$root\fixture.dll", [byte[]](1))
    Assert-Repair '--test' 'fixture.dll'
    [IO.File]::WriteAllBytes("$root\fixture.dll", [byte[]](1,2,3,4,5,6))
    Assert-Repair '--loader-error' '0xC0000135'
    Remove-Item "$root\taurscribe-core.exe"
    Assert-Repair '--test' 'taurscribe-core.exe'
    Write-Output 'Windows repair launcher tests passed.'
} finally { Remove-Item $root -Recurse -Force }

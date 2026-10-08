!include "LogicLib.nsh"
!define TAURSCRIBE_HOOK_DIR "${__FILEDIR__}"
!include "${TAURSCRIBE_HOOK_DIR}\runtime-config.nsh"

!macro NSIS_HOOK_PREINSTALL
  !ifdef TAURSCRIBE_ONLINE_CUDA
    !ifdef TAURSCRIBE_CUDA_RUNTIME_KIB
      ; Include downloaded files in NSIS's required installed-space estimate.
      AddSize ${TAURSCRIBE_CUDA_RUNTIME_KIB}
    !endif
    InitPluginsDir
    Push $OUTDIR
    SetOutPath "$PLUGINSDIR"
    File /oname=prepare-cuda-runtime.ps1 "${TAURSCRIBE_HOOK_DIR}\prepare-cuda-runtime.ps1"
    File /oname=cuda-redist.json "${TAURSCRIBE_CUDA_MANIFEST}"
    Pop $0
    SetOutPath "$0"
    DetailPrint "Preparing NVIDIA acceleration libraries..."
    nsExec::ExecToLog '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\prepare-cuda-runtime.ps1" -Manifest "$PLUGINSDIR\cuda-redist.json" -StageDirectory "$PLUGINSDIR\cuda-runtime" -InstallDirectory "$INSTDIR"'
    Pop $0
    ${If} $0 != 0
      IfSilent +2
      MessageBox MB_ICONSTOP "The NVIDIA libraries could not be prepared. Check your internet connection and run setup again, or use the offline installer."
      SetErrorLevel 1
      Abort
    ${EndIf}
  !endif
!macroend

!macro NSIS_HOOK_POSTINSTALL
  !ifdef TAURSCRIBE_ONLINE_CUDA
    ClearErrors
    CopyFiles /SILENT "$PLUGINSDIR\cuda-runtime\*.dll" "$INSTDIR"
    ${If} ${Errors}
      IfSilent +2
      MessageBox MB_ICONSTOP "The NVIDIA libraries could not be copied. Free disk space and run setup again."
      SetErrorLevel 1
      Abort
    ${EndIf}
  !endif

  ; Windows 10/11 transparently decompresses these immutable program files.
  ; Best effort: installation still succeeds on a filesystem without support.
  ; Only the app's root EXE/DLL files are touched, never models or user data.
  DetailPrint "Reducing installed disk space..."
  nsExec::ExecToLog '"$SYSDIR\compact.exe" /C /I /F /EXE:LZX "$INSTDIR\*.exe" "$INSTDIR\*.dll"'
  Pop $0
  ${If} $0 != 0
    DetailPrint "File compression unavailable; the app remains installed."
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  !ifdef TAURSCRIBE_ONLINE_CUDA
    ; These were downloaded, so they are absent from Tauri's resource manifest.
    ; Tauri has closed the app by this point, releasing its DLL handles.
    !insertmacro TAURSCRIBE_DELETE_CUDA_DLLS
    ; Tauri's earlier RMDir could not remove a directory containing these DLLs.
    RMDir "$INSTDIR"
  !endif
!macroend

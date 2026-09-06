; NSIS installer hooks for HashRename (Tauri v2 custom installer hooks)
; Ensures the per-user Explorer context menu entries are removed on uninstall.

!macro NSIS_HOOK_POSTUNINSTALL
  DetailPrint "Removing HashRename context menu entries..."
  DeleteRegKey HKCU "Software\Classes\Directory\shell\HashRename"
  DeleteRegKey HKCU "Software\Classes\Directory\Background\shell\HashRename"
!macroend

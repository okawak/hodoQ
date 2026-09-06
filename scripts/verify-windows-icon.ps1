# Verify the actual release executable, using the same resource ID and API as GPUI.
$ErrorActionPreference = 'Stop'
$executable = (Resolve-Path "$PSScriptRoot/../target/release/hodoq.exe").Path
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class HodoQIconCheck {
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern IntPtr LoadLibraryExW(string path, IntPtr file, uint flags);
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern IntPtr FindResourceW(IntPtr module, IntPtr name, IntPtr type);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern IntPtr LoadImageW(IntPtr module, IntPtr name, uint type,
        int width, int height, uint flags);
    [DllImport("user32.dll")]
    public static extern bool DestroyIcon(IntPtr icon);
    [DllImport("kernel32.dll")]
    public static extern bool FreeLibrary(IntPtr module);
}
'@
# LOAD_LIBRARY_AS_DATAFILE avoids executing the program or opening a database.
$module = [HodoQIconCheck]::LoadLibraryExW($executable, [IntPtr]::Zero, 0x2)
if ($module -eq [IntPtr]::Zero) { throw 'Could not load release executable resources.' }
try {
    # RT_GROUP_ICON = 14, resource ID = 1.
    $resource = [HodoQIconCheck]::FindResourceW($module, [IntPtr]1, [IntPtr]14)
    if ($resource -eq [IntPtr]::Zero) { throw 'Application icon resource ID 1 is missing.' }
    foreach ($size in @(16, 32, 48, 256)) {
        # IMAGE_ICON = 1. No LR_SHARED flag: release each handle ourselves.
        $icon = [HodoQIconCheck]::LoadImageW($module, [IntPtr]1, 1, $size, $size, 0)
        if ($icon -eq [IntPtr]::Zero) { throw "Cannot decode application icon at ${size}px." }
        [void][HodoQIconCheck]::DestroyIcon($icon)
    }
    Write-Output 'Application icon resource ID 1 loads at 16, 32, 48 and 256px.'
} finally {
    [void][HodoQIconCheck]::FreeLibrary($module)
}

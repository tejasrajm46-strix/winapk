# Captures one top-level window to a PNG, for visual verification of the real
# desktop app (the browser preview tools cannot see a native WebView window).
#
#   powershell -File scripts/capture-window.ps1 -Match "DroidWin" -Out screenshots/app.png
param(
    [Parameter(Mandatory = $true)][string]$Match,
    [Parameter(Mandatory = $true)][string]$Out
)

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class WinCap {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int cmd);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hWnd, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr hWnd);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
"@

$proc = Get-Process | Where-Object {
    $_.MainWindowHandle -ne 0 -and $_.MainWindowTitle -like "*$Match*"
} | Select-Object -First 1

if (-not $proc) {
    Write-Error "No visible window matching '$Match'."
    exit 1
}

$h = $proc.MainWindowHandle
# Restore, raise above other windows, and give the compositor a moment: a
# window that is merely focused can still be covered by a browser in front.
[void][WinCap]::ShowWindow($h, 9)                       # SW_RESTORE
[void][WinCap]::SetWindowPos($h, [IntPtr]::new(-1), 0, 0, 0, 0, 0x0001 -bor 0x0002)  # HWND_TOPMOST, NOSIZE|NOMOVE
[void][WinCap]::BringWindowToTop($h)
[void][WinCap]::SetForegroundWindow($h)
Start-Sleep -Milliseconds 1200

$rect = New-Object WinCap+RECT
[void][WinCap]::GetWindowRect($h, [ref]$rect)
$width = $rect.Right - $rect.Left
$height = $rect.Bottom - $rect.Top

$bitmap = New-Object System.Drawing.Bitmap $width, $height
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
$dir = Split-Path -Parent $Out
if ($dir -and -not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir | Out-Null }
$bitmap.Save((Join-Path (Get-Location) $Out), [System.Drawing.Imaging.ImageFormat]::Png)
$graphics.Dispose()
$bitmap.Dispose()

Write-Output ("captured '{0}' {1}x{2} -> {3}" -f $proc.MainWindowTitle, $width, $height, $Out)

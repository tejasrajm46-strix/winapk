# Captures the on-screen pixels of one app window (identified by host pid).
# Brings the window to the front first, then copies the window rectangle off the
# desktop, so the evidence is what the user actually sees on their display.
param([int]$ProcessId, [string]$Out)

Add-Type -AssemblyName System.Drawing
Add-Type -Namespace W -Name U -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetForegroundWindow(System.IntPtr h);
[DllImport("user32.dll")] public static extern bool ShowWindow(System.IntPtr h, int c);
[DllImport("user32.dll")] public static extern bool GetWindowRect(System.IntPtr h, out RECT r);
[DllImport("user32.dll")] public static extern bool PrintWindow(System.IntPtr h, System.IntPtr dc, uint flags);
public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }
'@

$p = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
if (-not $p) { Write-Error "process $ProcessId is gone"; exit 1 }
$h = $p.MainWindowHandle
[void][W.U]::ShowWindow($h, 9)
[void][W.U]::SetForegroundWindow($h)
Start-Sleep -Milliseconds 800

$rect = New-Object W.U+RECT
[void][W.U]::GetWindowRect($h, [ref]$rect)
$width = $rect.Right - $rect.Left
$height = $rect.Bottom - $rect.Top
if ($width -le 0 -or $height -le 0) { Write-Error "window $ProcessId has no size"; exit 1 }

# PW_RENDERFULLCONTENT (0x2) asks the window to render itself into the DC, so
# another window in front of it cannot end up in the picture.
$bmp = New-Object System.Drawing.Bitmap $width, $height
$graphics = [System.Drawing.Graphics]::FromImage($bmp)
$dc = $graphics.GetHdc()
[void][W.U]::PrintWindow($h, $dc, 2)
$graphics.ReleaseHdc($dc)
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$graphics.Dispose()
$bmp.Dispose()
Write-Output ("saved {0} ({1}x{2}) for pid {3}: {4}" -f $Out, $width, $height, $ProcessId, $p.MainWindowTitle)

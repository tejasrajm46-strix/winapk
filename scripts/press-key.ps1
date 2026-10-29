# Presses one key into one app window through the OS, the same way a keyboard
# does, and refuses to report success if the foreground window belongs to another
# process. Used by the key-mapping evidence runs.
param([int]$ProcessId, [string]$Key = "j")

Add-Type -Namespace W -Name U -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetForegroundWindow(System.IntPtr h);
[DllImport("user32.dll")] public static extern bool ShowWindow(System.IntPtr h, int c);
[DllImport("user32.dll")] public static extern System.IntPtr GetForegroundWindow();
[DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(System.IntPtr h, out uint pid);
'@

$p = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
if (-not $p) { Write-Error "window host $ProcessId is gone"; exit 1 }
$h = $p.MainWindowHandle
[void][W.U]::ShowWindow($h, 9)
[void][W.U]::SetForegroundWindow($h)
Start-Sleep -Milliseconds 700

$fg = [W.U]::GetForegroundWindow()
$fgpid = 0
[void][W.U]::GetWindowThreadProcessId($fg, [ref]$fgpid)
if ($fgpid -ne $ProcessId) {
    Write-Error ("focus is on process " + $fgpid + ", not " + $ProcessId + " (" + $p.MainWindowTitle + ")")
    exit 1
}

Add-Type -AssemblyName System.Windows.Forms
[System.Windows.Forms.SendKeys]::SendWait($Key)
Start-Sleep -Milliseconds 250
Write-Output ("pressed '" + $Key + "' into pid " + $fgpid + " (" + $p.MainWindowTitle + ")")

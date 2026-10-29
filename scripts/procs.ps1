# Prints the pid and full command line of every scrcpy window host process.
# Used by the diagnostics evidence runs to identify which window belongs to
# which Android display before a window is closed on purpose.
Get-CimInstance Win32_Process -Filter "Name = 'scrcpy.exe'" |
    ForEach-Object { "{0} :: {1}" -f $_.ProcessId, $_.CommandLine }

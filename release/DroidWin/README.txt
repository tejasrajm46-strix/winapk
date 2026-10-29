DroidWin 0.1.0 — Windows Android app windows
============================================

Run DroidWin.exe to start; Setup.exe installs it.

Game window
  Right rail   Screenshot, Rotate, More (Key mapping, labels, Controls)
  Bottom bar   Back, Home, Recent Apps

Universal keyboard (every Android app, rebindable in Key mapping)
  Shift+A Back   Shift+H Home   Shift+R Recent Apps   Shift+M Menu
  Shift+P Screenshot            Shift+O Rotate
  Ctrl+Shift+R  show/hide the mapping overlay (fixed)

Key mapping
  More > Key mapping > Universal controls: Change, then press the new chord.
  A chord another action or app mapping already uses is refused with a reason.
  Application controls: "Place on the game" > click the real control >
  press the key > Add. Saved per Android package, reloaded with the app.

Known limits
  - Ctrl+Shift+R cannot be rebound yet.
  - Record is not offered: screenrecord rejects a virtual display.
  - A rotated window keeps the old frame shape (scrcpy ignores window size
    for a new display); rotating back restores it.

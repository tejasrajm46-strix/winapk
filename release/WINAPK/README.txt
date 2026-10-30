WINAPK 0.1.0 — Windows Android app windows
==========================================

Setup.exe installs it (per-user, no admin prompt).

To run it without installing, unzip WINAPK-portable.zip and run the WINAPK.exe
inside it. Keep the resources\ folder beside that exe: it holds the window host
(scrcpy, adb, SDL) and without it an app window cannot open. There is no loose
WINAPK.exe here on purpose: on its own it has nothing to draw app windows with.

Neither binary is code-signed yet, so a copy downloaded from the internet gets
the SmartScreen "Windows protected your PC" prompt: More info > Run anyway.

First launch sets up its own Android runtime (SDK, system image, AVD) from
Google's distribution, then reuses it. Nothing else has to be installed first.

Application name
  Settings > Application name changes the name in the title bar, the top bar and
  the About panel, and every app window opened afterwards. The name is saved
  locally and is already in effect the next time you start. Reset to WINAPK
  returns to the shipped name. The executable, the installer's product name and
  the package identifier are build-time values and are not renamed.

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

Window size
  The window can be resized down to 900x560. Below 1040 px wide the sidebar
  becomes an icon rail and the header drops the SDK path, so the content keeps
  its width on a small laptop or a snapped window. Every navigation destination
  keeps its accessible name.

Known limits
  - Ctrl+Shift+R cannot be rebound yet.
  - Record is not offered: screenrecord rejects a virtual display.
  - A rotated window keeps the old frame shape (scrcpy ignores window size
    for a new display); rotating back restores it.
  - The AVD WINAPK creates is still named DroidWin_API36 on disk. That
    identifier is kept on purpose: renaming it would orphan the system image and
    settings of anyone upgrading from an earlier build.

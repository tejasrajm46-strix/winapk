#!/usr/bin/env bash
# Cold-boots the runtime, launches one app as the first window of the run, and
# records both the launch result and the adb connection state throughout.
# Evidence only: it never changes the app's behaviour.
set -u
cd "$(dirname "$0")/.." || exit 1
ADB="${LOCALAPPDATA}/Android/Sdk/platform-tools/adb.exe"
RUNS="${1:-3}"
PACKAGE="${2:-com.kiloo.subwaysurf}"
for run in $(seq 1 "$RUNS"); do
  echo "===== cold boot $run"
  node scripts/ui-shot.mjs --eval "window.__TAURI_INTERNALS__.invoke('runtime_stop').then(()=>'stopped')" >/dev/null
  sleep 6
  node scripts/ui-shot.mjs --eval "window.__TAURI_INTERNALS__.invoke('runtime_start').then(r=>'runtime '+r.state)"
  log=".droidwin/evidence/coldboot-${run}-adb.log"
  : > "$log"
  (
    for _ in $(seq 1 40); do
      printf '%s [%s]\n' "$(date +%H:%M:%S.%3N)" "$("$ADB" devices | sed -n 2p | tr -d '\r')" >> "$log"
      sleep 0.4
    done
  ) &
  sampler=$!
  node scripts/ui-shot.mjs --eval "window.__TAURI_INTERNALS__.invoke('launch_app',{packageId:'${PACKAGE}'}).then(r=>'cold ${run}: LAUNCH OK host pid '+r.pid).catch(e=>'cold ${run}: LAUNCH ERR '+e.summary+' | '+e.detail)"
  sleep 8
  kill "$sampler" 2>/dev/null
  echo "--- device states seen:"
  awk '{ key=$2; if (key != last) { printf "  %s %s\n", $1, key; last=key } }' "$log" | head -6
  echo "--- app pid now: $("$ADB" shell pidof "$PACKAGE" | tr -d '\r')"
done

#!/usr/bin/env bash
# Builds the blank-surface fixture APK used by examples/attention_proof.rs.
#
# It is a real, signed Android package whose launcher activity draws one black
# view and stays alive. That is the only way to test "a package that renders a
# blank surface" honestly: every app in the library draws something.
#
#   scripts/make-blank-apk.sh [output.apk]
set -euo pipefail

SDK="${ANDROID_SDK_ROOT:-${LOCALAPPDATA:-$HOME/AppData/Local}/Android/Sdk}"
BUILD_TOOLS="${BUILD_TOOLS:-36.0.0}"
BT="$SDK/build-tools/$BUILD_TOOLS"
ANDROID_JAR="$SDK/platforms/android-36/android.jar"
OUT="${1:-.droidwin/testdata/blank.apk}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

for tool in "$BT/d8.bat" "$BT/aapt2.exe" "$BT/zipalign.exe" "$BT/apksigner.bat" "$ANDROID_JAR"; do
  if [ ! -f "$tool" ]; then
    echo "missing $tool — set ANDROID_SDK_ROOT or BUILD_TOOLS" >&2
    exit 1
  fi
done

WORK="$(cygpath -m "$(mktemp -d)")"
trap 'rm -rf "$(cygpath -u "$WORK")"' EXIT
mkdir -p "$WORK/classes" "$WORK/dex" "$(dirname "$OUT")"

javac --release 17 -cp "$ANDROID_JAR" -d "$WORK/classes" "$ROOT/scripts/blank-app/BlankActivity.java"

(
  cd "$WORK/classes"
  # One class file, relative to this directory, so d8 gets a path it can open.
  "$BT/d8.bat" --lib "$(cygpath -w "$ANDROID_JAR")" --output "$WORK/dex" \
    org/droidwin/blank/BlankActivity.class
)

"$BT/aapt2.exe" link \
  -o "$WORK/base.apk" \
  -I "$ANDROID_JAR" \
  --manifest "$ROOT/scripts/blank-app/AndroidManifest.xml" \
  --min-sdk-version 23 \
  --target-sdk-version 36

(
  cd "$WORK/dex"
  jar uf "$(cygpath -w "$WORK/base.apk")" classes.dex
)

"$BT/zipalign.exe" -f 4 "$WORK/base.apk" "$WORK/aligned.apk"

# One stable debug key per output location, so rebuilding the fixture updates
# an installed copy instead of colliding with its own previous signature.
KEYSTORE="${OUT%.apk}.keystore"
if [ ! -f "$KEYSTORE" ]; then
  keytool -genkeypair \
    -keystore "$KEYSTORE" \
    -storepass android -keypass android \
    -alias androiddebugkey \
    -keyalg RSA -keysize 2048 -validity 10000 \
    -dname "CN=Android Debug,O=Android,C=US" >/dev/null 2>&1
fi

"$BT/apksigner.bat" sign \
  --ks "$KEYSTORE" \
  --ks-pass pass:android \
  --out "$OUT" \
  "$WORK/aligned.apk"

echo "built $OUT"

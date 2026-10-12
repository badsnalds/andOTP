#!/usr/bin/env bash
# Installs the debug APK on a running emulator, creates a vault through the on-screen UI and
# checks that the app is still alive. Screenshots go to android/build/smoke/.
set -euo pipefail
cd "$(dirname "$0")/.."
apk=app/build/outputs/apk/debug/app-debug.apk
pkg=org.shadowice.andotp
out=build/smoke
mkdir -p "$out"

alive() { adb shell pidof "$pkg" >/dev/null; }
shot() { adb exec-out screencap -p > "$out/$1.png"; }
fail() {
    echo "::error::$1"
    adb logcat -d -s RustStdoutStderr:V AndroidRuntime:E DEBUG:V libc:V '*:F' | tail -n 200 || true
    exit 1
}

adb install -r "$apk"
adb logcat -c
adb shell am start -W -n "$pkg/.MainActivity"
sleep 15
alive || fail "the app exited during start-up"
shot 1-start

# The password field has the focus on the first start: create a vault.
adb shell input text hunter22
adb shell input keyevent KEYCODE_TAB
adb shell input text hunter22
adb shell input keyevent KEYCODE_ENTER
sleep 20
alive || fail "the app exited after creating the vault"
shot 2-unlocked
adb shell run-as "$pkg" ls files | grep -q secrets.json.aes || fail "no vault was created"

# Background and resume (the GL surface is recreated).
adb shell input keyevent KEYCODE_HOME
sleep 3
adb shell am start -n "$pkg/.MainActivity"
sleep 8
alive || fail "the app exited after resuming"
shot 3-resumed
echo "Smoke test passed"

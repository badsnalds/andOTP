#!/usr/bin/env bash
# Installs the debug APK on a running emulator, creates a vault through the on-screen UI and
# checks that the app is still alive. Screenshots go to android/build/smoke/.
set -euo pipefail
cd "$(dirname "$0")/.."
apk=app/build/outputs/apk/debug/app-debug.apk
pkg=org.shadowice.andotp
activity="$pkg/.MainActivity"
out=build/smoke
mkdir -p "$out"

alive() { adb shell pidof "$pkg" >/dev/null; }
shot() { adb exec-out screencap -p > "$out/$1.png"; }
has_vault() { adb shell run-as "$pkg" ls files 2>/dev/null | grep -q secrets.json.aes; }
fail() {
    echo "::error::$1"
    adb logcat -d -b all > "$out/logcat.txt" 2>/dev/null || true
    echo "--- app log, crashes and process deaths ---"
    grep -E "andOTP|AndroidRuntime|FATAL|DEBUG +:|Fatal signal|$pkg" "$out/logcat.txt" | tail -n 150 || true
    exit 1
}

# Waits (up to $1 seconds) until our activity has the input focus, i.e. it is drawn and ready
# for key events. Emulators on shared CI machines can be slow to start an app.
wait_for_focus() {
    for _ in $(seq "$1"); do
        if adb shell dumpsys window 2>/dev/null | grep -E 'mCurrentFocus|mFocusedApp' | grep -q "$activity"; then
            sleep 3 # let the first frames render
            return 0
        fi
        sleep 1
    done
    return 1
}

adb install -r "$apk"
# Right after an install Android may still update the package's resources, which recreates a
# running activity. The app survives that (MainActivity.onDestroy), but give it a moment so the
# typing below doesn't go to a dying window.
sleep 5
adb logcat -c

# Typing goes to whichever field has the keyboard focus; if the app was not ready yet, the
# keystrokes are lost. Start from a fresh process for each attempt.
for attempt in 1 2 3; do
    adb shell am force-stop "$pkg"
    adb shell am start -n "$activity"
    wait_for_focus 90 || fail "the app did not come to the foreground"
    alive || fail "the app exited during start-up"
    shot 1-start

    # The password field has the focus on the first start: create a vault.
    adb shell input text hunter22
    adb shell input keyevent KEYCODE_TAB
    adb shell input text hunter22
    adb shell input keyevent KEYCODE_ENTER

    for _ in $(seq 30); do
        has_vault && break
        sleep 1
    done
    alive || fail "the app exited after creating the vault"
    has_vault && break
    echo "Attempt $attempt: no vault yet, retrying"
done
has_vault || fail "no vault was created"
sleep 3
shot 2-unlocked

# Stay open for a while: leaks that grow with every frame (e.g. JNI references) kill the app
# after a minute or two, not right away.
for _ in $(seq 15); do
    sleep 10
    alive || fail "the app exited while idle on the main screen"
done
shot 2b-idle

# Background and resume (the GL surface is recreated).
adb shell input keyevent KEYCODE_HOME
sleep 3
adb shell am start -n "$activity"
wait_for_focus 60 || fail "the app did not come back to the foreground"
alive || fail "the app exited after resuming"
shot 3-resumed

# Back closes the activity. Its onDestroy must not hang (the Rust thread never returns on its
# own); a hang shows up as an ANR after a few seconds.
adb shell input keyevent KEYCODE_BACK
sleep 10
if adb logcat -d -b events | grep -q "am_anr.*$pkg"; then
    fail "the app stopped responding when closed"
fi

# It starts again cleanly and the vault is still there.
adb shell am start -n "$activity"
wait_for_focus 60 || fail "the app did not start again after closing"
alive || fail "the app exited after starting again"
has_vault || fail "the vault is gone after closing the app"
shot 4-restarted
echo "Smoke test passed (attempt $attempt)"

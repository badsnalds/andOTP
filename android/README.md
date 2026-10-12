# andOTP for Android (Rust)

The Android app runs the same Rust/egui code as the desktop app (`crates/andotp-gui`). It is
packaged as `libandotp_android.so` (`crates/andotp-android`) inside a small `NativeActivity`
(`MainActivity.java`), which adds the Android services Rust cannot reach directly: the
clipboard, the system file picker and the window insets.

## Installing

Every CI run of the **Android** workflow attaches the APK as the `andOTP-apk` artifact
(Actions → the run → Artifacts). Unzip it, copy the `.apk` to your phone and open it; Android
asks you to allow installing apps from that source.

The package name is `org.shadowice.andotp`, so it installs next to the original andOTP
(`org.shadowice.flocke.andotp`). Move your accounts over with an encrypted backup: export it in
the original app, then use ☰ → *Import backup* here.

## Signing

Without a signing key, CI signs the APK with a temporary debug key that changes on every run.
Android only installs an update when the signature matches, so you would have to uninstall
(which deletes the vault) before installing a newer build. For updates, create a key once:

```sh
keytool -genkeypair -v -keystore release.jks -alias andotp -keyalg RSA -keysize 4096 -validity 10000
base64 -w0 release.jks   # → secret ANDROID_KEYSTORE_BASE64
```

and add the repository secrets `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`,
`ANDROID_KEY_ALIAS` and `ANDROID_KEY_PASSWORD`. Keep `release.jks` safe and out of the repository.

## Building locally

Requirements: Android SDK (API 35), NDK, JDK 17, Gradle 8.11+, and
`cargo install cargo-ndk` plus `rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android`.

```sh
android/scripts/build-native.sh            # all ABIs, or e.g. `build-native.sh arm64-v8a`
gradle -p android assembleRelease          # → android/app/build/outputs/apk/release/
```

## Differences from the desktop app

* Files are imported and exported through the system file picker.
* QR codes are scanned with the camera (the permission is asked for the first time you scan) or
  from images, e.g. a screenshot of the setup page. No Google Play Services are needed.
* Not available on Android: system keychain unlock, typing into other apps, automatic backup
  folders and the renderer choice.
* Screenshots of the app are blocked (`FLAG_SECURE`) in release builds.

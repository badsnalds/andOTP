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
* Fingerprint unlock (Android 11+) takes the place of the system keychain: Settings →
  Fingerprint unlock. The password is encrypted with an Android Keystore key that is released
  only after a strong biometric check and stops working when the enrolled fingerprints change.
* Passkeys (Android 14+): andOTP can store passkeys for websites and apps. Turn it on under
  Android Settings → Passwords, passkeys & accounts (or andOTP Settings → Passkeys → Open), then
  choose "Create a passkey" on a site. Details below.
* Not available on Android: typing into other apps, automatic backup
  folders and the renderer choice.
* Screenshots of the app are blocked (`FLAG_SECURE`) in release builds.

## Passkeys

andOTP is a passkey provider for the Android Credential Manager (Android 14 and later).
Browsers and apps that support passkeys offer andOTP in the system passkey sheet once it is
turned on in Settings → Passwords, passkeys & accounts.

* **Phone-only**: each passkey is a P-256 key created inside the Android Keystore (hardware
  backed where the phone has it). It cannot be exported, so passkeys are **not** part of andOTP
  backups and are not synced. Keep another way to sign in (the site's recovery codes, a second
  passkey or security key).
* Every use needs your fingerprint or screen lock; nothing is signed without it.
* Browsers on [Google's list of privileged browsers](https://www.gstatic.com/gpm-passkeys-privileged-apps/apps.json)
  (bundled as `app/src/main/assets/privileged_browsers.json`) pass the web origin; other apps
  are identified by the hash of their signing certificate (`android:apk-key-hash:…`).
* andOTP Settings → Passkeys lists the stored passkeys and deletes them. Deleting one on the
  phone does not remove it from the site; remove it in the site's account settings too.
* `scripts/webauthn-check.sh` verifies the registration and sign-in responses against
  [py_webauthn](https://github.com/duo-labs/py_webauthn), as a website would (runs in CI).

//! Native library loaded by the Android `MainActivity` (a `NativeActivity`).
//!
//! Build with `cargo ndk -t arm64-v8a build --release -p andotp-android`; see `android/README.md`.

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    if let Err(err) = andotp_gui::run_android(app) {
        eprintln!("andOTP failed: {err}");
    }
}

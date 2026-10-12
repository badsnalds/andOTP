//! Native library loaded by the Android `MainActivity` (a `NativeActivity`).
//!
//! Build with `cargo ndk -t arm64-v8a build --release -p andotp-android`; see `android/README.md`.

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    // stdout/stderr go nowhere on Android: route `log` (also used by eframe) and panics to logcat.
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("andOTP"),
    );
    std::panic::set_hook(Box::new(|info| {
        log::error!("panic: {info}");
    }));

    log::info!("starting");
    if let Err(err) = andotp_gui::run_android(app) {
        log::error!("andOTP failed: {err}");
    }
    log::info!("event loop ended");
}

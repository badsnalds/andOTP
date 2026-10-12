//! andOTP desktop – a native two-factor authenticator built on `andotp-core` and egui.

#[cfg(target_os = "android")]
pub mod android;
#[cfg(target_os = "android")]
mod android_camera;
mod app;
mod form;
pub mod renderer;
#[cfg(camera)]
mod scanner;
mod style;
mod thumbnails;
#[cfg(all(feature = "webcam", not(target_os = "android")))]
mod webcam;
mod widgets;

pub use app::AndOtpApp;

/// Entry point for the Android build (called from the `andotp-android` cdylib).
#[cfg(target_os = "android")]
pub fn run_android(app: winit::platform::android::activity::AndroidApp) -> eframe::Result {
    android::init(&app);
    let data = android::data_dir();
    let options = eframe::NativeOptions {
        android_app: Some(app),
        renderer: eframe::Renderer::Glow,
        persistence_path: Some(data.join("settings.ron")),
        ..Default::default()
    };
    eframe::run_native(
        "andOTP",
        options,
        Box::new(move |cc| {
            let mut app = AndOtpApp::new(cc);
            app.set_vault_path(data.join(andotp_core::vault::FILE_NAME));
            Ok(Box::new(app))
        }),
    )
}

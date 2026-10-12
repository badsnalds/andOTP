//! andOTP desktop – a native two-factor authenticator built on `andotp-core` and egui.

mod app;
mod form;
pub mod renderer;
#[cfg(feature = "webcam")]
mod webcam;
mod widgets;

pub use app::AndOtpApp;

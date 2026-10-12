//! Live QR code scanning: the webcam on desktop (`webcam`), the camera on Android
//! (`android_camera`). Both expose the same `Scanner`.

#[cfg(target_os = "android")]
pub use crate::android_camera::Scanner;
#[cfg(not(target_os = "android"))]
pub use crate::webcam::Scanner;

pub enum Status {
    Starting,
    Scanning,
    /// Contents of the `otpauth://` QR code that was found.
    Found(String),
    Failed(String),
}

//! Android integration: the bits egui cannot do on its own (clipboard, the system document
//! picker, window insets). The Java side lives in `android/app/src/main/java/.../MainActivity.java`.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use jni::objects::{JByteArray, JIntArray, JObject, JString, JValue};
use jni::{JNIEnv, JavaVM};
use winit::platform::android::activity::AndroidApp;

static APP: OnceLock<AndroidApp> = OnceLock::new();

/// Remembers the activity handle; call once from `android_main`.
pub fn init(app: &AndroidApp) {
    let _ = APP.set(app.clone());
}

fn app() -> Option<&'static AndroidApp> {
    APP.get()
}

/// Runs `f` with a JNI environment and the `MainActivity` object.
fn with_activity<R>(f: impl FnOnce(&mut JNIEnv, &JObject) -> jni::errors::Result<R>) -> Result<R, String> {
    let app = app().ok_or("Android integration is not initialised")?;
    // SAFETY: both pointers come from android-activity and stay valid for the process lifetime.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }.map_err(|e| e.to_string())?;
    let mut env = vm.attach_current_thread_permanently().map_err(|e| e.to_string())?;
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    // This thread never returns to Java, so local references (strings, arrays, call results) are
    // only released when we pop a frame. Without it they pile up with every call (the insets
    // are polled several times a second) until ART aborts the app on reference table overflow.
    env.with_local_frame(16, |env| {
        let result = f(env, &activity);
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
        }
        result
    })
    .map_err(|e: jni::errors::Error| e.to_string())
}

/// Copies `text` to the clipboard, flagged as sensitive so Android hides it in previews.
pub fn copy(text: &str) -> Result<(), String> {
    with_activity(|env, activity| {
        let text = env.new_string(text)?;
        env.call_method(activity, "copyToClipboard", "(Ljava/lang/String;)V", &[JValue::Object(&text)])?;
        Ok(())
    })
}

/// Opens the system document picker; the result arrives through [`take_opened`].
pub fn open_document(mime: &str) -> Result<(), String> {
    with_activity(|env, activity| {
        let mime = env.new_string(mime)?;
        env.call_method(activity, "openDocument", "(Ljava/lang/String;)V", &[JValue::Object(&mime)])?;
        Ok(())
    })
}

/// Lets the user choose where to save `data`; the outcome arrives through [`take_message`].
pub fn create_document(name: &str, mime: &str, data: &[u8]) -> Result<(), String> {
    with_activity(|env, activity| {
        let name = env.new_string(name)?;
        let mime = env.new_string(mime)?;
        let bytes = env.byte_array_from_slice(data)?;
        env.call_method(
            activity,
            "createDocument",
            "(Ljava/lang/String;Ljava/lang/String;[B)V",
            &[JValue::Object(&name), JValue::Object(&mime), JValue::Object(&bytes)],
        )?;
        Ok(())
    })
}

/// The file picked with [`open_document`], once it has been read.
pub fn take_opened() -> Option<Vec<u8>> {
    with_activity(|env, activity| {
        let data = env.call_method(activity, "takeOpened", "()[B", &[])?.l()?;
        if data.is_null() {
            return Ok(None);
        }
        let data = JByteArray::from(data);
        Ok(Some(env.convert_byte_array(&data)?))
    })
    .ok()
    .flatten()
}

/// A status message from the Java side (`!` prefix = error, `-` = picker cancelled).
pub fn take_message() -> Option<String> {
    with_activity(|env, activity| {
        let message = env.call_method(activity, "takeMessage", "()Ljava/lang/String;", &[])?.l()?;
        if message.is_null() {
            return Ok(None);
        }
        let message = JString::from(message);
        Ok(Some(env.get_string(&message)?.into()))
    })
    .ok()
    .flatten()
}

/// Private app storage (`/data/data/<package>/files`).
pub fn data_dir() -> PathBuf {
    app().and_then(|a| a.internal_data_path()).unwrap_or_else(|| PathBuf::from("."))
}

/// Space taken by the status bar, navigation bar, display cutout and on-screen keyboard, in
/// physical pixels: (top, bottom). Queried from Java at most every 200 ms.
pub fn insets_px() -> (f32, f32) {
    static CACHE: Mutex<Option<(Instant, (f32, f32))>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, insets)) = *cache
        && at.elapsed() < Duration::from_millis(200)
    {
        return insets;
    }
    let insets = with_activity(|env, activity| {
        let array = env.call_method(activity, "systemInsets", "()[I", &[])?.l()?;
        let array = JIntArray::from(array);
        let mut values = [0i32; 2];
        env.get_int_array_region(&array, 0, &mut values)?;
        Ok((values[0].max(0) as f32, values[1].max(0) as f32))
    })
    .unwrap_or((0.0, 0.0));
    *cache = Some((Instant::now(), insets));
    insets
}

/// Opens the back camera (asking for permission first if needed); frames arrive through
/// [`take_camera_frame`], problems through [`take_camera_error`].
pub fn camera_start() -> Result<(), String> {
    with_activity(|env, activity| {
        env.call_method(activity, "startCamera", "()V", &[])?;
        Ok(())
    })
}

pub fn camera_stop() {
    let _ = with_activity(|env, activity| {
        env.call_method(activity, "stopCamera", "()V", &[])?;
        Ok(())
    });
}

/// The latest camera frame as a greyscale image plus the sensor rotation in degrees.
pub fn take_camera_frame() -> Option<(image::GrayImage, i32)> {
    let bytes = with_activity(|env, activity| {
        let data = env.call_method(activity, "takeCameraFrame", "()[B", &[])?.l()?;
        if data.is_null() {
            return Ok(None);
        }
        let data = JByteArray::from(data);
        Ok(Some(env.convert_byte_array(&data)?))
    })
    .ok()
    .flatten()?;
    // Header: width, height, rotation as big-endian i32, then width * height luma bytes.
    let int = |i: usize| bytes.get(i..i + 4).map(|b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let (width, height, rotation) = (int(0)?, int(4)?, int(8)?);
    let (width, height) = (u32::try_from(width).ok()?, u32::try_from(height).ok()?);
    let pixels = bytes.get(12..)?.to_vec();
    image::GrayImage::from_raw(width, height, pixels).map(|image| (image, rotation))
}

/// Why the camera could not be used (permission denied, no camera, …), once.
pub fn take_camera_error() -> Option<String> {
    with_activity(|env, activity| {
        let message = env.call_method(activity, "takeCameraError", "()Ljava/lang/String;", &[])?.l()?;
        if message.is_null() {
            return Ok(None);
        }
        let message = JString::from(message);
        Ok(Some(env.get_string(&message)?.into()))
    })
    .ok()
    .flatten()
}

fn call_bool(method: &str) -> bool {
    with_activity(|env, activity| env.call_method(activity, method, "()Z", &[])?.z()).unwrap_or(false)
}

/// Fingerprint unlock is possible (Android 11+ with a strong biometric enrolled).
pub fn biometric_available() -> bool {
    call_bool("biometricAvailable")
}

/// A password is stored for fingerprint unlock.
pub fn biometric_enabled() -> bool {
    call_bool("biometricEnabled")
}

/// Asks for a fingerprint, then stores `password` encrypted with a biometric-bound Keystore key.
/// The outcome arrives through [`take_message`].
pub fn biometric_enroll(password: &str) -> Result<(), String> {
    with_activity(|env, activity| {
        let bytes = env.byte_array_from_slice(password.as_bytes())?;
        env.call_method(activity, "biometricEnroll", "([B)V", &[JValue::Object(&bytes)])?;
        Ok(())
    })
}

/// Asks for a fingerprint and decrypts the stored password; poll [`take_biometric_result`].
pub fn biometric_unlock() -> Result<(), String> {
    with_activity(|env, activity| {
        env.call_method(activity, "biometricUnlock", "()V", &[])?;
        Ok(())
    })
}

pub fn biometric_disable() {
    let _ = with_activity(|env, activity| {
        env.call_method(activity, "biometricDisable", "()V", &[])?;
        Ok(())
    });
}

/// The outcome of [`biometric_unlock`]: the password, or why it failed (`None` inside the error
/// means the user chose to type the password instead).
pub fn take_biometric_result() -> Option<Result<zeroize::Zeroizing<String>, Option<String>>> {
    with_activity(|env, activity| {
        let data = env.call_method(activity, "takeBiometricPassword", "()[B", &[])?.l()?;
        if !data.is_null() {
            let data = JByteArray::from(data);
            let bytes = zeroize::Zeroizing::new(env.convert_byte_array(&data)?);
            // Wipe the Java copy too.
            let zeros = vec![0i8; bytes.len()];
            env.set_byte_array_region(&data, 0, &zeros)?;
            return Ok(Some(match std::str::from_utf8(&bytes) {
                Ok(text) => Ok(zeroize::Zeroizing::new(text.to_owned())),
                Err(_) => Err(Some("The stored password is damaged".to_owned())),
            }));
        }
        let error = env.call_method(activity, "takeBiometricError", "()Ljava/lang/String;", &[])?.l()?;
        if error.is_null() {
            return Ok(None);
        }
        let error: String = env.get_string(&JString::from(error))?.into();
        Ok(Some(Err(if error == "-" { None } else { Some(error) })))
    })
    .ok()
    .flatten()
}

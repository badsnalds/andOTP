//! Desktop QR scanning with a webcam (see `scanner`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use eframe::egui;

use crate::scanner::Status;
use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{CameraIndex, RequestedFormat, RequestedFormatType};

/// State shared between the capture thread and the UI.
#[derive(Default)]
struct Shared {
    frame: Option<egui::ColorImage>,
    /// Contents of the first QR code found.
    result: Option<String>,
    error: Option<String>,
}

/// A running capture; stops the camera when dropped.
pub struct Scanner {
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    texture: Option<egui::TextureHandle>,
}

impl Scanner {
    pub fn start(ctx: &egui::Context) -> Self {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (shared, stop, ctx) = (shared.clone(), stop.clone(), ctx.clone());
            std::thread::spawn(move || {
                if let Err(err) = capture(&shared, &stop, &ctx) {
                    shared.lock().unwrap().error = Some(err);
                }
                ctx.request_repaint();
            })
        };
        Self { shared, stop, thread: Some(thread), texture: None }
    }

    /// Uploads the latest frame and reports the scan state.
    pub fn poll(&mut self, ctx: &egui::Context) -> Status {
        let mut shared = self.shared.lock().unwrap();
        if let Some(frame) = shared.frame.take() {
            match &mut self.texture {
                Some(texture) => texture.set(frame, egui::TextureOptions::LINEAR),
                None => self.texture = Some(ctx.load_texture("webcam", frame, egui::TextureOptions::LINEAR)),
            }
        }
        if let Some(result) = shared.result.take() {
            return Status::Found(result);
        }
        if let Some(err) = shared.error.clone() {
            return Status::Failed(err);
        }
        if self.texture.is_some() { Status::Scanning } else { Status::Starting }
    }

    pub fn texture(&self) -> Option<&egui::TextureHandle> {
        self.texture.as_ref()
    }
}

impl Drop for Scanner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn capture(shared: &Mutex<Shared>, stop: &AtomicBool, ctx: &egui::Context) -> Result<(), String> {
    // Asks for camera permission on macOS; a no-op elsewhere.
    let (tx, rx) = std::sync::mpsc::channel();
    nokhwa::nokhwa_initialize(move |granted| {
        let _ = tx.send(granted);
    });
    if !rx.recv().unwrap_or(false) {
        return Err("Camera access was denied".into());
    }

    let format = RequestedFormat::new::<RgbFormat>(RequestedFormatType::AbsoluteHighestFrameRate);
    let mut camera =
        nokhwa::Camera::new(CameraIndex::Index(0), format).map_err(|e| format!("No usable camera: {e}"))?;
    camera.open_stream().map_err(|e| format!("Cannot start the camera: {e}"))?;

    while !stop.load(Ordering::Relaxed) {
        let frame = camera.frame().map_err(|e| format!("Camera error: {e}"))?;
        let rgb = frame.decode_image::<RgbFormat>().map_err(|e| format!("Cannot decode frame: {e}"))?;
        let (width, height) = (rgb.width(), rgb.height());
        let found = crate::widgets::decode_otpauth(&image::DynamicImage::ImageRgb8(rgb.clone()).to_luma8());
        let preview = egui::ColorImage::from_rgb([width as usize, height as usize], rgb.as_raw());
        {
            let mut shared = shared.lock().unwrap();
            shared.frame = Some(preview);
            if found.is_some() {
                shared.result = found;
            }
        }
        ctx.request_repaint();
        if shared.lock().unwrap().result.is_some() {
            break;
        }
    }
    let _ = camera.stop_stream();
    Ok(())
}

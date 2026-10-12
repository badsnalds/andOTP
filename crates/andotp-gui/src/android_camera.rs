//! Android QR scanning with the device camera (see `scanner`). `MainActivity` runs Camera2 and
//! keeps the latest greyscale frame; a worker thread here polls it, looks for a QR code and
//! builds the preview.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use eframe::egui;
use image::GrayImage;
use image::imageops::{self, FilterType};

use crate::android;
use crate::scanner::Status;

/// Largest side of the preview texture, in pixels.
const PREVIEW_MAX: u32 = 480;
/// Frames larger than this (longest side) are scaled down before decoding.
const DECODE_MAX: u32 = 1024;

#[derive(Default)]
struct Shared {
    frame: Option<egui::ColorImage>,
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
        if let Err(err) = android::camera_start() {
            shared.lock().unwrap().error = Some(err);
            return Self { shared, stop, thread: None, texture: None };
        }
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
                None => self.texture = Some(ctx.load_texture("camera", frame, egui::TextureOptions::LINEAR)),
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
        android::camera_stop();
    }
}

fn capture(shared: &Mutex<Shared>, stop: &AtomicBool, ctx: &egui::Context) -> Result<(), String> {
    while !stop.load(Ordering::Relaxed) {
        if let Some(err) = android::take_camera_error() {
            return Err(err);
        }
        let Some((frame, rotation)) = android::take_camera_frame() else {
            std::thread::sleep(Duration::from_millis(40));
            continue;
        };
        let found = crate::widgets::decode_otpauth(&shrink(&frame, DECODE_MAX));
        let preview = rotate(&shrink(&frame, PREVIEW_MAX), rotation);
        let preview =
            egui::ColorImage::from_gray([preview.width() as usize, preview.height() as usize], preview.as_raw());
        {
            let mut shared = shared.lock().unwrap();
            shared.frame = Some(preview);
            if found.is_some() {
                shared.result = found;
                break;
            }
        }
        ctx.request_repaint();
    }
    ctx.request_repaint();
    Ok(())
}

fn shrink(image: &GrayImage, max: u32) -> GrayImage {
    let longest = image.width().max(image.height());
    if longest <= max {
        return image.clone();
    }
    let scale = max as f32 / longest as f32;
    let (w, h) = ((image.width() as f32 * scale) as u32, (image.height() as f32 * scale) as u32);
    imageops::resize(image, w.max(1), h.max(1), FilterType::Triangle)
}

/// Turns a sensor-oriented frame upright (`degrees` is the sensor orientation).
fn rotate(image: &GrayImage, degrees: i32) -> GrayImage {
    match degrees.rem_euclid(360) {
        90 => imageops::rotate90(image),
        180 => imageops::rotate180(image),
        270 => imageops::rotate270(image),
        _ => image.clone(),
    }
}

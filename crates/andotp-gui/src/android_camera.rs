//! Android QR scanning with the device camera (see `scanner`). `MainActivity` runs Camera2 and
//! hands over greyscale frames; a capture thread here turns each into a small preview right away
//! and passes the newest one to a decoder thread that looks for a QR code.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use eframe::egui;
use image::GrayImage;
use image::imageops;

use crate::android;
use crate::scanner::Status;

/// Largest side of the preview texture, in pixels.
const PREVIEW_MAX: u32 = 480;
/// Frames larger than this (longest side) are scaled down before decoding: plenty for a QR code
/// held in front of the camera, and much faster to decode than full camera frames.
const DECODE_MAX: u32 = 800;

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

/// Shows every frame as it arrives and hands the newest one to a separate decoder thread, so a
/// slow QR decode never holds up the preview.
fn capture(shared: &Arc<Mutex<Shared>>, stop: &Arc<AtomicBool>, ctx: &egui::Context) -> Result<(), String> {
    // The frame waiting to be decoded; replaced by newer frames while the decoder is busy.
    let pending: Arc<(Mutex<Option<GrayImage>>, Condvar)> = Arc::default();
    let decoder = {
        let (pending, shared, stop, ctx) = (pending.clone(), shared.clone(), stop.clone(), ctx.clone());
        std::thread::spawn(move || decode_loop(&pending, &shared, &stop, &ctx))
    };

    let result = (|| {
        while !stop.load(Ordering::Relaxed) {
            if let Some(err) = android::take_camera_error() {
                return Err(err);
            }
            let Some((frame, rotation)) = android::take_camera_frame() else {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            };
            let preview = rotate(&shrink(&frame, PREVIEW_MAX), rotation);
            let preview =
                egui::ColorImage::from_gray([preview.width() as usize, preview.height() as usize], preview.as_raw());
            shared.lock().unwrap().frame = Some(preview);
            ctx.request_repaint();
            {
                let (slot, ready) = &*pending;
                *slot.lock().unwrap() = Some(frame);
                ready.notify_one();
            }
            if shared.lock().unwrap().result.is_some() {
                break;
            }
        }
        Ok(())
    })();

    stop.store(true, Ordering::Relaxed);
    pending.1.notify_one();
    let _ = decoder.join();
    ctx.request_repaint();
    result
}

fn decode_loop(
    pending: &(Mutex<Option<GrayImage>>, Condvar),
    shared: &Mutex<Shared>,
    stop: &AtomicBool,
    ctx: &egui::Context,
) {
    let (slot, ready) = pending;
    loop {
        let frame = {
            let mut slot = slot.lock().unwrap();
            loop {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                if let Some(frame) = slot.take() {
                    break frame;
                }
                slot = ready.wait_timeout(slot, Duration::from_millis(100)).unwrap().0;
            }
        };
        if let Some(uri) = crate::widgets::decode_otpauth(&shrink(&frame, DECODE_MAX)) {
            shared.lock().unwrap().result = Some(uri);
            stop.store(true, Ordering::Relaxed); // ends the capture loop too
            ctx.request_repaint();
            return;
        }
    }
}

/// Scales `image` down so its longest side is at most `max`, with a fast box filter.
fn shrink(image: &GrayImage, max: u32) -> GrayImage {
    let longest = image.width().max(image.height());
    if longest <= max {
        return image.clone();
    }
    let scale = max as f32 / longest as f32;
    let (w, h) = ((image.width() as f32 * scale) as u32, (image.height() as f32 * scale) as u32);
    imageops::thumbnail(image, w.max(1), h.max(1))
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

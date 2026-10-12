//! Small custom widgets.

use eframe::egui::{self, Color32, Pos2, Shape, Stroke, Vec2};

/// A coloured circle with the first letter of `name`, like the Android app's letter thumbnails.
pub fn letter_avatar(ui: &mut egui::Ui, name: &str, size: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    let letter = name.trim().chars().next().map_or('#', |c| c.to_uppercase().next().unwrap_or(c));
    let hash = name.to_lowercase().bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ u32::from(b));
    let color = PALETTE[(hash as usize) % PALETTE.len()];
    let painter = ui.painter();
    painter.circle_filled(rect.center(), size / 2.0, color);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        letter,
        egui::FontId::proportional(size * 0.5),
        Color32::WHITE,
    );
    response
}

const PALETTE: [Color32; 10] = [
    Color32::from_rgb(0xE5, 0x39, 0x35),
    Color32::from_rgb(0xD8, 0x1B, 0x60),
    Color32::from_rgb(0x8E, 0x24, 0xAA),
    Color32::from_rgb(0x5E, 0x35, 0xB1),
    Color32::from_rgb(0x39, 0x49, 0xAB),
    Color32::from_rgb(0x1E, 0x88, 0xE5),
    Color32::from_rgb(0x00, 0x89, 0x7B),
    Color32::from_rgb(0x43, 0xA0, 0x47),
    Color32::from_rgb(0xF4, 0x51, 0x1E),
    Color32::from_rgb(0x6D, 0x4C, 0x41),
];

/// A circular countdown showing the remaining fraction of a token's validity.
pub fn countdown(ui: &mut egui::Ui, fraction: f32, seconds: u32, warn: bool, size: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    let visuals = ui.visuals();
    let color = if warn { visuals.error_fg_color } else { visuals.selection.bg_fill };
    let center = rect.center();
    let radius = size / 2.0 - 2.0;
    let painter = ui.painter();
    painter.circle_stroke(center, radius, Stroke::new(3.0, visuals.widgets.noninteractive.bg_stroke.color));
    let steps = 48;
    let sweep = fraction.clamp(0.0, 1.0) * std::f32::consts::TAU;
    if sweep > 0.0 {
        let points: Vec<Pos2> = (0..=steps)
            .map(|i| {
                let angle = -std::f32::consts::FRAC_PI_2 + sweep * i as f32 / steps as f32;
                center + radius * Vec2::angled(angle)
            })
            .collect();
        painter.add(Shape::line(points, Stroke::new(3.0, color)));
    }
    painter.text(
        center,
        egui::Align2::CENTER_CENTER,
        seconds.to_string(),
        egui::FontId::proportional(size * 0.36),
        if warn { color } else { visuals.text_color() },
    );
    response
}

/// Renders `text` as a QR code texture.
pub fn qr_texture(ctx: &egui::Context, text: &str) -> Option<egui::TextureHandle> {
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    let width = code.width();
    let quiet = 4;
    let size = width + 2 * quiet;
    let colors = code.to_colors();
    let mut pixels = vec![Color32::WHITE; size * size];
    for y in 0..width {
        for x in 0..width {
            if colors[y * width + x] == qrcode::Color::Dark {
                pixels[(y + quiet) * size + x + quiet] = Color32::BLACK;
            }
        }
    }
    let image = egui::ColorImage::new([size, size], pixels);
    Some(ctx.load_texture("qr", image, egui::TextureOptions::NEAREST))
}

/// Finds and decodes the first `otpauth://` QR code in an image file.
#[cfg_attr(target_os = "android", allow(dead_code))]
pub fn scan_qr(path: &std::path::Path) -> Result<String, String> {
    let image = image::open(path).map_err(|e| format!("Cannot open image: {e}"))?.to_luma8();
    decode_otpauth(&image).ok_or_else(|| "No account QR code found in the image".to_owned())
}

/// Like [`scan_qr`], for an image file's contents.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn scan_qr_bytes(data: &[u8]) -> Result<String, String> {
    let image = image::load_from_memory(data).map_err(|e| format!("Cannot open image: {e}"))?.to_luma8();
    decode_otpauth(&image).ok_or_else(|| "No account QR code found in the image".to_owned())
}

/// Returns the content of the first `otpauth://` QR code in the image.
pub fn decode_otpauth(image: &image::GrayImage) -> Option<String> {
    let mut prepared = rqrr::PreparedImage::prepare(image.clone());
    prepared
        .detect_grids()
        .into_iter()
        .filter_map(|grid| grid.decode().ok().map(|(_, content)| content))
        .find(|content| content.to_ascii_lowercase().starts_with("otpauth://"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Renders `text` as a QR code at `scale` pixels per module.
    fn qr_image(text: &str, scale: u32) -> image::GrayImage {
        let code = qrcode::QrCode::new(text.as_bytes()).unwrap();
        let width = code.width() as u32;
        let colors = code.to_colors();
        let size = (width + 8) * scale;
        image::GrayImage::from_fn(size, size, |x, y| {
            let (mx, my) = ((x / scale).wrapping_sub(4), (y / scale).wrapping_sub(4));
            let dark = mx < width && my < width && colors[(my * width + mx) as usize] == qrcode::Color::Dark;
            image::Luma([if dark { 0 } else { 255 }])
        })
    }

    #[test]
    fn decodes_account_qr_codes_only() {
        let uri = "otpauth://totp/GitHub:me?secret=JBSWY3DPEHPK3PXP&issuer=GitHub";
        assert_eq!(decode_otpauth(&qr_image(uri, 4)).as_deref(), Some(uri));
        assert_eq!(decode_otpauth(&qr_image("https://example.com", 4)), None);
        assert_eq!(decode_otpauth(&image::GrayImage::new(64, 64)), None);
    }
}

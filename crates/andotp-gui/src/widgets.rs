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

/// Finds and decodes the first QR code in an image file.
pub fn scan_qr(path: &std::path::Path) -> Result<String, String> {
    let image = image::open(path).map_err(|e| format!("Cannot open image: {e}"))?.to_luma8();
    let mut prepared = rqrr::PreparedImage::prepare(image);
    prepared
        .detect_grids()
        .into_iter()
        .find_map(|grid| grid.decode().ok().map(|(_, content)| content))
        .ok_or_else(|| "No QR code found in the image".to_owned())
}

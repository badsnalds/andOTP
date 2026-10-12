//! Service logos bundled with the app – the thumbnails of the original andOTP, converted to SVG.
//!
//! The logos are embedded rather than downloaded: fetching them would tell a server which
//! accounts are in the vault.

use eframe::egui;
use resvg::{tiny_skia, usvg};

include!(concat!(env!("OUT_DIR"), "/thumbnails.rs"));

/// The SVG of a thumbnail (names as stored in andOTP backups, e.g. `"GitHub"`).
pub fn svg(name: &str) -> Option<&'static [u8]> {
    THUMBNAILS.binary_search_by(|(n, _)| n.cmp(&name)).ok().map(|i| THUMBNAILS[i].1)
}

/// Renders an SVG centred in a transparent `px` × `px` square.
pub fn rasterize(svg: &[u8], px: u32) -> Option<egui::ColorImage> {
    let tree = usvg::Tree::from_data(svg, &usvg::Options::default()).ok()?;
    let size = tree.size();
    let scale = px as f32 / size.width().max(size.height());
    let dx = (px as f32 - size.width() * scale) / 2.0;
    let dy = (px as f32 - size.height() * scale) / 2.0;
    let mut pixmap = tiny_skia::Pixmap::new(px, px)?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale).post_translate(dx, dy), &mut pixmap.as_mut());
    Some(egui::ColorImage::from_rgba_premultiplied([px as usize, px as usize], pixmap.data()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_logo_renders() {
        assert!(THUMBNAILS.len() > 300);
        for (name, data) in THUMBNAILS {
            let image = rasterize(data, 48).unwrap_or_else(|| panic!("{name} does not render"));
            let visible = image.pixels.iter().filter(|p| p.a() > 0).count();
            assert!(visible > 20, "{name} renders empty");
        }
    }

    #[test]
    fn names_match_the_core_list() {
        for (name, _) in THUMBNAILS {
            assert_eq!(andotp_core::thumbnail::lookup(name), Some(*name), "{name} is unknown to andotp-core");
        }
        assert!(svg("GitHub").is_some());
        assert!(svg("Default").is_none());
    }
}

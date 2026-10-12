//! A look modelled on the Mantine UI kit (<https://mantine.dev>): its default colour palette,
//! the Inter typeface, 4 px controls and 8 px cards, bordered inputs and filled primary buttons.
//! Colours follow Mantine v7: the light scheme uses shade 6 of the primary colour, the dark
//! scheme shade 8 on the `dark` grey scale.

use std::sync::Arc;

use eframe::egui::{self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Stroke, TextStyle};

/// Font family for titles, buttons and other emphasised text (Inter SemiBold).
pub const SEMIBOLD: &str = "Inter SemiBold";

/// Mantine's `sm` and `md` radii.
pub const RADIUS_SM: u8 = 4;
pub const RADIUS_MD: u8 = 8;

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// Mantine's `blue`, `red`, `orange`, `gray` and `dark` scales (the shades used here).
mod mantine {
    use super::hex;
    use eframe::egui::Color32;

    pub const BLUE_0: Color32 = hex(0xe7f5ff);
    pub const BLUE_4: Color32 = hex(0x4dabf7);
    pub const BLUE_6: Color32 = hex(0x228be6);
    pub const BLUE_7: Color32 = hex(0x1c7ed6);
    pub const BLUE_8: Color32 = hex(0x1971c2);
    pub const BLUE_9: Color32 = hex(0x1864ab);
    pub const RED_6: Color32 = hex(0xfa5252);
    pub const RED_4: Color32 = hex(0xff8787);
    pub const ORANGE_6: Color32 = hex(0xfd7e14);
    pub const ORANGE_4: Color32 = hex(0xffa94d);
    pub const GRAY_0: Color32 = hex(0xf8f9fa);
    pub const GRAY_1: Color32 = hex(0xf1f3f5);
    pub const GRAY_3: Color32 = hex(0xdee2e6);
    pub const GRAY_4: Color32 = hex(0xced4da);
    pub const GRAY_6: Color32 = hex(0x868e96);
    pub const BLACK: Color32 = hex(0x000000);
    pub const DARK_0: Color32 = hex(0xc9c9c9);
    pub const DARK_2: Color32 = hex(0x828282);
    pub const DARK_4: Color32 = hex(0x424242);
    pub const DARK_5: Color32 = hex(0x3b3b3b);
    pub const DARK_6: Color32 = hex(0x2e2e2e);
    pub const DARK_7: Color32 = hex(0x242424);
    pub const DARK_8: Color32 = hex(0x1f1f1f);
}

/// The colours of one scheme.
struct Palette {
    dark: bool,
    /// Page background (`body`).
    body: Color32,
    /// Cards, modals and menus (`Paper`).
    paper: Color32,
    /// Text inputs.
    input: Color32,
    /// Borders of inputs, cards and default buttons.
    border: Color32,
    /// Default (outlined) buttons and their hover state.
    control: Color32,
    control_hover: Color32,
    control_active: Color32,
    text: Color32,
    dimmed: Color32,
    primary: Color32,
    primary_hover: Color32,
    /// Light variant background of the primary colour (selected items).
    primary_light: Color32,
    /// Readable primary colour for links and outlines.
    primary_text: Color32,
    error: Color32,
    warning: Color32,
}

const LIGHT: Palette = Palette {
    dark: false,
    body: Color32::WHITE,
    paper: Color32::WHITE,
    input: Color32::WHITE,
    border: mantine::GRAY_4,
    control: Color32::WHITE,
    control_hover: mantine::GRAY_0,
    control_active: mantine::GRAY_1,
    text: mantine::BLACK,
    dimmed: mantine::GRAY_6,
    primary: mantine::BLUE_6,
    primary_hover: mantine::BLUE_7,
    primary_light: mantine::BLUE_0,
    primary_text: mantine::BLUE_6,
    error: mantine::RED_6,
    warning: mantine::ORANGE_6,
};

const DARK: Palette = Palette {
    dark: true,
    body: mantine::DARK_7,
    paper: mantine::DARK_7,
    input: mantine::DARK_6,
    border: mantine::DARK_4,
    control: mantine::DARK_6,
    control_hover: mantine::DARK_5,
    control_active: mantine::DARK_4,
    text: mantine::DARK_0,
    dimmed: mantine::DARK_2,
    primary: mantine::BLUE_8,
    primary_hover: mantine::BLUE_9,
    // rgba(25, 113, 194, 0.15) over the dark body.
    primary_light: Color32::from_rgb(0x23, 0x35, 0x47),
    primary_text: mantine::BLUE_4,
    error: mantine::RED_4,
    warning: mantine::ORANGE_4,
};

/// Installs the fonts and both colour schemes.
pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.set_visuals_of(egui::Theme::Light, visuals(&LIGHT));
    ctx.set_visuals_of(egui::Theme::Dark, visuals(&DARK));
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, egui::FontId::proportional(12.0)),
            (TextStyle::Body, egui::FontId::proportional(14.0)),
            (TextStyle::Button, egui::FontId::proportional(14.0)),
            (TextStyle::Heading, egui::FontId::new(20.0, FontFamily::Name(SEMIBOLD.into()))),
            (TextStyle::Monospace, egui::FontId::monospace(13.0)),
        ]
        .into();
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.interact_size.y = 30.0;
        style.spacing.window_margin = egui::Margin::same(16);
        style.spacing.menu_margin = egui::Margin::same(6);
    });
}

fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("Inter".into(), Arc::new(FontData::from_static(include_bytes!("../fonts/Inter-Regular.ttf"))));
    fonts
        .font_data
        .insert(SEMIBOLD.into(), Arc::new(FontData::from_static(include_bytes!("../fonts/Inter-SemiBold.ttf"))));
    // egui's own fonts stay as fallbacks for emoji and symbols that Inter lacks.
    let fallback = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "Inter".into());
    let mut semibold = vec![SEMIBOLD.to_owned()];
    semibold.extend(fallback);
    fonts.families.insert(FontFamily::Name(SEMIBOLD.into()), semibold);
    fonts
}

fn visuals(p: &Palette) -> egui::Visuals {
    let mut v = if p.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    let sm = CornerRadius::same(RADIUS_SM);
    let md = CornerRadius::same(RADIUS_MD);

    v.override_text_color = None;
    v.weak_text_color = Some(p.dimmed);
    v.hyperlink_color = p.primary_text;
    v.panel_fill = p.body;
    v.window_fill = p.paper;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = md;
    v.menu_corner_radius = md;
    v.window_shadow = egui::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(48) };
    v.popup_shadow = egui::Shadow { offset: [0, 2], blur: 8, spread: 0, color: Color32::from_black_alpha(32) };
    v.faint_bg_color = if p.dark { mantine::DARK_6 } else { mantine::GRAY_0 };
    v.extreme_bg_color = p.input;
    v.text_edit_bg_color = Some(p.input);
    v.code_bg_color = if p.dark { mantine::DARK_8 } else { mantine::GRAY_1 };
    v.error_fg_color = p.error;
    v.warn_fg_color = p.warning;
    // Selected toggles and the focused input outline: Mantine's "light" variant.
    v.selection.bg_fill = p.primary_light;
    v.selection.stroke = Stroke::new(1.0, p.primary_text);
    v.text_cursor.stroke = Stroke::new(2.0, p.primary_text);
    v.striped = false;
    v.indent_has_left_vline = false;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.paper;
    w.noninteractive.weak_bg_fill = p.paper;
    w.noninteractive.bg_stroke = Stroke::new(1.0, if p.dark { mantine::DARK_4 } else { mantine::GRAY_3 });
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = sm;

    w.inactive.bg_fill = p.control;
    w.inactive.weak_bg_fill = p.control;
    w.inactive.bg_stroke = Stroke::new(1.0, p.border);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = sm;
    w.inactive.expansion = 0.0;

    w.hovered.bg_fill = p.control_hover;
    w.hovered.weak_bg_fill = p.control_hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.border);
    w.hovered.fg_stroke = Stroke::new(1.0, p.text);
    w.hovered.corner_radius = sm;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = p.control_active;
    w.active.weak_bg_fill = p.control_active;
    w.active.bg_stroke = Stroke::new(1.0, p.primary_text);
    w.active.fg_stroke = Stroke::new(1.0, p.text);
    w.active.corner_radius = sm;
    w.active.expansion = 0.0;

    w.open.bg_fill = p.control_hover;
    w.open.weak_bg_fill = p.control_hover;
    w.open.bg_stroke = Stroke::new(1.0, p.border);
    w.open.fg_stroke = Stroke::new(1.0, p.text);
    w.open.corner_radius = sm;
    v
}

fn palette(ui: &egui::Ui) -> &'static Palette {
    if ui.visuals().dark_mode { &DARK } else { &LIGHT }
}

/// The filled primary colour (primary buttons).
pub fn primary(ui: &egui::Ui) -> Color32 {
    palette(ui).primary
}

/// Hover shade of [`primary`].
pub fn primary_hover(ui: &egui::Ui) -> Color32 {
    palette(ui).primary_hover
}

/// The light variant of the primary colour, a background for selected items.
pub fn primary_light(ui: &egui::Ui) -> Color32 {
    palette(ui).primary_light
}

/// Primary colour for text, outlines and icons.
pub fn primary_text(ui: &egui::Ui) -> Color32 {
    ui.visuals().hyperlink_color
}

/// Text in the semibold weight.
pub fn semibold(text: impl Into<String>, size: f32) -> egui::RichText {
    egui::RichText::new(text).family(FontFamily::Name(SEMIBOLD.into())).size(size)
}

#[cfg(test)]
mod tests {
    #[test]
    fn fonts_load() {
        let ctx = eframe::egui::Context::default();
        super::apply(&ctx);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            ui.label(super::semibold("Add account", 20.0));
            ui.label("Inter body text …");
        });
        assert!(!output.shapes.is_empty());
        output.textures_delta.clear();
    }
}

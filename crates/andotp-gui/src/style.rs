//! Shared look of dialogs: headers, labelled fields and buttons.

use eframe::egui::{self, Color32, RichText};

/// Accent colour for primary actions.
pub const ACCENT: Color32 = Color32::from_rgb(0x1E, 0x88, 0xE5);

/// Width of a dialog: most of a phone screen, capped on large windows.
pub fn dialog_width(ctx: &egui::Context) -> f32 {
    (ctx.content_rect().width() - 32.0).clamp(260.0, 440.0)
}

/// Dialog title with an optional one-line explanation.
pub fn header(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.label(RichText::new(title).size(20.0).strong());
    if !subtitle.is_empty() {
        ui.label(RichText::new(subtitle).weak());
    }
    ui.add_space(10.0);
}

/// A field label above its input.
pub fn label(ui: &mut egui::Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(text).small().strong());
}

/// A full-width, comfortably tall text field.
pub fn text_field<'t>(text: &'t mut dyn egui::TextBuffer) -> egui::TextEdit<'t> {
    egui::TextEdit::singleline(text).desired_width(f32::INFINITY).margin(egui::Margin::symmetric(10, 8))
}

/// The main action of a dialog: filled with the accent colour.
pub fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let button = egui::Button::new(RichText::new(text).strong().color(Color32::WHITE))
        .fill(ACCENT)
        .corner_radius(8.0)
        .min_size(egui::vec2(96.0, 36.0));
    ui.add(button)
}

/// A secondary action (Cancel, Close).
pub fn secondary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(egui::Button::new(text).corner_radius(8.0).min_size(egui::vec2(80.0, 36.0)))
}

/// A large tile button with an icon and a caption, for the main choices of a dialog.
pub fn tile_button(ui: &mut egui::Ui, icon: &str, text: &str, width: f32) -> egui::Response {
    let button = egui::Button::new(RichText::new(format!("{icon}\n{text}")).size(14.0))
        .corner_radius(10.0)
        .min_size(egui::vec2(width, 64.0));
    ui.add(button)
}

/// Lays out the dialog's buttons on the right; `add` adds them right to left (primary first).
pub fn button_row(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(12.0);
    let size = egui::vec2(ui.available_width(), 40.0);
    ui.allocate_ui_with_layout(size, egui::Layout::right_to_left(egui::Align::Center), add);
}

/// An error message line.
pub fn error(ui: &mut egui::Ui, text: &str) {
    ui.add_space(6.0);
    ui.colored_label(ui.visuals().error_fg_color, text);
}

/// A full-width selectable card with an icon, a title and a description (radio-button style).
pub fn option_card(ui: &mut egui::Ui, selected: bool, icon: &str, title: &str, description: &str) -> egui::Response {
    const RADIO: f32 = 28.0;
    let visuals = ui.visuals().clone();
    let stroke = if selected {
        egui::Stroke::new(2.0, ACCENT)
    } else {
        egui::Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color)
    };
    let fill = if selected { ACCENT.gamma_multiply(0.12) } else { visuals.faint_bg_color };
    let response = egui::Frame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon).size(22.0));
                ui.vertical(|ui| {
                    ui.set_max_width(ui.available_width() - RADIO);
                    ui.label(RichText::new(title).strong());
                    ui.add(egui::Label::new(RichText::new(description).small().weak()).wrap());
                });
            });
        })
        .response;
    // The radio indicator, painted so it does not depend on font glyphs.
    let center = egui::pos2(response.rect.right() - 12.0 - RADIO / 2.0 + 6.0, response.rect.center().y);
    let painter = ui.painter();
    if selected {
        painter.circle_stroke(center, 9.0, egui::Stroke::new(2.0, ACCENT));
        painter.circle_filled(center, 5.0, ACCENT);
    } else {
        painter.circle_stroke(center, 9.0, egui::Stroke::new(1.5, visuals.weak_text_color()));
    }

    let response = response.interact(egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, title));
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    ui.add_space(6.0);
    response
}

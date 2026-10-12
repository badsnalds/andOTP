//! Shared look of dialogs, after Mantine's components: modal title, labelled inputs, filled and
//! default buttons, and radio cards. Colours come from [`crate::theme`].

use eframe::egui::{self, Color32, RichText};

use crate::theme::{self, RADIUS_MD, RADIUS_SM};

/// Inner padding of dialogs (Mantine `Modal` padding `md`).
pub const MODAL_PADDING: i8 = 16;

/// The frame of a dialog.
pub fn modal_frame(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::popup(&ctx.global_style()).inner_margin(MODAL_PADDING).corner_radius(RADIUS_MD)
}

/// Height of buttons and inputs (Mantine `sm` size).
const CONTROL_HEIGHT: f32 = 36.0;

/// Width of a dialog: most of a phone screen, capped on large windows.
pub fn dialog_width(ctx: &egui::Context) -> f32 {
    (ctx.content_rect().width() - 2.0 * (16.0 + MODAL_PADDING as f32)).clamp(240.0, 420.0)
}

/// Dialog title with an optional one-line description.
pub fn header(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.label(theme::semibold(title, 18.0));
    if !subtitle.is_empty() {
        ui.label(RichText::new(subtitle).size(13.0).weak());
    }
    ui.add_space(8.0);
}

/// An input label (Mantine `Input.Label`).
pub fn label(ui: &mut egui::Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(theme::semibold(text, 13.0));
}

/// A full-width text input.
pub fn text_field<'t>(text: &'t mut dyn egui::TextBuffer) -> egui::TextEdit<'t> {
    egui::TextEdit::singleline(text)
        .desired_width(f32::INFINITY)
        .min_size(egui::vec2(0.0, CONTROL_HEIGHT))
        .vertical_align(egui::Align::Center)
        .margin(egui::Margin::symmetric(12, 6))
}

/// Whether the next widget added to `ui` was hovered last frame.
fn next_hovered(ui: &egui::Ui) -> bool {
    ui.ctx().read_response(ui.next_auto_id()).is_some_and(|r| r.hovered())
}

/// A filled button (Mantine's `filled` variant).
pub fn primary(ui: &egui::Ui, text: &str) -> egui::Button<'static> {
    let fill = if next_hovered(ui) { theme::primary_hover(ui) } else { theme::primary(ui) };
    egui::Button::new(theme::semibold(text, 14.0).color(Color32::WHITE))
        .fill(fill)
        .stroke(egui::Stroke::NONE)
        .corner_radius(RADIUS_SM)
}

/// The main action of a dialog: a filled button.
pub fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let button = primary(ui, text).min_size(egui::vec2(88.0, CONTROL_HEIGHT));
    ui.add(button)
}

/// A secondary action (Cancel, Close): Mantine's `default` variant.
pub fn secondary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let button = egui::Button::new(theme::semibold(text, 14.0))
        .corner_radius(RADIUS_SM)
        .min_size(egui::vec2(80.0, CONTROL_HEIGHT));
    ui.add(button)
}

/// A large tile button with an icon and a caption, for the main choices of a dialog.
pub fn tile_button(ui: &mut egui::Ui, icon: &str, text: &str, width: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 68.0), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{icon}\n{text}")));
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        ui.painter().rect(rect, RADIUS_MD, visuals.bg_fill, visuals.bg_stroke, egui::StrokeKind::Inside);
        let painter = ui.painter();
        painter.text(
            rect.center() - egui::vec2(0.0, 10.0),
            egui::Align2::CENTER_CENTER,
            icon,
            egui::FontId::proportional(20.0),
            theme::primary_text(ui),
        );
        painter.text(
            rect.center() + egui::vec2(0.0, 14.0),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::new(13.0, egui::FontFamily::Name(theme::SEMIBOLD.into())),
            ui.visuals().text_color(),
        );
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// Lays out the dialog's buttons on the right; `add` adds them right to left (primary first).
pub fn button_row(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(12.0);
    let size = egui::vec2(ui.available_width(), CONTROL_HEIGHT + 4.0);
    ui.allocate_ui_with_layout(size, egui::Layout::right_to_left(egui::Align::Center), add);
}

/// An error message line.
pub fn error(ui: &mut egui::Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(text).size(13.0).color(ui.visuals().error_fg_color));
}

/// A full-width selectable card with an icon, a title and a description (radio-button style).
pub fn option_card(ui: &mut egui::Ui, selected: bool, icon: &str, title: &str, description: &str) -> egui::Response {
    const RADIO: f32 = 28.0;
    let visuals = ui.visuals().clone();
    let primary = theme::primary(ui);
    let stroke = egui::Stroke::new(
        1.0,
        if selected { theme::primary_text(ui) } else { visuals.widgets.inactive.bg_stroke.color },
    );
    let fill = if selected { theme::primary_light(ui) } else { visuals.window_fill };
    let response = egui::Frame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(RADIUS_MD)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon).size(22.0));
                ui.vertical(|ui| {
                    ui.set_max_width(ui.available_width() - RADIO);
                    ui.label(theme::semibold(title, 14.0));
                    ui.add(egui::Label::new(RichText::new(description).small().weak()).wrap());
                });
            });
        })
        .response;
    // The radio indicator, painted so it does not depend on font glyphs.
    let center = egui::pos2(response.rect.right() - 12.0 - RADIO / 2.0 + 6.0, response.rect.center().y);
    let painter = ui.painter();
    if selected {
        // Mantine's checked radio: a filled circle with a white dot.
        painter.circle_filled(center, 10.0, primary);
        painter.circle_filled(center, 4.0, Color32::WHITE);
    } else {
        painter.circle_stroke(center, 9.5, egui::Stroke::new(1.0, visuals.widgets.inactive.bg_stroke.color));
    }

    let response = response.interact(egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, title));
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    ui.add_space(6.0);
    response
}

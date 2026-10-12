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
    ui.add_space(2.0);
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

/// An outlined button (Mantine's `default` variant).
pub fn secondary(text: &str) -> egui::Button<'static> {
    egui::Button::new(theme::semibold(text, 14.0)).corner_radius(RADIUS_SM)
}

/// A secondary action (Cancel, Close).
pub fn secondary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(secondary(text).min_size(egui::vec2(80.0, CONTROL_HEIGHT)))
}

/// A large tile button with an icon and a caption, for the main choices of a dialog.
pub fn tile_button(ui: &mut egui::Ui, icon: &str, text: &str, width: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 58.0), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{icon}\n{text}")));
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        ui.painter().rect(rect, RADIUS_MD, visuals.bg_fill, visuals.bg_stroke, egui::StrokeKind::Inside);
        let painter = ui.painter();
        painter.text(
            rect.center() - egui::vec2(0.0, 9.0),
            egui::Align2::CENTER_CENTER,
            icon,
            egui::FontId::proportional(20.0),
            theme::primary_text(ui),
        );
        painter.text(
            rect.center() + egui::vec2(0.0, 12.0),
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

/// A destructive action: a filled red button (Mantine `color="red"`).
pub fn danger_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    let red =
        if ui.visuals().dark_mode { Color32::from_rgb(0xe0, 0x31, 0x31) } else { Color32::from_rgb(0xfa, 0x52, 0x52) };
    let button = egui::Button::new(theme::semibold(text, 14.0).color(Color32::WHITE))
        .fill(red)
        .stroke(egui::Stroke::NONE)
        .corner_radius(RADIUS_SM)
        .min_size(egui::vec2(88.0, CONTROL_HEIGHT));
    ui.add_enabled(enabled, button)
}

/// A small dimmed heading that groups the rows of a settings list.
pub fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(theme::semibold(title.to_uppercase(), 11.0).color(ui.visuals().weak_text_color()));
    ui.add_space(2.0);
}

/// A label with an optional description on the left and a control on the right, the layout of
/// Mantine's Switch with `label` and `description`.
pub fn setting_row<R>(
    ui: &mut egui::Ui,
    label: &str,
    description: &str,
    control_width: f32,
    control: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.horizontal(|ui| {
        let text_width = (ui.available_width() - control_width - 12.0).max(80.0);
        ui.vertical(|ui| {
            ui.set_width(text_width);
            ui.label(RichText::new(label).size(14.0));
            if !description.is_empty() {
                ui.add(egui::Label::new(RichText::new(description).size(12.0).weak()).wrap());
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), control).inner
    })
    .inner
}

/// A Mantine Switch: a pill with a sliding knob. Returns true when toggled.
pub fn switch(ui: &mut egui::Ui, on: &mut bool, label: &str) -> egui::Response {
    let size = egui::vec2(38.0, 22.0);
    let (rect, mut response) = ui.allocate_exact_size(size, egui::Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, *on, label));
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_responsive(response.id, *on);
        let off = ui.visuals().widgets.inactive.bg_stroke.color;
        let track = if *on {
            theme::primary(ui)
        } else {
            if ui.visuals().dark_mode { off } else { Color32::from_rgb(0xe9, 0xec, 0xef) }
        };
        let painter = ui.painter();
        painter.rect(
            rect,
            rect.height() / 2.0,
            track,
            egui::Stroke::new(1.0, if *on { track } else { off }),
            egui::StrokeKind::Inside,
        );
        let radius = rect.height() / 2.0 - 3.0;
        let x = egui::lerp((rect.left() + radius + 3.0)..=(rect.right() - radius - 3.0), t);
        painter.circle_filled(egui::pos2(x, rect.center().y), radius, Color32::WHITE);
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// A Mantine SegmentedControl: equal segments in a tinted track, the selected one raised.
/// Returns true when the value changed.
pub fn segmented<T: PartialEq + Copy>(ui: &mut egui::Ui, value: &mut T, options: &[(T, &str)], width: f32) -> bool {
    let height = 32.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let dark = ui.visuals().dark_mode;
    let track = if dark { Color32::from_rgb(0x1f, 0x1f, 0x1f) } else { Color32::from_rgb(0xf1, 0xf3, 0xf5) };
    ui.painter().rect_filled(rect, RADIUS_MD, track);
    let segment = (rect.width() - 8.0) / options.len() as f32;
    // Several controls can share a Ui: tell them apart by their first option.
    let base = ui.make_persistent_id(("segmented", options.first().map(|o| o.1), options.len()));
    let mut changed = false;
    for (i, (option, text)) in options.iter().enumerate() {
        let seg = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 4.0 + i as f32 * segment, rect.top() + 4.0),
            egui::vec2(segment, height - 8.0),
        );
        let id = base.with(i);
        let response = ui.interact(seg, id, egui::Sense::click());
        let selected = *value == *option;
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, *text));
        if response.clicked() && !selected {
            *value = *option;
            changed = true;
        }
        let painter = ui.painter();
        if selected {
            let fill = if dark { Color32::from_rgb(0x42, 0x42, 0x42) } else { Color32::WHITE };
            painter.rect_filled(seg.translate(egui::vec2(0.0, 1.0)), RADIUS_SM, Color32::from_black_alpha(18));
            painter.rect_filled(seg, RADIUS_SM, fill);
        }
        let color = if selected || response.hovered() {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().weak_text_color()
        };
        painter.text(
            seg.center(),
            egui::Align2::CENTER_CENTER,
            *text,
            egui::FontId::new(13.0, egui::FontFamily::Name(theme::SEMIBOLD.into())),
            color,
        );
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }
    changed
}

/// A tinted message box (Mantine `Alert`, light variant).
pub fn alert(ui: &mut egui::Ui, color: Color32, text: &str) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.12))
        .stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.5)))
        .corner_radius(RADIUS_MD)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(text).size(13.0).color(color)).wrap());
        });
}

/// A square icon button (Mantine `ActionIcon`, default variant).
pub fn icon_button(icon: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(icon).size(16.0))
        .corner_radius(RADIUS_SM)
        .min_size(egui::vec2(CONTROL_HEIGHT, CONTROL_HEIGHT))
}

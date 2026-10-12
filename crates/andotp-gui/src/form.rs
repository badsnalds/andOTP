//! The add / edit entry form.

use andotp_core::otp::HashAlgorithm;
use andotp_core::{Entry, OtpType, entry};
use eframe::egui;
use zeroize::Zeroizing;

use crate::style;

pub struct EntryForm {
    /// Index of the entry being edited, `None` when adding.
    pub index: Option<usize>,
    pub uri: Zeroizing<String>,
    pub otp_type: OtpType,
    pub issuer: String,
    pub label: String,
    pub secret: Zeroizing<String>,
    pub digits: u32,
    pub period: u32,
    pub counter: u64,
    pub algorithm: HashAlgorithm,
    pub tags: String,
    pub show_secret: bool,
    /// The "paste a link" field is open.
    pub show_uri: bool,
    pub error: Option<String>,
}

impl Default for EntryForm {
    fn default() -> Self {
        Self {
            index: None,
            uri: Zeroizing::default(),
            otp_type: OtpType::Totp,
            issuer: String::new(),
            label: String::new(),
            secret: Zeroizing::default(),
            digits: 6,
            period: 30,
            counter: 1,
            algorithm: HashAlgorithm::Sha1,
            tags: String::new(),
            show_secret: false,
            show_uri: false,
            error: None,
        }
    }
}

pub enum FormResult {
    None,
    Cancel,
    Save(Entry),
    /// Scan a QR code with the camera / from an image, then come back with the result.
    #[cfg(camera)]
    ScanCamera,
    ScanImage,
}

impl EntryForm {
    pub fn from_entry(index: Option<usize>, e: &Entry) -> Self {
        Self {
            index,
            otp_type: e.otp_type,
            issuer: e.issuer.clone(),
            label: e.label.clone(),
            secret: Zeroizing::new(e.secret_encoded()),
            digits: e.digits,
            period: e.period,
            counter: e.counter,
            algorithm: e.algorithm,
            tags: e.tags.join(", "),
            ..Self::default()
        }
    }

    fn fill_from_uri(&mut self) {
        match Entry::from_uri(&self.uri) {
            Ok(e) => {
                *self = Self::from_entry(self.index, &e);
                self.error = None;
            }
            Err(err) => self.error = Some(err.to_string()),
        }
    }

    fn build(&self, original: Option<&Entry>) -> Result<Entry, String> {
        let mut e = match original {
            Some(o) => o.clone(),
            None => Entry::default(),
        };
        let issuer_changed = e.issuer != self.issuer.trim();
        e.otp_type = self.otp_type;
        e.secret = entry::decode_secret(self.otp_type, &self.secret).map_err(|err| err.to_string())?;
        e.issuer = self.issuer.trim().to_owned();
        e.label = self.label.trim().to_owned();
        e.digits = self.digits;
        e.period = self.period;
        e.counter = self.counter;
        e.algorithm = self.algorithm;
        e.tags = self.tags.split(',').map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned).collect();
        e.tags.dedup();
        if original.is_none() || issuer_changed {
            e.update_thumbnail();
        }
        if e.label.is_empty() && e.issuer.is_empty() {
            return Err("Enter an issuer or a label".into());
        }
        e.validate().map_err(|err| err.to_string())?;
        Ok(e)
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, original: Option<&Entry>) -> FormResult {
        let mut result = FormResult::None;
        let adding = self.index.is_none();
        if adding {
            style::header(ui, "Add account", "Scan the QR code shown by the service, or enter the details.");
            ui.horizontal(|ui| {
                let tiles = if cfg!(camera) { 3.0 } else { 2.0 };
                let width = (ui.available_width() - ui.spacing().item_spacing.x * (tiles - 1.0)) / tiles;
                #[cfg(camera)]
                if style::tile_button(ui, "📷", "Camera", width).clicked() {
                    result = FormResult::ScanCamera;
                }
                if style::tile_button(ui, "🖼", "Image", width).clicked() {
                    result = FormResult::ScanImage;
                }
                if style::tile_button(ui, "🔗", "Paste link", width).clicked() {
                    self.show_uri = !self.show_uri;
                }
            });
            if self.show_uri {
                style::label(ui, "otpauth:// link");
                ui.horizontal(|ui| {
                    let edit = ui.add(
                        style::text_field(&mut *self.uri)
                            .hint_text("otpauth://totp/Issuer:account?secret=…")
                            .desired_width(ui.available_width() - 72.0),
                    );
                    let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if ui.button("Apply").clicked() || enter {
                        self.fill_from_uri();
                    }
                });
            }
            ui.add_space(6.0);
            ui.separator();
        } else {
            style::header(ui, "Edit account", "");
        }

        style::label(ui, "Service");
        ui.add(style::text_field(&mut self.issuer).hint_text("e.g. GitHub"));
        style::label(ui, "Account");
        ui.add(style::text_field(&mut self.label).hint_text("e.g. you@example.com"));
        style::label(ui, "Secret key");
        ui.horizontal(|ui| {
            ui.add(
                style::text_field(&mut *self.secret)
                    .password(!self.show_secret)
                    .hint_text(if self.otp_type == OtpType::Motp { "hex" } else { "Base32, e.g. JBSW Y3DP …" })
                    .desired_width(ui.available_width() - 44.0),
            );
            ui.toggle_value(&mut self.show_secret, "👁").on_hover_text("Show secret");
        });

        style::label(ui, "Type");
        ui.horizontal(|ui| {
            for t in OtpType::ALL {
                if ui.selectable_value(&mut self.otp_type, t, t.as_str()).changed() {
                    self.digits = t.default_digits();
                }
            }
        });

        ui.add_space(4.0);
        egui::CollapsingHeader::new("Advanced").default_open(!adding).show(ui, |ui| {
            egui::Grid::new("entry_advanced").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("Digits");
                ui.add(egui::DragValue::new(&mut self.digits).range(1..=10));
                ui.end_row();
                match self.otp_type {
                    OtpType::Totp | OtpType::Steam => {
                        ui.label("Period");
                        ui.add(egui::DragValue::new(&mut self.period).range(1..=3600).suffix(" s"));
                        ui.end_row();
                    }
                    OtpType::Hotp => {
                        ui.label("Counter");
                        ui.add(egui::DragValue::new(&mut self.counter));
                        ui.end_row();
                    }
                    OtpType::Motp => {}
                }
                if self.otp_type != OtpType::Motp {
                    ui.label("Algorithm");
                    ui.horizontal(|ui| {
                        for a in HashAlgorithm::ALL {
                            ui.selectable_value(&mut self.algorithm, a, a.as_str());
                        }
                    });
                    ui.end_row();
                }
            });
        });

        style::label(ui, "Tags");
        ui.add(style::text_field(&mut self.tags).hint_text("work, personal"));

        if let Some(err) = &self.error {
            style::error(ui, err);
        }

        style::button_row(ui, |ui| {
            if style::primary_button(ui, "Save").clicked() {
                match self.build(original) {
                    Ok(e) => result = FormResult::Save(e),
                    Err(err) => self.error = Some(err),
                }
            }
            if style::secondary_button(ui, "Cancel").clicked() {
                result = FormResult::Cancel;
            }
        });
        result
    }
}

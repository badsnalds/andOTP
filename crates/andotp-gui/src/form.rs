//! The add / edit entry form.

use andotp_core::otp::HashAlgorithm;
use andotp_core::{Entry, OtpType, entry};
use eframe::egui;
use zeroize::Zeroizing;

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
            error: None,
        }
    }
}

pub enum FormResult {
    None,
    Cancel,
    Save(Entry),
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
        ui.heading(if self.index.is_some() { "Edit account" } else { "Add account" });
        ui.add_space(8.0);

        if self.index.is_none() {
            ui.label("Paste an otpauth:// link to fill in the form:");
            ui.horizontal(|ui| {
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut *self.uri)
                        .hint_text("otpauth://totp/Issuer:account?secret=…")
                        .desired_width(ui.available_width() - 70.0),
                );
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.button("Apply").clicked() || enter {
                    self.fill_from_uri();
                }
            });
            ui.separator();
        }

        egui::Grid::new("entry_form").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Type");
            ui.horizontal(|ui| {
                for t in OtpType::ALL {
                    if ui.selectable_value(&mut self.otp_type, t, t.as_str()).changed() {
                        self.digits = t.default_digits();
                    }
                }
            });
            ui.end_row();

            ui.label("Issuer");
            ui.text_edit_singleline(&mut self.issuer);
            ui.end_row();

            ui.label("Account");
            ui.text_edit_singleline(&mut self.label);
            ui.end_row();

            ui.label("Secret");
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut *self.secret)
                        .password(!self.show_secret)
                        .hint_text(if self.otp_type == OtpType::Motp { "hex" } else { "Base32" }),
                );
                ui.toggle_value(&mut self.show_secret, "👁").on_hover_text("Show secret");
            });
            ui.end_row();

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

            ui.label("Tags");
            ui.add(egui::TextEdit::singleline(&mut self.tags).hint_text("work, personal"));
            ui.end_row();
        });

        if let Some(err) = &self.error {
            ui.add_space(4.0);
            ui.colored_label(ui.visuals().error_fg_color, err);
        }

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                result = FormResult::Cancel;
            }
            if ui.button(egui::RichText::new("Save").strong()).clicked() {
                match self.build(original) {
                    Ok(e) => result = FormResult::Save(e),
                    Err(err) => self.error = Some(err),
                }
            }
        });
        result
    }
}

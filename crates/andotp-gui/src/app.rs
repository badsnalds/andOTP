//! Application state and screens.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use andotp_core::backup::{self, BackupFormat};
#[cfg(not(target_os = "android"))]
use andotp_core::keychain;
use andotp_core::list::{self, SortMode, TagMode};
use andotp_core::otp;
use andotp_core::{Entry, OtpType, Vault};
use eframe::egui::{self, RichText};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use crate::form::{EntryForm, FormResult};
use crate::renderer::Renderer;
#[cfg(all(feature = "webcam", not(target_os = "android")))]
use crate::webcam;
use crate::widgets;

const SETTINGS_KEY: &str = "settings";
const MIN_PASSWORD_LENGTH: usize = 6;
const TOAST_SECONDS: f64 = 3.0;
const REVEAL_SECONDS: f64 = 15.0;
/// Time for the window to minimise and focus to return to the previous window before typing.
const AUTOTYPE_DELAY: Duration = Duration::from_millis(800);
/// Word to type to confirm wiping everything.
const WIPE_CONFIRMATION: &str = "DELETE";
/// Desktop-only features (keychain, typing into other windows, file dialogs, renderer choice).
const DESKTOP: bool = cfg!(not(target_os = "android"));

/// Android stand-ins for desktop APIs, so the shared code compiles unchanged. The UI hides the
/// corresponding features (see [`DESKTOP`]); file access goes through the system picker instead.
#[cfg(target_os = "android")]
mod keychain {
    use std::path::Path;
    pub fn load(_: &Path) -> Result<Option<zeroize::Zeroizing<String>>, String> {
        Ok(None)
    }
    pub fn store(_: &Path, _: &str) -> Result<(), String> {
        Err("The system keychain is not available on Android".into())
    }
    pub fn delete(_: &Path) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(target_os = "android")]
mod rfd {
    use std::path::PathBuf;
    pub struct FileDialog;
    impl FileDialog {
        pub fn new() -> Self {
            Self
        }
        pub fn set_title(self, _: &str) -> Self {
            self
        }
        pub fn add_filter(self, _: &str, _: &[&str]) -> Self {
            self
        }
        pub fn set_file_name(self, _: impl Into<String>) -> Self {
            self
        }
        pub fn pick_file(self) -> Option<PathBuf> {
            None
        }
        pub fn save_file(self) -> Option<PathBuf> {
            None
        }
        pub fn pick_folder(self) -> Option<PathBuf> {
            None
        }
    }
}

#[cfg(target_os = "android")]
fn type_text(_: &str, _: Duration) -> Result<(), String> {
    Err("Typing into other apps is not available on Android".into())
}

#[cfg(not(target_os = "android"))]
fn type_text(text: &str, delay: Duration) -> Result<(), String> {
    andotp_core::autotype::type_text(text, delay).map_err(|e| e.to_string())
}

fn copy_to_clipboard(ctx: &egui::Context, text: String) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        let _ = ctx;
        crate::android::copy(&text)
    }
    #[cfg(not(target_os = "android"))]
    {
        ctx.copy_text(text);
        Ok(())
    }
}

/// What a file picked with the Android document picker is for.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[derive(Clone, Copy)]
enum OpenPurpose {
    ImportBackup,
    ScanImage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Theme {
    System,
    Light,
    Dark,
}

/// User preferences, persisted by eframe (never contains secrets).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    theme: Theme,
    sort: SortMode,
    tag_mode: TagMode,
    /// Hide tokens until clicked.
    tap_to_reveal: bool,
    /// Digit group size (0 = no grouping).
    group_size: usize,
    /// Lock after this many minutes without input (0 = never).
    auto_lock_minutes: u32,
    show_previous: bool,
    vault_path: Option<PathBuf>,
    /// The vault password is stored in the system keychain.
    keychain: bool,
    /// Copy the encrypted vault to `backup_dir` after every change.
    auto_backup: bool,
    backup_dir: Option<PathBuf>,
    /// Number of automatic backups to keep (0 = all).
    backup_keep: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            sort: SortMode::Unsorted,
            tag_mode: TagMode::Or,
            tap_to_reveal: false,
            group_size: 3,
            auto_lock_minutes: 5,
            show_previous: false,
            vault_path: None,
            keychain: false,
            auto_backup: false,
            backup_dir: None,
            backup_keep: 10,
        }
    }
}

/// The lock screen.
#[derive(Default)]
struct Lock {
    password: Zeroizing<String>,
    confirm: Zeroizing<String>,
    error: Option<String>,
    /// Key derivation runs on a worker thread so the window stays responsive.
    pending: Option<Receiver<Result<Vault, String>>>,
}

enum Dialog {
    Entry(EntryForm),
    ConfirmDelete(usize),
    Qr {
        title: String,
        texture: egui::TextureHandle,
    },
    ImportPassword {
        data: Vec<u8>,
        password: Zeroizing<String>,
        error: Option<String>,
    },
    Export {
        format: BackupFormat,
        password: Zeroizing<String>,
        confirm: Zeroizing<String>,
        error: Option<String>,
    },
    ChangePassword {
        password: Zeroizing<String>,
        confirm: Zeroizing<String>,
        error: Option<String>,
    },
    KeychainEnable {
        password: Zeroizing<String>,
        error: Option<String>,
    },
    Wipe {
        confirm: String,
        error: Option<String>,
    },
    #[cfg(all(feature = "webcam", not(target_os = "android")))]
    Webcam(webcam::Scanner),
    Settings,
    About,
}

/// An unlocked vault and its view state.
struct Session {
    vault: Vault,
    query: String,
    selected_tags: Vec<String>,
    /// mOTP PINs, kept in memory only.
    pins: HashMap<usize, Zeroizing<String>>,
    /// Entry index → time until which the token is revealed.
    revealed: HashMap<usize, f64>,
    dialog: Option<Dialog>,
    /// Set by a dialog to return to the lock screen (e.g. after wiping the vault).
    lock_after: bool,
    /// Android: a document picker is open for this purpose.
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    pending_open: Option<OpenPurpose>,
}

enum Screen {
    Locked(Lock),
    Unlocked(Box<Session>),
}

struct Toast {
    text: String,
    error: bool,
    until: f64,
}

pub struct AndOtpApp {
    settings: Settings,
    /// Vault given on the command line / environment; not persisted.
    vault_override: Option<PathBuf>,
    screen: Screen,
    toast: Option<Toast>,
    last_activity: f64,
    /// Whether the automatic keychain unlock at start-up was attempted already.
    keychain_tried: bool,
    /// Messages from worker threads (e.g. auto-type failures).
    inbox: Arc<Mutex<Vec<(String, bool)>>>,
    /// Renderer stored for the next start.
    renderer: Renderer,
}

/// Something the user asked for while the UI was being drawn.
enum Action {
    Copy(usize),
    NextCounter(usize),
    Edit(usize),
    Delete(usize),
    ShowQr(usize),
    MoveUp(usize),
    MoveDown(usize),
    Reveal(usize),
    Type(usize),
}

impl AndOtpApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings: Settings = cc.storage.and_then(|s| eframe::get_value(s, SETTINGS_KEY)).unwrap_or_default();
        apply_theme(&cc.egui_ctx, settings.theme);
        Self {
            settings,
            vault_override: None,
            screen: Screen::Locked(Lock::default()),
            toast: None,
            last_activity: 0.0,
            keychain_tried: false,
            inbox: Arc::default(),
            renderer: Renderer::load(),
        }
    }

    /// Uses `path` as the vault file for this session (not persisted).
    pub fn set_vault_path(&mut self, path: PathBuf) {
        self.vault_override = Some(path);
    }

    fn vault_path(&self) -> PathBuf {
        self.vault_override
            .clone()
            .or_else(|| self.settings.vault_path.clone())
            .or_else(Vault::default_path)
            .unwrap_or_else(|| PathBuf::from(andotp_core::vault::FILE_NAME))
    }

    fn notify(&mut self, ctx: &egui::Context, text: impl Into<String>, error: bool) {
        let now = ctx.input(|i| i.time);
        self.toast = Some(Toast { text: text.into(), error, until: now + TOAST_SECONDS });
    }

    fn lock(&mut self) {
        self.screen = Screen::Locked(Lock::default());
    }

    // --- lock screen ----------------------------------------------------------------------

    fn lock_ui(&mut self, ui: &mut egui::Ui) {
        let path = self.vault_path();
        let exists = path.exists();
        let Screen::Locked(lock) = &mut self.screen else { return };

        if let Some(rx) = &lock.pending {
            match rx.try_recv() {
                Ok(Ok(vault)) => {
                    self.screen = Screen::Unlocked(Box::new(Session {
                        vault,
                        query: String::new(),
                        selected_tags: Vec::new(),
                        pins: HashMap::new(),
                        revealed: HashMap::new(),
                        dialog: None,
                        lock_after: false,
                        pending_open: None,
                    }));
                    return;
                }
                Ok(Err(err)) => {
                    lock.error = Some(err);
                    lock.pending = None;
                }
                Err(_) => ui.ctx().request_repaint_after(std::time::Duration::from_millis(50)),
            }
        }

        // Unlock from the system keychain once at start-up (not after an auto-lock, which would
        // defeat its purpose).
        let use_keychain = exists && self.settings.keychain;
        if use_keychain && !self.keychain_tried && lock.pending.is_none() {
            self.keychain_tried = true;
            lock.pending = Some(spawn_keychain_unlock(ui.ctx(), path.clone()));
        }
        self.keychain_tried = true;

        let busy = lock.pending.is_some();
        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.15);
                ui.label(RichText::new("🔐").size(56.0));
                ui.heading(RichText::new("andOTP").size(28.0).strong());
                ui.add_space(4.0);
                ui.label(if exists {
                    "Enter your password to unlock"
                } else {
                    "Create a password to protect your accounts"
                });
                ui.add_space(16.0);

                let width = 260.0;
                let pw = ui.add_enabled(
                    !busy,
                    egui::TextEdit::singleline(&mut *lock.password)
                        .password(true)
                        .hint_text("Password")
                        .desired_width(width),
                );
                if !busy && lock.password.is_empty() && lock.error.is_none() {
                    pw.request_focus();
                }
                let mut submit = pw.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if !exists {
                    let confirm = ui.add_enabled(
                        !busy,
                        egui::TextEdit::singleline(&mut *lock.confirm)
                            .password(true)
                            .hint_text("Repeat password")
                            .desired_width(width),
                    );
                    submit = confirm.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                }
                ui.add_space(8.0);
                if busy {
                    ui.spinner();
                } else {
                    let label = if exists { "Unlock" } else { "Create vault" };
                    submit |= ui.add_sized([width, 32.0], egui::Button::new(RichText::new(label).strong())).clicked();
                }
                if use_keychain
                    && !busy
                    && ui.add_sized([width, 28.0], egui::Button::new("🔑 Unlock with system keychain")).clicked()
                {
                    lock.error = None;
                    lock.pending = Some(spawn_keychain_unlock(ui.ctx(), path.clone()));
                }
                if let Some(err) = &lock.error {
                    ui.add_space(8.0);
                    ui.colored_label(ui.visuals().error_fg_color, err);
                }

                if submit && !busy {
                    lock.error = None;
                    if !exists && lock.password.chars().count() < MIN_PASSWORD_LENGTH {
                        lock.error = Some(format!("Use at least {MIN_PASSWORD_LENGTH} characters"));
                    } else if !exists && *lock.password != *lock.confirm {
                        lock.error = Some("The passwords do not match".into());
                    } else {
                        let (tx, rx) = channel();
                        let password = std::mem::take(&mut lock.password);
                        lock.confirm.zeroize();
                        let path = path.clone();
                        let ctx = ui.ctx().clone();
                        std::thread::spawn(move || {
                            let result = if exists {
                                Vault::open(&path, &password).map_err(|e| match e {
                                    andotp_core::Error::Decryption => "Wrong password".to_owned(),
                                    other => other.to_string(),
                                })
                            } else {
                                Vault::create(&path, &password).map_err(|e| e.to_string())
                            };
                            let _ = tx.send(result);
                            ctx.request_repaint();
                        });
                        lock.pending = Some(rx);
                    }
                }
                let busy = lock.pending.is_some();

                ui.add_space(24.0);
                ui.label(RichText::new(path.display().to_string()).small().weak());
                if DESKTOP
                    && !busy
                    && ui.small_button("Use another vault file…").clicked()
                    && let Some(p) = rfd::FileDialog::new()
                        .set_title("Choose or create a vault file")
                        .add_filter("andOTP vault", &["aes"])
                        .set_file_name(andotp_core::vault::FILE_NAME)
                        .save_file()
                {
                    self.settings.vault_path = Some(p);
                    self.vault_override = None;
                }
            });
        });
    }

    // --- main screen ----------------------------------------------------------------------

    fn main_ui(&mut self, ui: &mut egui::Ui) {
        let now_time = ui.input(|i| i.time);
        let mut lock_requested = false;
        let mut messages: Vec<(String, bool)> = Vec::new();
        let settings = &mut self.settings;
        let renderer = &mut self.renderer;
        let inbox = &self.inbox;
        let Screen::Unlocked(session) = &mut self.screen else { return };

        // Top bar: search, filters and menu.
        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.menu_button("☰", |ui| {
                    if ui.button("➕ Add account…").clicked() {
                        session.dialog = Some(Dialog::Entry(EntryForm::default()));
                    }
                    #[cfg(target_os = "android")]
                    if ui.button("📷 Scan QR code from image…").clicked() {
                        match crate::android::open_document("image/*") {
                            Ok(()) => session.pending_open = Some(OpenPurpose::ScanImage),
                            Err(err) => messages.push((err, true)),
                        }
                    }
                    #[cfg(target_os = "android")]
                    if ui.button("📥 Import backup…").clicked() {
                        match crate::android::open_document("*/*") {
                            Ok(()) => session.pending_open = Some(OpenPurpose::ImportBackup),
                            Err(err) => messages.push((err, true)),
                        }
                    }
                    if DESKTOP
                        && ui.button("📷 Scan QR code from image…").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp", "bmp"])
                            .pick_file()
                    {
                        match widgets::scan_qr(&path).and_then(|uri| Entry::from_uri(&uri).map_err(|e| e.to_string())) {
                            Ok(entry) => session.dialog = Some(Dialog::Entry(EntryForm::from_entry(None, &entry))),
                            Err(err) => messages.push((err, true)),
                        }
                    }
                    #[cfg(all(feature = "webcam", not(target_os = "android")))]
                    if ui.button("📷 Scan QR code with webcam…").clicked() {
                        session.dialog = Some(Dialog::Webcam(webcam::Scanner::start(ui.ctx())));
                    }
                    ui.separator();
                    if DESKTOP
                        && ui.button("📥 Import backup…").clicked()
                        && let Some(path) =
                            rfd::FileDialog::new().add_filter("andOTP backups", &["json", "aes"]).pick_file()
                    {
                        match std::fs::read(&path) {
                            Ok(data) => messages.extend(open_backup(session, settings, data)),
                            Err(err) => messages.push((format!("Cannot read file: {err}"), true)),
                        }
                    }
                    if ui.button("📤 Export encrypted backup…").clicked() {
                        session.dialog = Some(Dialog::Export {
                            format: BackupFormat::Encrypted,
                            password: Zeroizing::default(),
                            confirm: Zeroizing::default(),
                            error: None,
                        });
                    }
                    if ui.button("📤 Export plain-text backup…").clicked() {
                        session.dialog = Some(Dialog::Export {
                            format: BackupFormat::Plain,
                            password: Zeroizing::default(),
                            confirm: Zeroizing::default(),
                            error: None,
                        });
                    }
                    ui.separator();
                    if ui.button("🔑 Change password…").clicked() {
                        session.dialog = Some(Dialog::ChangePassword {
                            password: Zeroizing::default(),
                            confirm: Zeroizing::default(),
                            error: None,
                        });
                    }
                    if ui.button("⚙ Settings…").clicked() {
                        session.dialog = Some(Dialog::Settings);
                    }
                    if ui.button("ℹ About").clicked() {
                        session.dialog = Some(Dialog::About);
                    }
                    ui.separator();
                    if ui.button(RichText::new("☢ Wipe all data…").color(ui.visuals().error_fg_color)).clicked() {
                        session.dialog = Some(Dialog::Wipe { confirm: String::new(), error: None });
                    }
                });
                ui.add(
                    egui::TextEdit::singleline(&mut session.query)
                        .hint_text("🔍 Search")
                        .desired_width(ui.available_width() - 110.0),
                );
                if ui.button("🔒").on_hover_text("Lock").clicked() {
                    lock_requested = true;
                }
                if ui.button("➕").on_hover_text("Add account").clicked() {
                    session.dialog = Some(Dialog::Entry(EntryForm::default()));
                }
            });
            ui.horizontal_wrapped(|ui| {
                egui::ComboBox::from_id_salt("sort").selected_text(format!("⬍ {}", settings.sort.label())).show_ui(
                    ui,
                    |ui| {
                        for mode in SortMode::ALL {
                            ui.selectable_value(&mut settings.sort, mode, mode.label());
                        }
                    },
                );
                let tags = list::all_tags(&session.vault.entries);
                session.selected_tags.retain(|t| tags.contains(t));
                for tag in tags {
                    let mut on = session.selected_tags.contains(&tag);
                    if ui.toggle_value(&mut on, format!("🏷 {tag}")).changed() {
                        if on {
                            session.selected_tags.push(tag);
                        } else {
                            session.selected_tags.retain(|t| *t != tag);
                        }
                    }
                }
                if session.selected_tags.len() > 1 {
                    ui.selectable_value(&mut settings.tag_mode, TagMode::Or, "any");
                    ui.selectable_value(&mut settings.tag_mode, TagMode::And, "all");
                }
            });
            ui.add_space(4.0);
        });

        // Entry list.
        let mut actions = Vec::new();
        egui::CentralPanel::default().show(ui, |ui| {
            let view = list::view(
                &session.vault.entries,
                &session.query,
                &session.selected_tags,
                settings.tag_mode,
                settings.sort,
            );
            if session.vault.entries.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(80.0);
                    ui.label(RichText::new("No accounts yet").size(18.0));
                    ui.label("Add one with ➕, scan a QR code image or import an andOTP backup from the ☰ menu.");
                });
                return;
            }
            if view.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.label("Nothing matches your search.");
                });
            }
            let manual = settings.sort == SortMode::Unsorted;
            let count = session.vault.entries.len();
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                for idx in view {
                    let entry = &session.vault.entries[idx];
                    let pin = session.pins.entry(idx).or_default();
                    let revealed =
                        !settings.tap_to_reveal || session.revealed.get(&idx).is_some_and(|until| *until > now_time);
                    entry_card(ui, idx, entry, pin, revealed, settings, manual, count, &mut actions);
                    ui.add_space(6.0);
                }
            });
        });

        #[cfg(target_os = "android")]
        poll_android(session, settings, &mut messages);

        // Apply actions.
        let mut dirty = false;
        for action in actions {
            match action {
                Action::Copy(i) => {
                    let pin = session.pins.get(&i).map(|p| p.as_str());
                    let token = session.vault.entries[i].token(pin);
                    if token.code == andotp_core::entry::MOTP_NO_PIN_CODE {
                        messages.push(("Enter the PIN first".into(), true));
                        continue;
                    }
                    match copy_to_clipboard(ui.ctx(), token.code) {
                        Ok(()) => {
                            session.vault.entries[i].mark_used();
                            dirty = true;
                            messages.push(("Copied to clipboard".into(), false));
                        }
                        Err(err) => messages.push((err, true)),
                    }
                }
                Action::NextCounter(i) => {
                    session.vault.entries[i].counter += 1;
                    dirty = true;
                }
                Action::Type(i) => {
                    let pin = session.pins.get(&i).map(|p| p.as_str());
                    let token = session.vault.entries[i].token(pin);
                    if token.code == andotp_core::entry::MOTP_NO_PIN_CODE {
                        messages.push(("Enter the PIN first".into(), true));
                        continue;
                    }
                    // Hand the focus back to the previous window, then type there.
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    let code = Zeroizing::new(token.code);
                    let inbox = inbox.clone();
                    let ctx = ui.ctx().clone();
                    std::thread::spawn(move || {
                        if let Err(err) = type_text(&code, AUTOTYPE_DELAY) {
                            inbox.lock().unwrap().push((err, true));
                            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                        }
                        ctx.request_repaint();
                    });
                    let entry = &mut session.vault.entries[i];
                    if entry.otp_type == OtpType::Hotp {
                        entry.counter += 1;
                    }
                    entry.mark_used();
                    dirty = true;
                }
                Action::Reveal(i) => {
                    session.revealed.insert(i, now_time + REVEAL_SECONDS);
                }
                Action::Edit(i) => {
                    session.dialog = Some(Dialog::Entry(EntryForm::from_entry(Some(i), &session.vault.entries[i])));
                }
                Action::Delete(i) => session.dialog = Some(Dialog::ConfirmDelete(i)),
                Action::ShowQr(i) => {
                    let entry = &session.vault.entries[i];
                    if let Some(texture) = widgets::qr_texture(ui.ctx(), &entry.to_uri()) {
                        session.dialog = Some(Dialog::Qr { title: entry.display_name(), texture });
                    }
                }
                Action::MoveUp(i) if i > 0 => {
                    session.vault.move_entry(i, i - 1);
                    session.pins.clear();
                    dirty = true;
                }
                Action::MoveDown(i) => {
                    session.vault.move_entry(i, i + 1);
                    session.pins.clear();
                    dirty = true;
                }
                Action::MoveUp(_) => {}
            }
        }

        if let Some(msg) = dialog_ui(ui.ctx(), session, settings, renderer) {
            messages.push(msg);
        }

        if dirty && let Err(err) = persist(&session.vault, settings) {
            messages.push((err, true));
        }
        lock_requested |= session.lock_after;
        messages.append(&mut inbox.lock().unwrap());

        for (text, error) in messages {
            self.notify(ui.ctx(), text, error);
        }
        if lock_requested {
            self.lock();
        }
    }
}

fn spawn_keychain_unlock(ctx: &egui::Context, path: PathBuf) -> Receiver<Result<Vault, String>> {
    let (tx, rx) = channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let result = match keychain::load(&path) {
            Ok(Some(password)) => Vault::open(&path, &password).map_err(|e| match e {
                andotp_core::Error::Decryption => "The password in the keychain is out of date".to_owned(),
                other => other.to_string(),
            }),
            Ok(None) => Err("No password stored in the system keychain".to_owned()),
            Err(err) => Err(err.to_string()),
        };
        let _ = tx.send(result);
        ctx.request_repaint();
    });
    rx
}

/// Saves the vault and, if enabled, writes an automatic backup.
fn persist(vault: &Vault, settings: &Settings) -> Result<(), String> {
    vault.save().map_err(|e| format!("Saving failed: {e}"))?;
    auto_backup(vault, settings)
}

fn auto_backup(vault: &Vault, settings: &Settings) -> Result<(), String> {
    let Some(dir) = settings.backup_dir.as_ref().filter(|_| settings.auto_backup) else { return Ok(()) };
    let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    vault.backup_to(dir, &timestamp).map_err(|e| format!("Automatic backup failed: {e}"))?;
    if settings.backup_keep > 0 {
        backup::prune(dir, settings.backup_keep).map_err(|e| format!("Cannot remove old backups: {e}"))?;
    }
    Ok(())
}

/// Imports a backup file: directly if it is plain text, otherwise after asking for its password.
fn open_backup(session: &mut Session, settings: &Settings, data: Vec<u8>) -> Option<(String, bool)> {
    match backup::import(&data, None) {
        Ok((entries, _)) => Some(import_entries(&mut session.vault, settings, entries)),
        Err(_) => {
            session.dialog = Some(Dialog::ImportPassword { data, password: Zeroizing::default(), error: None });
            None
        }
    }
}

/// Picks up results of the Android document picker.
#[cfg(target_os = "android")]
fn poll_android(session: &mut Session, settings: &Settings, messages: &mut Vec<(String, bool)>) {
    if let Some(message) = crate::android::take_message() {
        if let Some(cancelled) = message.strip_prefix('-') {
            let _ = cancelled;
            session.pending_open = None;
        } else if let Some(error) = message.strip_prefix('!') {
            session.pending_open = None;
            messages.push((error.to_owned(), true));
        } else {
            messages.push((message, false));
        }
    }
    let Some(purpose) = session.pending_open else { return };
    let Some(data) = crate::android::take_opened() else { return };
    session.pending_open = None;
    match purpose {
        OpenPurpose::ImportBackup => messages.extend(open_backup(session, settings, data)),
        OpenPurpose::ScanImage => {
            match widgets::scan_qr_bytes(&data).and_then(|uri| Entry::from_uri(&uri).map_err(|e| e.to_string())) {
                Ok(entry) => session.dialog = Some(Dialog::Entry(EntryForm::from_entry(None, &entry))),
                Err(err) => messages.push((err, true)),
            }
        }
    }
}

fn import_entries(vault: &mut Vault, settings: &Settings, entries: Vec<Entry>) -> (String, bool) {
    let summary = vault.import(entries);
    match persist(vault, settings) {
        Ok(()) => (format!("Imported {} accounts ({} duplicates skipped)", summary.added, summary.skipped), false),
        Err(err) => (err, true),
    }
}

#[allow(clippy::too_many_arguments)]
fn entry_card(
    ui: &mut egui::Ui,
    idx: usize,
    entry: &Entry,
    pin: &mut Zeroizing<String>,
    revealed: bool,
    settings: &Settings,
    manual: bool,
    count: usize,
    actions: &mut Vec<Action>,
) {
    let token = entry.token(Some(pin.as_str()));
    egui::Frame::group(ui.style())
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .fill(ui.visuals().faint_bg_color)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let name = if entry.issuer.is_empty() { &entry.label } else { &entry.issuer };
                widgets::letter_avatar(ui, name, 40.0);
                ui.vertical(|ui| {
                    if entry.issuer.is_empty() {
                        ui.label(RichText::new(&entry.label).strong());
                    } else {
                        ui.label(RichText::new(&entry.issuer).strong());
                        if !entry.label.is_empty() {
                            ui.label(RichText::new(&entry.label).weak());
                        }
                    }
                    if !entry.tags.is_empty() {
                        ui.label(
                            RichText::new(entry.tags.iter().map(|t| format!("🏷 {t}")).collect::<Vec<_>>().join("  "))
                                .small()
                                .weak(),
                        );
                    }
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.menu_button("⏷", |ui| {
                        if ui.button("✏ Edit").clicked() {
                            actions.push(Action::Edit(idx));
                        }
                        if DESKTOP && ui.button("⌨ Type into last window").clicked() {
                            actions.push(Action::Type(idx));
                        }
                        if ui.button("🔳 Show QR code").clicked() {
                            actions.push(Action::ShowQr(idx));
                        }
                        if manual {
                            if idx > 0 && ui.button("⬆ Move up").clicked() {
                                actions.push(Action::MoveUp(idx));
                            }
                            if idx + 1 < count && ui.button("⬇ Move down").clicked() {
                                actions.push(Action::MoveDown(idx));
                            }
                        }
                        ui.separator();
                        if ui.button(RichText::new("🗑 Delete").color(ui.visuals().error_fg_color)).clicked() {
                            actions.push(Action::Delete(idx));
                        }
                    });
                    if ui.button("🗐").on_hover_text("Copy").clicked() {
                        actions.push(Action::Copy(idx));
                    }
                    if let Some(remaining) = token.remaining {
                        widgets::countdown(ui, token.progress(), remaining, token.is_expiring(), 34.0)
                            .on_hover_text(format!("Changes in {remaining} s"));
                    } else if entry.otp_type == OtpType::Hotp
                        && ui.button("⟳").on_hover_text(format!("Next code (counter {})", entry.counter)).clicked()
                    {
                        actions.push(Action::NextCounter(idx));
                    }

                    if entry.otp_type == OtpType::Motp && token.remaining.is_none() {
                        ui.add(
                            egui::TextEdit::singleline(&mut **pin).password(true).hint_text("PIN").desired_width(70.0),
                        );
                        return;
                    }
                    let shown = if revealed {
                        otp::group_token(&token.code, settings.group_size)
                    } else {
                        "•".repeat(token.code.chars().count())
                    };
                    let color = if token.is_expiring() {
                        ui.visuals().error_fg_color
                    } else {
                        ui.visuals().strong_text_color()
                    };
                    let code = ui
                        .add(
                            egui::Label::new(RichText::new(shown).monospace().size(24.0).color(color))
                                .sense(egui::Sense::click()),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(if revealed { "Click to copy" } else { "Click to reveal" });
                    if code.clicked() {
                        actions.push(if revealed { Action::Copy(idx) } else { Action::Reveal(idx) });
                    }
                    if revealed
                        && settings.show_previous
                        && let Some(prev) = &token.previous
                    {
                        ui.label(RichText::new(otp::group_token(prev, settings.group_size)).monospace().weak())
                            .on_hover_text("Previous token");
                    }
                });
            });
        });
}

/// Draws the open dialog (if any). Returns a message for the toast.
fn dialog_ui(
    ctx: &egui::Context,
    session: &mut Session,
    settings: &mut Settings,
    renderer: &mut Renderer,
) -> Option<(String, bool)> {
    let dialog = session.dialog.as_mut()?;
    let mut close = false;
    let mut message = None;
    let mut open_keychain_dialog = false;
    #[cfg_attr(not(all(feature = "webcam", not(target_os = "android"))), allow(unused_mut))]
    let mut next_dialog = None;

    let modal = egui::Modal::new(egui::Id::new("dialog")).show(ctx, |ui| {
        ui.set_max_width(420.0);
        match dialog {
            Dialog::Entry(form) => {
                let original = form.index.map(|i| &session.vault.entries[i]);
                match form.ui(ui, original) {
                    FormResult::None => {}
                    FormResult::Cancel => close = true,
                    FormResult::Save(entry) => {
                        let name = entry.display_name();
                        let duplicate = session
                            .vault
                            .entries
                            .iter()
                            .enumerate()
                            .any(|(i, e)| Some(i) != form.index && e.same_account(&entry));
                        if duplicate {
                            form.error = Some("This account already exists".into());
                        } else {
                            match form.index {
                                Some(i) => session.vault.entries[i] = entry,
                                None => session.vault.entries.push(entry),
                            }
                            message = Some(match persist(&session.vault, settings) {
                                Ok(()) => (format!("Saved {name}"), false),
                                Err(err) => (err, true),
                            });
                            close = true;
                        }
                    }
                }
            }
            Dialog::ConfirmDelete(i) => {
                let i = *i;
                ui.heading("Delete account?");
                ui.label(format!(
                    "“{}” will be removed. Make sure you no longer need it or have another way to sign in.",
                    session.vault.entries[i].display_name()
                ));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    if ui.button(RichText::new("Delete").color(ui.visuals().error_fg_color)).clicked() {
                        session.vault.entries.remove(i);
                        session.pins.clear();
                        session.revealed.clear();
                        message = Some(match persist(&session.vault, settings) {
                            Ok(()) => ("Account deleted".into(), false),
                            Err(err) => (err, true),
                        });
                        close = true;
                    }
                });
            }
            Dialog::Qr { title, texture } => {
                ui.heading(title.as_str());
                ui.label("Scan with andOTP or another authenticator app. Anyone who sees this code can generate your tokens.");
                ui.vertical_centered(|ui| {
                    ui.add(egui::Image::new(&*texture).fit_to_exact_size(egui::vec2(280.0, 280.0)));
                });
                if ui.button("Close").clicked() {
                    close = true;
                }
            }
            Dialog::ImportPassword { data, password, error } => {
                ui.heading("Encrypted backup");
                ui.label("Enter the password of the backup:");
                let edit = ui.add(egui::TextEdit::singleline(&mut **password).password(true));
                edit.request_focus();
                if let Some(err) = error {
                    ui.colored_label(ui.visuals().error_fg_color, err.as_str());
                }
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    if ui.button("Import").clicked() || enter {
                        match backup::import(data, Some(password)) {
                            Ok((entries, _)) => {
                                message = Some(import_entries(&mut session.vault, settings, entries));
                                close = true;
                            }
                            Err(err) => *error = Some(format!("Cannot decrypt: {err}")),
                        }
                    }
                });
            }
            Dialog::Export { format, password, confirm, error } => {
                let encrypted = *format != BackupFormat::Plain;
                ui.heading(if encrypted { "Export encrypted backup" } else { "Export plain-text backup" });
                if encrypted {
                    ui.label("The backup can be restored in andOTP for Android and andOTP desktop.");
                    ui.add(egui::TextEdit::singleline(&mut **password).password(true).hint_text("Backup password"));
                    ui.add(egui::TextEdit::singleline(&mut **confirm).password(true).hint_text("Repeat password"));
                } else {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        "⚠ A plain-text backup contains all secrets unencrypted. Store it somewhere safe and delete it when done.",
                    );
                }
                if let Some(err) = error {
                    ui.colored_label(ui.visuals().error_fg_color, err.as_str());
                }
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    if ui.button("Choose file…").clicked() {
                        if encrypted && password.chars().count() < MIN_PASSWORD_LENGTH {
                            *error = Some(format!("Use at least {MIN_PASSWORD_LENGTH} characters"));
                            return;
                        }
                        if encrypted && **password != **confirm {
                            *error = Some("The passwords do not match".into());
                            return;
                        }
                        let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
                        let name = format.file_name(Some(&timestamp));
                        let data = if encrypted {
                            backup::export_encrypted(&session.vault.entries, password)
                        } else {
                            backup::to_json(&session.vault.entries).map(String::into_bytes)
                        };
                        let data = match data {
                            Ok(data) => Zeroizing::new(data),
                            Err(err) => {
                                *error = Some(err.to_string());
                                return;
                            }
                        };
                        // Android: the system "save as" screen writes the file; the outcome
                        // arrives later as a message.
                        #[cfg(target_os = "android")]
                        {
                            let mime = if encrypted { "application/octet-stream" } else { "application/json" };
                            match crate::android::create_document(&name, mime, &data) {
                                Ok(()) => close = true,
                                Err(err) => *error = Some(err),
                            }
                        }
                        #[cfg(not(target_os = "android"))]
                        {
                            let Some(path) = rfd::FileDialog::new().set_file_name(name).save_file() else {
                                return;
                            };
                            match andotp_core::vault::write_private(&path, &data) {
                                Ok(()) => {
                                    message =
                                        Some((format!("Exported {} accounts", session.vault.entries.len()), false));
                                    close = true;
                                }
                                Err(err) => *error = Some(err.to_string()),
                            }
                        }
                    }
                });
            }
            Dialog::ChangePassword { password, confirm, error } => {
                ui.heading("Change password");
                ui.add(egui::TextEdit::singleline(&mut **password).password(true).hint_text("New password"));
                ui.add(egui::TextEdit::singleline(&mut **confirm).password(true).hint_text("Repeat new password"));
                if let Some(err) = error {
                    ui.colored_label(ui.visuals().error_fg_color, err.as_str());
                }
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    if ui.button("Change").clicked() {
                        if password.chars().count() < MIN_PASSWORD_LENGTH {
                            *error = Some(format!("Use at least {MIN_PASSWORD_LENGTH} characters"));
                        } else if **password != **confirm {
                            *error = Some("The passwords do not match".into());
                        } else {
                            match session.vault.change_password(password) {
                                Ok(()) => {
                                    let mut text = "Password changed".to_owned();
                                    if settings.keychain
                                        && let Err(err) = keychain::store(session.vault.path(), password)
                                    {
                                        text = format!("Password changed, but the keychain update failed: {err}");
                                    }
                                    if let Err(err) = auto_backup(&session.vault, settings) {
                                        text = err;
                                    }
                                    message = Some((text, false));
                                    close = true;
                                }
                                Err(err) => *error = Some(err.to_string()),
                            }
                        }
                    }
                });
            }
            Dialog::Settings => {
                ui.heading("Settings");
                if settings.auto_backup && settings.backup_dir.is_none() {
                    ui.colored_label(ui.visuals().warn_fg_color, "Choose a folder for automatic backups.");
                }
                egui::Grid::new("settings").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
                    ui.label("Theme");
                    ui.horizontal(|ui| {
                        for (theme, label) in [(Theme::System, "System"), (Theme::Light, "Light"), (Theme::Dark, "Dark")] {
                            if ui.selectable_value(&mut settings.theme, theme, label).changed() {
                                apply_theme(ctx, theme);
                            }
                        }
                    });
                    ui.end_row();

                    ui.label("Digit grouping");
                    ui.add(egui::Slider::new(&mut settings.group_size, 0..=5).text("digits"));
                    ui.end_row();

                    ui.label("Auto-lock");
                    ui.add(egui::Slider::new(&mut settings.auto_lock_minutes, 0..=60).text("min (0 = never)"));
                    ui.end_row();

                    ui.label("Privacy");
                    ui.checkbox(&mut settings.tap_to_reveal, "Hide tokens until clicked");
                    ui.end_row();

                    ui.label("");
                    ui.checkbox(&mut settings.show_previous, "Show previous token");
                    ui.end_row();

                    // Desktop only: on Android these need platform services we do not use.
                    if DESKTOP {
                    ui.label("Keychain");
                    let mut keychain_on = settings.keychain;
                    if ui
                        .checkbox(&mut keychain_on, "Unlock with the system keychain")
                        .on_hover_text("Stores the vault password in the OS credential store")
                        .changed()
                    {
                        if keychain_on {
                            open_keychain_dialog = true;
                        } else {
                            settings.keychain = false;
                            if let Err(err) = keychain::delete(session.vault.path()) {
                                message = Some((err.to_string(), true));
                            }
                        }
                    }
                    ui.end_row();

                    ui.label("Backups");
                    ui.checkbox(&mut settings.auto_backup, "Back up automatically after every change");
                    ui.end_row();

                    if settings.auto_backup {
                        ui.label("");
                        ui.horizontal(|ui| {
                            let label = settings
                                .backup_dir
                                .as_ref()
                                .map_or_else(|| "Choose folder…".to_owned(), |d| d.display().to_string());
                            if ui.button(label).clicked()
                                && let Some(dir) = rfd::FileDialog::new().set_title("Backup folder").pick_folder()
                            {
                                settings.backup_dir = Some(dir);
                                message = Some(match auto_backup(&session.vault, settings) {
                                    Ok(()) => ("Backup written".into(), false),
                                    Err(err) => (err, true),
                                });
                            }
                        });
                        ui.end_row();

                        ui.label("");
                        ui.add(egui::Slider::new(&mut settings.backup_keep, 0..=100).text("kept (0 = all)"));
                        ui.end_row();
                    }

                    ui.label("Renderer");
                    ui.horizontal(|ui| {
                        for choice in [Renderer::Glow, Renderer::Wgpu] {
                            if ui.selectable_value(renderer, choice, choice.label()).changed() {
                                message = Some(match renderer.save() {
                                    Ok(()) => ("Takes effect after a restart".into(), false),
                                    Err(err) => (format!("Cannot save the renderer choice: {err}"), true),
                                });
                            }
                        }
                    });
                    ui.end_row();
                    }
                });
                ui.label(RichText::new(format!("Vault: {}", session.vault.path().display())).small().weak());
                if ui.button("Close").clicked() {
                    close = true;
                }
            }
            Dialog::KeychainEnable { password, error } => {
                ui.heading("Unlock with the system keychain");
                ui.label(
                    "Your vault password will be stored in the operating system's credential store, \
                     so andOTP can unlock without asking while you are logged in.",
                );
                let edit = ui.add(egui::TextEdit::singleline(&mut **password).password(true).hint_text("Vault password"));
                if let Some(err) = error {
                    ui.colored_label(ui.visuals().error_fg_color, err.as_str());
                }
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    if ui.button("Enable").clicked() || enter {
                        let path = session.vault.path().to_path_buf();
                        match Vault::open(&path, password) {
                            Err(andotp_core::Error::Decryption) => *error = Some("Wrong password".into()),
                            Err(err) => *error = Some(err.to_string()),
                            Ok(_) => match keychain::store(&path, password) {
                                Ok(()) => {
                                    settings.keychain = true;
                                    message = Some(("Password stored in the system keychain".into(), false));
                                    close = true;
                                }
                                Err(err) => *error = Some(err.to_string()),
                            },
                        }
                    }
                });
            }
            Dialog::Wipe { confirm, error } => {
                ui.heading(RichText::new("☢ Wipe all data").color(ui.visuals().error_fg_color));
                ui.label(format!(
                    "This irrecoverably destroys the vault with all {} accounts and removes the password from \
                     the system keychain. Backups are not touched. Make sure you can still sign in to your \
                     accounts.",
                    session.vault.entries.len()
                ));
                ui.label(format!("Type {WIPE_CONFIRMATION} to confirm:"));
                ui.text_edit_singleline(confirm);
                if let Some(err) = error {
                    ui.colored_label(ui.visuals().error_fg_color, err.as_str());
                }
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    let armed = confirm.trim() == WIPE_CONFIRMATION;
                    let button = egui::Button::new(RichText::new("Wipe").color(ui.visuals().error_fg_color));
                    if ui.add_enabled(armed, button).clicked() {
                        let path = session.vault.path().to_path_buf();
                        match andotp_core::vault::wipe(&path) {
                            Ok(_) => {
                                let _ = keychain::delete(&path);
                                settings.keychain = false;
                                session.vault.entries.clear();
                                session.lock_after = true;
                                message = Some(("All data wiped".into(), false));
                                close = true;
                            }
                            Err(err) => *error = Some(format!("Wiping failed: {err}")),
                        }
                    }
                });
            }
            #[cfg(all(feature = "webcam", not(target_os = "android")))]
            Dialog::Webcam(scanner) => {
                ui.heading("Scan QR code");
                match scanner.poll(ctx) {
                    webcam::Status::Starting => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Starting camera…");
                        });
                    }
                    webcam::Status::Scanning => {
                        ui.label("Hold the setup QR code in front of the camera.");
                    }
                    webcam::Status::Found(uri) => match Entry::from_uri(&uri) {
                        Ok(entry) => next_dialog = Some(Dialog::Entry(EntryForm::from_entry(None, &entry))),
                        Err(err) => message = Some((err.to_string(), true)),
                    },
                    webcam::Status::Failed(err) => {
                        ui.colored_label(ui.visuals().error_fg_color, err);
                    }
                }
                if let Some(texture) = scanner.texture() {
                    let size = texture.size_vec2();
                    let scale = (400.0 / size.x).min(300.0 / size.y).min(1.0);
                    ui.add(egui::Image::new(texture).fit_to_exact_size(size * scale));
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            }
            Dialog::About => {
                ui.heading(format!("andOTP {}", env!("CARGO_PKG_VERSION")));
                ui.label("Open-source two-factor authenticator, rewritten in Rust.");
                ui.label("Supports TOTP (RFC 6238), HOTP (RFC 4226), Steam Guard and mOTP.");
                ui.label("Backups are compatible with andOTP for Android.");
                ui.hyperlink("https://github.com/badsnalds/andOTP");
                ui.label(RichText::new("MIT licensed. © andOTP contributors, Jakob Nixdorf, Bruno Bierbaumer.").small().weak());
                if ui.button("Close").clicked() {
                    close = true;
                }
            }
        }
    });

    let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    if close
        || modal.should_close() && (escape || matches!(dialog, Dialog::Qr { .. } | Dialog::About | Dialog::Settings))
    {
        session.dialog = None;
    }
    if open_keychain_dialog {
        session.dialog = Some(Dialog::KeychainEnable { password: Zeroizing::default(), error: None });
    }
    if next_dialog.is_some() {
        session.dialog = next_dialog;
    }
    message
}

fn apply_theme(ctx: &egui::Context, theme: Theme) {
    ctx.set_theme(match theme {
        Theme::System => egui::ThemePreference::System,
        Theme::Light => egui::ThemePreference::Light,
        Theme::Dark => egui::ThemePreference::Dark,
    });
}

impl eframe::App for AndOtpApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Keep clear of the Android status and navigation bars.
        #[cfg(target_os = "android")]
        {
            let ppp = ui.ctx().pixels_per_point();
            let (top, bottom) = crate::android::insets_px();
            let margin = egui::Margin {
                top: (top / ppp).round().min(120.0) as i8,
                bottom: (bottom / ppp).round().min(120.0) as i8,
                ..Default::default()
            };
            egui::Frame::new().inner_margin(margin).show(ui, |ui| self.app_ui(ui));
        }
        #[cfg(not(target_os = "android"))]
        self.app_ui(ui);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, SETTINGS_KEY, &self.settings);
    }
}

impl AndOtpApp {
    fn app_ui(&mut self, ui: &mut egui::Ui) {
        let now = ui.input(|i| i.time);
        let active = ui.input(|i| !i.events.is_empty() || i.pointer.is_moving());
        if active || self.last_activity == 0.0 {
            self.last_activity = now;
        }
        if matches!(self.screen, Screen::Unlocked(_))
            && self.settings.auto_lock_minutes > 0
            && now - self.last_activity > f64::from(self.settings.auto_lock_minutes) * 60.0
        {
            self.lock();
            self.notify(ui.ctx(), "Locked after inactivity", false);
        }

        // Toast at the bottom.
        if let Some(toast) = self.toast.as_ref().filter(|t| t.until > now) {
            egui::Panel::bottom("toast").show(ui, |ui| {
                let color = if toast.error { ui.visuals().error_fg_color } else { ui.visuals().text_color() };
                ui.vertical_centered(|ui| ui.label(RichText::new(&toast.text).color(color)));
            });
        }

        match self.screen {
            Screen::Locked(_) => self.lock_ui(ui),
            Screen::Unlocked(_) => self.main_ui(ui),
        }

        // Tokens change every second; also re-check toasts and auto-lock.
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
    }
}

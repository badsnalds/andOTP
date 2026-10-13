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
#[cfg(camera)]
use crate::scanner;
use crate::style;
use crate::theme;
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

#[cfg(target_os = "android")]
mod keychain {
    //! On Android the "keychain" is fingerprint unlock: the password is kept encrypted with a
    //! Keystore key that only a strong biometric check releases (see `MainActivity`).
    use std::path::Path;
    use std::time::{Duration, Instant};

    use zeroize::Zeroizing;

    use crate::android;

    /// Shows the fingerprint prompt and waits for it (call from a worker thread).
    pub fn load(_: &Path) -> Result<Option<Zeroizing<String>>, String> {
        if !android::biometric_enabled() {
            return Ok(None);
        }
        android::biometric_unlock()?;
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(300) {
            match android::take_biometric_result() {
                Some(Ok(password)) => return Ok(Some(password)),
                Some(Err(Some(err))) => return Err(err),
                Some(Err(None)) => return Err(CANCELLED.into()),
                None => std::thread::sleep(Duration::from_millis(50)),
            }
        }
        Err("Fingerprint unlock timed out".into())
    }

    /// Starts storing the password; the fingerprint prompt reports the outcome as a message.
    pub fn store(_: &Path, password: &str) -> Result<(), String> {
        if !android::biometric_available() {
            return Err("Fingerprint unlock needs Android 11 or later and an enrolled fingerprint".into());
        }
        android::biometric_enroll(password)
    }

    pub fn delete(_: &Path) -> Result<(), String> {
        android::biometric_disable();
        Ok(())
    }

    /// Returned by [`load`] when the user chose to type the password instead.
    pub const CANCELLED: &str = "Fingerprint unlock cancelled";
}

/// Android stand-ins for desktop APIs, so the shared code compiles unchanged. The UI hides the
/// corresponding features (see [`DESKTOP`]); file access goes through the system picker instead.
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
    /// The password field was focused once; afterwards the user decides (on Android, tapping
    /// elsewhere closes the keyboard and must not bring it back).
    autofocused: bool,
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
        autofocused: bool,
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
    #[cfg(camera)]
    Scan(scanner::Scanner),
    Settings,
    /// Passkeys stored by the Android passkey provider; `confirm` is the one about to be deleted.
    #[cfg(target_os = "android")]
    Passkeys {
        list: Vec<crate::android::Passkey>,
        confirm: Option<usize>,
    },
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
    /// Whether the window had the focus last frame (Android keyboard handling).
    #[cfg(target_os = "android")]
    window_focused: bool,
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
        crate::theme::apply(&cc.egui_ctx);
        apply_theme(&cc.egui_ctx, settings.theme);
        cc.egui_ctx.all_styles_mut(|style| {
            // Text is read and tapped, not selected: selectable labels would swallow card taps.
            style.interaction.selectable_labels = false;
            // Comfortable touch targets on phones.
            if !DESKTOP {
                style.spacing.interact_size.y = 40.0;
                style.spacing.button_padding = egui::vec2(12.0, 8.0);
                style.spacing.item_spacing = egui::vec2(10.0, 8.0);
                style.spacing.menu_margin = egui::Margin::same(10);
            }
        });
        Self {
            settings,
            vault_override: None,
            screen: Screen::Locked(Lock::default()),
            toast: None,
            last_activity: 0.0,
            keychain_tried: false,
            inbox: Arc::default(),
            renderer: Renderer::load(),
            #[cfg(target_os = "android")]
            window_focused: false,
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
                    // Choosing "Use password" in the fingerprint prompt is not an error.
                    #[cfg(target_os = "android")]
                    let err = Some(err).filter(|e| e != keychain::CANCELLED);
                    #[cfg(not(target_os = "android"))]
                    let err = Some(err);
                    lock.error = err;
                    lock.pending = None;
                }
                Err(_) => ui.ctx().request_repaint_after(std::time::Duration::from_millis(50)),
            }
        }

        // Unlock from the system keychain once at start-up (not after an auto-lock, which would
        // defeat its purpose).
        let use_keychain = exists && remembered_unlock(&self.settings);
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
                ui.label(crate::theme::semibold("andOTP", 28.0));
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
                        .margin(egui::Margin::symmetric(12, 10))
                        .desired_width(width),
                );
                // Focus the field once. Requesting focus restarts the input method, so doing it
                // every frame makes the Android keyboard blink.
                if !busy && !lock.autofocused && can_autofocus(ui) {
                    pw.request_focus();
                    lock.autofocused = true;
                }
                let mut submit = pw.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if !exists {
                    let confirm = ui.add_enabled(
                        !busy,
                        egui::TextEdit::singleline(&mut *lock.confirm)
                            .password(true)
                            .hint_text("Repeat password")
                            .margin(egui::Margin::symmetric(12, 10))
                            .desired_width(width),
                    );
                    submit = confirm.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                }
                ui.add_space(8.0);
                if busy {
                    ui.spinner();
                } else {
                    let label = if exists { "Unlock" } else { "Create vault" };
                    submit |= ui.add_sized([width, 40.0], style::primary(ui, label)).clicked();
                }
                let keychain_label =
                    if DESKTOP { "🔑 Unlock with system keychain" } else { "👆 Unlock with fingerprint" };
                if use_keychain && !busy && ui.add_sized([width, 36.0], style::secondary(keychain_label)).clicked() {
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
                if DESKTOP {
                    ui.label(RichText::new(path.display().to_string()).small().weak());
                }
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
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                egui::containers::menu::MenuButton::from_button(style::icon_button("☰")).ui(ui, |ui| {
                    if ui.button("➕ Add account…").clicked() {
                        session.dialog = Some(Dialog::Entry(EntryForm::default()));
                    }
                    #[cfg(camera)]
                    if ui.button("📷 Scan QR code with camera…").clicked() {
                        session.dialog = Some(Dialog::Scan(scanner::Scanner::start(ui.ctx())));
                    }
                    if ui.button("🖼 Scan QR code from image…").clicked() {
                        messages.extend(start_image_scan(session));
                    }
                    #[cfg(target_os = "android")]
                    if ui.button("📥 Import backup…").clicked() {
                        match crate::android::open_document("*/*") {
                            Ok(()) => session.pending_open = Some(OpenPurpose::ImportBackup),
                            Err(err) => messages.push((err, true)),
                        }
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
                    if ui.button("📤 Export backup…").clicked() {
                        session.dialog = Some(Dialog::Export {
                            format: BackupFormat::Encrypted,
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
                let spacing = ui.spacing().item_spacing.x;
                ui.add(
                    style::text_field(&mut session.query)
                        .hint_text("🔍 Search")
                        .desired_width(ui.available_width() - 2.0 * (36.0 + spacing)),
                );
                if ui.add(style::icon_button("🔒")).on_hover_text("Lock").clicked() {
                    lock_requested = true;
                }
                let add = style::primary(ui, "➕").min_size(egui::vec2(36.0, 36.0));
                if ui.add(add).on_hover_text("Add account").clicked() {
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
                // Keep the cards clear of the scroll bar.
                ui.set_max_width(ui.available_width() - 12.0);
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

/// Whether the vault password is remembered: in the system keychain on desktop, for fingerprint
/// unlock on Android.
fn remembered_unlock(settings: &Settings) -> bool {
    #[cfg(target_os = "android")]
    {
        let _ = settings;
        crate::android::biometric_enabled()
    }
    #[cfg(not(target_os = "android"))]
    settings.keychain
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

/// Lets the user pick an image and adds the account in its QR code (asynchronously on Android).
fn start_image_scan(session: &mut Session) -> Option<(String, bool)> {
    #[cfg(target_os = "android")]
    {
        match crate::android::open_document("image/*") {
            Ok(()) => session.pending_open = Some(OpenPurpose::ScanImage),
            Err(err) => return Some((err, true)),
        }
        None
    }
    #[cfg(not(target_os = "android"))]
    {
        let path =
            rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp", "bmp"]).pick_file()?;
        match widgets::scan_qr(&path).and_then(|uri| Entry::from_uri(&uri).map_err(|e| e.to_string())) {
            Ok(entry) => {
                session.dialog = Some(Dialog::Entry(EntryForm::from_entry(None, &entry)));
                None
            }
            Err(err) => Some((err, true)),
        }
    }
}

/// Imports a backup file: directly if it is plain text, otherwise after asking for its password.
fn open_backup(session: &mut Session, settings: &Settings, data: Vec<u8>) -> Option<(String, bool)> {
    match backup::import(&data, None) {
        Ok((entries, _)) => Some(import_entries(&mut session.vault, settings, entries)),
        Err(_) => {
            session.dialog =
                Some(Dialog::ImportPassword { data, password: Zeroizing::default(), error: None, autofocused: false });
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

/// One account: a two-line card that fits phone screens. Tapping the card copies the token (or
/// reveals it first in "hide tokens" mode); the menu holds the other actions.
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
    let needs_pin = entry.otp_type == OtpType::Motp && token.remaining.is_none();
    let (title, subtitle) = if entry.issuer.is_empty() {
        (entry.label.as_str(), "")
    } else {
        (entry.issuer.as_str(), entry.label.as_str())
    };

    // The card itself senses clicks; widgets inside it (menu, PIN field, ⟳) still get theirs.
    let card = ui.scope_builder(egui::UiBuilder::new().id_salt(("card", idx)).sense(egui::Sense::click()), |ui| {
        let hovered = ui.response().hovered();
        let visuals = ui.visuals();
        // Mantine `Card withBorder`.
        let fill = if hovered { visuals.faint_bg_color } else { visuals.window_fill };
        egui::Frame::new()
            .fill(fill)
            .stroke(visuals.widgets.noninteractive.bg_stroke)
            .corner_radius(crate::theme::RADIUS_MD)
            .inner_margin(egui::Margin::symmetric(14, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    // Entries created before thumbnails existed (or by other apps) say "Default":
                    // still try to recognise the issuer.
                    let thumbnail = match entry.thumbnail.as_str() {
                        andotp_core::thumbnail::DEFAULT => andotp_core::thumbnail::for_issuer(&entry.issuer),
                        name => name,
                    };
                    widgets::service_icon(ui, thumbnail, title, 44.0);
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        // Line 1: name (cut short with … when too long) and the actions menu.
                        ui.horizontal(|ui| {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                entry_menu(ui, idx, manual, count, actions);
                                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                    ui.add(egui::Label::new(crate::theme::semibold(title, 15.0)).truncate());
                                });
                            });
                        });
                        if !subtitle.is_empty() {
                            ui.add(egui::Label::new(RichText::new(subtitle).weak()).truncate());
                        }
                        if !entry.tags.is_empty() {
                            let tags = entry.tags.iter().map(|t| format!("🏷 {t}")).collect::<Vec<_>>().join("  ");
                            ui.add(egui::Label::new(RichText::new(tags).small().weak()).truncate());
                        }
                        ui.add_space(4.0);

                        // Line 2: the token, with its timer (or the HOTP "next" button) on the right.
                        ui.horizontal(|ui| {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if let Some(remaining) = token.remaining {
                                    widgets::countdown(ui, token.progress(), remaining, token.is_expiring(), 30.0)
                                        .on_hover_text(format!("Changes in {remaining} s"));
                                } else if entry.otp_type == OtpType::Hotp
                                    && ui
                                        .button(RichText::new("⟳").size(18.0))
                                        .on_hover_text(format!("Next code (counter {})", entry.counter))
                                        .clicked()
                                {
                                    actions.push(Action::NextCounter(idx));
                                }
                                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                    if needs_pin {
                                        ui.add(
                                            egui::TextEdit::singleline(&mut **pin)
                                                .password(true)
                                                .hint_text("Enter PIN")
                                                .desired_width(120.0),
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
                                        ui.visuals().hyperlink_color
                                    };
                                    ui.label(RichText::new(shown).monospace().size(30.0).color(color));
                                    if revealed
                                        && settings.show_previous
                                        && let Some(prev) = &token.previous
                                    {
                                        ui.label(
                                            RichText::new(otp::group_token(prev, settings.group_size))
                                                .monospace()
                                                .weak(),
                                        )
                                        .on_hover_text("Previous token");
                                    }
                                });
                            });
                        });
                    });
                });
            });
    });

    let response = card.response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let response = if needs_pin {
        response
    } else {
        response.on_hover_text(if revealed { "Click to copy" } else { "Click to reveal" })
    };
    if response.clicked() && !needs_pin {
        actions.push(if revealed { Action::Copy(idx) } else { Action::Reveal(idx) });
    }
}

fn entry_menu(ui: &mut egui::Ui, idx: usize, manual: bool, count: usize, actions: &mut Vec<Action>) {
    ui.menu_button(RichText::new("⏷").size(16.0), |ui| {
        if ui.button("🗐 Copy").clicked() {
            actions.push(Action::Copy(idx));
        }
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
    #[cfg_attr(not(camera), allow(unused_mut))]
    let mut next_dialog = None;

    let mut scan_image = false;
    let width = style::dialog_width(ctx);
    let max_height = ctx.content_rect().height() * 0.85;
    let modal = egui::Modal::new(egui::Id::new("dialog")).frame(style::modal_frame(ctx)).show(ctx, |ui| {
        ui.set_width(width);
        // The modal area starts out smaller than the screen; let long dialogs use most of it.
        ui.set_max_height(max_height);
        // A solid scroll bar: the default floating one is drawn over the inputs' right edge.
        ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
        egui::ScrollArea::vertical().max_height(max_height).show(ui, |ui| match dialog {
            Dialog::Entry(form) => {
                let original = form.index.map(|i| &session.vault.entries[i]);
                match form.ui(ui, original) {
                    FormResult::None => {}
                    FormResult::Cancel => close = true,
                    #[cfg(camera)]
                    FormResult::ScanCamera => next_dialog = Some(Dialog::Scan(scanner::Scanner::start(ctx))),
                    FormResult::ScanImage => scan_image = true,
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
                style::header(
                    ui,
                    "Delete account?",
                    &format!(
                        "“{}” will be removed. Make sure you no longer need it or have another way to sign in.",
                        session.vault.entries[i].display_name()
                    ),
                );
                style::button_row(ui, |ui| {
                    if style::danger_button(ui, "Delete", true).clicked() {
                        session.vault.entries.remove(i);
                        session.pins.clear();
                        session.revealed.clear();
                        message = Some(match persist(&session.vault, settings) {
                            Ok(()) => ("Account deleted".into(), false),
                            Err(err) => (err, true),
                        });
                        close = true;
                    }
                    if style::secondary_button(ui, "Cancel").clicked() {
                        close = true;
                    }
                });
            }
            Dialog::Qr { title, texture } => {
                style::header(ui, title, "Scan with andOTP or another authenticator app.");
                ui.vertical_centered(|ui| {
                    let side = ui.available_width().min(260.0);
                    ui.add(
                        egui::Image::new(&*texture)
                            .corner_radius(theme::RADIUS_MD)
                            .fit_to_exact_size(egui::vec2(side, side)),
                    );
                });
                ui.add_space(8.0);
                style::alert(ui, ui.visuals().warn_fg_color, "Anyone who sees this code can generate your tokens.");
                style::button_row(ui, |ui| {
                    if style::secondary_button(ui, "Close").clicked() {
                        close = true;
                    }
                });
            }
            Dialog::ImportPassword { data, password, error, autofocused } => {
                style::header(ui, "Encrypted backup", "Enter the password this backup was protected with.");
                style::label(ui, "Backup password");
                let edit = ui.add(style::text_field(&mut **password).password(true));
                if !*autofocused && can_autofocus(ui) {
                    edit.request_focus();
                    *autofocused = true;
                }
                if let Some(err) = error {
                    style::error(ui, err);
                }
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                style::button_row(ui, |ui| {
                    if style::primary_button(ui, "Import").clicked() || enter {
                        match backup::import(data, Some(password)) {
                            Ok((entries, _)) => {
                                message = Some(import_entries(&mut session.vault, settings, entries));
                                close = true;
                            }
                            Err(err) => *error = Some(format!("Cannot decrypt: {err}")),
                        }
                    }
                    if style::secondary_button(ui, "Cancel").clicked() {
                        close = true;
                    }
                });
            }
            Dialog::Export { format, password, confirm, error } => {
                let count = session.vault.entries.len();
                style::header(
                    ui,
                    "Export backup",
                    &if count == 1 {
                        "Save your account to a file.".to_owned()
                    } else {
                        format!("Save all {count} accounts to a file.")
                    },
                );
                if style::option_card(
                    ui,
                    *format != BackupFormat::Plain,
                    "🔒",
                    "Encrypted (recommended)",
                    "Protected by a password. Restores in andOTP for Android and desktop.",
                )
                .clicked()
                {
                    *format = BackupFormat::Encrypted;
                    *error = None;
                }
                if style::option_card(
                    ui,
                    *format == BackupFormat::Plain,
                    "📄",
                    "Plain text",
                    "Readable JSON for moving to another app. Anyone with the file can use your accounts.",
                )
                .clicked()
                {
                    *format = BackupFormat::Plain;
                    *error = None;
                }
                let encrypted = *format != BackupFormat::Plain;
                if encrypted {
                    style::label(ui, "Backup password");
                    ui.add(
                        style::text_field(&mut **password)
                            .password(true)
                            .hint_text(format!("At least {MIN_PASSWORD_LENGTH} characters")),
                    );
                    style::label(ui, "Repeat password");
                    ui.add(style::text_field(&mut **confirm).password(true));
                } else {
                    ui.add_space(4.0);
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        "⚠ Every secret is stored unencrypted. Keep the file somewhere safe and delete it when done.",
                    );
                }
                if let Some(err) = error {
                    style::error(ui, err);
                }
                style::button_row(ui, |ui| {
                    if style::primary_button(ui, "Save backup…").clicked() {
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
                    if style::secondary_button(ui, "Cancel").clicked() {
                        close = true;
                    }
                });
            }
            Dialog::ChangePassword { password, confirm, error } => {
                style::header(ui, "Change password", "Your accounts are encrypted again with the new password.");
                style::label(ui, "New password");
                ui.add(
                    style::text_field(&mut **password)
                        .password(true)
                        .hint_text(format!("At least {MIN_PASSWORD_LENGTH} characters")),
                );
                style::label(ui, "Repeat new password");
                ui.add(style::text_field(&mut **confirm).password(true));
                if let Some(err) = error {
                    style::error(ui, err);
                }
                style::button_row(ui, |ui| {
                    if style::primary_button(ui, "Change").clicked() {
                        if password.chars().count() < MIN_PASSWORD_LENGTH {
                            *error = Some(format!("Use at least {MIN_PASSWORD_LENGTH} characters"));
                        } else if **password != **confirm {
                            *error = Some("The passwords do not match".into());
                        } else {
                            match session.vault.change_password(password) {
                                Ok(()) => {
                                    let mut text = "Password changed".to_owned();
                                    if remembered_unlock(settings)
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
                    if style::secondary_button(ui, "Cancel").clicked() {
                        close = true;
                    }
                });
            }
            Dialog::Settings => {
                style::header(ui, "Settings", "");
                let width = ui.available_width();

                style::section(ui, "Appearance");
                style::label(ui, "Theme");
                let themes = [(Theme::System, "System"), (Theme::Light, "Light"), (Theme::Dark, "Dark")];
                if style::segmented(ui, &mut settings.theme, &themes, width) {
                    apply_theme(ctx, settings.theme);
                }
                style::label(ui, "Group digits");
                let groups = [(0, "Off"), (2, "2"), (3, "3"), (4, "4")];
                style::segmented(ui, &mut settings.group_size, &groups, width);

                style::section(ui, "Security");
                style::label(ui, "Lock after");
                let locks = [(0, "Never"), (1, "1 min"), (5, "5 min"), (15, "15 min"), (60, "1 h")];
                style::segmented(ui, &mut settings.auto_lock_minutes, &locks, width);
                ui.add_space(4.0);
                style::setting_row(ui, "Hide tokens", "Show a token only after tapping it", 40.0, |ui| {
                    style::switch(ui, &mut settings.tap_to_reveal, "Hide tokens")
                });
                style::setting_row(ui, "Previous token", "Also show the token of the last period", 40.0, |ui| {
                    style::switch(ui, &mut settings.show_previous, "Previous token")
                });
                // Android: fingerprint unlock, the counterpart of the desktop keychain.
                #[cfg(target_os = "android")]
                if crate::android::biometric_available() || crate::android::biometric_enabled() {
                    let mut on = crate::android::biometric_enabled();
                    let toggled = style::setting_row(
                        ui,
                        "Fingerprint unlock",
                        "Unlock with your fingerprint instead of typing the password",
                        40.0,
                        |ui| style::switch(ui, &mut on, "Fingerprint unlock").changed(),
                    );
                    if toggled {
                        if on {
                            open_keychain_dialog = true;
                        } else {
                            let _ = keychain::delete(session.vault.path());
                            message = Some(("Fingerprint unlock is off".into(), false));
                        }
                    }
                }
                // Android 14+: andOTP as a passkey provider for websites and apps.
                #[cfg(target_os = "android")]
                if crate::android::passkeys_supported() {
                    style::section(ui, "Passkeys");
                    let count = crate::android::passkey_list().len();
                    let description = match count {
                        0 => "None yet. Stored on this phone only".to_owned(),
                        1 => "1 passkey on this phone".to_owned(),
                        n => format!("{n} passkeys on this phone"),
                    };
                    let manage = style::setting_row(ui, "Saved passkeys", &description, 90.0, |ui| {
                        ui.add(style::secondary("Manage").min_size(egui::vec2(80.0, 32.0))).clicked()
                    });
                    if manage {
                        next_dialog = Some(Dialog::Passkeys { list: crate::android::passkey_list(), confirm: None });
                    }
                    let open = style::setting_row(
                        ui,
                        "Use andOTP for passkeys",
                        "Choose andOTP in Android's passkey settings",
                        90.0,
                        |ui| ui.add(style::secondary("Open").min_size(egui::vec2(80.0, 32.0))).clicked(),
                    );
                    if open {
                        crate::android::open_passkey_settings();
                    }
                }
                // Desktop only: on Android these need platform services we do not use.
                if DESKTOP {
                    let mut keychain_on = settings.keychain;
                    let toggled = style::setting_row(
                        ui,
                        "System keychain",
                        "Unlock with the password stored in the OS credential store",
                        40.0,
                        |ui| style::switch(ui, &mut keychain_on, "System keychain").changed(),
                    );
                    if toggled {
                        if keychain_on {
                            open_keychain_dialog = true;
                        } else {
                            settings.keychain = false;
                            if let Err(err) = keychain::delete(session.vault.path()) {
                                message = Some((err.to_string(), true));
                            }
                        }
                    }

                    style::section(ui, "Backups");
                    style::setting_row(
                        ui,
                        "Automatic backups",
                        "Copy the encrypted vault after every change",
                        40.0,
                        |ui| style::switch(ui, &mut settings.auto_backup, "Automatic backups"),
                    );
                    if settings.auto_backup {
                        if settings.backup_dir.is_none() {
                            style::alert(ui, ui.visuals().warn_fg_color, "Choose a folder for automatic backups.");
                        }
                        style::label(ui, "Folder");
                        let label = settings
                            .backup_dir
                            .as_ref()
                            .map_or_else(|| "Choose folder…".to_owned(), |d| d.display().to_string());
                        let button = egui::Button::new(RichText::new(format!("📁 {label}")).size(13.0))
                            .truncate()
                            .corner_radius(theme::RADIUS_SM)
                            .min_size(egui::vec2(width, 34.0));
                        if ui.add(button).clicked()
                            && let Some(dir) = rfd::FileDialog::new().set_title("Backup folder").pick_folder()
                        {
                            settings.backup_dir = Some(dir);
                            message = Some(match auto_backup(&session.vault, settings) {
                                Ok(()) => ("Backup written".into(), false),
                                Err(err) => (err, true),
                            });
                        }
                        ui.add_space(4.0);
                        style::setting_row(ui, "Backups to keep", "0 keeps all of them", 90.0, |ui| {
                            ui.add(egui::DragValue::new(&mut settings.backup_keep).range(0..=100).speed(0.2))
                        });
                    }

                    style::section(ui, "Advanced");
                    style::label(ui, "Renderer");
                    let choices = [(Renderer::Glow, "OpenGL"), (Renderer::Wgpu, "wgpu")];
                    if style::segmented(ui, renderer, &choices, width) {
                        message = Some(match renderer.save() {
                            Ok(()) => ("Takes effect after a restart".into(), false),
                            Err(err) => (format!("Cannot save the renderer choice: {err}"), true),
                        });
                    }
                }
                ui.add_space(8.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("Vault: {}", session.vault.path().display())).size(11.0).weak(),
                    )
                    .truncate(),
                );
                style::button_row(ui, |ui| {
                    if style::secondary_button(ui, "Close").clicked() {
                        close = true;
                    }
                });
            }
            Dialog::KeychainEnable { password, error } => {
                if DESKTOP {
                    style::header(
                        ui,
                        "Unlock with the system keychain",
                        "Your vault password is stored in the operating system's credential store, so andOTP \
                         can unlock without asking while you are logged in.",
                    );
                } else {
                    style::header(
                        ui,
                        "Fingerprint unlock",
                        "Your vault password is stored encrypted on this phone and released only after a \
                         fingerprint check. The password keeps working too.",
                    );
                }
                style::label(ui, "Vault password");
                let edit = ui.add(style::text_field(&mut **password).password(true));
                if let Some(err) = error {
                    style::error(ui, err);
                }
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                style::button_row(ui, |ui| {
                    if style::primary_button(ui, "Enable").clicked() || enter {
                        let path = session.vault.path().to_path_buf();
                        match Vault::open(&path, password) {
                            Err(andotp_core::Error::Decryption) => *error = Some("Wrong password".into()),
                            Err(err) => *error = Some(err.to_string()),
                            Ok(_) => match keychain::store(&path, password) {
                                Ok(()) => {
                                    settings.keychain = true;
                                    // Android: the fingerprint prompt opens now and reports the result.
                                    if DESKTOP {
                                        message = Some(("Password stored in the system keychain".into(), false));
                                    }
                                    close = true;
                                }
                                Err(err) => *error = Some(err.to_string()),
                            },
                        }
                    }
                    if style::secondary_button(ui, "Cancel").clicked() {
                        close = true;
                    }
                });
            }
            Dialog::Wipe { confirm, error } => {
                ui.label(theme::semibold("☢ Wipe all data", 18.0).color(ui.visuals().error_fg_color));
                ui.add_space(6.0);
                style::alert(
                    ui,
                    ui.visuals().error_fg_color,
                    &format!(
                        "This destroys the vault with all {} accounts for good and removes the password from \
                         the system keychain. Backups are not touched. Make sure you can still sign in to your \
                         accounts.",
                        session.vault.entries.len()
                    ),
                );
                style::label(ui, &format!("Type {WIPE_CONFIRMATION} to confirm"));
                ui.add(style::text_field(confirm));
                if let Some(err) = error {
                    style::error(ui, err);
                }
                style::button_row(ui, |ui| {
                    let armed = confirm.trim() == WIPE_CONFIRMATION;
                    if style::danger_button(ui, "Wipe", armed).clicked() {
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
                    if style::secondary_button(ui, "Cancel").clicked() {
                        close = true;
                    }
                });
            }
            #[cfg(camera)]
            Dialog::Scan(scanner) => {
                style::header(ui, "Scan QR code", "Point the camera at the code shown by the service.");
                let status = scanner.poll(ctx);
                let width = ui.available_width();
                let (rect, _) = ui.allocate_exact_size(egui::vec2(width, width.min(320.0)), egui::Sense::hover());
                ui.painter().rect_filled(rect, 12.0, egui::Color32::from_gray(20));
                if let Some(texture) = scanner.texture() {
                    let size = texture.size_vec2();
                    let scale = (rect.width() / size.x).min(rect.height() / size.y);
                    let image_rect = egui::Rect::from_center_size(rect.center(), size * scale);
                    egui::Image::new(texture).corner_radius(12.0).paint_at(ui, image_rect);
                    // A viewfinder square to aim with.
                    let side = rect.height().min(rect.width()) * 0.7;
                    let finder = egui::Rect::from_center_size(rect.center(), egui::vec2(side, side));
                    ui.painter().rect_stroke(
                        finder,
                        10.0,
                        egui::Stroke::new(3.0, egui::Color32::from_white_alpha(200)),
                        egui::StrokeKind::Middle,
                    );
                }
                ui.add_space(8.0);
                match status {
                    scanner::Status::Starting => {
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "Starting camera…",
                            egui::FontId::proportional(15.0),
                            egui::Color32::from_gray(200),
                        );
                    }
                    scanner::Status::Scanning => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Looking for a QR code…");
                        });
                    }
                    scanner::Status::Found(uri) => match Entry::from_uri(&uri) {
                        Ok(entry) => next_dialog = Some(Dialog::Entry(EntryForm::from_entry(None, &entry))),
                        Err(err) => message = Some((err.to_string(), true)),
                    },
                    scanner::Status::Failed(err) => style::error(ui, &err),
                }
                style::button_row(ui, |ui| {
                    if style::secondary_button(ui, "Cancel").clicked() {
                        close = true;
                    }
                });
            }
            #[cfg(target_os = "android")]
            Dialog::Passkeys { list, confirm } => {
                style::header(
                    ui,
                    "Passkeys",
                    "Sign-in keys for websites and apps, kept on this phone only and unlocked with your \
                     fingerprint or screen lock. They are not part of andOTP backups.",
                );
                if list.is_empty() {
                    style::alert(
                        ui,
                        theme::primary_text(ui),
                        "No passkeys yet. Turn on andOTP in Android's passkey settings, then choose \
                         \"Create a passkey\" on a website or in an app.",
                    );
                }
                let mut delete = None;
                for (i, passkey) in list.iter().enumerate() {
                    let card = egui::Frame::new()
                        .fill(ui.visuals().window_fill)
                        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                        .corner_radius(theme::RADIUS_MD)
                        .inner_margin(egui::Margin::symmetric(12, 10));
                    card.show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            let title = if passkey.rp_name.is_empty() { &passkey.rp_id } else { &passkey.rp_name };
                            widgets::service_icon(ui, andotp_core::thumbnail::for_issuer(title), title, 36.0);
                            ui.vertical(|ui| {
                                ui.set_max_width(ui.available_width() - 44.0);
                                ui.add(egui::Label::new(theme::semibold(title.as_str(), 14.0)).truncate());
                                let user = if passkey.user_name.is_empty() {
                                    &passkey.display_name
                                } else {
                                    &passkey.user_name
                                };
                                ui.add(egui::Label::new(RichText::new(user.as_str()).size(12.0).weak()).truncate());
                                ui.add(
                                    egui::Label::new(RichText::new(passkey.rp_id.as_str()).size(11.0).weak())
                                        .truncate(),
                                );
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.add(style::icon_button("🗑")).on_hover_text("Delete").clicked() {
                                    *confirm = Some(i);
                                }
                            });
                        });
                        if *confirm == Some(i) {
                            ui.add_space(6.0);
                            style::alert(
                                ui,
                                ui.visuals().error_fg_color,
                                "Delete this passkey? You can no longer sign in with it. Remove it from the \
                                 site's account settings too.",
                            );
                            style::button_row(ui, |ui| {
                                if style::danger_button(ui, "Delete", true).clicked() {
                                    delete = Some(i);
                                }
                                if style::secondary_button(ui, "Keep").clicked() {
                                    *confirm = None;
                                }
                            });
                        }
                    });
                    ui.add_space(6.0);
                }
                if let Some(i) = delete {
                    crate::android::passkey_delete(&list[i].credential_id);
                    *list = crate::android::passkey_list();
                    *confirm = None;
                    message = Some(("Passkey deleted".into(), false));
                }
                style::button_row(ui, |ui| {
                    if style::secondary_button(ui, "Close").clicked() {
                        close = true;
                    }
                    if ui.add(style::secondary("Android settings…").min_size(egui::vec2(80.0, 36.0))).clicked() {
                        crate::android::open_passkey_settings();
                    }
                });
            }
            Dialog::About => {
                style::header(
                    ui,
                    &format!("andOTP {}", env!("CARGO_PKG_VERSION")),
                    "Open-source two-factor authenticator, rewritten in Rust.",
                );
                ui.label(RichText::new("Supports TOTP (RFC 6238), HOTP (RFC 4226), Steam Guard and mOTP.").size(13.0));
                ui.label(RichText::new("Backups are compatible with andOTP for Android.").size(13.0));
                ui.add_space(4.0);
                ui.hyperlink("https://github.com/badsnalds/andOTP");
                ui.add_space(4.0);
                ui.label(
                    RichText::new("MIT licensed. © andOTP contributors, Jakob Nixdorf, Bruno Bierbaumer.")
                        .size(11.0)
                        .weak(),
                );
                style::button_row(ui, |ui| {
                    if style::secondary_button(ui, "Close").clicked() {
                        close = true;
                    }
                });
            }
        });
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
    if scan_image && let Some(err) = start_image_scan(session) {
        message = Some(err);
    }
    message
}

/// Whether a field may take the focus (and open the keyboard) on its own: nothing else has it,
/// and the window itself is focused. On Android a keyboard requested while the window is not
/// focused yet (e.g. right after the file picker closes) is hidden again by the system.
fn can_autofocus(ui: &egui::Ui) -> bool {
    ui.memory(|m| m.focused().is_none()) && ui.input(|i| i.focused)
}

/// Android hides the keyboard whenever the window regains focus (back from the file picker or
/// another app). egui only asks for it when a field gains focus, so ask again here.
#[cfg(target_os = "android")]
fn restore_keyboard(ctx: &egui::Context, was_focused: &mut bool) {
    let focused = ctx.input(|i| i.focused);
    if focused && !*was_focused && ctx.egui_wants_keyboard_input() {
        ctx.send_viewport_cmd(egui::ViewportCommand::IMEAllowed(true));
    }
    *was_focused = focused;
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
            restore_keyboard(ui.ctx(), &mut self.window_focused);
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

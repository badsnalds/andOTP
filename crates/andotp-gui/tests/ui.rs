//! End-to-end tests of the desktop app, driven headlessly through its accessibility tree.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use andotp_core::{Entry, OtpType, Vault};
use andotp_gui::AndOtpApp;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

const PASSWORD: &str = "hunter22";
/// RFC 4226 test key ("12345678901234567890").
const RFC_SECRET: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";

fn vault_with_entries(dir: &Path) -> PathBuf {
    let path = dir.join("vault.aes");
    let mut vault = Vault::create_with_iterations(&path, PASSWORD, 1000).unwrap();
    let mut hotp = Entry::new(OtpType::Hotp, RFC_SECRET, "", "counter").unwrap();
    hotp.counter = 0;
    vault.add(hotp);
    let mut totp = Entry::new(OtpType::Totp, "JBSWY3DPEHPK3PXP", "GitHub", "octocat").unwrap();
    totp.tags = vec!["work".into()];
    vault.add(totp);
    vault.save().unwrap();
    path
}

fn harness(path: &Path) -> Harness<'static, AndOtpApp> {
    let path = path.to_path_buf();
    Harness::builder().with_size([440.0, 680.0]).build_eframe(move |cc| {
        let mut app = AndOtpApp::new(cc);
        app.set_vault_path(path.clone());
        app
    })
}

/// Focuses the `nth` input with the given role and types `text` into it.
fn type_into(harness: &mut Harness<'_, AndOtpApp>, nth: usize, role: Role, text: &str) {
    harness.get_all_by_role(role).nth(nth).expect("no such input").focus();
    harness.run_steps(1);
    harness.get_all_by_role(role).nth(nth).unwrap().type_text(text);
    harness.run_steps(1);
}

/// Whether a label with exactly this text is shown.
fn shows(harness: &Harness<'_, AndOtpApp>, text: &str) -> bool {
    harness.query_by(|n| n.role() == Role::Label && n.value().as_deref() == Some(text)).is_some()
}

#[track_caller]
fn assert_shows(harness: &Harness<'_, AndOtpApp>, text: &str) {
    assert!(shows(harness, text), "`{text}` is not shown");
}

/// Steps the app (key derivation runs on a worker thread) until `done` holds.
fn wait_for(harness: &mut Harness<'_, AndOtpApp>, what: &str, done: impl Fn(&Harness<'_, AndOtpApp>) -> bool) {
    let start = Instant::now();
    while !done(harness) {
        assert!(start.elapsed() < Duration::from_secs(20), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
        harness.step();
    }
}

fn unlock(harness: &mut Harness<'_, AndOtpApp>, password: &str) {
    harness.run_steps(2);
    type_into(harness, 0, Role::PasswordInput, password);
    harness.get_by_label("Unlock").click();
    harness.run_steps(1);
}

#[test]
fn unlock_shows_tokens_and_advances_hotp() {
    let dir = tempfile::tempdir().unwrap();
    let path = vault_with_entries(dir.path());
    let mut harness = harness(&path);

    unlock(&mut harness, PASSWORD);
    wait_for(&mut harness, "the entry list", |h| shows(h, "octocat"));
    assert!(shows(&harness, "GitHub"));

    // RFC 4226: counter 0 → 755224, counter 1 → 287082.
    assert_shows(&harness, "755 224");
    harness.get_by_label("⟳").click();
    harness.run_steps(2);
    assert_shows(&harness, "287 082");
    assert_eq!(Vault::open(&path, PASSWORD).unwrap().entries[0].counter, 1);

    // Search filters the list.
    type_into(&mut harness, 0, Role::TextInput, "git");
    harness.run_steps(2);
    assert!(!shows(&harness, "counter"));
    assert!(shows(&harness, "octocat"));

    // Locking returns to the password prompt.
    harness.get_by_label("🔒").click();
    harness.run_steps(2);
    harness.get_by_role(Role::PasswordInput);
}

#[test]
fn wrong_password_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = vault_with_entries(dir.path());
    let mut harness = harness(&path);

    unlock(&mut harness, "not the password");
    wait_for(&mut harness, "the error message", |h| shows(h, "Wrong password"));
    assert!(!shows(&harness, "octocat"));
}

#[test]
fn create_vault_and_add_entry_from_uri() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("new.aes");
    let mut harness = harness(&path);
    harness.run_steps(2);
    assert_shows(&harness, "Create a password to protect your accounts");

    assert_eq!(harness.get_all_by_role(Role::PasswordInput).count(), 2);
    type_into(&mut harness, 0, Role::PasswordInput, "correct horse");
    type_into(&mut harness, 1, Role::PasswordInput, "correct horse");
    harness.get_by_label("Create vault").click();
    wait_for(&mut harness, "the empty vault", |h| shows(h, "No accounts yet"));
    assert!(path.exists());

    harness.get_by_label("➕").click();
    harness.run_steps(2);
    let uri = "otpauth://totp/GitLab:dev@example.com?secret=JBSWY3DPEHPK3PXP&issuer=GitLab&tags=ci";
    // The URI field is the first text input of the dialog (after the search field).
    type_into(&mut harness, 1, Role::TextInput, uri);
    harness.run_steps(1);
    harness.get_by_label("Apply").click();
    harness.run_steps(2);
    harness.get_by_label("Save").click();
    harness.run_steps(3);

    assert_shows(&harness, "dev@example.com");
    let vault = Vault::open(&path, "correct horse").unwrap();
    assert_eq!(vault.entries.len(), 1);
    assert_eq!(vault.entries[0].issuer, "GitLab");
    assert_eq!(vault.entries[0].tags, ["ci"]);
}

#[test]
fn wipe_requires_confirmation_and_destroys_vault() {
    let dir = tempfile::tempdir().unwrap();
    let path = vault_with_entries(dir.path());
    let mut harness = harness(&path);
    unlock(&mut harness, PASSWORD);
    wait_for(&mut harness, "the entry list", |h| shows(h, "octocat"));

    harness.get_by_label("☰").click();
    harness.run_steps(2);
    harness.get_by_label("☢ Wipe all data…").click();
    harness.run_steps(2);
    harness.get_by_label("Wipe").click();
    harness.run_steps(2);
    assert!(path.exists(), "wiping must need the confirmation word");

    // The confirmation field is the last text input (after the search field).
    let n = harness.get_all_by_role(Role::TextInput).count();
    type_into(&mut harness, n - 1, Role::TextInput, "DELETE");
    harness.run_steps(1);
    harness.get_by_label("Wipe").click();
    harness.run_steps(3);

    assert!(!path.exists());
    assert_shows(&harness, "Create a password to protect your accounts");
}

/// Re-requesting focus every frame restarts the input method, which makes the Android on-screen
/// keyboard hide and show again (blink) while the lock screen is open.
#[test]
fn lock_screen_does_not_restart_the_keyboard() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = harness(&dir.path().join("new.aes"));
    harness.run_steps(3);
    for _ in 0..5 {
        harness.step();
        let ime = harness.output().platform_output.ime.as_ref().expect("the password field should want the keyboard");
        assert!(!ime.should_interrupt_composition, "the keyboard was restarted");
    }
}

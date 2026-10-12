//! Runs the `andotp` binary end to end against a temporary vault.

use std::path::Path;
use std::process::{Command, Output};

const PASSWORD: &str = "hunter22";

fn andotp(vault: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_andotp"))
        .args(args)
        .env("ANDOTP_VAULT", vault)
        .env("ANDOTP_PASSWORD", PASSWORD)
        .env("NO_COLOR", "1")
        .output()
        .expect("failed to run andotp")
}

#[track_caller]
fn ok(vault: &Path, args: &[&str]) -> String {
    let out = andotp(vault, args);
    assert!(out.status.success(), "andotp {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn full_workflow() {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().join("vault.aes");

    ok(&vault, &["init"]);
    assert!(!andotp(&vault, &["init"]).status.success(), "init must not overwrite");

    ok(&vault, &["add", "otpauth://hotp/Corp:me?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=Corp&counter=0"]);
    ok(&vault, &["add", "--secret", "JBSWY3DPEHPK3PXP", "--issuer", "GitHub", "--label", "octocat", "-t", "work"]);
    assert!(
        !andotp(&vault, &["add", "--secret", "JBSWY3DPEHPK3PXP", "--issuer", "GitHub", "--label", "octocat"])
            .status
            .success()
    );

    // HOTP codes follow RFC 4226 and the counter advances after each use.
    assert_eq!(ok(&vault, &["code", "Corp"]).trim(), "755224");
    assert_eq!(ok(&vault, &["code", "Corp"]).trim(), "287082");

    let list = ok(&vault, &["list", "--tag", "work"]);
    assert!(list.contains("GitHub - octocat") && !list.contains("Corp"), "{list}");

    let uri = ok(&vault, &["uri", "octocat"]);
    assert_eq!(uri.trim(), "otpauth://totp/GitHub:octocat?secret=JBSWY3DPEHPK3PXP&issuer=GitHub&tags=work");

    // Export, wipe, re-import.
    let backup = dir.path().join("backup.json.aes");
    ok(&vault, &["export", backup.to_str().unwrap(), "--backup-password", "backup-pw"]);
    ok(&vault, &["rm", "octocat", "--yes"]);
    assert!(!ok(&vault, &["list"]).contains("octocat"));
    let out = ok(&vault, &["import", backup.to_str().unwrap(), "--backup-password", "backup-pw"]);
    assert!(out.contains("Imported 1 entries (1 duplicates skipped)"), "{out}");

    // A wrong vault password fails cleanly.
    let out = Command::new(env!("CARGO_BIN_EXE_andotp"))
        .args(["list"])
        .env("ANDOTP_VAULT", &vault)
        .env("ANDOTP_PASSWORD", "wrong")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot unlock vault"));
}

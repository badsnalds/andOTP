# andOTP – open-source two-factor authenticator, rewritten in Rust

![andOTP](./assets/logo.png)

andOTP generates the one-time codes used for two-factor authentication (2FA). This is
a rewrite of the [andOTP Android app](legacy/android/), which is no longer maintained,
as a modern Rust workspace. It includes a desktop app, an Android app and a command-line client.

Backups work in both directions with the Android app. You can export from andOTP on your
phone and import here, or the other way around.

## Features

* **Algorithms**
  * TOTP ([RFC 6238](https://tools.ietf.org/html/rfc6238)) with SHA-1, SHA-256 and SHA-512, and any period or digit count
  * HOTP ([RFC 4226](https://tools.ietf.org/html/rfc4226))
  * Steam Guard
  * mOTP (the PIN is kept only in memory)
* **Encrypted storage**: the vault uses AES-256-GCM, with the key derived from your password by PBKDF2.
  The vault file is a regular andOTP password-protected backup, so andOTP for Android can import it directly.
* **Backups**: imports plain-text, password-protected and *old* password-protected andOTP
  backups, and exports plain-text or password-protected ones.
* **`otpauth://` URIs**: add accounts from links, QR-code images or the webcam, and show any account
  as a QR code to move it to a phone.
* **Organisation**: tags (match any or all), search, sorting (manual, issuer, label, last used, most used)
  and logos for about 350 known services. These are andOTP's original thumbnails, bundled with the app;
  nothing is downloaded, so no server learns which accounts you have.
* **Desktop app**: light/dark/system theme, a countdown ring, click to copy, optional hide-until-clicked,
  digit grouping and auto-lock after inactivity.
* **Desktop versions of the Android-only features**:

  | Android | Desktop |
  | --- | --- |
  | KeyStore | Optionally remember the vault password in the system keychain (macOS Keychain, Windows Credential Manager, Secret Service on Linux) |
  | Auto-fill | Type the current code into the window you were last using (*⌨ Type into last window*, `andotp type`) |
  | Tasker broadcasts | Automatic encrypted backup to a folder after every change, with old backups pruned; `andotp backup` for cron/Task Scheduler (needs no password) |
  | Panic trigger | *☢ Wipe all data* / `andotp panic` overwrite and delete the vault and remove the keychain entry |
  | Camera scanner | Scan setup QR codes with the webcam |
* **CLI**: for scripts and terminals, including a live `watch` view and QR codes drawn in the terminal.

## Workspace layout

| Crate | Description |
| --- | --- |
| [`crates/andotp-core`](crates/andotp-core) | Library: OTP algorithms, the account model, `otpauth://` URIs, crypto, backups and the vault. No UI code. |
| [`crates/andotp-cli`](crates/andotp-cli) | `andotp`, the command-line client |
| [`crates/andotp-gui`](crates/andotp-gui) | `andotp-gui`, the app ([egui](https://github.com/emilk/egui)) for Linux, macOS, Windows and Android |
| [`crates/andotp-android`](crates/andotp-android) + [`android`](android) | The Android packaging: native library and a small `NativeActivity` wrapper |
| [`legacy/android`](legacy/android) | The original Java Android app (unmaintained, kept for reference) |

## Building

The core library and the CLI need Rust 1.85 or newer. The desktop app needs Rust 1.95 or newer.

```sh
cargo build --release                   # core + CLI
cargo build --release -p andotp-gui     # desktop app
cargo test --workspace                  # all tests, including headless GUI tests
```

The binaries end up in `target/release/`. On Linux, the desktop app needs OpenGL and the usual
X11/Wayland libraries at runtime, such as `libxkbcommon-x11` and `libGL`. The keychain needs a
Secret Service provider (GNOME Keyring, KWallet, KeePassXC). Typing into other windows uses X11, so it may
not work in native Wayland apps.

The desktop app draws with OpenGL by default. You can switch to wgpu (Vulkan/Metal/DirectX 12) in *Settings → Renderer*,
or set `ANDOTP_RENDERER=wgpu`. If wgpu fails to start, the app switches back to OpenGL for the next start.
To build without the webcam scanner, use `cargo build -p andotp-gui --no-default-features`.

## Android

Every CI run of the **Android** workflow attaches an installable APK (`andOTP-apk` artifact). It runs the
same Rust UI as the desktop app; installation, signing and the differences from the desktop app are in
[android/README.md](android/README.md).

## Using the CLI

```sh
andotp init                                    # create a vault (asks for a password)
andotp add 'otpauth://totp/GitHub:me?secret=JBSWY3DPEHPK3PXP&issuer=GitHub' -t work
andotp add --type hotp --issuer Corp --label vpn --secret GEZDGNBVGY3TQOJQ
andotp list                                    # all tokens with time remaining
andotp list --tag work --sort issuer           # filter and sort
andotp watch                                   # live view, refreshed every second
andotp code github                             # print one code (for scripts)
andotp edit github --add-tag personal --position 1
andotp qr github                               # QR code in the terminal
andotp import otp_accounts.json.aes            # andOTP backup from your phone
andotp export ~/backups --timestamp            # password-protected andOTP backup
andotp passwd                                  # change the vault password
andotp type github --delay 3                   # type the code into the focused window
andotp keychain enable                         # stop asking for the password (system keychain)
andotp backup --dir ~/backups --keep 30        # copy the encrypted vault (e.g. from cron)
andotp panic                                   # destroy the vault (asks for confirmation)
```

Pick an entry by its number from `list`, or by any text that matches only one entry.

| Environment variable | Purpose |
| --- | --- |
| `ANDOTP_VAULT` | Vault path. The default is the platform data directory, e.g. `~/.local/share/andotp/secrets.json.aes`. The desktop app reads it too. |
| `ANDOTP_RENDERER` | Desktop app only: `glow` (OpenGL) or `wgpu` |
| `ANDOTP_PASSWORD` | Vault password, for automation. Prefer the interactive prompt. |
| `ANDOTP_BACKUP_PASSWORD` | Password for `import`/`export` |
| `ANDOTP_PIN` | mOTP PIN for `code` |
| `NO_COLOR` | Turns off coloured output |

## File formats

All formats match andOTP for Android byte for byte:

* **Plain text** (`otp_accounts.json`): a JSON array of entries.
* **Password protected** (`otp_accounts.json.aes`):
  `iterations (u32, big-endian) ‖ salt (12 B) ‖ nonce (12 B) ‖ AES-256-GCM(JSON)`.
  The key is PBKDF2-HMAC-SHA1(password, salt, iterations). New files use 600 000–660 000 iterations;
  the Android app used about 150 000. Older Android versions can still read the files, because the count is stored in the header.
* **Old password-protected format**: `nonce ‖ AES-256-GCM(JSON)`, with the key SHA-256(password). Import only.
* **OpenPGP backups**: decrypt them with `gpg --decrypt` first, then import the JSON.

The vault the CLI and desktop app use is a password-protected backup file. Each save also
leaves the previous version next to it as `*.bck`. On Unix, files are created with mode `0600`.

## What changed from the Android app

* The whole codebase is Rust, replacing about 12 000 lines of Java.
* Encryption keys, passwords and decrypted vault contents are zeroized after use, and debug output never shows secrets.
* Stronger key derivation for new files: 4× the iterations, and the count stays in the header for compatibility.
* `otpauth://` parsing is more tolerant: parameter names in any case, the issuer read from an `Issuer:account`
  label when there is no `issuer` parameter, and Base32 with spaces or dashes.
* Importing a corrupt or unknown file fails quickly instead of hanging on a bad iteration count.
* The core logic has unit tests, including RFC 4226/6238, GCM and PBKDF2 test vectors. The CLI has end-to-end tests,
  and the desktop app has headless UI tests.

The Android-only features have desktop versions (see the table above). The original app is still in
[`legacy/android`](legacy/android).

## Credits

andOTP was created by [Jakob Nixdorf](https://github.com/flocke) and contributors, starting from
[OTP Authenticator](https://github.com/0xbb/otp-authenticator) by [Bruno Bierbaumer](https://github.com/0xbb).

## License

MIT – see [LICENSE.txt](LICENSE.txt).

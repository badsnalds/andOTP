//! `andotp` – a command-line two-factor authenticator compatible with andOTP.

use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use andotp_core::backup::{self, BackupFormat};
use andotp_core::list::{self, SortMode, TagMode};
use andotp_core::otp::{self, HashAlgorithm};
use andotp_core::{Entry, OtpType, Vault};
use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use zeroize::Zeroizing;

#[derive(Parser)]
#[command(
    name = "andotp",
    version,
    about = "Two-factor authentication (TOTP/HOTP/Steam/mOTP) compatible with andOTP backups"
)]
struct Cli {
    /// Path of the encrypted vault file.
    #[arg(long, global = true, env = "ANDOTP_VAULT")]
    vault: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new, empty vault.
    Init,
    /// Show the current tokens.
    #[command(visible_alias = "ls")]
    List(ListArgs),
    /// Print the token of a single entry (handy for scripts).
    Code {
        /// Entry number (as shown by `list`) or search text.
        entry: String,
        /// PIN for mOTP entries.
        #[arg(long, env = "ANDOTP_PIN")]
        pin: Option<String>,
    },
    /// Continuously display tokens, refreshing every second (Ctrl-C to quit).
    Watch(ListArgs),
    /// Add an entry from an otpauth:// URI or from individual fields.
    Add(AddArgs),
    /// Change fields of an entry.
    Edit(EditArgs),
    /// Remove an entry.
    #[command(visible_alias = "rm")]
    Remove {
        entry: String,
        /// Do not ask for confirmation.
        #[arg(short, long)]
        yes: bool,
    },
    /// Print the otpauth:// URI of an entry.
    Uri { entry: String },
    /// Render the otpauth:// URI of an entry as a QR code in the terminal.
    Qr { entry: String },
    /// Import an andOTP backup (plain, password protected or legacy password protected).
    Import {
        file: PathBuf,
        /// Password of the backup (prompted for if needed and not given).
        #[arg(long, env = "ANDOTP_BACKUP_PASSWORD", hide_env_values = true)]
        backup_password: Option<String>,
    },
    /// Export all entries as an andOTP backup.
    Export {
        /// Target file or directory (a directory receives the default andOTP file name).
        target: PathBuf,
        #[arg(long, value_enum, default_value_t = ExportFormat::Encrypted)]
        format: ExportFormat,
        /// Append the date and time to the generated file name.
        #[arg(long)]
        timestamp: bool,
        #[arg(long, env = "ANDOTP_BACKUP_PASSWORD", hide_env_values = true)]
        backup_password: Option<String>,
    },
    /// Change the vault password.
    Passwd,
    /// Print the location of the vault.
    Path,
}

#[derive(Clone, Copy, ValueEnum)]
enum ExportFormat {
    /// Password protected (`.json.aes`).
    Encrypted,
    /// Unencrypted JSON – handle with care!
    Plain,
}

#[derive(Args)]
struct ListArgs {
    /// Only show entries whose issuer, label or tags contain this text.
    query: Option<String>,
    #[arg(long, value_enum, default_value_t = Sort::Manual)]
    sort: Sort,
    /// Only show entries with this tag (repeatable).
    #[arg(short, long = "tag")]
    tags: Vec<String>,
    /// Require all given tags instead of any.
    #[arg(long)]
    all_tags: bool,
    /// Group digits, e.g. `123 456` (0 disables grouping).
    #[arg(long, default_value_t = 3)]
    group: usize,
}

#[derive(Clone, Copy, ValueEnum)]
enum Sort {
    Manual,
    Issuer,
    Label,
    LastUsed,
    MostUsed,
}

impl From<Sort> for SortMode {
    fn from(s: Sort) -> Self {
        match s {
            Sort::Manual => SortMode::Unsorted,
            Sort::Issuer => SortMode::Issuer,
            Sort::Label => SortMode::Label,
            Sort::LastUsed => SortMode::LastUsed,
            Sort::MostUsed => SortMode::MostUsed,
        }
    }
}

#[derive(Args)]
struct AddArgs {
    /// An otpauth:// URI (as encoded in setup QR codes).
    uri: Option<String>,
    /// Secret (Base32, or the hex string for mOTP). Prompted for when omitted.
    #[arg(long, conflicts_with = "uri")]
    secret: Option<String>,
    #[arg(long, value_enum, default_value_t = Kind::Totp, conflicts_with = "uri")]
    r#type: Kind,
    #[arg(long, default_value = "", conflicts_with = "uri")]
    issuer: String,
    #[arg(long, default_value = "", conflicts_with = "uri")]
    label: String,
    #[arg(long, conflicts_with = "uri")]
    digits: Option<u32>,
    #[arg(long, conflicts_with = "uri")]
    period: Option<u32>,
    #[arg(long, conflicts_with = "uri")]
    counter: Option<u64>,
    #[arg(long, value_enum, conflicts_with = "uri")]
    algorithm: Option<Algorithm>,
    /// Tag to attach (repeatable).
    #[arg(short, long = "tag")]
    tags: Vec<String>,
}

#[derive(Args)]
struct EditArgs {
    entry: String,
    #[arg(long)]
    issuer: Option<String>,
    #[arg(long)]
    label: Option<String>,
    #[arg(long)]
    digits: Option<u32>,
    #[arg(long)]
    period: Option<u32>,
    #[arg(long)]
    counter: Option<u64>,
    #[arg(long, value_enum)]
    algorithm: Option<Algorithm>,
    /// Add a tag (repeatable).
    #[arg(long)]
    add_tag: Vec<String>,
    /// Remove a tag (repeatable).
    #[arg(long)]
    remove_tag: Vec<String>,
    /// Move the entry to this position (1-based) in the manual order.
    #[arg(long)]
    position: Option<usize>,
}

#[derive(Clone, Copy, ValueEnum)]
enum Kind {
    Totp,
    Hotp,
    Steam,
    Motp,
}

impl From<Kind> for OtpType {
    fn from(k: Kind) -> Self {
        match k {
            Kind::Totp => OtpType::Totp,
            Kind::Hotp => OtpType::Hotp,
            Kind::Steam => OtpType::Steam,
            Kind::Motp => OtpType::Motp,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum Algorithm {
    Sha1,
    Sha256,
    Sha512,
}

impl From<Algorithm> for HashAlgorithm {
    fn from(a: Algorithm) -> Self {
        match a {
            Algorithm::Sha1 => HashAlgorithm::Sha1,
            Algorithm::Sha256 => HashAlgorithm::Sha256,
            Algorithm::Sha512 => HashAlgorithm::Sha512,
        }
    }
}

fn main() {
    if let Err(err) = run(Cli::parse()) {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    let path = match cli.vault {
        Some(path) => path,
        None => Vault::default_path().context("cannot determine a data directory; use --vault")?,
    };

    match cli.command {
        Command::Init => init(&path),
        Command::Path => {
            println!("{}", path.display());
            Ok(())
        }
        Command::List(args) => {
            let vault = open(&path)?;
            print_list(&vault, &args, false)
        }
        Command::Watch(args) => watch(&open(&path)?, &args),
        Command::Code { entry, pin } => {
            let mut vault = open(&path)?;
            let idx = select(&vault, &entry)?;
            let e = &mut vault.entries[idx];
            if e.otp_type == OtpType::Motp && pin.is_none() {
                let pin = prompt_secret("mOTP PIN: ")?;
                println!("{}", e.token(Some(&pin)).code);
            } else {
                println!("{}", e.token(pin.as_deref()).code);
            }
            // Counter based tokens are consumed when shown.
            if e.otp_type == OtpType::Hotp {
                e.counter += 1;
            }
            e.mark_used();
            vault.save()?;
            Ok(())
        }
        Command::Add(args) => add(&mut open(&path)?, args),
        Command::Edit(args) => edit(&mut open(&path)?, args),
        Command::Remove { entry, yes } => {
            let mut vault = open(&path)?;
            let idx = select(&vault, &entry)?;
            let name = vault.entries[idx].display_name();
            if !yes && !confirm(&format!("Remove `{name}`? This cannot be undone. [y/N] "))? {
                bail!("aborted");
            }
            vault.entries.remove(idx);
            vault.save()?;
            println!("Removed `{name}`.");
            Ok(())
        }
        Command::Uri { entry } => {
            let vault = open(&path)?;
            println!("{}", vault.entries[select(&vault, &entry)?].to_uri());
            Ok(())
        }
        Command::Qr { entry } => {
            let vault = open(&path)?;
            let uri = vault.entries[select(&vault, &entry)?].to_uri();
            let code = qrcode::QrCode::new(uri.as_bytes()).context("URI too long for a QR code")?;
            let rendered = code
                .render::<qrcode::render::unicode::Dense1x2>()
                .dark_color(qrcode::render::unicode::Dense1x2::Light)
                .light_color(qrcode::render::unicode::Dense1x2::Dark)
                .build();
            println!("{rendered}");
            Ok(())
        }
        Command::Import { file, backup_password } => {
            let mut vault = open(&path)?;
            let data = std::fs::read(&file).with_context(|| format!("cannot read {}", file.display()))?;
            let entries = match backup::import(&data, backup_password.as_deref()) {
                Ok((entries, _)) => entries,
                Err(_) if backup_password.is_none() => {
                    let pw = prompt_secret("Backup password: ")?;
                    backup::import(&data, Some(&pw)).context("cannot read backup")?.0
                }
                Err(e) => return Err(e).context("cannot read backup"),
            };
            let summary = vault.import(entries);
            vault.save()?;
            println!("Imported {} entries ({} duplicates skipped).", summary.added, summary.skipped);
            Ok(())
        }
        Command::Export { target, format, timestamp, backup_password } => {
            let vault = open(&path)?;
            export(&vault, &target, format, timestamp, backup_password)
        }
        Command::Passwd => {
            let mut vault = open(&path)?;
            let password = new_password()?;
            vault.change_password(&password)?;
            println!("Password changed.");
            Ok(())
        }
    }
}

fn init(path: &Path) -> Result<()> {
    if path.exists() {
        bail!("a vault already exists at {}", path.display());
    }
    let password = new_password()?;
    Vault::create(path, &password)?;
    println!("Created vault at {}", path.display());
    Ok(())
}

fn open(path: &Path) -> Result<Vault> {
    if !path.exists() {
        bail!("no vault at {} – run `andotp init` first", path.display());
    }
    let password = match std::env::var("ANDOTP_PASSWORD") {
        Ok(pw) => Zeroizing::new(pw),
        Err(_) => prompt_secret("Vault password: ")?,
    };
    Vault::open(path, &password).context("cannot unlock vault")
}

fn prompt_secret(prompt: &str) -> Result<Zeroizing<String>> {
    Ok(Zeroizing::new(rpassword::prompt_password(prompt)?))
}

fn new_password() -> Result<Zeroizing<String>> {
    if let Ok(pw) = std::env::var("ANDOTP_PASSWORD") {
        return Ok(Zeroizing::new(pw));
    }
    let password = prompt_secret("New password: ")?;
    if password.chars().count() < 6 {
        bail!("the password must be at least 6 characters long");
    }
    if *prompt_secret("Repeat password: ")? != *password {
        bail!("passwords do not match");
    }
    Ok(password)
}

fn confirm(prompt: &str) -> Result<bool> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}

/// Resolves an entry by its 1-based number or by a unique search match.
fn select(vault: &Vault, query: &str) -> Result<usize> {
    if let Ok(n) = query.trim_start_matches('#').parse::<usize>() {
        if (1..=vault.entries.len()).contains(&n) {
            return Ok(n - 1);
        }
    }
    let matches = list::view(&vault.entries, query, &[], TagMode::Or, SortMode::Unsorted);
    let exact: Vec<usize> = matches
        .iter()
        .copied()
        .filter(|&i| {
            let e = &vault.entries[i];
            e.display_name().eq_ignore_ascii_case(query) || e.label.eq_ignore_ascii_case(query)
        })
        .collect();
    match (matches.as_slice(), exact.as_slice()) {
        ([only], _) | (_, [only]) => Ok(*only),
        ([], _) => bail!("no entry matches `{query}`"),
        (many, _) => {
            let names: Vec<String> =
                many.iter().map(|&i| format!("  #{} {}", i + 1, vault.entries[i].display_name())).collect();
            bail!("`{query}` matches several entries:\n{}", names.join("\n"))
        }
    }
}

fn print_list(vault: &Vault, args: &ListArgs, clear: bool) -> Result<()> {
    let indices = list::view(
        &vault.entries,
        args.query.as_deref().unwrap_or(""),
        &args.tags,
        if args.all_tags { TagMode::And } else { TagMode::Or },
        args.sort.into(),
    );
    let color = io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let mut out = String::new();
    if clear {
        out.push_str("\x1b[2J\x1b[H");
    }
    if indices.is_empty() {
        out.push_str("No entries.\n");
    }
    let name_width = indices.iter().map(|&i| vault.entries[i].display_name().chars().count()).max().unwrap_or(0);
    for i in indices {
        let entry = &vault.entries[i];
        let token = entry.token(None);
        let code = match entry.otp_type {
            OtpType::Motp if token.remaining.is_none() => "(PIN needed)".to_owned(),
            _ => otp::group_token(&token.code, args.group),
        };
        let timer = match token.remaining {
            Some(r) => format!("{r:>3}s"),
            None if entry.otp_type == OtpType::Hotp => format!(" #{}", entry.counter),
            None => String::new(),
        };
        let (start, end) = match (color, token.is_expiring()) {
            (true, true) => ("\x1b[31m", "\x1b[0m"),
            (true, false) => ("\x1b[1m", "\x1b[0m"),
            _ => ("", ""),
        };
        let tags = if entry.tags.is_empty() { String::new() } else { format!("  [{}]", entry.tags.join(", ")) };
        out.push_str(&format!(
            "{:>3}  {:<name_width$}  {start}{code:>12}{end} {timer}{tags}\n",
            i + 1,
            entry.display_name(),
        ));
    }
    print!("{out}");
    io::stdout().flush()?;
    Ok(())
}

fn watch(vault: &Vault, args: &ListArgs) -> Result<()> {
    loop {
        print_list(vault, args, true)?;
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn add(vault: &mut Vault, args: AddArgs) -> Result<()> {
    let mut entry = match args.uri {
        Some(uri) => Entry::from_uri(&uri)?,
        None => {
            let secret = match args.secret {
                Some(s) => Zeroizing::new(s),
                None => prompt_secret("Secret: ")?,
            };
            let otp_type = OtpType::from(args.r#type);
            let mut entry = Entry::new(otp_type, &secret, &args.issuer, &args.label)?;
            if let Some(d) = args.digits {
                entry.digits = d;
            }
            if let Some(p) = args.period {
                entry.period = p;
            }
            if let Some(c) = args.counter {
                entry.counter = c;
            }
            if let Some(a) = args.algorithm {
                entry.algorithm = a.into();
            }
            entry
        }
    };
    for tag in args.tags {
        if !entry.tags.contains(&tag) {
            entry.tags.push(tag);
        }
    }
    entry.validate()?;
    let name = entry.display_name();
    if !vault.add(entry) {
        bail!("`{name}` is already in the vault");
    }
    vault.save()?;
    println!("Added `{name}`.");
    Ok(())
}

fn edit(vault: &mut Vault, args: EditArgs) -> Result<()> {
    let idx = select(vault, &args.entry)?;
    let entry = &mut vault.entries[idx];
    if let Some(issuer) = args.issuer {
        entry.issuer = issuer;
        entry.update_thumbnail();
    }
    if let Some(label) = args.label {
        entry.label = label;
    }
    if let Some(d) = args.digits {
        entry.digits = d;
    }
    if let Some(p) = args.period {
        entry.period = p;
    }
    if let Some(c) = args.counter {
        entry.counter = c;
    }
    if let Some(a) = args.algorithm {
        entry.algorithm = a.into();
    }
    entry.tags.retain(|t| !args.remove_tag.contains(t));
    for tag in args.add_tag {
        if !entry.tags.contains(&tag) {
            entry.tags.push(tag);
        }
    }
    entry.validate()?;
    let name = entry.display_name();
    if let Some(pos) = args.position {
        vault.move_entry(idx, pos.clamp(1, vault.entries.len()) - 1);
    }
    vault.save()?;
    println!("Updated `{name}`.");
    Ok(())
}

fn export(vault: &Vault, target: &Path, format: ExportFormat, timestamp: bool, password: Option<String>) -> Result<()> {
    let backup_format = match format {
        ExportFormat::Encrypted => BackupFormat::Encrypted,
        ExportFormat::Plain => BackupFormat::Plain,
    };
    let path = if target.is_dir() {
        let ts = timestamp.then(|| chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string());
        target.join(backup_format.file_name(ts.as_deref()))
    } else {
        target.to_path_buf()
    };
    let data = match format {
        ExportFormat::Plain => backup::to_json(&vault.entries)?.into_bytes(),
        ExportFormat::Encrypted => {
            let password = match password {
                Some(p) => Zeroizing::new(p),
                None => {
                    let p = prompt_secret("Backup password: ")?;
                    if *prompt_secret("Repeat backup password: ")? != *p {
                        bail!("passwords do not match");
                    }
                    p
                }
            };
            backup::export_encrypted(&vault.entries, &password)?
        }
    };
    andotp_core::vault::write_private(&path, &data).with_context(|| format!("cannot write {}", path.display()))?;
    println!("Exported {} entries to {}", vault.entries.len(), path.display());
    if matches!(format, ExportFormat::Plain) {
        eprintln!("warning: plain-text backups contain your secrets unencrypted");
    }
    Ok(())
}

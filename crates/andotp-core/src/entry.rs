//! The account model.

use std::fmt;
use std::str::FromStr;

use percent_encoding::{AsciiSet, CONTROLS, percent_decode_str, utf8_percent_encode};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use url::Url;

use crate::otp::{self, HashAlgorithm};
use crate::{Error, Result, base32, thumbnail};

/// Token returned for an mOTP entry while no PIN has been entered.
pub const MOTP_NO_PIN_CODE: &str = "PINREQ";
/// Seconds before expiry at which a time based token is considered "expiring".
pub const EXPIRY_WARNING_SECS: u32 = 8;

/// The kind of one-time password an entry produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum OtpType {
    #[default]
    Totp,
    Hotp,
    Motp,
    Steam,
}

impl OtpType {
    pub const ALL: [OtpType; 4] = [Self::Totp, Self::Hotp, Self::Motp, Self::Steam];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Totp => "TOTP",
            Self::Hotp => "HOTP",
            Self::Motp => "MOTP",
            Self::Steam => "STEAM",
        }
    }

    pub fn is_time_based(self) -> bool {
        !matches!(self, Self::Hotp)
    }

    pub fn default_digits(self) -> u32 {
        match self {
            Self::Steam => otp::STEAM_DEFAULT_DIGITS,
            _ => otp::TOTP_DEFAULT_DIGITS,
        }
    }
}

impl fmt::Display for OtpType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for OtpType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_uppercase().as_str() {
            "TOTP" => Ok(Self::Totp),
            "HOTP" => Ok(Self::Hotp),
            "MOTP" => Ok(Self::Motp),
            "STEAM" => Ok(Self::Steam),
            other => Err(format!("unknown OTP type `{other}`")),
        }
    }
}

/// A generated token together with its validity window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub code: String,
    /// The token of the previous time step, if the entry is time based.
    pub previous: Option<String>,
    /// Seconds until the token changes (`None` for counter based entries).
    pub remaining: Option<u32>,
    /// Length of one time step in seconds (`None` for counter based entries).
    pub period: Option<u32>,
}

impl Token {
    /// Whether the token is about to expire.
    pub fn is_expiring(&self) -> bool {
        self.remaining.is_some_and(|r| r <= EXPIRY_WARNING_SECS)
    }

    /// Remaining validity as a fraction in `0.0..=1.0`.
    pub fn progress(&self) -> f32 {
        match (self.remaining, self.period) {
            (Some(r), Some(p)) if p > 0 => r as f32 / p as f32,
            _ => 1.0,
        }
    }
}

/// One account.
///
/// The serialised form matches the JSON objects of andOTP backups exactly.
#[derive(Clone, PartialEq, Eq)]
pub struct Entry {
    pub otp_type: OtpType,
    /// Raw secret bytes. For mOTP this is the hex string as typed by the user.
    pub secret: Vec<u8>,
    pub issuer: String,
    pub label: String,
    pub period: u32,
    pub counter: u64,
    pub digits: u32,
    pub algorithm: HashAlgorithm,
    pub tags: Vec<String>,
    pub thumbnail: String,
    /// Unix time in milliseconds of the last use.
    pub last_used: i64,
    pub used_frequency: i64,
}

impl fmt::Debug for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Entry")
            .field("otp_type", &self.otp_type)
            .field("secret", &"<redacted>")
            .field("issuer", &self.issuer)
            .field("label", &self.label)
            .field("period", &self.period)
            .field("counter", &self.counter)
            .field("digits", &self.digits)
            .field("algorithm", &self.algorithm)
            .field("tags", &self.tags)
            .finish_non_exhaustive()
    }
}

impl Default for Entry {
    fn default() -> Self {
        Self {
            otp_type: OtpType::Totp,
            secret: Vec::new(),
            issuer: String::new(),
            label: String::new(),
            period: otp::TOTP_DEFAULT_PERIOD,
            counter: 0,
            digits: otp::TOTP_DEFAULT_DIGITS,
            algorithm: HashAlgorithm::Sha1,
            tags: Vec::new(),
            thumbnail: thumbnail::DEFAULT.to_owned(),
            last_used: 0,
            used_frequency: 0,
        }
    }
}

impl Entry {
    /// Creates an entry from a user supplied secret (Base32, or the hex string for mOTP).
    pub fn new(otp_type: OtpType, secret: &str, issuer: &str, label: &str) -> Result<Self> {
        let mut entry = Self {
            otp_type,
            secret: decode_secret(otp_type, secret)?,
            issuer: issuer.trim().to_owned(),
            label: label.trim().to_owned(),
            digits: otp_type.default_digits(),
            counter: if otp_type == OtpType::Hotp { otp::HOTP_INITIAL_COUNTER } else { 0 },
            ..Self::default()
        };
        entry.update_thumbnail();
        Ok(entry)
    }

    /// Sets the thumbnail from the issuer name.
    pub fn update_thumbnail(&mut self) {
        self.thumbnail = thumbnail::for_issuer(&self.issuer).to_owned();
    }

    /// The secret as the user would type it (Base32, or the raw string for mOTP).
    pub fn secret_encoded(&self) -> String {
        match self.otp_type {
            OtpType::Motp => String::from_utf8_lossy(&self.secret).into_owned(),
            _ => base32::encode_unpadded(&self.secret),
        }
    }

    /// Validates field ranges; returns a human readable reason when invalid.
    pub fn validate(&self) -> Result<()> {
        if self.secret.is_empty() {
            return Err(Error::InvalidEntry("secret is empty".into()));
        }
        if !(1..=10).contains(&self.digits) {
            return Err(Error::InvalidEntry("digits must be between 1 and 10".into()));
        }
        if self.otp_type.is_time_based() && self.period == 0 {
            return Err(Error::InvalidEntry("period must be greater than zero".into()));
        }
        Ok(())
    }

    /// The length of one time step in seconds.
    pub fn effective_period(&self) -> u32 {
        match self.otp_type {
            OtpType::Motp => otp::MOTP_PERIOD,
            _ => self.period.max(1),
        }
    }

    /// Generates the token at `time` (seconds since the Unix epoch).
    ///
    /// `pin` is only used by mOTP entries.
    pub fn token_at(&self, time: u64, pin: Option<&str>) -> Token {
        let period = self.effective_period();
        let time_window = |code: String, previous: String| Token {
            code,
            previous: Some(previous),
            remaining: Some(period - (time % u64::from(period)) as u32),
            period: Some(period),
        };
        match self.otp_type {
            OtpType::Totp => time_window(
                otp::totp(&self.secret, time, period, self.digits, self.algorithm, 0),
                otp::totp(&self.secret, time, period, self.digits, self.algorithm, -1),
            ),
            OtpType::Steam => time_window(
                otp::steam(&self.secret, time, period, self.digits, self.algorithm, 0),
                otp::steam(&self.secret, time, period, self.digits, self.algorithm, -1),
            ),
            OtpType::Motp => {
                let secret = String::from_utf8_lossy(&self.secret);
                match pin.filter(|p| !p.is_empty()) {
                    Some(pin) => time_window(otp::motp(pin, &secret, time, 0), otp::motp(pin, &secret, time, -1)),
                    None => Token { code: MOTP_NO_PIN_CODE.into(), previous: None, remaining: None, period: None },
                }
            }
            OtpType::Hotp => Token {
                code: otp::hotp(&self.secret, self.counter, self.digits, self.algorithm),
                previous: None,
                remaining: None,
                period: None,
            },
        }
    }

    /// Generates the token for the current system time.
    pub fn token(&self, pin: Option<&str>) -> Token {
        self.token_at(unix_now(), pin)
    }

    /// Records a use of the entry (for the "last used" and "most used" sort modes).
    pub fn mark_used(&mut self) {
        self.last_used = unix_now_millis();
        self.used_frequency += 1;
    }

    /// `issuer - label`, or whichever of the two is set.
    pub fn display_name(&self) -> String {
        match (self.issuer.is_empty(), self.label.is_empty()) {
            (false, false) => format!("{} - {}", self.issuer, self.label),
            (false, true) => self.issuer.clone(),
            _ => self.label.clone(),
        }
    }

    /// Whether `other` describes the same account (the duplicate check of the Android app).
    pub fn same_account(&self, other: &Entry) -> bool {
        self.otp_type == other.otp_type
            && self.period == other.period
            && self.counter == other.counter
            && self.digits == other.digits
            && self.algorithm == other.algorithm
            && self.secret == other.secret
            && self.label == other.label
            && self.issuer == other.issuer
    }

    /// Parses an `otpauth://` URI as produced by most services' QR codes.
    pub fn from_uri(uri: &str) -> Result<Self> {
        let invalid = |msg: &str| Error::InvalidUri(msg.to_owned());
        let url = Url::parse(uri.trim()).map_err(|e| Error::InvalidUri(e.to_string()))?;
        if !url.scheme().eq_ignore_ascii_case("otpauth") {
            return Err(invalid("scheme must be otpauth"));
        }
        let otp_type: OtpType = url
            .host_str()
            .ok_or_else(|| invalid("missing OTP type"))?
            .parse()
            .map_err(|e: String| Error::InvalidUri(e))?;

        let mut secret = None;
        let mut issuer = None;
        let mut counter = None;
        let mut period = None;
        let mut digits = None;
        let mut algorithm = None;
        let mut tags = Vec::new();
        for (key, value) in url.query_pairs() {
            match key.to_ascii_lowercase().as_str() {
                "secret" => secret = Some(value.into_owned()),
                "issuer" => issuer = Some(value.into_owned()),
                "counter" => counter = Some(value.parse::<u64>().map_err(|_| invalid("invalid counter"))?),
                "period" => period = Some(value.parse::<u32>().map_err(|_| invalid("invalid period"))?),
                "digits" => digits = Some(value.parse::<u32>().map_err(|_| invalid("invalid digits"))?),
                "algorithm" => algorithm = Some(value.parse::<HashAlgorithm>().map_err(Error::InvalidUri)?),
                "tags" => tags.push(value.into_owned()),
                _ => {}
            }
        }

        let path = url.path().strip_prefix('/').unwrap_or(url.path());
        let path = percent_decode(path);
        let (issuer, label) = split_label(issuer, &path);

        let secret = secret.ok_or_else(|| invalid("missing secret"))?;
        if otp_type == OtpType::Hotp && counter.is_none() {
            return Err(invalid("missing counter for HOTP"));
        }

        let mut entry = Self {
            otp_type,
            secret: decode_secret(otp_type, &secret)?,
            issuer,
            label,
            period: period.unwrap_or(otp::TOTP_DEFAULT_PERIOD),
            counter: counter.unwrap_or(0),
            digits: digits.unwrap_or(otp_type.default_digits()),
            algorithm: algorithm.unwrap_or_default(),
            tags,
            ..Self::default()
        };
        entry.update_thumbnail();
        entry.validate()?;
        Ok(entry)
    }

    /// Serialises the entry as an `otpauth://` URI (e.g. for QR codes or other apps).
    pub fn to_uri(&self) -> String {
        let mut url = Url::parse(&format!("otpauth://{}", self.otp_type.as_str().to_ascii_lowercase()))
            .expect("static URL is valid");
        let path = if self.issuer.is_empty() { self.label.clone() } else { format!("{}:{}", self.issuer, self.label) };
        url.set_path(&format!("/{}", percent_encode(&path)));
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("secret", &self.secret_encoded());
            if !self.issuer.is_empty() {
                q.append_pair("issuer", &self.issuer);
            }
            if self.otp_type == OtpType::Hotp {
                q.append_pair("counter", &self.counter.to_string());
            }
            if matches!(self.otp_type, OtpType::Totp | OtpType::Steam) && self.period != otp::TOTP_DEFAULT_PERIOD {
                q.append_pair("period", &self.period.to_string());
            }
            if self.digits != self.otp_type.default_digits() {
                q.append_pair("digits", &self.digits.to_string());
            }
            if self.algorithm != HashAlgorithm::Sha1 {
                q.append_pair("algorithm", self.algorithm.as_str());
            }
            for tag in &self.tags {
                q.append_pair("tags", tag);
            }
        }
        url.to_string()
    }
}

/// Decodes a user supplied secret for the given OTP type.
pub fn decode_secret(otp_type: OtpType, secret: &str) -> Result<Vec<u8>> {
    let secret = secret.trim();
    let bytes = match otp_type {
        OtpType::Motp => {
            if secret.is_empty() || !secret.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(Error::InvalidSecret);
            }
            secret.as_bytes().to_vec()
        }
        _ => base32::decode(secret)?,
    };
    if bytes.is_empty() {
        return Err(Error::InvalidSecret);
    }
    Ok(bytes)
}

/// Splits the URI label into issuer and account name.
///
/// An explicit `issuer` parameter wins; otherwise an `Issuer:` prefix of the label is used.
fn split_label(issuer: Option<String>, path: &str) -> (String, String) {
    match issuer {
        Some(issuer) => {
            let label = match path.strip_prefix(&format!("{issuer}:")) {
                Some(rest) if !issuer.is_empty() => rest,
                _ => path,
            };
            (issuer.trim().to_owned(), label.trim().to_owned())
        }
        None => match path.split_once(':') {
            Some((prefix, rest)) if !prefix.trim().is_empty() => (prefix.trim().to_owned(), rest.trim().to_owned()),
            _ => (String::new(), path.trim().to_owned()),
        },
    }
}

/// Characters escaped in the label part of an `otpauth://` URI.
const LABEL_ESCAPE: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'/')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}')
    .add(b'&')
    .add(b'+')
    .add(b'=');

fn percent_decode(s: &str) -> String {
    percent_decode_str(s).decode_utf8_lossy().into_owned()
}

fn percent_encode(s: &str) -> String {
    utf8_percent_encode(s, LABEL_ESCAPE).to_string()
}

pub(crate) fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub(crate) fn unix_now_millis() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

// --- andOTP JSON representation -------------------------------------------------------------

/// Field order mirrors the Android app so that files diff cleanly between implementations.
#[derive(Serialize)]
struct JsonOut<'a> {
    secret: String,
    issuer: &'a str,
    label: &'a str,
    digits: u32,
    #[serde(rename = "type")]
    otp_type: OtpType,
    algorithm: HashAlgorithm,
    thumbnail: &'a str,
    last_used: i64,
    used_frequency: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    period: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    counter: Option<u64>,
    tags: &'a [String],
}

#[derive(Deserialize)]
struct JsonIn {
    secret: String,
    label: String,
    #[serde(default)]
    issuer: Option<String>,
    #[serde(default, rename = "type")]
    otp_type: Option<serde_json::Value>,
    #[serde(default)]
    period: Option<serde_json::Value>,
    #[serde(default)]
    counter: Option<serde_json::Value>,
    #[serde(default)]
    digits: Option<serde_json::Value>,
    #[serde(default)]
    algorithm: Option<serde_json::Value>,
    #[serde(default)]
    tags: Option<serde_json::Value>,
    #[serde(default)]
    thumbnail: Option<serde_json::Value>,
    #[serde(default)]
    last_used: Option<serde_json::Value>,
    #[serde(default)]
    used_frequency: Option<serde_json::Value>,
}

/// Lenient number parsing like `org.json` (`"30"` and `30.0` are both accepted).
fn json_u64(value: &Option<serde_json::Value>) -> Option<u64> {
    match value.as_ref()? {
        serde_json::Value::Number(n) => n.as_u64().or_else(|| n.as_f64().map(|f| f as u64)),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn json_str(value: &Option<serde_json::Value>) -> Option<&str> {
    value.as_ref()?.as_str()
}

impl Serialize for Entry {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        JsonOut {
            secret: base32::encode(&self.secret),
            issuer: &self.issuer,
            label: &self.label,
            digits: self.digits,
            otp_type: self.otp_type,
            algorithm: self.algorithm,
            thumbnail: &self.thumbnail,
            last_used: self.last_used,
            used_frequency: self.used_frequency,
            period: matches!(self.otp_type, OtpType::Totp | OtpType::Steam).then_some(self.period),
            counter: (self.otp_type == OtpType::Hotp).then_some(self.counter),
            tags: &self.tags,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Entry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;
        let json = JsonIn::deserialize(deserializer)?;
        let otp_type = json_str(&json.otp_type).and_then(|s| s.parse().ok()).unwrap_or_default();
        let counter = json_u64(&json.counter);
        if otp_type == OtpType::Hotp && counter.is_none() {
            return Err(D::Error::custom("missing counter for HOTP"));
        }
        let secret = base32::decode(&json.secret).map_err(D::Error::custom)?;
        let tags = match json.tags {
            Some(serde_json::Value::Array(items)) => {
                items.into_iter().filter_map(|t| t.as_str().map(str::to_owned)).collect()
            }
            _ => Vec::new(),
        };
        Ok(Entry {
            otp_type,
            secret,
            issuer: json.issuer.unwrap_or_default(),
            label: json.label,
            period: json_u64(&json.period).map_or(otp::TOTP_DEFAULT_PERIOD, |p| p as u32),
            counter: counter.unwrap_or(0),
            digits: json_u64(&json.digits).map_or(otp::TOTP_DEFAULT_DIGITS, |d| d as u32),
            algorithm: json_str(&json.algorithm).and_then(|s| s.parse().ok()).unwrap_or_default(),
            tags,
            thumbnail: json_str(&json.thumbnail).and_then(thumbnail::lookup).unwrap_or(thumbnail::DEFAULT).to_owned(),
            last_used: json_u64(&json.last_used).unwrap_or(0) as i64,
            used_frequency: json_u64(&json.used_frequency).unwrap_or(0) as i64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "HXDMVJECJJWSRB3HWIZR4IFUGFTMXBOZ";

    #[test]
    fn json_round_trip_matches_android_output() {
        let secret = b"Das System ist sicher";
        let json = format!(
            "{{\"secret\":\"{}\",\"issuer\":\"\",\"label\":\"5 von 5 Sterne\",\"digits\":6,\"type\":\"TOTP\",\
             \"algorithm\":\"SHA1\",\"thumbnail\":\"Default\",\"last_used\":0,\"used_frequency\":0,\
             \"period\":30,\"tags\":[\"test1\",\"test2\"]}}",
            base32::encode(secret)
        );
        let entry: Entry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry.secret, secret);
        assert_eq!(entry.label, "5 von 5 Sterne");
        assert_eq!(entry.tags, ["test1", "test2"]);
        assert_eq!(serde_json::to_string(&entry).unwrap(), json);
    }

    #[test]
    fn json_defaults_for_old_backups() {
        let entry: Entry = serde_json::from_str(r#"{"secret":"MZXW6","label":"old"}"#).unwrap();
        assert_eq!(entry.otp_type, OtpType::Totp);
        assert_eq!(entry.issuer, "");
        assert_eq!(entry.period, 30);
        assert_eq!(entry.digits, 6);
        assert_eq!(entry.thumbnail, "Default");

        let err = serde_json::from_str::<Entry>(r#"{"secret":"MZXW6","label":"x","type":"HOTP"}"#);
        assert!(err.is_err());
    }

    #[test]
    fn invalid_uris() {
        assert!(Entry::from_uri("DON'T CARE").is_err());
        assert!(Entry::from_uri("https://github.com/0xbb/").is_err());
        assert!(Entry::from_uri(&format!("otpauth://hotp/ACME%20Co:john.doe@email.com?secret={SECRET}")).is_err());
        assert!(Entry::from_uri("otpauth://totp/ACME").is_err());
    }

    #[test]
    fn uri_labels() {
        let e = Entry::from_uri(&format!(
            "otpauth://totp/ACME%20Co:john.doe@email.com?secret={SECRET}&issuer=ACME%20Co&ALGORITHM=SHA1&digits=6&period=30"
        ))
        .unwrap();
        assert_eq!(e.label, "john.doe@email.com");
        assert_eq!(e.issuer, "ACME Co");
        assert_eq!(base32::encode(&e.secret), SECRET);

        let e = Entry::from_uri(&format!("otpauth://totp/%20:john.doe@email.com?secret={SECRET}&digits=6")).unwrap();
        assert_eq!(e.label, ":john.doe@email.com");

        let e = Entry::from_uri(&format!("otpauth://totp/%20:john.doe@email.com?secret={SECRET}&issuer=%20")).unwrap();
        assert_eq!(e.label, "john.doe@email.com");

        // Issuer taken from the label prefix when the parameter is missing.
        let e = Entry::from_uri(&format!("otpauth://totp/GitHub:octocat?secret={SECRET}")).unwrap();
        assert_eq!((e.issuer.as_str(), e.label.as_str(), e.thumbnail.as_str()), ("GitHub", "octocat", "GitHub"));
    }

    #[test]
    fn uri_tags_and_parameters() {
        let e = Entry::from_uri(&format!(
            "otpauth://hotp/Corp:me?secret={SECRET}&issuer=Corp&counter=42&digits=8&algorithm=SHA512&tags=a&tags=b%20c"
        ))
        .unwrap();
        assert_eq!(e.otp_type, OtpType::Hotp);
        assert_eq!(e.counter, 42);
        assert_eq!(e.digits, 8);
        assert_eq!(e.algorithm, HashAlgorithm::Sha512);
        assert_eq!(e.tags, ["a", "b c"]);
    }

    #[test]
    fn uri_round_trip() {
        for uri in [
            format!("otpauth://totp/ACME%20Co:john@example.com?secret={SECRET}&issuer=ACME+Co&period=60&tags=work"),
            format!("otpauth://hotp/label?secret={SECRET}&counter=7&digits=8&algorithm=SHA256"),
            format!("otpauth://steam/Steam:gamer?secret={SECRET}&issuer=Steam"),
            "otpauth://motp/phone?secret=0123456789abcdef".to_owned(),
        ] {
            let e = Entry::from_uri(&uri).unwrap();
            let again = Entry::from_uri(&e.to_uri()).unwrap();
            assert!(e.same_account(&again), "{uri} -> {}", e.to_uri());
            assert_eq!(e.tags, again.tags);
        }
    }

    #[test]
    fn tokens() {
        let mut e = Entry { secret: b"12345678901234567890".to_vec(), digits: 8, ..Entry::default() };
        let t = e.token_at(59, None);
        assert_eq!(t.code, "94287082");
        assert_eq!(t.remaining, Some(1));
        assert!(t.is_expiring());

        e.otp_type = OtpType::Hotp;
        e.digits = 6;
        e.counter = 1;
        assert_eq!(e.token_at(0, None).code, "287082");

        let m = Entry::new(OtpType::Motp, "0123456789abcdef", "", "phone").unwrap();
        assert_eq!(m.token_at(100, None).code, MOTP_NO_PIN_CODE);
        assert_eq!(m.token_at(100, Some("1234")).code.len(), 6);
        assert!(Entry::new(OtpType::Motp, "not hex", "", "x").is_err());
    }
}

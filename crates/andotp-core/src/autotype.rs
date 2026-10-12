//! Typing tokens into the focused window – the desktop counterpart of Android auto-fill.

use std::time::Duration;

use enigo::{Enigo, Keyboard, Settings};

use crate::{Error, Result};

/// Waits `delay` (time for the user or the caller to focus the target window), then types `text`
/// as keystrokes into whatever window has the keyboard focus.
pub fn type_text(text: &str, delay: Duration) -> Result<()> {
    std::thread::sleep(delay);
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| Error::AutoType(e.to_string()))?;
    enigo.text(text).map_err(|e| Error::AutoType(e.to_string()))
}

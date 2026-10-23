//! Pronunciation through the built-in Windows speech engine (SAPI).
//!
//! The voice is created lazily on the UI thread that owns the details window.
//! COM is initialized in apartment-threaded mode; if the thread was already
//! initialized with another model we keep going, because SAPI only needs an
//! initialized apartment and returns its own error if it cannot start.

use windows::Win32::Media::Speech::ISpVoice;
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::core::{GUID, PCWSTR};

/// `SVSFlagsAsync`: queue the utterance and return immediately.
const SVSF_ASYNC: u32 = 1;
/// `SVSFPurgeBeforeSpeak`: a new click replaces the previous utterance.
const SVSF_PURGE: u32 = 2;
/// `SVSFIsXML`: the text is a small SSML fragment.
const SVSF_IS_XML: u32 = 8;

/// CLSID_SpVoice, the default SAPI voice object.
const CLSID_SP_VOICE: GUID = GUID::from_u128(0x96749377_3391_11d2_9ee3_00c04f797396);

/// The bundled vocabulary is English. SAPI's `langid` is hexadecimal, so
/// `409` means U.S. English.
const EN_US_LANG_ID: &str = "409";

/// A SAPI voice bound to the current thread's COM apartment.
pub struct Voice {
    inner: ISpVoice,
}

impl Voice {
    pub fn new() -> Result<Voice, String> {
        // SAFETY: called on a UI thread that may or may not have initialized
        // COM already; a changed-mode error is harmless for this use.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let inner: ISpVoice = CoCreateInstance(&CLSID_SP_VOICE, None, CLSCTX_ALL)
                .map_err(|error| format!("Windows speech is unavailable: {error}"))?;
            Ok(Voice { inner })
        }
    }

    /// Queues `text` for pronunciation without blocking the window.
    pub fn speak(&self, text: &str) -> Result<(), String> {
        if text.trim().is_empty() {
            return Ok(());
        }
        // Prefer an English voice, even when Windows defaults to another
        // language. If no English voice is installed SAPI rejects the SSML and
        // we speak the plain text with whatever voice the system provides.
        let ssml = format!(
            "<lang langid=\"{EN_US_LANG_ID}\">{}</lang>",
            escape_xml(text)
        );
        let buffer = crate::win::wide(&ssml);
        // SAFETY: the buffer is NUL-terminated and lives until Speak returns.
        let english = unsafe {
            self.inner.Speak(
                PCWSTR::from_raw(buffer.as_ptr()),
                SVSF_ASYNC | SVSF_PURGE | SVSF_IS_XML,
                None,
            )
        };
        if english.is_ok() {
            return Ok(());
        }
        let plain = crate::win::wide(text);
        // SAFETY: same contract as above.
        unsafe {
            self.inner
                .Speak(
                    PCWSTR::from_raw(plain.as_ptr()),
                    SVSF_ASYNC | SVSF_PURGE,
                    None,
                )
                .map_err(|error| format!("Could not start pronunciation: {error}"))?;
        }
        Ok(())
    }
}

/// Escapes the five XML metacharacters so the word can live inside SSML.
fn escape_xml(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sapi_voice_is_available() {
        let voice = Voice::new().expect("the Windows speech engine should be available");
        voice.speak("").expect("an empty utterance is a no-op");
    }

    #[test]
    fn xml_metacharacters_are_escaped() {
        assert_eq!(
            escape_xml("rock & roll <live> \"quoted\" 'x'"),
            "rock &amp; roll &lt;live&gt; &quot;quoted&quot; &apos;x&apos;"
        );
        assert_eq!(escape_xml("sesgo — optimismo"), "sesgo — optimismo");
    }
}

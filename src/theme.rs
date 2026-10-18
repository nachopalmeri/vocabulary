//! Colours for the card and for application dialogs.

use crate::model::ThemeMode;
use windows::Win32::Foundation::COLORREF;

/// `COLORREF` is `0x00BBGGRR`.
pub const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF((r as u32) | ((g as u32) << 8) | ((b as u32) << 16))
}

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub dark: bool,
    pub bg: COLORREF,
    pub border: COLORREF,
    pub word: COLORREF,
    pub text: COLORREF,
    pub muted: COLORREF,
    pub button: COLORREF,
    pub button_disabled: COLORREF,
    pub button_bg: COLORREF,
    pub button_bg_pressed: COLORREF,
    pub dialog_bg: COLORREF,
    pub field_bg: COLORREF,
    pub field_text: COLORREF,
}

const LIGHT: Palette = Palette {
    dark: false,
    bg: rgb(0xFF, 0xFF, 0xFF),
    border: rgb(0xDF, 0xE3, 0xEA),
    word: rgb(0x1B, 0x4D, 0xB1),
    text: rgb(0x1F, 0x23, 0x28),
    muted: rgb(0x63, 0x6B, 0x77),
    button: rgb(0x55, 0x5D, 0x69),
    button_disabled: rgb(0xB6, 0xBC, 0xC5),
    button_bg: rgb(0xF2, 0xF4, 0xF7),
    button_bg_pressed: rgb(0xDD, 0xE2, 0xEA),
    dialog_bg: rgb(0xF6, 0xF7, 0xF9),
    field_bg: rgb(0xFF, 0xFF, 0xFF),
    field_text: rgb(0x1F, 0x23, 0x28),
};

const DARK: Palette = Palette {
    dark: true,
    bg: rgb(0x1E, 0x1F, 0x22),
    border: rgb(0x35, 0x38, 0x3D),
    word: rgb(0x84, 0xB4, 0xFF),
    text: rgb(0xE7, 0xE8, 0xEA),
    muted: rgb(0x9E, 0xA4, 0xAC),
    button: rgb(0xC3, 0xC8, 0xCF),
    button_disabled: rgb(0x5B, 0x60, 0x68),
    button_bg: rgb(0x2A, 0x2C, 0x30),
    button_bg_pressed: rgb(0x3A, 0x3D, 0x42),
    dialog_bg: rgb(0x20, 0x21, 0x24),
    field_bg: rgb(0x2B, 0x2D, 0x31),
    field_text: rgb(0xE7, 0xE8, 0xEA),
};

/// Whether the resolved theme should be dark.
pub fn is_dark(mode: ThemeMode) -> bool {
    match mode {
        ThemeMode::Light => false,
        ThemeMode::Dark => true,
        ThemeMode::System => system_prefers_dark(),
    }
}

/// Windows "app mode" setting: `AppsUseLightTheme == 0` means dark.
pub fn system_prefers_dark() -> bool {
    const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
    crate::win::registry::read_dword(PERSONALIZE, "AppsUseLightTheme")
        .map(|value| value == 0)
        .unwrap_or(false)
}

pub fn palette(mode: ThemeMode) -> Palette {
    if is_dark(mode) { DARK } else { LIGHT }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_and_dark_palettes_are_distinct_and_opaque() {
        assert_ne!(
            palette(ThemeMode::Light).bg.0,
            palette(ThemeMode::Dark).bg.0
        );
        // Fully opaque colors: the high byte of every COLORREF stays zero.
        for value in [
            LIGHT.bg.0,
            LIGHT.text.0,
            DARK.bg.0,
            DARK.text.0,
            DARK.word.0,
        ] {
            assert_eq!(value >> 24, 0);
        }
    }

    #[test]
    fn explicit_modes_ignore_the_system_setting() {
        assert!(!is_dark(ThemeMode::Light));
        assert!(is_dark(ThemeMode::Dark));
    }

    #[test]
    fn rgb_packs_channels_in_bbggrr_order() {
        assert_eq!(rgb(0x12, 0x34, 0x56).0, 0x00_56_34_12);
    }
}

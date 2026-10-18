//! Settings dialog: rotation interval, order, active collection, theme, card
//! size, learned filtering, always-on-top and launch at login.
//!
//! The dialog works on a private copy of `Config`. Nothing is written until the
//! user presses OK or Apply, and Cancel really cancels.

use std::ffi::c_void;

use crate::app::App;
use crate::model::{
    Config, MAX_CARD_WIDTH_DIP, MAX_FONT_SIZE_PT, MAX_INTERVAL_MINUTES, MIN_CARD_WIDTH_DIP,
    MIN_FONT_SIZE_PT, MIN_INTERVAL_MINUTES, Order, ThemeMode,
};
use crate::ui::{self, Chrome, DipRect, id, rect};
use crate::win;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::HDC;
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetWindowLongPtrW,
    SetWindowLongPtrW, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_CTLCOLORBTN, WM_CTLCOLOREDIT,
    WM_CTLCOLORLISTBOX, WM_CTLCOLORSTATIC, WM_DPICHANGED, WM_ERASEBKGND, WM_NCCREATE, WM_NCDESTROY,
    WM_PAINT,
};

pub const CLASS: &str = "WordDockSettingsDialog";

const CLIENT: (i32, i32) = (420, 344);
const LABEL_X: i32 = 14;
const FIELD_X: i32 = 130;
const HINT_X: i32 = 202;
const ROW: i32 = 30;

/// Every control id, so the font can be re-applied after a DPI change.
const CONTROLS: [u16; 21] = [
    id::SET_INTERVAL_LABEL,
    id::SET_INTERVAL,
    id::SET_INTERVAL_HINT,
    id::SET_ORDER_LABEL,
    id::SET_ORDER,
    id::SET_COLLECTION_LABEL,
    id::SET_COLLECTION,
    id::SET_THEME_LABEL,
    id::SET_THEME,
    id::SET_FONT_LABEL,
    id::SET_FONT,
    id::SET_FONT_HINT,
    id::SET_WIDTH_LABEL,
    id::SET_WIDTH,
    id::SET_WIDTH_HINT,
    id::SET_TOPMOST,
    id::SET_LAUNCH,
    id::SET_EXCLUDE_LEARNED,
    id::SET_HINT,
    id::APPLY,
    id::CANCEL,
];

struct State {
    app: *mut App,
    chrome: Chrome,
    config: Config,
    /// `(id, name)` of every collection, in combo order.
    collections: Vec<(String, String)>,
    /// Collection that was active when the dialog opened.
    active: Option<String>,
}

pub fn class() -> ui::ClassSpec {
    ui::ClassSpec {
        name: CLASS,
        proc: Some(proc),
        cursor: ui::CURSOR_ARROW,
        background: ui::SYS_BTNFACE,
    }
}

pub fn open(app: &mut App) {
    if let Some(hwnd) = app.settings_hwnd {
        if win::is_window(hwnd) {
            win::activate_window(hwnd);
            return;
        }
        app.settings_hwnd = None;
    }
    let dpi = win::dpi_of_window(ui::reference_window(app)).max(96);
    let state = State {
        app: app as *mut App,
        chrome: Chrome::new(app.config.theme, dpi),
        config: app.config.clone(),
        collections: app
            .library
            .collections
            .iter()
            .map(|collection| (collection.id.clone(), collection.name.clone()))
            .collect(),
        active: app.library.active_collection.clone(),
    };
    let state = Box::new(state);
    let Some(hwnd) = ui::create_dialog_window(
        None,
        ui::reference_window(app),
        win::module_instance(),
        CLASS,
        "WordDock settings",
        CLIENT,
        Box::into_raw(state) as *mut c_void,
    ) else {
        return;
    };
    app.settings_hwnd = Some(hwnd);
}

fn state(hwnd: HWND) -> *mut State {
    // SAFETY: this is our own user-data slot, written in WM_NCCREATE and cleared
    // in WM_NCDESTROY; only the UI thread reads it.
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State }
}

fn place_controls(hwnd: HWND, dpi: u32) {
    ui::place(
        hwnd,
        id::SET_INTERVAL_LABEL,
        rect(LABEL_X, 16, 112, 16),
        dpi,
    );
    ui::place(hwnd, id::SET_INTERVAL, rect(FIELD_X, 12, 64, 24), dpi);
    ui::place(hwnd, id::SET_INTERVAL_HINT, rect(HINT_X, 16, 200, 16), dpi);
    ui::place(
        hwnd,
        id::SET_ORDER_LABEL,
        rect(LABEL_X, 16 + ROW, 112, 16),
        dpi,
    );
    ui::place(hwnd, id::SET_ORDER, rect(FIELD_X, 12 + ROW, 160, 200), dpi);
    ui::place(
        hwnd,
        id::SET_COLLECTION_LABEL,
        rect(LABEL_X, 16 + ROW * 2, 112, 16),
        dpi,
    );
    ui::place(
        hwnd,
        id::SET_COLLECTION,
        rect(FIELD_X, 12 + ROW * 2, 260, 200),
        dpi,
    );
    ui::place(
        hwnd,
        id::SET_THEME_LABEL,
        rect(LABEL_X, 16 + ROW * 3, 112, 16),
        dpi,
    );
    ui::place(
        hwnd,
        id::SET_THEME,
        rect(FIELD_X, 12 + ROW * 3, 160, 200),
        dpi,
    );
    ui::place(
        hwnd,
        id::SET_FONT_LABEL,
        rect(LABEL_X, 16 + ROW * 4, 112, 16),
        dpi,
    );
    ui::place(hwnd, id::SET_FONT, rect(FIELD_X, 12 + ROW * 4, 64, 24), dpi);
    ui::place(
        hwnd,
        id::SET_FONT_HINT,
        rect(HINT_X, 16 + ROW * 4, 200, 16),
        dpi,
    );
    ui::place(
        hwnd,
        id::SET_WIDTH_LABEL,
        rect(LABEL_X, 16 + ROW * 5, 112, 16),
        dpi,
    );
    ui::place(
        hwnd,
        id::SET_WIDTH,
        rect(FIELD_X, 12 + ROW * 5, 64, 24),
        dpi,
    );
    ui::place(
        hwnd,
        id::SET_WIDTH_HINT,
        rect(HINT_X, 16 + ROW * 5, 200, 16),
        dpi,
    );
    ui::place(
        hwnd,
        id::SET_TOPMOST,
        rect(LABEL_X, 16 + ROW * 6, 380, 20),
        dpi,
    );
    ui::place(
        hwnd,
        id::SET_LAUNCH,
        rect(LABEL_X, 16 + ROW * 7, 380, 20),
        dpi,
    );
    ui::place(
        hwnd,
        id::SET_EXCLUDE_LEARNED,
        rect(LABEL_X, 16 + ROW * 8, 380, 20),
        dpi,
    );
    ui::place(
        hwnd,
        id::SET_HINT,
        rect(LABEL_X, 16 + ROW * 9, 392, 32),
        dpi,
    );
    ui::place(hwnd, id::APPLY, rect(140, 310, 88, 26), dpi);
    ui::place(hwnd, id::CANCEL, rect(236, 310, 80, 26), dpi);
    ui::place(hwnd, id::OK, rect(320, 310, 86, 26), dpi);
}

fn create_controls(hwnd: HWND, state: &State) {
    let dpi = state.chrome.dpi;
    let font = state.chrome.font_handle();
    let static_ = |text: &str, id: u16, rect: DipRect| {
        ui::create_child(hwnd, "STATIC", text, ui::STYLE_STATIC, id, rect, dpi, font);
    };

    static_(
        "Rotate every",
        id::SET_INTERVAL_LABEL,
        rect(LABEL_X, 16, 112, 16),
    );
    ui::create_child(
        hwnd,
        "EDIT",
        "",
        ui::STYLE_EDIT,
        id::SET_INTERVAL,
        rect(FIELD_X, 12, 64, 24),
        dpi,
        font,
    );
    static_(
        &format!("minutes ({MIN_INTERVAL_MINUTES}–{MAX_INTERVAL_MINUTES})"),
        id::SET_INTERVAL_HINT,
        rect(HINT_X, 16, 200, 16),
    );

    static_(
        "Order",
        id::SET_ORDER_LABEL,
        rect(LABEL_X, 16 + ROW, 112, 16),
    );
    ui::create_child(
        hwnd,
        "COMBOBOX",
        "",
        ui::STYLE_COMBO,
        id::SET_ORDER,
        rect(FIELD_X, 12 + ROW, 160, 200),
        dpi,
        font,
    );

    static_(
        "Collection",
        id::SET_COLLECTION_LABEL,
        rect(LABEL_X, 16 + ROW * 2, 112, 16),
    );
    ui::create_child(
        hwnd,
        "COMBOBOX",
        "",
        ui::STYLE_COMBO,
        id::SET_COLLECTION,
        rect(FIELD_X, 12 + ROW * 2, 260, 200),
        dpi,
        font,
    );

    static_(
        "Theme",
        id::SET_THEME_LABEL,
        rect(LABEL_X, 16 + ROW * 3, 112, 16),
    );
    ui::create_child(
        hwnd,
        "COMBOBOX",
        "",
        ui::STYLE_COMBO,
        id::SET_THEME,
        rect(FIELD_X, 12 + ROW * 3, 160, 200),
        dpi,
        font,
    );

    static_(
        "Word font size",
        id::SET_FONT_LABEL,
        rect(LABEL_X, 16 + ROW * 4, 112, 16),
    );
    ui::create_child(
        hwnd,
        "EDIT",
        "",
        ui::STYLE_EDIT,
        id::SET_FONT,
        rect(FIELD_X, 12 + ROW * 4, 64, 24),
        dpi,
        font,
    );
    static_(
        &format!("points ({MIN_FONT_SIZE_PT}–{MAX_FONT_SIZE_PT})"),
        id::SET_FONT_HINT,
        rect(HINT_X, 16 + ROW * 4, 200, 16),
    );

    static_(
        "Card width",
        id::SET_WIDTH_LABEL,
        rect(LABEL_X, 16 + ROW * 5, 112, 16),
    );
    ui::create_child(
        hwnd,
        "EDIT",
        "",
        ui::STYLE_EDIT,
        id::SET_WIDTH,
        rect(FIELD_X, 12 + ROW * 5, 64, 24),
        dpi,
        font,
    );
    static_(
        &format!("px ({MIN_CARD_WIDTH_DIP}–{MAX_CARD_WIDTH_DIP})"),
        id::SET_WIDTH_HINT,
        rect(HINT_X, 16 + ROW * 5, 200, 16),
    );

    ui::create_child(
        hwnd,
        "BUTTON",
        "Keep the card above other windows",
        ui::STYLE_CHECKBOX,
        id::SET_TOPMOST,
        rect(LABEL_X, 16 + ROW * 6, 380, 20),
        dpi,
        font,
    );
    ui::create_child(
        hwnd,
        "BUTTON",
        "Start WordDock when I sign in",
        ui::STYLE_CHECKBOX,
        id::SET_LAUNCH,
        rect(LABEL_X, 16 + ROW * 7, 380, 20),
        dpi,
        font,
    );
    ui::create_child(
        hwnd,
        "BUTTON",
        "Skip entries marked as learned",
        ui::STYLE_CHECKBOX,
        id::SET_EXCLUDE_LEARNED,
        rect(LABEL_X, 16 + ROW * 8, 380, 20),
        dpi,
        font,
    );
    static_("", id::SET_HINT, rect(LABEL_X, 16 + ROW * 9, 392, 32));

    ui::create_child(
        hwnd,
        "BUTTON",
        "Apply",
        ui::STYLE_BUTTON,
        id::APPLY,
        rect(140, 310, 88, 26),
        dpi,
        font,
    );
    ui::create_child(
        hwnd,
        "BUTTON",
        "Cancel",
        ui::STYLE_BUTTON,
        id::CANCEL,
        rect(236, 310, 80, 26),
        dpi,
        font,
    );
    ui::create_child(
        hwnd,
        "BUTTON",
        "OK",
        ui::STYLE_BUTTON_DEFAULT,
        id::OK,
        rect(320, 310, 86, 26),
        dpi,
        font,
    );
}

/// Writes the stored configuration into the controls.
fn load_controls(hwnd: HWND, state: &State) {
    let config = &state.config;
    ui::set_edit_text(hwnd, id::SET_INTERVAL, &config.interval_minutes.to_string());
    ui::set_edit_text(hwnd, id::SET_FONT, &config.font_size_pt.to_string());
    ui::set_edit_text(hwnd, id::SET_WIDTH, &config.card_width_dip.to_string());

    if let Some(combo) = win::child(hwnd, id::SET_ORDER) {
        ui::combo_reset(combo);
        for order in [Order::Sequential, Order::Shuffled] {
            ui::combo_add(combo, order.label());
        }
        ui::combo_select(
            combo,
            Some(if config.order == Order::Sequential {
                0
            } else {
                1
            }),
        );
    }
    if let Some(combo) = win::child(hwnd, id::SET_COLLECTION) {
        ui::combo_reset(combo);
        for (_, name) in &state.collections {
            ui::combo_add(combo, name);
        }
        let selected = state
            .active
            .as_ref()
            .and_then(|id| state.collections.iter().position(|(other, _)| other == id))
            .or(if state.collections.is_empty() {
                None
            } else {
                Some(0)
            });
        ui::combo_select(combo, selected);
    }
    if let Some(combo) = win::child(hwnd, id::SET_THEME) {
        ui::combo_reset(combo);
        for theme in [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark] {
            ui::combo_add(combo, theme.label());
        }
        ui::combo_select(
            combo,
            Some(match config.theme {
                ThemeMode::System => 0,
                ThemeMode::Light => 1,
                ThemeMode::Dark => 2,
            }),
        );
    }

    win::set_checked(hwnd, id::SET_TOPMOST, config.always_on_top);
    win::set_checked(hwnd, id::SET_LAUNCH, config.launch_at_login);
    win::set_checked(hwnd, id::SET_EXCLUDE_LEARNED, config.exclude_learned);

    let mut hint = String::from("Settings are written to your WordDock folder when you press OK.");
    if let Some(app) = unsafe { app_of(state) } {
        hint.push('\n');
        hint.push_str(&format!("Data folder: {}", app.paths.root.display()));
    }
    ui::static_text(hwnd, id::SET_HINT, &hint);
}

pub extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        // SAFETY: CREATESTRUCTW is valid during WM_NCCREATE; lpCreateParams is the
        // Box<State> handed over by `open`.
        let state = unsafe { (*(lparam.0 as *const CREATESTRUCTW)).lpCreateParams as *mut State };
        if !state.is_null() {
            // SAFETY: storing our own state in the window's user-data slot.
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
            }
        }
        return LRESULT(1);
    }

    let state_ptr = state(hwnd);
    if state_ptr.is_null() {
        // SAFETY: default processing outside our window's lifetime.
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }

    match msg {
        WM_CREATE => {
            // SAFETY: the pointer was stored in WM_NCCREATE and is still valid.
            let state = unsafe { &*state_ptr };
            create_controls(hwnd, state);
            load_controls(hwnd, state);
            place_controls(hwnd, state.chrome.dpi);
            ui::focus(hwnd, id::SET_INTERVAL);
            ui::edit_select_all(hwnd, id::SET_INTERVAL);
            LRESULT(0)
        }
        WM_COMMAND => {
            let command = ui::command_id(wparam);
            if command == id::OK || command == id::APPLY {
                if apply(hwnd, state_ptr) && command == id::OK {
                    // SAFETY: destroying our own dialog; WM_NCDESTROY frees the state.
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                }
            } else if command == id::CANCEL {
                // SAFETY: as above.
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            // SAFETY: as above.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => {
            // SAFETY: `state_ptr` is valid for this message.
            let state = unsafe { &*state_ptr };
            let hdc = HDC(wparam.0 as *mut c_void);
            if state.chrome.erase(hwnd, hdc) {
                LRESULT(1)
            } else {
                // SAFETY: default handling paints the standard window colour.
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN => {
            // SAFETY: `state_ptr` is valid for this message.
            let state = unsafe { &*state_ptr };
            match state.chrome.ctl_color(msg, HDC(wparam.0 as *mut c_void)) {
                Some(result) => result,
                // SAFETY: default colours for this control type.
                None => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
            }
        }
        WM_PAINT => {
            // SAFETY: `state_ptr` is valid for this message.
            unsafe { &*state_ptr }.chrome.paint(hwnd);
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let dpi = (wparam.0 & 0xFFFF) as u32;
            // SAFETY: `state_ptr` is valid for this message.
            let state = unsafe { &mut *state_ptr };
            state.chrome.set_dpi(dpi);
            ui::apply_font_to_children(hwnd, &CONTROLS, state.chrome.font_handle());
            ui::place(hwnd, id::OK, rect(320, 310, 86, 26), dpi);
            place_controls(hwnd, dpi);
            resize_to_suggested(hwnd, dpi, lparam);
            LRESULT(0)
        }
        WM_NCDESTROY => {
            // SAFETY: release the box and the back-pointer exactly once.
            unsafe {
                let state = &mut *state_ptr;
                if let Some(app) = app_of_mut(state) {
                    app.settings_hwnd = None;
                }
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(state_ptr));
            }
            // SAFETY: default processing while the window goes away.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        _ => {
            // SAFETY: default processing for unhandled messages.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
    }
}

/// Reads the controls, validates them and applies the configuration.
/// Returns `true` when the values were accepted.
fn apply(hwnd: HWND, state_ptr: *mut State) -> bool {
    let interval = match read_number(hwnd, id::SET_INTERVAL, "Interval") {
        Ok(value) => value,
        Err(error) => return complain(hwnd, id::SET_INTERVAL, &error),
    };
    if !(MIN_INTERVAL_MINUTES..=MAX_INTERVAL_MINUTES).contains(&interval) {
        return complain(
            hwnd,
            id::SET_INTERVAL,
            &format!(
                "Interval must be between {MIN_INTERVAL_MINUTES} and {MAX_INTERVAL_MINUTES} minutes."
            ),
        );
    }
    let font = match read_number(hwnd, id::SET_FONT, "Font size") {
        Ok(value) => value,
        Err(error) => return complain(hwnd, id::SET_FONT, &error),
    };
    if !(MIN_FONT_SIZE_PT..=MAX_FONT_SIZE_PT).contains(&font) {
        return complain(
            hwnd,
            id::SET_FONT,
            &format!("Font size must be between {MIN_FONT_SIZE_PT} and {MAX_FONT_SIZE_PT} points."),
        );
    }
    let width = match read_number(hwnd, id::SET_WIDTH, "Card width") {
        Ok(value) => value,
        Err(error) => return complain(hwnd, id::SET_WIDTH, &error),
    };
    if !(MIN_CARD_WIDTH_DIP as u32..=MAX_CARD_WIDTH_DIP as u32).contains(&width) {
        return complain(
            hwnd,
            id::SET_WIDTH,
            &format!(
                "Card width must be between {MIN_CARD_WIDTH_DIP} and {MAX_CARD_WIDTH_DIP} pixels."
            ),
        );
    }

    // SAFETY: `state_ptr` is the live state of this dialog.
    let state = unsafe { &mut *state_ptr };
    let order = match win::child(hwnd, id::SET_ORDER).and_then(ui::combo_selection) {
        Some(1) => Order::Shuffled,
        _ => Order::Sequential,
    };
    let theme = match win::child(hwnd, id::SET_THEME).and_then(ui::combo_selection) {
        Some(1) => ThemeMode::Light,
        Some(2) => ThemeMode::Dark,
        _ => ThemeMode::System,
    };
    let active = win::child(hwnd, id::SET_COLLECTION)
        .and_then(ui::combo_selection)
        .and_then(|index| state.collections.get(index).map(|(id, _)| id.clone()));

    let mut config = state.config.clone();
    config.interval_minutes = interval;
    config.font_size_pt = font;
    config.card_width_dip = width as i32;
    config.order = order;
    config.theme = theme;
    config.always_on_top = win::is_checked(hwnd, id::SET_TOPMOST);
    config.launch_at_login = win::is_checked(hwnd, id::SET_LAUNCH);
    config.exclude_learned = win::is_checked(hwnd, id::SET_EXCLUDE_LEARNED);
    state.config = config.clone();

    // SAFETY: the state owns the only pointer to the application.
    let Some(app) = (unsafe { app_of_mut(state) }) else {
        return false;
    };
    app.apply_config(config, hwnd, active);
    true
}

/// Shows a validation error and returns `false` so the dialog stays open.
fn complain(hwnd: HWND, focus: u16, message: &str) -> bool {
    win::warn(hwnd, message, "WordDock settings");
    ui::focus(hwnd, focus);
    ui::edit_select_all(hwnd, focus);
    false
}

fn read_number(hwnd: HWND, id: u16, label: &str) -> Result<u32, String> {
    parse_number(&win::control_text(hwnd, id), label)
}

/// Pure helper so the validation rules can be unit tested.
fn parse_number(text: &str, label: &str) -> Result<u32, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(format!("{label} cannot be empty."));
    }
    text.parse::<u32>()
        .map_err(|_| format!("{label} must be a whole number (found “{text}”)."))
}

unsafe fn app_of(state: &State) -> Option<&'_ App> {
    // SAFETY: the caller guarantees the application outlives the dialog.
    unsafe { state.app.as_ref() }
}

unsafe fn app_of_mut(state: &mut State) -> Option<&'_ mut App> {
    // SAFETY: single-threaded UI, no other borrow is alive at the call sites.
    unsafe { state.app.as_mut() }
}

fn resize_to_suggested(hwnd: HWND, dpi: u32, lparam: LPARAM) {
    let suggested = lparam.0 as *const windows::Win32::Foundation::RECT;
    if suggested.is_null() {
        return;
    }
    // SAFETY: Windows passes a valid RECT with WM_DPICHANGED.
    let rect = unsafe { *suggested };
    win::set_window_rect_no_activate(
        hwnd,
        rect.left,
        rect.top,
        rect.right - rect.left,
        rect.bottom - rect.top,
    );
    let _ = dpi;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_fields_reject_empty_and_text_values() {
        assert!(parse_number("  ", "Interval").is_err());
        assert!(parse_number("soon", "Interval").is_err());
        assert!(parse_number("42", "Interval").is_ok());
        assert_eq!(parse_number(" 15 ", "Interval").unwrap(), 15);
    }
}

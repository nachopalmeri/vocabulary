//! Native-control UI: the compact GDI card plus settings/collections/details
//! dialogs built from standard Windows controls.
//!
//! Re-entrancy policy (see also `app.rs`): windows never hold a Rust reference to
//! `App` across a call that can pump messages (message boxes, file dialogs, popup
//! menus). Those calls are made at the outermost level of a handler.

pub mod card;
pub mod collections;
pub mod details;
pub mod import;
pub mod settings;

use crate::model::{ThemeMode, UI_FONT_FAMILY};
use crate::theme::{self, Palette};
use crate::win::{self, Brush, Font};
use std::ffi::c_void;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{CreateSolidBrush, FillRect, HBRUSH, HDC, HFONT};
use windows::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BS_AUTOCHECKBOX, BS_DEFPUSHBUTTON, BS_LEFT, CB_ADDSTRING, CB_GETCURSEL,
    CB_SETCURSEL, CBS_DROPDOWNLIST, CreateWindowExW, ES_AUTOHSCROLL, ES_AUTOVSCROLL, ES_MULTILINE,
    ES_READONLY, ES_WANTRETURN, GA_ROOT, GetAncestor, GetClientRect, HMENU, IsDialogMessageW,
    LB_ADDSTRING, LB_DELETESTRING, LB_INITSTORAGE, LB_RESETCONTENT, LB_SETCURSEL,
    LBS_NOINTEGRALHEIGHT, LBS_NOTIFY, MSG, RegisterClassExW, SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER,
    SendMessageW, SetForegroundWindow, SetWindowPos, ShowWindow, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_CTLCOLORBTN, WM_CTLCOLOREDIT, WM_CTLCOLORLISTBOX, WM_CTLCOLORSTATIC, WM_SETFONT,
    WNDCLASS_STYLES, WNDCLASSEXW, WNDPROC, WS_BORDER, WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN,
    WS_EX_CONTROLPARENT, WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};
use windows::core::PCWSTR;

pub const CURSOR_ARROW: PCWSTR = windows::Win32::UI::WindowsAndMessaging::IDC_ARROW;

/// A rectangle in device-independent pixels (96 DPI baseline).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DipRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

pub const fn rect(x: i32, y: i32, w: i32, h: i32) -> DipRect {
    DipRect { x, y, w, h }
}

pub fn px(value: i32, dpi: u32) -> i32 {
    win::dip(value, dpi)
}

/// Common control ids used by the dialogs.
pub mod id {
    pub const OK: u16 = 1;
    pub const CANCEL: u16 = 2;
    pub const CLOSE: u16 = 3;
    pub const APPLY: u16 = 4;

    // Settings
    pub const SET_INTERVAL: u16 = 10;
    pub const SET_ORDER: u16 = 11;
    pub const SET_COLLECTION: u16 = 12;
    pub const SET_THEME: u16 = 13;
    pub const SET_FONT: u16 = 14;
    pub const SET_WIDTH: u16 = 15;
    pub const SET_TOPMOST: u16 = 16;
    pub const SET_LAUNCH: u16 = 17;
    pub const SET_EXCLUDE_LEARNED: u16 = 18;
    pub const SET_INTERVAL_LABEL: u16 = 19;
    pub const SET_ORDER_LABEL: u16 = 20;
    pub const SET_COLLECTION_LABEL: u16 = 21;
    pub const SET_THEME_LABEL: u16 = 22;
    pub const SET_FONT_LABEL: u16 = 23;
    pub const SET_WIDTH_LABEL: u16 = 24;
    pub const SET_HINT: u16 = 25;
    pub const SET_INTERVAL_HINT: u16 = 26;
    pub const SET_FONT_HINT: u16 = 27;
    pub const SET_WIDTH_HINT: u16 = 28;

    // Collections manager
    pub const COL_LIST: u16 = 40;
    pub const COL_NEW: u16 = 41;
    pub const COL_RENAME: u16 = 42;
    pub const COL_DELETE: u16 = 43;
    pub const COL_ACTIVE: u16 = 44;
    pub const COL_HEADING: u16 = 45;
    pub const ENTRY_LIST: u16 = 46;
    pub const ENTRY_ADD: u16 = 47;
    pub const ENTRY_EDIT: u16 = 48;
    pub const ENTRY_DELETE: u16 = 49;
    pub const ENTRY_IMPORT: u16 = 50;
    pub const ENTRY_EXPORT_CSV: u16 = 51;
    pub const ENTRY_EXPORT_JSON: u16 = 52;
    pub const ENTRY_HEADING: u16 = 53;
    pub const ENTRY_SUMMARY: u16 = 54;
    pub const COL_ACTIVE_LABEL: u16 = 55;

    // Entry editor
    pub const FIELD_WORD: u16 = 70;
    pub const FIELD_MEANING: u16 = 71;
    pub const FIELD_EXAMPLE: u16 = 72;
    pub const FIELD_SOURCE: u16 = 73;
    pub const FIELD_SOURCE_LANG: u16 = 74;
    pub const FIELD_TARGET_LANG: u16 = 75;
    pub const FIELD_LEARNED: u16 = 76;
    pub const FIELD_WORD_LABEL: u16 = 77;
    pub const FIELD_MEANING_LABEL: u16 = 78;
    pub const FIELD_EXAMPLE_LABEL: u16 = 79;
    pub const FIELD_SOURCE_LABEL: u16 = 80;
    pub const FIELD_SOURCE_LANG_LABEL: u16 = 81;
    pub const FIELD_TARGET_LANG_LABEL: u16 = 82;
    pub const FIELD_HINT: u16 = 83;

    // Prompt (new/rename collection)
    pub const PROMPT_VALUE: u16 = 90;
    pub const PROMPT_LABEL: u16 = 91;

    // Import preview
    pub const IMPORT_SUMMARY: u16 = 100;
    pub const IMPORT_LIST: u16 = 101;
    pub const IMPORT_NOTES: u16 = 102;
    pub const IMPORT_APPLY: u16 = 103;
    pub const IMPORT_SKIP_DUPES: u16 = 104;

    // Details
    pub const DETAIL_BODY: u16 = 110;
    pub const DETAIL_META: u16 = 111;
    pub const DETAIL_TOGGLE: u16 = 112;
}

/// Paints the window background with the palette brush.
pub struct Chrome {
    pub mode: ThemeMode,
    pub dark: bool,
    pub palette: Palette,
    pub dpi: u32,
    font: Option<Font>,
    bg_brush: Option<Brush>,
    field_brush: Option<Brush>,
}

impl Chrome {
    pub fn new(mode: ThemeMode, dpi: u32) -> Chrome {
        let palette = theme::palette(mode);
        let font = win::create_font(UI_FONT_FAMILY, 9.0, dpi, false);
        let bg_brush = if palette.dark {
            let brush = unsafe {
                // SAFETY: creating an owned solid brush; released by the wrapper.
                CreateSolidBrush(palette.dialog_bg)
            };
            let brush = Brush(brush);
            (!brush.is_null()).then_some(brush)
        } else {
            None
        };
        let field_brush = if palette.dark {
            let brush = unsafe {
                // SAFETY: as above.
                CreateSolidBrush(palette.field_bg)
            };
            let brush = Brush(brush);
            (!brush.is_null()).then_some(brush)
        } else {
            None
        };
        Chrome {
            mode,
            dark: palette.dark,
            palette,
            dpi,
            font,
            bg_brush,
            field_brush,
        }
    }

    pub fn font_handle(&self) -> Option<HFONT> {
        self.font.as_ref().map(|font| font.raw())
    }

    /// Recreates the font for a new DPI (used on `WM_DPICHANGED`).
    pub fn set_dpi(&mut self, dpi: u32) {
        if self.dpi != dpi {
            self.dpi = dpi;
            self.font = win::create_font(UI_FONT_FAMILY, 9.0, dpi, false);
        }
    }

    /// Fills the window background; returns the value for `WM_ERASEBKGND`.
    pub fn erase(&self, hwnd: HWND, hdc: HDC) -> bool {
        if !self.dark {
            return false; // default handling paints the standard window colour
        }
        let Some(brush) = &self.bg_brush else {
            return false;
        };
        let mut client = RECT::default();
        // SAFETY: filling a RECT of an existing window.
        unsafe {
            let _ = GetClientRect(hwnd, &mut client);
            FillRect(hdc, &client, brush.raw());
        }
        true
    }

    /// Paints the dialog background inside a `WM_PAINT` handler.
    pub fn paint(&self, hwnd: HWND) {
        use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
        let mut paint = PAINTSTRUCT::default();
        // SAFETY: BeginPaint/EndPaint bracket the drawing of an existing window.
        unsafe {
            let hdc = BeginPaint(hwnd, &mut paint);
            if self.dark {
                if let Some(brush) = &self.bg_brush {
                    FillRect(hdc, &paint.rcPaint, brush.raw());
                }
            }
            let _ = EndPaint(hwnd, &paint);
        }
    }

    /// Answers the `WM_CTLCOLOR*` family for dark mode. Returns `None` to fall
    /// back to the standard colours.
    pub fn ctl_color(&self, msg: u32, hdc: HDC) -> Option<LRESULT> {
        if !self.dark {
            return None;
        }
        let (use_field, text_color) = match msg {
            WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => (true, self.palette.field_text),
            WM_CTLCOLORSTATIC => (false, self.palette.text),
            WM_CTLCOLORBTN => return None, // leave push buttons to the system theme
            _ => return None,
        };
        let brush = if use_field {
            self.field_brush.as_ref()
        } else {
            self.bg_brush.as_ref()
        }?;
        // SAFETY: both calls only change DC state.
        unsafe {
            windows::Win32::Graphics::Gdi::SetBkMode(
                hdc,
                windows::Win32::Graphics::Gdi::TRANSPARENT,
            );
            windows::Win32::Graphics::Gdi::SetTextColor(hdc, text_color);
        }
        Some(LRESULT(brush.raw().0 as isize))
    }
}

/// Creates a child control with a DIP rectangle and the shared UI font.
pub fn create_child(
    parent: HWND,
    class: &str,
    text: &str,
    style: u32,
    id: u16,
    rect: DipRect,
    dpi: u32,
    font: Option<HFONT>,
) -> Option<HWND> {
    let class = win::wide(class);
    let text = win::wide(text);
    // SAFETY: creating a standard control as a child of an existing window; the
    // id is passed through the HMENU parameter as Windows expects.
    let hwnd = unsafe {
        CreateWindowExW(
            windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE(0),
            win::pcw(&class),
            win::pcw(&text),
            windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(style),
            px(rect.x, dpi),
            px(rect.y, dpi),
            px(rect.w, dpi),
            px(rect.h, dpi),
            Some(parent),
            Some(HMENU(id as usize as *mut core::ffi::c_void)),
            None,
            None,
        )
    }
    .ok()?;
    if let Some(font) = font {
        set_font(hwnd, font);
    }
    Some(hwnd)
}

pub fn set_font(hwnd: HWND, font: HFONT) {
    // SAFETY: WM_SETFONT with a font owned by the dialog's Chrome value.
    unsafe {
        SendMessageW(
            hwnd,
            WM_SETFONT,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(1)),
        );
    }
}

/// Moves an existing child control to a (possibly rescaled) DIP rectangle.
pub fn place(parent: HWND, id: u16, rect: DipRect, dpi: u32) {
    if let Some(hwnd) = win::child(parent, id) {
        // SAFETY: repositioning an existing child window.
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                None,
                px(rect.x, dpi),
                px(rect.y, dpi),
                px(rect.w, dpi),
                px(rect.h, dpi),
                SWP_NOACTIVATE | SWP_NOZORDER,
            );
        }
    }
}

pub fn focus(parent: HWND, id: u16) {
    if let Some(hwnd) = win::child(parent, id) {
        // SAFETY: setting focus inside the active window.
        unsafe {
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(hwnd));
        }
    }
}

// --- List box / combo box helpers -------------------------------------------

pub fn list_reset(list: HWND) {
    win::send_message(list, LB_RESETCONTENT, WPARAM(0), LPARAM(0));
}

pub fn list_reserve(list: HWND, count: usize) {
    win::send_message(
        list,
        LB_INITSTORAGE,
        WPARAM(count),
        LPARAM(count.saturating_mul(40) as isize),
    );
}

pub fn list_add(list: HWND, text: &str) {
    let text = win::wide(text);
    // SAFETY: LB_ADDSTRING copies the string into the list box.
    unsafe {
        SendMessageW(
            list,
            LB_ADDSTRING,
            None,
            Some(LPARAM(text.as_ptr() as isize)),
        );
    }
}

pub fn list_selection(list: HWND) -> Option<usize> {
    let index = win::send_message(
        list,
        windows::Win32::UI::WindowsAndMessaging::LB_GETCURSEL,
        WPARAM(0),
        LPARAM(0),
    )
    .0;
    if index < 0 {
        None
    } else {
        Some(index as usize)
    }
}

pub fn list_select(list: HWND, index: Option<usize>) {
    let index = index.map(|i| i as isize).unwrap_or(-1);
    win::send_message(list, LB_SETCURSEL, WPARAM(index as usize), LPARAM(0));
}

pub fn combo_reset(combo: HWND) {
    win::send_message(
        combo,
        windows::Win32::UI::WindowsAndMessaging::CB_RESETCONTENT,
        WPARAM(0),
        LPARAM(0),
    );
}

pub fn combo_add(combo: HWND, text: &str) {
    let text = win::wide(text);
    // SAFETY: CB_ADDSTRING copies the string into the combo box.
    unsafe {
        SendMessageW(
            combo,
            CB_ADDSTRING,
            None,
            Some(LPARAM(text.as_ptr() as isize)),
        );
    }
}

pub fn combo_selection(combo: HWND) -> Option<usize> {
    let index = win::send_message(combo, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0;
    if index < 0 {
        None
    } else {
        Some(index as usize)
    }
}

pub fn combo_select(combo: HWND, index: Option<usize>) {
    let index = index.map(|i| i as isize).unwrap_or(-1);
    win::send_message(combo, CB_SETCURSEL, WPARAM(index as usize), LPARAM(0));
}

pub fn checkbox_checked(parent: HWND, id: u16) -> bool {
    win::child(parent, id)
        .map(|hwnd| win::send_message(hwnd, BM_GETCHECK, WPARAM(0), LPARAM(0)).0 == 1)
        .unwrap_or(false)
}

/// Extracts the control id from a `WM_COMMAND` wparam.
pub fn command_id(wparam: WPARAM) -> u16 {
    (wparam.0 & 0xFFFF) as u16
}

pub fn command_notification(wparam: WPARAM) -> u16 {
    ((wparam.0 >> 16) & 0xFFFF) as u16
}

/// Client size of a dialog window for a DIP size (frame included).
pub fn frame_size(client: (i32, i32), dpi: u32) -> (i32, i32) {
    win::window_rect_for_client_styled(
        (px(client.0, dpi), px(client.1, dpi)),
        dpi,
        WINDOW_STYLE(DIALOG_STYLE),
        WINDOW_EX_STYLE(DIALOG_EX_STYLE),
    )
}

// ---------------------------------------------------------------------------
// Dialog windows
// ---------------------------------------------------------------------------

/// Style shared by every settings/management dialog: fixed size, no maximise.
pub const DIALOG_STYLE: u32 = WS_OVERLAPPED.0 | WS_CAPTION.0 | WS_SYSMENU.0 | WS_CLIPCHILDREN.0;
pub const DIALOG_EX_STYLE: u32 = WS_EX_CONTROLPARENT.0;

/// Push-button and edit styles that `windows` does not export as named
/// constants (they are zero in `winuser.h`).
pub const BS_PUSHBUTTON: u32 = 0;
/// `EM_SETSEL` (not exported by `windows` 0.62).
pub const EM_SETSEL: u32 = 0x00B1;

/// System-colour brushes used as window-class backgrounds.
pub const SYS_BTNFACE: HBRUSH =
    HBRUSH((windows::Win32::Graphics::Gdi::COLOR_BTNFACE.0 + 1) as *mut c_void);

/// Control styles shared by the dialogs (all include `WS_VISIBLE`).
pub const STYLE_STATIC: u32 = WS_CHILD.0 | WS_VISIBLE.0;
pub const STYLE_EDIT: u32 =
    WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | WS_BORDER.0 | ES_AUTOHSCROLL as u32;
pub const STYLE_EDIT_MULTI: u32 = WS_CHILD.0
    | WS_VISIBLE.0
    | WS_TABSTOP.0
    | WS_BORDER.0
    | WS_VSCROLL.0
    | ES_MULTILINE as u32
    | ES_AUTOVSCROLL as u32
    | ES_WANTRETURN as u32;
pub const STYLE_EDIT_READONLY: u32 = STYLE_EDIT_MULTI | ES_READONLY as u32;
pub const STYLE_BUTTON: u32 = WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_PUSHBUTTON;
pub const STYLE_BUTTON_DEFAULT: u32 = STYLE_BUTTON | BS_DEFPUSHBUTTON as u32;
pub const STYLE_CHECKBOX: u32 =
    WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_AUTOCHECKBOX as u32 | BS_LEFT as u32;
pub const STYLE_LISTBOX: u32 = WS_CHILD.0
    | WS_VISIBLE.0
    | WS_TABSTOP.0
    | WS_BORDER.0
    | WS_VSCROLL.0
    | LBS_NOTIFY as u32
    | LBS_NOINTEGRALHEIGHT as u32;
pub const STYLE_COMBO: u32 =
    WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | WS_VSCROLL.0 | CBS_DROPDOWNLIST as u32;

/// Everything needed to register one window class.
pub struct ClassSpec {
    pub name: &'static str,
    pub proc: WNDPROC,
    pub cursor: PCWSTR,
    pub background: HBRUSH,
}

/// Registers a window class; a class that already exists counts as success.
pub fn register_class(spec: &ClassSpec, instance: HINSTANCE) -> bool {
    let name = win::wide(spec.name);
    let class = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: WNDCLASS_STYLES(0),
        lpfnWndProc: spec.proc,
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: Default::default(),
        hCursor: win::load_cursor(spec.cursor),
        hbrBackground: spec.background,
        lpszMenuName: PCWSTR::null(),
        lpszClassName: win::pcw(&name),
        hIconSm: Default::default(),
    };
    // SAFETY: `class` points at a live, NUL-terminated class name for the call.
    let atom = unsafe { RegisterClassExW(&class) };
    atom != 0 || win::last_error() == windows::Win32::Foundation::ERROR_CLASS_ALREADY_EXISTS.0
}

/// Creates, shows and centres a dialog window.
///
/// `owner` only affects z-order (pass `None` for an independent window);
/// `reference` decides which monitor the dialog is centred on.
pub fn create_dialog_window(
    owner: Option<HWND>,
    reference: HWND,
    instance: HINSTANCE,
    class: &str,
    title: &str,
    client_size: (i32, i32),
    state: *mut c_void,
) -> Option<HWND> {
    let dpi = win::dpi_of_window(reference).max(96);
    let (width, height) = frame_size(client_size, dpi);
    let work = win::monitor_for_window(reference)
        .map(|monitor| monitor.work)
        .unwrap_or_else(win::primary_work_area);
    let x = work.left + ((work.right - work.left - width) / 2).max(0);
    let y = work.top + ((work.bottom - work.top - height) / 2).max(0);

    let class = win::wide(class);
    let title = win::wide(title);
    // SAFETY: the class is registered by the caller before this point, the
    // buffers outlive the call and `state` is a heap pointer owned by the window
    // procedure (freed in WM_NCDESTROY).
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(DIALOG_EX_STYLE),
            win::pcw(&class),
            win::pcw(&title),
            WINDOW_STYLE(DIALOG_STYLE),
            x,
            y,
            width,
            height,
            owner,
            None,
            Some(instance),
            Some(state),
        )
    }
    .ok()?;
    // SAFETY: showing and focusing a window we just created.
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
    Some(hwnd)
}

/// Window a dialog should be centred on: the card when it is visible.
pub fn reference_window(app: &crate::app::App) -> HWND {
    match app.card.hwnd {
        Some(hwnd) if app.card_visible() => hwnd,
        _ => app.controller,
    }
}

/// Routes keyboard messages (Tab, Enter, Escape) to our dialog windows.
/// Returns `true` when the message was consumed.
pub fn is_dialog_message(message: &mut MSG) -> bool {
    if message.hwnd.0.is_null() {
        return false;
    }
    // SAFETY: pure query for an existing window.
    let root = unsafe { GetAncestor(message.hwnd, GA_ROOT) };
    if root.0.is_null() {
        return false;
    }
    let class = win::window_class_name(root);
    if !class.starts_with("WordDock")
        || class == crate::ui::card::CARD_CLASS
        || class == crate::app::CONTROLLER_CLASS
    {
        return false;
    }
    // SAFETY: `root` is one of our dialogs and `message` is the message being
    // dispatched; IsDialogMessageW only touches that message and `root`.
    unsafe { IsDialogMessageW(root, message).as_bool() }
}

/// Re-sends the dialog font to the given child controls (after a DPI change).
pub fn apply_font_to_children(parent: HWND, ids: &[u16], font: Option<HFONT>) {
    let Some(font) = font else {
        return;
    };
    for id in ids {
        if let Some(hwnd) = win::child(parent, *id) {
            set_font(hwnd, font);
        }
    }
}

// --- Small control helpers ------------------------------------------------

pub fn set_enabled(parent: HWND, id: u16, enabled: bool) {
    if let Some(hwnd) = win::child(parent, id) {
        win::enable_window(hwnd, enabled);
    }
}

pub fn static_text(parent: HWND, id: u16, text: &str) {
    if let Some(hwnd) = win::child(parent, id) {
        win::set_window_text(hwnd, text);
    }
}

pub fn edit_text(parent: HWND, id: u16) -> String {
    win::control_text(parent, id)
}

pub fn set_edit_text(parent: HWND, id: u16, text: &str) {
    static_text(parent, id, text);
}

/// Replaces a multiline edit's content (used by the details window).
pub fn set_multiline_text(parent: HWND, id: u16, text: &str) {
    static_text(parent, id, text);
}

/// Selects the whole content of an edit control.
pub fn edit_select_all(parent: HWND, id: u16) {
    if let Some(hwnd) = win::child(parent, id) {
        win::send_message(hwnd, EM_SETSEL, WPARAM(0), LPARAM(-1));
    }
}

pub fn list_delete(list: HWND, index: usize) {
    win::send_message(list, LB_DELETESTRING, WPARAM(index), LPARAM(0));
}

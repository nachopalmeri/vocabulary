//! Details window for the current word; offers a direct learned/review toggle.

use std::ffi::c_void;

use crate::app::App;
use crate::ui::{self, Chrome, id, rect};
use crate::win;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::HDC;
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetWindowLongPtrW,
    SetWindowLongPtrW, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_CTLCOLORBTN, WM_CTLCOLORSTATIC,
    WM_DPICHANGED, WM_ERASEBKGND, WM_NCCREATE, WM_NCDESTROY, WM_PAINT,
};

pub const CLASS: &str = "WordDockDetailsDialog";
const SIZE: (i32, i32) = (460, 310);
const TEXT: u16 = 130;

struct State {
    app: *mut App,
    chrome: Chrome,
    entry_id: Option<String>,
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
    if let Some(hwnd) = app.details_hwnd {
        if win::is_window(hwnd) {
            win::activate_window(hwnd);
            refresh(app, hwnd);
            return;
        }
        app.details_hwnd = None;
    }
    let dpi = win::dpi_of_window(ui::reference_window(app)).max(96);
    let state = Box::new(State {
        app: app as *mut App,
        chrome: Chrome::new(app.config.theme, dpi),
        entry_id: app.current_entry().map(|e| e.id.clone()),
    });
    let pointer = Box::into_raw(state);
    if let Some(hwnd) = ui::create_dialog_window(
        None,
        ui::reference_window(app),
        win::module_instance(),
        CLASS,
        "Word details",
        SIZE,
        pointer as *mut c_void,
    ) {
        app.details_hwnd = Some(hwnd);
    } else {
        unsafe {
            drop(Box::from_raw(pointer));
        }
    }
}

pub fn refresh(app: &mut App, hwnd: HWND) {
    let p = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State };
    if p.is_null() {
        return;
    }
    unsafe {
        (*p).entry_id = app.current_entry().map(|e| e.id.clone());
    }
    let body = if let Some(entry) = app.current_entry() {
        format!(
            "{}\r\n\r\n{}\r\n\r\nExample: {}\r\nSource: {}\r\n{} → {}\r\nStatus: {}",
            entry.word,
            entry.meaning,
            if entry.example.is_empty() {
                "—"
            } else {
                &entry.example
            },
            if entry.source.is_empty() {
                "—"
            } else {
                &entry.source
            },
            if entry.source_language.is_empty() {
                "English"
            } else {
                &entry.source_language
            },
            if entry.target_language.is_empty() {
                "Spanish"
            } else {
                &entry.target_language
            },
            if entry.learned {
                "learned"
            } else {
                "not learned"
            }
        )
    } else {
        "There is no current entry. Choose a collection or add a word.".to_string()
    };
    ui::set_multiline_text(hwnd, TEXT, &body);
    if let Some(entry) = app.current_entry() {
        ui::static_text(
            hwnd,
            id::DETAIL_TOGGLE,
            if entry.learned {
                "Review again"
            } else {
                "Mark learned"
            },
        );
        ui::set_enabled(hwnd, id::DETAIL_TOGGLE, true);
    } else {
        ui::set_enabled(hwnd, id::DETAIL_TOGGLE, false);
    }
}

fn make_controls(hwnd: HWND, state: &State) {
    let dpi = state.chrome.dpi;
    let font = state.chrome.font_handle();
    ui::create_child(
        hwnd,
        "EDIT",
        "",
        ui::STYLE_EDIT_READONLY,
        TEXT,
        rect(14, 14, 432, 224),
        dpi,
        font,
    );
    ui::create_child(
        hwnd,
        "BUTTON",
        "Mark learned",
        ui::STYLE_BUTTON,
        id::DETAIL_TOGGLE,
        rect(14, 252, 132, 28),
        dpi,
        font,
    );
    ui::create_child(
        hwnd,
        "BUTTON",
        "Close",
        ui::STYLE_BUTTON_DEFAULT,
        id::CLOSE,
        rect(360, 252, 86, 28),
        dpi,
        font,
    );
}

pub extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let p = unsafe { (*(lparam.0 as *const CREATESTRUCTW)).lpCreateParams as *mut State };
        if !p.is_null() {
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, p as isize);
            }
        }
        return LRESULT(1);
    }
    let p = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State };
    if p.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    match msg {
        WM_CREATE => {
            make_controls(hwnd, unsafe { &*p });
            if let Some(app) = unsafe { (*p).app.as_mut() } {
                refresh(app, hwnd);
            }
            LRESULT(0)
        }
        WM_COMMAND if ui::command_id(wparam) == id::DETAIL_TOGGLE => {
            if let Some(app) = unsafe { (*p).app.as_mut() } {
                if app.current_entry().is_some_and(|e| e.learned) {
                    app.review_again();
                } else {
                    app.mark_learned();
                }
                refresh(app, hwnd);
            }
            LRESULT(0)
        }
        WM_COMMAND if ui::command_id(wparam) == id::CLOSE => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => {
            let s = unsafe { &*p };
            if s.chrome.erase(hwnd, HDC(wparam.0 as *mut c_void)) {
                LRESULT(1)
            } else {
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
            let s = unsafe { &*p };
            s.chrome
                .ctl_color(msg, HDC(wparam.0 as *mut c_void))
                .unwrap_or_else(|| unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) })
        }
        WM_PAINT => {
            unsafe { &*p }.chrome.paint(hwnd);
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let dpi = (wparam.0 & 0xffff) as u32;
            unsafe { &mut *p }.chrome.set_dpi(dpi);
            LRESULT(0)
        }
        WM_NCDESTROY => unsafe {
            if let Some(app) = (*p).app.as_mut() {
                app.details_hwnd = None;
            }
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(Box::from_raw(p));
            DefWindowProcW(hwnd, msg, wparam, lparam)
        },
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

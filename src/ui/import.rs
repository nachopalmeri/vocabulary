//! Review screen for validated CSV/JSON import rows.

use std::ffi::c_void;

use crate::app::App;
use crate::csvio::{ImportReport, PREVIEW_ROW_LIMIT};
use crate::ui::{self, Chrome, id, rect};
use crate::win;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::HDC;
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetWindowLongPtrW,
    SetWindowLongPtrW, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_CTLCOLORBTN, WM_CTLCOLORLISTBOX,
    WM_CTLCOLORSTATIC, WM_DPICHANGED, WM_ERASEBKGND, WM_NCCREATE, WM_NCDESTROY, WM_PAINT,
};

pub const CLASS: &str = "WordDockImportDialog";
const SIZE: (i32, i32) = (720, 500);

struct State {
    app: *mut App,
    chrome: Chrome,
    report: ImportReport,
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
    if let Some(hwnd) = app.import_hwnd {
        if win::is_window(hwnd) {
            win::activate_window(hwnd);
            return;
        }
        app.import_hwnd = None;
    }
    let Some(report) = app.pending_import.take() else {
        return;
    };
    let dpi = win::dpi_of_window(ui::reference_window(app)).max(96);
    let state = Box::new(State {
        app: app as *mut App,
        chrome: Chrome::new(app.config.theme, dpi),
        report,
    });
    let pointer = Box::into_raw(state);
    if let Some(hwnd) = ui::create_dialog_window(
        None,
        ui::reference_window(app),
        win::module_instance(),
        CLASS,
        "Review vocabulary import",
        SIZE,
        pointer as *mut c_void,
    ) {
        app.import_hwnd = Some(hwnd);
    } else {
        let state = unsafe { Box::from_raw(pointer) };
        app.pending_import = Some(state.report);
    }
}

fn controls(hwnd: HWND, state: &State) {
    let dpi = state.chrome.dpi;
    let font = state.chrome.font_handle();
    let _ = ui::create_child(
        hwnd,
        "STATIC",
        "",
        ui::STYLE_STATIC,
        id::IMPORT_SUMMARY,
        rect(14, 12, 690, 22),
        dpi,
        font,
    );
    let _ = ui::create_child(
        hwnd,
        "LISTBOX",
        "",
        ui::STYLE_LISTBOX,
        id::IMPORT_LIST,
        rect(14, 40, 690, 332),
        dpi,
        font,
    );
    let _ = ui::create_child(
        hwnd,
        "BUTTON",
        "Skip duplicates (recommended)",
        ui::STYLE_CHECKBOX,
        id::IMPORT_SKIP_DUPES,
        rect(14, 382, 260, 22),
        dpi,
        font,
    );
    let _ = ui::create_child(
        hwnd,
        "STATIC",
        "",
        ui::STYLE_STATIC,
        id::IMPORT_NOTES,
        rect(14, 410, 480, 38),
        dpi,
        font,
    );
    let _ = ui::create_child(
        hwnd,
        "BUTTON",
        "Cancel",
        ui::STYLE_BUTTON,
        id::CANCEL,
        rect(516, 454, 86, 28),
        dpi,
        font,
    );
    let _ = ui::create_child(
        hwnd,
        "BUTTON",
        "Import",
        ui::STYLE_BUTTON_DEFAULT,
        id::IMPORT_APPLY,
        rect(612, 454, 92, 28),
        dpi,
        font,
    );
}

fn fill(hwnd: HWND, state: &State) {
    ui::static_text(
        hwnd,
        id::IMPORT_SUMMARY,
        &format!(
            "{} {} · {}",
            state.report.format.label(),
            state.report.path.display(),
            state.report.summary()
        ),
    );
    if let Some(list) = win::child(hwnd, id::IMPORT_LIST) {
        ui::list_reset(list);
        for row in state.report.rows.iter().take(PREVIEW_ROW_LIMIT) {
            ui::list_add(list, &format!("Row {} {}", row.row, row.preview_line()));
        }
        if state.report.rows.len() > PREVIEW_ROW_LIMIT {
            ui::list_add(
                list,
                &format!(
                    "… {} more rows not shown",
                    state.report.rows.len() - PREVIEW_ROW_LIMIT
                ),
            );
        }
    }
    win::set_checked(hwnd, id::IMPORT_SKIP_DUPES, true);
    let notes = if state.report.notes.is_empty() {
        "Invalid rows are never imported.".to_string()
    } else {
        format!(
            "{}\r\nInvalid rows are never imported.",
            state.report.notes.join("\r\n")
        )
    };
    ui::static_text(hwnd, id::IMPORT_NOTES, &notes);
    ui::set_enabled(
        hwnd,
        id::IMPORT_APPLY,
        state.report.ready_count() > 0 || state.report.duplicate_count() > 0,
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
            let s = unsafe { &*p };
            controls(hwnd, s);
            fill(hwnd, s);
            LRESULT(0)
        }
        WM_COMMAND if ui::command_id(wparam) == id::CANCEL => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_COMMAND if ui::command_id(wparam) == id::IMPORT_APPLY => {
            let include_dupes = !win::is_checked(hwnd, id::IMPORT_SKIP_DUPES);
            let app_ptr = unsafe { (*p).app };
            let report = unsafe { (*p).report.clone() };
            if let Some(app) = unsafe { app_ptr.as_mut() } {
                app.pending_import = Some(report);
                app.apply_import(include_dupes);
            }
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
        WM_CTLCOLORSTATIC | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN => {
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
                app.import_hwnd = None;
            }
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(Box::from_raw(p));
            DefWindowProcW(hwnd, msg, wparam, lparam)
        },
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

//! Clothesline: a small clipboard-history shelf.
//!
//! This is a first, text-focused implementation. It keeps the last 10 distinct
//! clipboard text items, shows them in a topmost bar, and copies an item back
//! to the clipboard when clicked.

use std::ffi::c_void;

use crate::app::App;
use crate::ui::{self, Chrome, DipRect, id, rect};
use crate::win;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetWindowLongPtrW,
    SetWindowLongPtrW, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_CTLCOLORSTATIC, WM_NCCREATE, WM_NCDESTROY,
    WS_CHILD, WS_VISIBLE,
};

pub const CLASS: &str = "WordDockClothesline";

const CLIENT: (i32, i32) = (900, 72);
const MAX_ITEMS: usize = 10;
const LIST_ID: u16 = 200;

struct State {
    app: *mut App,
    chrome: Chrome,
    items: Vec<String>,
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
    if let Some(hwnd) = app.clothesline_hwnd {
        if win::is_window(hwnd) {
            win::show_without_activation(hwnd);
            return;
        }
        app.clothesline_hwnd = None;
    }
    let instance = win::module_instance();
    let dpi = 96u32;
    let state = Box::new(State {
        app: app as *mut App,
        chrome: ui::Chrome::new(crate::model::ThemeMode::System, dpi),
        items: app.clothesline_items.clone(),
    });
    let pointer = Box::into_raw(state);
    let work = win::primary_work_area();
    let width = (work.right - work.left).min(1200);
    let hwnd = unsafe {
        windows::Win32::UI::WindowsAndMessaging::CreateWindowExW(
            windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE(0),
            win::pcw(&win::wide(CLASS)),
            win::pcw(&win::wide("Clothesline")),
            windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(
                windows::Win32::UI::WindowsAndMessaging::WS_POPUP.0
                    | windows::Win32::UI::WindowsAndMessaging::WS_VISIBLE.0,
            ),
            work.left + (work.right - work.left - width) / 2,
            work.top + 8,
            width,
            72,
            None,
            None,
            Some(instance),
            Some(pointer as *mut c_void),
        )
    }
    .ok();
    if let Some(hwnd) = hwnd {
        app.clothesline_hwnd = Some(hwnd);
        win::set_topmost(hwnd, true);
    } else {
        unsafe {
            drop(Box::from_raw(pointer));
        }
    }
}

pub fn close(app: &mut App) {
    if let Some(hwnd) = app.clothesline_hwnd.take() {
        if win::is_window(hwnd) {
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd);
            }
        }
    }
}

pub fn push_item(app: &mut App, text: String) {
    let trimmed = text.trim().to_string();
    if trimmed.is_empty() {
        return;
    }
    app.clothesline_items.retain(|x| x != &trimmed);
    app.clothesline_items.insert(0, trimmed);
    if app.clothesline_items.len() > MAX_ITEMS {
        app.clothesline_items.truncate(MAX_ITEMS);
    }
    if let Some(hwnd) = app.clothesline_hwnd {
        if win::is_window(hwnd) {
            // Re-populate list
            let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State };
            if !ptr.is_null() {
                let st = unsafe { &mut *ptr };
                st.items = app.clothesline_items.clone();
                populate(hwnd, st);
            }
        }
    }
}

fn populate(hwnd: HWND, state: &State) {
    if let Some(list) = win::child(hwnd, LIST_ID) {
        ui::list_reset(list);
        for item in &state.items {
            ui::list_add(list, item);
        }
    }
}

pub extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == windows::Win32::UI::WindowsAndMessaging::WM_NCCREATE {
        let p = unsafe {
            (*(lparam.0 as *const CREATESTRUCTW)).lpCreateParams as *mut State
        };
        if !p.is_null() {
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, p as isize);
            }
        }
        return LRESULT(1);
    }
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State };
    if pointer.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    match msg {
        windows::Win32::UI::WindowsAndMessaging::WM_CREATE => {
            let s = unsafe { &mut *pointer };
            let dpi = s.chrome.dpi;
            let font = s.chrome.font_handle();
            let _ = ui::create_child(
                hwnd,
                "LISTBOX",
                "",
                ui::STYLE_LISTBOX,
                LIST_ID,
                ui::rect(8, 8, 884, 56),
                dpi,
                font,
            );
            populate(hwnd, s);
            LRESULT(0)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_COMMAND => {
            let command = ui::command_id(wparam);
            if command == LIST_ID {
                let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State };
                if !ptr.is_null() {
                    let st = unsafe { &*ptr };
                    if let Some(list) = win::child(hwnd, LIST_ID) {
                        if let Some(idx) = ui::list_selection(list) {
                            if let Some(text) = st.items.get(idx) {
                                let _ = win::set_clipboard_text(text);
                            }
                        }
                    }
                }
            }
            LRESULT(0)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_CLOSE => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_NCDESTROY => unsafe {
            if let Some(s) = (pointer as *mut State).as_mut() {
                if let Some(app) = s.app.as_mut() {
                    app.clothesline_hwnd = None;
                }
            }
            drop(Box::from_raw(pointer));
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        },
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

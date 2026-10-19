//! Clothesline: a small clipboard-history shelf.
//!
//! Text-focused implementation: keeps the last 10 distinct clipboard text items,
//! shows them as cards hanging from a line at the top of the screen, and copies
//! an item back to the clipboard when clicked. The window never takes focus.

use std::ffi::c_void;

use crate::app::App;
use crate::ui::{self, Chrome};
use crate::win;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{InvalidateRect, MoveToEx, LineTo};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetWindowLongPtrW,
    SetWindowLongPtrW, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_LBUTTONDOWN, WM_NCCREATE, WM_NCDESTROY,
    WM_PAINT, CREATESTRUCTW, WS_CHILD, WS_VISIBLE, WS_POPUP, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, ShowWindow, SW_HIDE, SW_SHOWNOACTIVATE,
};

pub const CLASS: &str = "WordDockClothesline";
const MAX_ITEMS: usize = 10;
const BTN_BASE: u16 = 300;
const WINDOW_HEIGHT: i32 = 118;

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
            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            }
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
    let width = ((work.right - work.left) * 8 / 10).clamp(320, 1200);
    let x = work.left + (work.right - work.left - width) / 2;
    let y = work.top + 6;
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            win::pcw(&win::wide(CLASS)),
            win::pcw(&win::wide("Clothesline")),
            WS_POPUP,
            x,
            y,
            width,
            WINDOW_HEIGHT,
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
        refresh_buttons(hwnd);
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
                let _ = DestroyWindow(hwnd);
            }
        }
    }
}

pub fn toggle(app: &mut App) {
    if let Some(hwnd) = app.clothesline_hwnd {
        if win::is_window(hwnd) {
            unsafe {
                let visible = windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(hwnd).as_bool();
                if visible {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                } else {
                    let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                }
            }
            return;
        }
        app.clothesline_hwnd = None;
    }
    open(app);
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
            let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State };
            if !ptr.is_null() {
                let st = unsafe { &mut *ptr };
                st.items = app.clothesline_items.clone();
            }
            refresh_buttons(hwnd);
            unsafe {
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
        }
    }
}

fn refresh_buttons(hwnd: HWND) {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State };
    if ptr.is_null() {
        return;
    }
    let st = unsafe { &*ptr };
    // remove old child buttons
    for i in 0..MAX_ITEMS {
        let id = BTN_BASE + i as u16;
        if let Some(child) = win::child(hwnd, id) {
            unsafe {
                let _ = DestroyWindow(child);
            }
        }
    }
    let width = {
        let mut r = RECT::default();
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut r);
        }
        r.right - r.left
    };
    let count = st.items.len().max(1);
    let margin = 12;
    let gap = 8;
    let btn_w = ((width - margin * 2 - gap * (count as i32 - 1)) / count as i32).clamp(60, 160);
    let btn_h = 46;
    let y = 58;
    for (i, text) in st.items.iter().enumerate() {
        let x = margin + i as i32 * (btn_w + gap);
        let label = if text.chars().count() > 18 {
            let mut s: String = text.chars().take(17).collect();
            s.push('…');
            s
        } else {
            text.clone()
        };
        let _ = ui::create_child(
            hwnd,
            "BUTTON",
            &label,
            ui::STYLE_BUTTON,
            BTN_BASE + i as u16,
            ui::rect(x, y, btn_w, btn_h),
            st.chrome.dpi,
            st.chrome.font_handle(),
        );
    }
}

pub extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == windows::Win32::UI::WindowsAndMessaging::WM_NCCREATE {
        let p = unsafe { (*(lparam.0 as *const CREATESTRUCTW)).lpCreateParams as *mut State };
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
            // draw line placeholder none
            let _ = (dpi, font);
            LRESULT(0)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_COMMAND => {
            let command = ui::command_id(wparam);
            if (BTN_BASE..BTN_BASE + MAX_ITEMS as u16).contains(&command) {
                let idx = (command - BTN_BASE) as usize;
                let st = unsafe { &*pointer };
                if let Some(text) = st.items.get(idx) {
                    let _ = win::set_clipboard_text(text);
                }
            }
            LRESULT(0)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_MOUSEMOVE => {
            let mut tme = windows::Win32::UI::Input::KeyboardAndMouse::TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<windows::Win32::UI::Input::KeyboardAndMouse::TRACKMOUSEEVENT>() as u32,
                dwFlags: windows::Win32::UI::Input::KeyboardAndMouse::TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            unsafe {
                let _ = windows::Win32::UI::Input::KeyboardAndMouse::TrackMouseEvent(&mut tme);
            }
            LRESULT(0)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_PAINT => {
            let mut ps = windows::Win32::Graphics::Gdi::PAINTSTRUCT::default();
            unsafe {
                let dc = windows::Win32::Graphics::Gdi::BeginPaint(hwnd, &mut ps);
                let mut client = RECT::default();
                let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut client);
                // draw clothesline
                let y = 52;
                let pen = windows::Win32::Graphics::Gdi::CreatePen(
                    windows::Win32::Graphics::Gdi::PS_SOLID,
                    2,
                    windows::Win32::Foundation::COLORREF(0x00808080),
                );
                let old = windows::Win32::Graphics::Gdi::SelectObject(dc, windows::Win32::Graphics::Gdi::HGDIOBJ(pen.0));
                let _ = MoveToEx(dc, 12, y, None);
                let _ = LineTo(dc, client.right - 12, y);
                windows::Win32::Graphics::Gdi::SelectObject(dc, old);
                let _ = windows::Win32::Graphics::Gdi::DeleteObject(windows::Win32::Graphics::Gdi::HGDIOBJ(pen.0));
                let _ = windows::Win32::Graphics::Gdi::EndPaint(hwnd, &ps);
            }
            LRESULT(0)
        }
        windows::Win32::UI::Controls::WM_MOUSELEAVE => {
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
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

//! Notification-area (tray) icon and its context menu.
//!
//! The icon is registered once on the controller window. Menu commands are
//! returned by `TrackPopupMenu` with `TPM_RETURNCMD`, which avoids depending on
//! `WM_COMMAND` routing from a popup menu.

use crate::win::{self, Icon};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_SETVERSION,
    NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, MF_SEPARATOR, MF_STRING, PostMessageW,
    SetForegroundWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu,
};

/// Command ids used by the tray menu.
pub const CMD_SHOW: usize = 1;
pub const CMD_NEXT: usize = 2;
pub const CMD_PREVIOUS: usize = 3;
pub const CMD_PAUSE: usize = 4;
pub const CMD_MARK_LEARNED: usize = 5;
pub const CMD_REVIEW_AGAIN: usize = 6;
pub const CMD_SETTINGS: usize = 7;
pub const CMD_COLLECTIONS: usize = 8;
pub const CMD_EXIT: usize = 9;

/// What the tray asked the application to do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrayCommand {
    ToggleCard,
    Next,
    Previous,
    TogglePause,
    MarkLearned,
    ReviewAgain,
    Settings,
    Collections,
    Exit,
}

pub struct TrayIcon {
    icon: Option<Icon>,
    added: bool,
}

impl TrayIcon {
    pub fn create(dark: bool) -> TrayIcon {
        // A 16px icon: the tray picks the right size and scales the rest.
        let background = if dark {
            crate::theme::rgb(0x2B, 0x2D, 0x31)
        } else {
            crate::theme::rgb(0x1B, 0x4D, 0xB1)
        };
        let glyph = crate::theme::rgb(0xFF, 0xFF, 0xFF);
        let icon = win::create_app_icon(32, background, glyph);
        TrayIcon { icon, added: false }
    }

    fn handle(&self) -> windows::Win32::UI::WindowsAndMessaging::HICON {
        match &self.icon {
            Some(icon) => icon.0,
            None => win::stock_application_icon(),
        }
    }

    /// Adds the icon to the notification area.
    pub fn add(&mut self, hwnd: HWND, callback_message: u32, tip: &str) -> bool {
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: callback_message,
            hIcon: self.handle(),
            ..Default::default()
        };
        let tip = win::wide(tip);
        let count = tip.len().min(data.szTip.len());
        data.szTip[..count].copy_from_slice(&tip[..count]);

        // SAFETY: `data` is fully initialized and matches the struct size reported
        // in `cbSize`, as required by Shell_NotifyIconW.
        let added = unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool();
        if added {
            // Version 4 callbacks report the event in the low word of lparam.
            data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            // SAFETY: same struct, now with the version member set.
            unsafe {
                let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);
            }
            self.added = true;
        }
        added
    }

    pub fn remove(&mut self, hwnd: HWND) {
        if !self.added {
            return;
        }
        let data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            ..Default::default()
        };
        // SAFETY: removing an icon previously added by this process.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        }
        self.added = false;
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        // NIM_DELETE is issued by `remove()` while the controller window still
        // exists; the owned icon is released by its own Drop implementation.
    }
}

/// Builds and shows the context menu. Returns the chosen command, if any.
pub fn show_menu(
    hwnd: HWND,
    card_visible: bool,
    paused: bool,
    has_entry: bool,
) -> Option<TrayCommand> {
    // SAFETY: creating a popup menu and appending items; the menu is destroyed
    // before returning, whatever the user picks.
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        let add = |text: &str, id: usize| {
            let text = win::wide(text);
            let _ = AppendMenuW(menu, MF_STRING, id, win::pcw(&text));
        };
        let separator = || {
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        };

        add(
            if card_visible {
                "Hide card"
            } else {
                "Show card"
            },
            CMD_SHOW,
        );
        separator();
        add("Next word", CMD_NEXT);
        add("Previous word", CMD_PREVIOUS);
        add(
            if paused {
                "Resume rotation"
            } else {
                "Pause rotation"
            },
            CMD_PAUSE,
        );
        if has_entry {
            add("Mark as learned", CMD_MARK_LEARNED);
            add("Review again", CMD_REVIEW_AGAIN);
        }
        separator();
        add("Settings…", CMD_SETTINGS);
        add("Manage collections…", CMD_COLLECTIONS);
        separator();
        add("Exit", CMD_EXIT);

        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        // Required so that the popup menu closes when the user clicks elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let command = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            None,
            hwnd,
            None,
        );
        let _ = DestroyMenu(menu);

        match command.0 as usize {
            CMD_SHOW => Some(TrayCommand::ToggleCard),
            CMD_NEXT => Some(TrayCommand::Next),
            CMD_PREVIOUS => Some(TrayCommand::Previous),
            CMD_PAUSE => Some(TrayCommand::TogglePause),
            CMD_MARK_LEARNED => Some(TrayCommand::MarkLearned),
            CMD_REVIEW_AGAIN => Some(TrayCommand::ReviewAgain),
            CMD_SETTINGS => Some(TrayCommand::Settings),
            CMD_COLLECTIONS => Some(TrayCommand::Collections),
            CMD_EXIT => Some(TrayCommand::Exit),
            _ => None,
        }
    }
}

/// Hands foreground to the (off-screen) controller window so the popup menu owns
/// activation and closes when the user clicks elsewhere, then posts the
/// documented `WM_NULL` message.
pub fn activate_for_menu(hwnd: HWND) {
    // SAFETY: both calls only affect window activation state.
    unsafe {
        let _ = SetForegroundWindow(hwnd);
        let _ = PostMessageW(
            Some(hwnd),
            windows::Win32::UI::WindowsAndMessaging::WM_NULL,
            windows::Win32::Foundation::WPARAM(0),
            windows::Win32::Foundation::LPARAM(0),
        );
    }
}

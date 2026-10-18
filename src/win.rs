//! Thin, documented wrappers around the Win32 APIs used by WordDock.
//!
//! Everything unsafe in this module is a direct call to one Win32 function with
//! arguments checked beforehand. Owned resources (GDI objects, icons, menus,
//! registry keys) are wrapped in RAII types that release them exactly once when
//! dropped; borrowed handles coming from Windows (`HICON` of a stock icon, the
//! `HDC` passed to `WM_PAINT`, ...) are never deleted.

#![allow(non_snake_case)]

use std::ffi::c_void;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateBitmap, CreateCompatibleBitmap,
    CreateCompatibleDC, CreateFontW, CreateRoundRectRgn, CreateSolidBrush, DEFAULT_CHARSET,
    DEFAULT_PITCH, DRAW_TEXT_FORMAT, DT_CENTER, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteDC,
    DeleteObject, DrawTextW, EnumDisplayMonitors, FONT_CLIP_PRECISION, FONT_OUTPUT_PRECISION,
    FW_BOLD, FW_NORMAL, GetDC, GetMonitorInfoW, HBITMAP, HBRUSH, HDC, HFONT, HGDIOBJ, HMONITOR,
    HPEN, MONITOR_DEFAULTTONEAREST, MONITORINFO, MONITORINFOEXW, MonitorFromPoint,
    MonitorFromWindow, OUT_DEFAULT_PRECIS, ReleaseDC, SRCCOPY, SelectObject, SetBkMode,
    SetTextColor, SetWindowRgn, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW};
use windows::Win32::UI::Controls::Dialogs::{
    CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW, ChooseColorW, GetOpenFileNameW, GetSaveFileNameW,
    OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST,
    OPENFILENAMEW,
};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BM_SETCHECK, CreateIconIndirect, DestroyIcon, DrawIcon, GetClassNameW,
    GetClientRect, GetSystemMetrics, GetWindowRect, GetWindowTextW, HICON, HWND_NOTOPMOST,
    HWND_TOPMOST, ICONINFO, IDI_APPLICATION, IsWindow, LoadIconW, MB_ICONERROR, MB_ICONINFORMATION,
    MB_ICONQUESTION, MB_ICONWARNING, MB_OK, MB_YESNO, MESSAGEBOX_RESULT, MESSAGEBOX_STYLE,
    MessageBoxW, SM_CXSCREEN, SM_CYSCREEN, SPI_GETWORKAREA, SW_HIDE, SW_SHOW, SW_SHOWNOACTIVATE,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    SetForegroundWindow, SetWindowPos, SetWindowTextW, ShowWindow, SystemParametersInfoW,
    WINDOW_EX_STYLE, WINDOW_STYLE, WS_CAPTION, WS_SYSMENU, WS_THICKFRAME,
};
use windows::core::BOOL as WinBool;
use windows::core::PCWSTR;

// ---------------------------------------------------------------------------
// String helpers
// ---------------------------------------------------------------------------

/// NUL-terminated UTF-16 buffer for the duration of a call.
pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn pcw(buffer: &[u16]) -> PCWSTR {
    PCWSTR::from_raw(buffer.as_ptr())
}

/// Converts a UTF-16 buffer (NUL-terminated or not) into a `String`.
pub fn from_wide(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

pub fn window_text(hwnd: HWND) -> String {
    let mut buffer = [0u16; 512];
    // SAFETY: `buffer` is a valid writable slice; GetWindowTextW never writes
    // more than the slice length and always NUL-terminates.
    let len = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    from_wide(&buffer[..len.max(0) as usize])
}

pub fn set_window_text(hwnd: HWND, text: &str) {
    let buffer = wide(text);
    // SAFETY: the pointer is valid for the duration of the call.
    unsafe {
        let _ = SetWindowTextW(hwnd, pcw(&buffer));
    }
}

/// Reads a control's text as a trimmed `String`.
pub fn control_text(parent: HWND, id: u16) -> String {
    match child(parent, id) {
        Some(hwnd) => window_text(hwnd).trim().to_string(),
        None => String::new(),
    }
}

pub fn child(parent: HWND, id: u16) -> Option<HWND> {
    // SAFETY: pure handle lookup, returns a null handle when the id is unknown.
    let hwnd =
        unsafe { windows::Win32::UI::WindowsAndMessaging::GetDlgItem(Some(parent), id as i32) }
            .unwrap_or_default();
    if hwnd.0.is_null() { None } else { Some(hwnd) }
}

pub fn is_checked(parent: HWND, id: u16) -> bool {
    match child(parent, id) {
        // SAFETY: BM_GETCHECK takes no extra parameters.
        Some(hwnd) => send_message(hwnd, BM_GETCHECK, WPARAM(0), LPARAM(0)).0 == 1,
        None => false,
    }
}

pub fn set_checked(parent: HWND, id: u16, checked: bool) {
    if let Some(hwnd) = child(parent, id) {
        // BST_UNCHECKED is 0 in winuser.h; windows-rs only exports BST_CHECKED.
        let state = if checked { 1 } else { 0 };
        // SAFETY: BM_SETCHECK takes the state in wparam and no lparam.
        send_message(hwnd, BM_SETCHECK, WPARAM(state), LPARAM(0));
    }
}

pub fn send_message(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
    // SAFETY: the caller is responsible for passing parameters matching `msg`.
    unsafe { SendMessageW(hwnd, msg, Some(wparam), Some(lparam)) }
}

pub fn enable_window(hwnd: HWND, enabled: bool) {
    // SAFETY: toggling the enabled state of an existing window.
    unsafe {
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(hwnd, enabled);
    }
}

// ---------------------------------------------------------------------------
// Message boxes
// ---------------------------------------------------------------------------

pub fn message_box(
    owner: HWND,
    text: &str,
    caption: &str,
    style: MESSAGEBOX_STYLE,
) -> MESSAGEBOX_RESULT {
    let text = wide(text);
    let caption = wide(caption);
    // SAFETY: both buffers are NUL-terminated and stay alive for the call.
    unsafe { MessageBoxW(Some(owner), pcw(&text), pcw(&caption), style) }
}

pub fn info(owner: HWND, text: &str, caption: &str) {
    message_box(owner, text, caption, MB_OK | MB_ICONINFORMATION);
}

pub fn warn(owner: HWND, text: &str, caption: &str) {
    message_box(owner, text, caption, MB_OK | MB_ICONWARNING);
}

pub fn error(owner: HWND, text: &str, caption: &str) {
    message_box(owner, text, caption, MB_OK | MB_ICONERROR);
}

/// Yes/No confirmation with "No" as the default button.
pub fn confirm(owner: HWND, text: &str, caption: &str) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{IDYES, MB_DEFBUTTON2};
    let result = message_box(
        owner,
        text,
        caption,
        MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
    );
    result == IDYES
}

// ---------------------------------------------------------------------------
// Owned resources
// ---------------------------------------------------------------------------

macro_rules! gdi_object {
    ($name:ident, $type:ty, $doc:literal) => {
        #[doc = $doc]
        pub struct $name(pub $type);

        impl $name {
            pub fn raw(&self) -> $type {
                self.0
            }

            /// The same object as a generic `HGDIOBJ` (for `SelectObject`).
            pub fn obj(&self) -> HGDIOBJ {
                HGDIOBJ(self.0.0)
            }

            pub fn is_null(&self) -> bool {
                self.0.0.is_null()
            }
        }

        impl Drop for $name {
            fn drop(&mut self) {
                if !self.is_null() {
                    // SAFETY: the handle is owned by this value and deleted once.
                    unsafe {
                        let _ = DeleteObject(self.obj());
                    }
                }
            }
        }
    };
}

gdi_object!(Font, HFONT, "Owned GDI font.");
gdi_object!(Brush, HBRUSH, "Owned GDI brush.");
gdi_object!(Pen, HPEN, "Owned GDI pen.");
gdi_object!(Bitmap, HBITMAP, "Owned GDI bitmap.");

/// Owned icon created by this application (`DestroyIcon`).
pub struct Icon(pub HICON);

impl Drop for Icon {
    fn drop(&mut self) {
        if !self.0.0.is_null() {
            // SAFETY: the icon was created by CreateIconIndirect and is destroyed once.
            unsafe {
                let _ = DestroyIcon(self.0);
            }
        }
    }
}

/// Memory DC with a compatible bitmap selected into it. Restores and deletes
/// both objects when dropped.
pub struct MemoryDc {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    width: i32,
    height: i32,
}

impl MemoryDc {
    pub fn new(width: i32, height: i32) -> Option<MemoryDc> {
        if width <= 0 || height <= 0 {
            return None;
        }
        // SAFETY: creating a memory DC compatible with the screen and a bitmap of
        // the requested size; both are deleted in `Drop`.
        unsafe {
            let screen = GetDC(None);
            if screen.0.is_null() {
                return None;
            }
            let dc = CreateCompatibleDC(Some(screen));
            let bitmap = CreateCompatibleBitmap(screen, width, height);
            let _ = ReleaseDC(None, screen);
            if dc.0.is_null() || bitmap.0.is_null() {
                if !bitmap.0.is_null() {
                    let _ = DeleteObject(HGDIOBJ(bitmap.0));
                }
                if !dc.0.is_null() {
                    let _ = DeleteDC(dc);
                }
                return None;
            }
            let previous = SelectObject(dc, HGDIOBJ(bitmap.0));
            Some(MemoryDc {
                dc,
                bitmap,
                previous,
                width,
                height,
            })
        }
    }

    pub fn dc(&self) -> HDC {
        self.dc
    }

    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    /// Copies the whole buffer to `target`.
    pub fn blit_to(&self, target: HDC) {
        // SAFETY: both DCs are valid; the source has been selected into `self.dc`.
        unsafe {
            let _ = BitBlt(
                target,
                0,
                0,
                self.width,
                self.height,
                Some(self.dc),
                0,
                0,
                SRCCOPY,
            );
        }
    }
}

impl Drop for MemoryDc {
    fn drop(&mut self) {
        // SAFETY: restoring the original object and deleting the objects this
        // value owns, exactly once.
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            let _ = DeleteDC(self.dc);
        }
    }
}

// ---------------------------------------------------------------------------
// Fonts and colours
// ---------------------------------------------------------------------------

/// Creates a font of `size_pt` points for a monitor with `dpi` dots per inch.
pub fn create_font(family: &str, size_pt: f32, dpi: u32, bold: bool) -> Option<Font> {
    let height = -((size_pt * dpi as f32 / 72.0).round() as i32);
    let face = wide(family);
    // SAFETY: `face` is NUL-terminated; the returned font is owned by the wrapper.
    let font = unsafe {
        CreateFontW(
            height,
            0,
            0,
            0,
            if bold {
                FW_BOLD.0 as i32
            } else {
                FW_NORMAL.0 as i32
            },
            0,
            0,
            0,
            DEFAULT_CHARSET,
            FONT_OUTPUT_PRECISION(OUT_DEFAULT_PRECIS.0),
            FONT_CLIP_PRECISION(CLIP_DEFAULT_PRECIS.0),
            CLEARTYPE_QUALITY,
            DEFAULT_PITCH.0 as u32,
            pcw(&face),
        )
    };
    let font = Font(font);
    if font.is_null() { None } else { Some(font) }
}

/// Scales a device-independent pixel value to the given DPI.
pub fn dip(value: i32, dpi: u32) -> i32 {
    ((value as i64 * dpi as i64 + 48) / 96) as i32
}

/// Inverse of [`dip`]: device pixels back to device-independent pixels.
pub fn dip_to_dip(value: i32, dpi: u32) -> i32 {
    ((value as i64 * 96 + dpi as i64 / 2) / dpi as i64) as i32
}

pub fn dpi_of_window(hwnd: HWND) -> u32 {
    // SAFETY: pure query for an existing window.
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 { system_dpi() } else { dpi }
}

pub fn system_dpi() -> u32 {
    // SAFETY: documented process-wide query; returns 0 only on pre-1607 systems.
    let dpi = unsafe { GetDpiForSystem() };
    if dpi == 0 { 96 } else { dpi }
}

// ---------------------------------------------------------------------------
// Monitors and work area
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct MonitorInfo {
    pub device: String,
    pub work: RECT,
    pub bounds: RECT,
}

impl MonitorInfo {
    pub fn work_size(&self) -> (i32, i32) {
        (
            self.work.right - self.work.left,
            self.work.bottom - self.work.top,
        )
    }
}

/// Monitor nearest to a window, or the primary monitor when `hwnd` is null.
pub fn monitor_for_window(hwnd: HWND) -> Option<MonitorInfo> {
    let monitor = if hwnd.0.is_null() {
        // SAFETY: querying the primary monitor.
        let point = POINT { x: 0, y: 0 };
        unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) }
    } else {
        // SAFETY: existing window handle.
        unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) }
    };
    if monitor.0.is_null() {
        return None;
    }
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: the struct is initialized with its size field set, as required.
    let ok =
        unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo as *mut MONITORINFO) }.as_bool();
    if !ok {
        return None;
    }
    Some(MonitorInfo {
        device: from_wide(&info.szDevice),
        work: info.monitorInfo.rcWork,
        bounds: info.monitorInfo.rcMonitor,
    })
}

/// Finds a monitor by device name (e.g. `\\.\DISPLAY2`).
pub fn monitor_by_device(name: &str) -> Option<MonitorInfo> {
    #[derive(Default)]
    struct Search {
        target: Vec<u16>,
        found: Option<MonitorInfo>,
    }

    unsafe extern "system" fn callback(
        monitor: HMONITOR,
        _dc: HDC,
        _rect: *mut RECT,
        data: LPARAM,
    ) -> WinBool {
        // SAFETY: `data` is the pointer passed to EnumDisplayMonitors below and
        // stays valid (and exclusively borrowed) for the whole enumeration.
        let search = unsafe { &mut *(data.0 as *mut Search) };
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        // SAFETY: as above, the struct has its size field set.
        if unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo as *mut MONITORINFO) }.as_bool()
        {
            let device = from_wide(&info.szDevice);
            if device.eq_ignore_ascii_case(&from_wide(&search.target)) {
                search.found = Some(MonitorInfo {
                    device,
                    work: info.monitorInfo.rcWork,
                    bounds: info.monitorInfo.rcMonitor,
                });
                return WinBool(0); // stop enumerating
            }
        }
        WinBool(1)
    }

    let mut search = Search {
        target: wide(name),
        found: None,
    };
    // SAFETY: `callback` receives `&mut search` through lparam and only writes to it.
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(callback),
            LPARAM(&mut search as *mut Search as isize),
        );
    }
    search.found
}

/// Enumerates every attached monitor.
pub fn all_monitors() -> Vec<MonitorInfo> {
    #[derive(Default)]
    struct Collect {
        items: Vec<MonitorInfo>,
    }
    unsafe extern "system" fn callback(
        monitor: HMONITOR,
        _dc: HDC,
        _rect: *mut RECT,
        data: LPARAM,
    ) -> WinBool {
        let collect = unsafe { &mut *(data.0 as *mut Collect) };
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo as *mut MONITORINFO) }.as_bool()
        {
            collect.items.push(MonitorInfo {
                device: from_wide(&info.szDevice),
                work: info.monitorInfo.rcWork,
                bounds: info.monitorInfo.rcMonitor,
            });
        }
        WinBool(1)
    }
    let mut collect = Collect::default();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(callback),
            LPARAM(&mut collect as *mut Collect as isize),
        );
    }
    collect.items
}

/// Work area of the primary monitor.
pub fn primary_work_area() -> RECT {
    if let Some(info) = monitor_for_window(HWND::default()) {
        return info.work;
    }
    let mut rect = RECT::default();
    // SAFETY: documented query filling a RECT; falls back to the screen size.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut rect as *mut RECT as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .is_ok();
    if ok {
        return rect;
    }
    // SAFETY: pure queries for the primary screen metrics.
    let (w, h) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    RECT {
        left: 0,
        top: 0,
        right: w,
        bottom: h,
    }
}

/// Adds the window frame to a client rect so a window of that client size fits.
pub fn window_rect_for_client(size: (i32, i32), dpi: u32) -> (i32, i32) {
    window_rect_for_client_styled(
        size,
        dpi,
        WS_CAPTION | WS_SYSMENU | WS_THICKFRAME,
        WINDOW_EX_STYLE(0),
    )
}

/// Same as [`window_rect_for_client`], for an explicit style pair.
pub fn window_rect_for_client_styled(
    size: (i32, i32),
    dpi: u32,
    style: WINDOW_STYLE,
    ex_style: WINDOW_EX_STYLE,
) -> (i32, i32) {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: size.0,
        bottom: size.1,
    };
    // SAFETY: AdjustWindowRectExForDpi only reads the rect and the style flags.
    unsafe {
        let _ = AdjustWindowRectExForDpi(&mut rect, style, false, ex_style, dpi);
    }
    (rect.right - rect.left, rect.bottom - rect.top)
}

/// Whether the handle still refers to a live window.
pub fn is_window(hwnd: HWND) -> bool {
    // SAFETY: pure query; a stale handle simply reports false.
    unsafe { IsWindow(Some(hwnd)).as_bool() }
}

/// Shows and focuses an existing window (dialogs reopened from the tray).
pub fn activate_window(hwnd: HWND) {
    // SAFETY: both calls only change visibility and activation state.
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
}

/// Class name of a window, used to route keyboard messages to dialogs.
pub fn window_class_name(hwnd: HWND) -> String {
    if !is_window(hwnd) {
        return String::new();
    }
    let mut buffer = [0u16; 256];
    // SAFETY: `buffer` is writable and GetClassNameW NUL-terminates.
    let len = unsafe { GetClassNameW(hwnd, &mut buffer) };
    if len <= 0 {
        String::new()
    } else {
        from_wide(&buffer[..len as usize])
    }
}

/// Moves/resizes without activating the window.
pub fn set_window_rect_no_activate(hwnd: HWND, x: i32, y: i32, width: i32, height: i32) {
    // SAFETY: positioning an existing window; SWP_NOACTIVATE keeps focus untouched.
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE | SWP_NOZORDER,
        );
    }
}

pub fn set_topmost(hwnd: HWND, topmost: bool) {
    // SAFETY: changing the z-order band of an existing window.
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(if topmost {
                HWND_TOPMOST
            } else {
                HWND_NOTOPMOST
            }),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

pub fn show_without_activation(hwnd: HWND) {
    // SAFETY: SW_SHOWNOACTIVATE shows the window without stealing keyboard focus.
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
}

pub fn hide_window(hwnd: HWND) {
    // SAFETY: hiding an existing window.
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

pub fn client_rect(hwnd: HWND) -> RECT {
    let mut rect = RECT::default();
    // SAFETY: filling a RECT for an existing window.
    unsafe {
        let _ = GetClientRect(hwnd, &mut rect);
    }
    rect
}

/// Rounds the window corners; the region becomes owned by the system on success.
pub fn set_rounded_window_region(hwnd: HWND, radius: i32) {
    let mut rect = RECT::default();
    // SAFETY: filling a RECT for an existing window; region ownership is handled
    // explicitly below.
    unsafe {
        let _ = GetWindowRect(hwnd, &mut rect);
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        if width <= 0 || height <= 0 {
            return;
        }
        // The coordinates are inclusive, hence the +1 on the right/bottom edges.
        let region = CreateRoundRectRgn(0, 0, width + 1, height + 1, radius, radius);
        if region.0.is_null() {
            return;
        }
        if SetWindowRgn(hwnd, Some(region), true) == 0 {
            // The system did not take ownership, so we still own the region.
            let _ = DeleteObject(HGDIOBJ(region.0));
        }
    }
}

// ---------------------------------------------------------------------------
// Application icon
// ---------------------------------------------------------------------------

/// Draws a simple "W" badge icon at runtime (no icon file needed).
pub fn create_app_icon(size: i32, background: COLORREF, glyph: COLORREF) -> Option<Icon> {
    if size < 8 {
        return None;
    }
    // SAFETY: all GDI objects created here are deleted before returning; the icon
    // is owned by the returned wrapper.
    unsafe {
        let screen = GetDC(None);
        if screen.0.is_null() {
            return None;
        }
        let dc = CreateCompatibleDC(Some(screen));
        let color = CreateCompatibleBitmap(screen, size, size);
        if dc.0.is_null() || color.0.is_null() {
            if !color.0.is_null() {
                let _ = DeleteObject(HGDIOBJ(color.0));
            }
            if !dc.0.is_null() {
                let _ = DeleteDC(dc);
            }
            let _ = ReleaseDC(None, screen);
            return None;
        }
        let previous = SelectObject(dc, HGDIOBJ(color.0));

        let brush = CreateSolidBrush(background);
        let rect = RECT {
            left: 0,
            top: 0,
            right: size,
            bottom: size,
        };
        windows::Win32::Graphics::Gdi::FillRect(dc, &rect, brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));

        let font = create_font(crate::model::UI_FONT_FAMILY, size as f32 * 0.68, 96, true);
        let previous_font = font.as_ref().map(|f| SelectObject(dc, f.obj()));
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, glyph);
        let mut text = wide("W");
        DrawTextW(
            dc,
            &mut text,
            &mut RECT {
                left: 0,
                top: 0,
                right: size,
                bottom: size,
            },
            DRAW_TEXT_FORMAT(DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_NOPREFIX.0),
        );
        if let Some(previous_font) = previous_font {
            SelectObject(dc, previous_font);
        }
        drop(font);

        // An all-zero mask makes the whole bitmap opaque.
        let mask = CreateBitmap(size, size, 1, 1, None);
        let info = ICONINFO {
            fIcon: WinBool(1),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&info);
        SelectObject(dc, previous);
        let _ = DeleteObject(HGDIOBJ(mask.0));
        let _ = DeleteObject(HGDIOBJ(color.0));
        let _ = DeleteDC(dc);
        let _ = ReleaseDC(None, screen);

        let icon = icon.ok().map(Icon);
        icon.filter(|i| !i.0.0.is_null())
    }
}

/// Stock fallback icon, owned by the system and therefore never destroyed.
pub fn stock_application_icon() -> HICON {
    // SAFETY: loading a shared system icon; shared icons must not be destroyed.
    unsafe { LoadIconW(None, IDI_APPLICATION).unwrap_or_default() }
}

// ---------------------------------------------------------------------------
// File dialogs
// ---------------------------------------------------------------------------

pub struct FileFilter {
    pub label: &'static str,
    pub pattern: &'static str,
}

fn build_filter(filters: &[FileFilter]) -> Vec<u16> {
    let mut buffer: Vec<u16> = Vec::new();
    for filter in filters {
        buffer.extend(filter.label.encode_utf16());
        buffer.push(0);
        buffer.extend(filter.pattern.encode_utf16());
        buffer.push(0);
    }
    buffer.push(0);
    buffer
}

pub fn pick_open_file(owner: HWND, title: &str, filters: &[FileFilter]) -> Option<PathBuf> {
    let filter = build_filter(filters);
    let title = wide(title);
    let mut buffer = vec![0u16; 4096];
    let mut ofn = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: pcw(&filter),
        nFilterIndex: 1,
        lpstrFile: windows::core::PWSTR::from_raw(buffer.as_mut_ptr()),
        nMaxFile: buffer.len() as u32,
        lpstrTitle: pcw(&title),
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_EXPLORER | OFN_NOCHANGEDIR,
        ..Default::default()
    };
    // SAFETY: `ofn` points at live, NUL-terminated buffers for the whole call.
    let accepted = unsafe { GetOpenFileNameW(&mut ofn) }.as_bool();
    if accepted {
        Some(PathBuf::from(from_wide(&buffer)))
    } else {
        None
    }
}

pub fn pick_save_file(
    owner: HWND,
    title: &str,
    filters: &[FileFilter],
    default_extension: &str,
    default_name: &str,
) -> Option<PathBuf> {
    let filter = build_filter(filters);
    let title = wide(title);
    let extension = wide(default_extension);
    let mut buffer = vec![0u16; 4096];
    let initial: Vec<u16> = default_name.encode_utf16().collect();
    let copy_len = initial.len().min(buffer.len() - 1);
    buffer[..copy_len].copy_from_slice(&initial[..copy_len]);
    let mut ofn = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: pcw(&filter),
        nFilterIndex: 1,
        lpstrFile: windows::core::PWSTR::from_raw(buffer.as_mut_ptr()),
        nMaxFile: buffer.len() as u32,
        lpstrTitle: pcw(&title),
        lpstrDefExt: pcw(&extension),
        Flags: OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_EXPLORER | OFN_NOCHANGEDIR,
        ..Default::default()
    };
    // SAFETY: as above.
    let accepted = unsafe { GetSaveFileNameW(&mut ofn) }.as_bool();
    if accepted {
        Some(PathBuf::from(from_wide(&buffer)))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Registry (current user only)
// ---------------------------------------------------------------------------

pub mod registry {
    use super::*;
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
        RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteValueW,
        RegGetValueW, RegSetValueExW,
    };

    /// Owned registry key handle.
    struct Key(HKEY);

    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: the handle is owned by this value and closed once.
            unsafe {
                let _ = RegCloseKey(self.0);
            }
        }
    }

    fn open(subkey: &str) -> Option<Key> {
        let subkey = wide(subkey);
        let mut handle = HKEY::default();
        // SAFETY: the subkey is NUL-terminated and `handle` is a valid out pointer.
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                pcw(&subkey),
                None,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE | KEY_QUERY_VALUE,
                None,
                &mut handle,
                None,
            )
        };
        if status.is_ok() && !handle.0.is_null() {
            Some(Key(handle))
        } else {
            None
        }
    }

    pub fn write_string(subkey: &str, name: &str, value: &str) -> Result<(), String> {
        let Some(key) = open(subkey) else {
            return Err(format!("cannot open HKEY_CURRENT_USER\\{subkey}"));
        };
        let name = wide(name);
        let data = wide(value);
        let bytes: &[u8] = unsafe {
            // SAFETY: reinterpreting the UTF-16 buffer (including the terminator)
            // as bytes, which is what REG_SZ expects.
            std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 2)
        };
        // SAFETY: the buffers outlive the call.
        let status = unsafe { RegSetValueExW(key.0, pcw(&name), None, REG_SZ, Some(bytes)) };
        if status.is_ok() {
            Ok(())
        } else {
            Err(format!("RegSetValueExW failed ({status:?})"))
        }
    }

    pub fn delete_value(subkey: &str, name: &str) -> Result<(), String> {
        let Some(key) = open(subkey) else {
            return Err(format!("cannot open HKEY_CURRENT_USER\\{subkey}"));
        };
        let name = wide(name);
        // SAFETY: the buffer outlives the call; a missing value is not an error
        // for the caller's purpose (disabling an entry that never existed).
        let _ = unsafe { RegDeleteValueW(key.0, pcw(&name)) };
        Ok(())
    }

    pub fn read_string(subkey: &str, name: &str) -> Option<String> {
        let key = open(subkey)?;
        let name = wide(name);
        let mut buffer = [0u16; 1024];
        let mut size = (buffer.len() * 2) as u32;
        // SAFETY: buffer and size describe the same allocation.
        let status = unsafe {
            RegGetValueW(
                key.0,
                PCWSTR::null(),
                pcw(&name),
                RRF_RT_REG_SZ,
                None,
                Some(buffer.as_mut_ptr() as *mut c_void),
                Some(&mut size),
            )
        };
        if status.is_ok() {
            Some(from_wide(&buffer))
        } else {
            None
        }
    }

    pub fn read_dword(subkey: &str, name: &str) -> Option<u32> {
        let key = open(subkey)?;
        let name = wide(name);
        let mut value: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        // SAFETY: `value` and `size` describe the same allocation.
        let status = unsafe {
            RegGetValueW(
                key.0,
                PCWSTR::null(),
                pcw(&name),
                RRF_RT_REG_DWORD,
                None,
                Some(&mut value as *mut u32 as *mut c_void),
                Some(&mut size),
            )
        };
        if status.is_ok() { Some(value) } else { None }
    }
}

// ---------------------------------------------------------------------------
// Misc
// ---------------------------------------------------------------------------

/// Full path of the running executable.
pub fn pick_color(owner: HWND, initial: u32) -> Option<u32> {
    let mut custom = [COLORREF(0); 16];
    let mut cc = CHOOSECOLORW {
        lStructSize: std::mem::size_of::<CHOOSECOLORW>() as u32,
        hwndOwner: owner,
        rgbResult: COLORREF(initial),
        lpCustColors: custom.as_mut_ptr(),
        Flags: CC_RGBINIT | CC_FULLOPEN,
        ..Default::default()
    };
    let ok = unsafe { ChooseColorW(&mut cc).as_bool() };
    if ok { Some(cc.rgbResult.0) } else { None }
}

pub fn clipboard_text() -> Option<String> {
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::Foundation::HGLOBAL;
    use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
    unsafe {
        if OpenClipboard(None).is_err() {
            return None;
        }
        let handle = GetClipboardData(13).ok();
        let result = handle.and_then(|h| {
            let ptr = windows::Win32::System::Memory::GlobalLock(
                windows::Win32::Foundation::HGLOBAL(h.0),
            );
            if ptr.is_null() {
                None
            } else {
                let mut len = 0usize;
                let p = ptr as *const u16;
                while *p.add(len) != 0 {
                    len += 1;
                }
                let slice = std::slice::from_raw_parts(p, len);
                let os = std::ffi::OsString::from_wide(slice);
                let _ = windows::Win32::System::Memory::GlobalUnlock(
                    windows::Win32::Foundation::HGLOBAL(h.0),
                );
                os.to_str().map(|s| s.to_string())
            }
        });
        let _ = CloseClipboard();
        result
    }
}

pub fn set_clipboard_text(text: &str) -> bool {
    use windows::Win32::Foundation::HGLOBAL;
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc};
    unsafe {
        if OpenClipboard(None).is_err() {
            return false;
        }
        let _ = EmptyClipboard();
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes = wide.len() * 2;
        let h = match windows::Win32::System::Memory::GlobalAlloc(
            windows::Win32::System::Memory::GMEM_MOVEABLE,
            bytes,
        ) {
            Ok(h) => h,
            Err(_) => {
                let _ = CloseClipboard();
                return false;
            }
        };
        let ptr =
            windows::Win32::System::Memory::GlobalLock(windows::Win32::Foundation::HGLOBAL(h.0));
        if ptr.is_null() {
            let _ = CloseClipboard();
            return false;
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr as *mut u16, wide.len());
        let _ =
            windows::Win32::System::Memory::GlobalUnlock(windows::Win32::Foundation::HGLOBAL(h.0));
        let ok = SetClipboardData(13, Some(windows::Win32::Foundation::HANDLE(h.0))).is_ok();
        let _ = CloseClipboard();
        ok
    }
}

pub fn add_clipboard_listener(hwnd: HWND) -> bool {
    use windows::Win32::System::DataExchange::AddClipboardFormatListener;
    unsafe { AddClipboardFormatListener(hwnd).is_ok() }
}

pub fn remove_clipboard_listener(hwnd: HWND) {
    use windows::Win32::System::DataExchange::RemoveClipboardFormatListener;
    unsafe {
        let _ = RemoveClipboardFormatListener(hwnd);
    }
}

pub fn exe_path() -> Option<PathBuf> {
    let mut buffer = [0u16; 1024];
    // SAFETY: the buffer is writable and GetModuleFileNameW NUL-terminates.
    let len = unsafe { GetModuleFileNameW(None, &mut buffer) };
    if len == 0 {
        return None;
    }
    Some(PathBuf::from(from_wide(&buffer[..len as usize])))
}

pub fn module_instance() -> HINSTANCE {
    // SAFETY: pure query returning the module handle of the executable.
    unsafe { HINSTANCE(GetModuleHandleW(PCWSTR::null()).unwrap_or_default().0) }
}

/// Splits a `WPARAM`/`LPARAM` style word pair (used for tray callbacks).
pub fn loword(value: isize) -> u32 {
    (value as usize & 0xFFFF) as u32
}

pub fn hiword(value: isize) -> u32 {
    ((value as usize >> 16) & 0xFFFF) as u32
}

/// Draws an owned icon in a device context (used by dark/light dialog headers).
pub fn draw_icon(dc: HDC, icon: &Icon, x: i32, y: i32) {
    if icon.0.0.is_null() {
        return;
    }
    // SAFETY: drawing an existing icon into a valid DC.
    unsafe {
        let _ = DrawIcon(dc, x, y, icon.0);
    }
}

/// Convenience for `SetBkMode(TRANSPARENT)` + text colour on a DC.
pub fn prepare_text(dc: HDC, color: COLORREF) {
    // SAFETY: both calls only set DC state.
    unsafe {
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, color);
    }
}

/// Path helper used in user-facing messages.
pub fn display_path(path: &Path) -> String {
    path.display().to_string()
}

/// Stock cursor identified by an `IDC_*` constant; falls back to the arrow.
pub fn load_cursor(identifier: PCWSTR) -> windows::Win32::UI::WindowsAndMessaging::HCURSOR {
    use windows::Win32::UI::WindowsAndMessaging::{HCURSOR, IDC_ARROW, LoadCursorW};
    // SAFETY: loading a shared system cursor; shared cursors are never destroyed.
    unsafe {
        LoadCursorW(None, identifier)
            .or_else(|_| LoadCursorW(None, IDC_ARROW))
            .unwrap_or(HCURSOR::default())
    }
}

/// Value of the calling thread's last-error code.
pub fn last_error() -> u32 {
    // SAFETY: per-thread state, always readable.
    unsafe { windows::Win32::Foundation::GetLastError().0 }
}

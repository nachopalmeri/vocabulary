//! Clothesline: a small clipboard-history shelf.
//!
//! Native clipboard shelf for text, screenshots/images and copied file paths.
//! The top-edge trigger and non-activating shelf keep it out of the way until
//! the user reaches for it.

use serde::{Deserialize, Serialize};
use std::{ffi::c_void, time::Instant};

use crate::app::App;
use crate::win;
use crate::{
    theme,
    ui::{self, Chrome},
};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_BITFIELDS, BITMAPINFO, BITMAPINFOHEADER, CreatePen, CreateSolidBrush, DEFAULT_GUI_FONT,
    DIB_RGB_COLORS, DRAW_TEXT_FORMAT, DT_CENTER, DT_END_ELLIPSIS, DT_RIGHT, DT_SINGLELINE,
    DT_VCENTER, DT_WORDBREAK, DeleteObject, DrawTextW, EndPaint, FillRect, GetStockObject,
    HALFTONE, HGDIOBJ, InvalidateRect, LineTo, MoveToEx, NULL_BRUSH, PAINTSTRUCT, PS_SOLID,
    RoundRect, SRCCOPY, SelectObject, SetBkMode, SetBrushOrgEx, SetStretchBltMode, SetTextColor,
    StretchDIBits, TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetClientRect,
    GetCursorPos, GetWindowLongPtrW, GetWindowRect, KillTimer, LWA_ALPHA, LWA_COLORKEY, SW_HIDE,
    SW_SHOWNOACTIVATE, SetLayeredWindowAttributes, SetTimer, SetWindowLongPtrW, ShowWindow,
    WM_CLOSE, WM_COMMAND, WM_CREATE, WM_NCCREATE, WM_NCDESTROY, WM_PAINT, WM_TIMER, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
};

pub const CLASS: &str = "WordDockClothesline";
const MAX_ITEMS: usize = 10;
const MAX_VISIBLE_ITEMS: usize = 3;
const WINDOW_HEIGHT: i32 = 152;
const CARD_TOP: i32 = 48;
const CARD_BOTTOM: i32 = 142;
const COLOR_KEY: u32 = 0x00FF00FF;
const EDGE_CLASS: &str = "WordDockClotheslineEdge";
const HIDE_TIMER: usize = 41;
const ANIMATE_TIMER: usize = 42;
const ANIMATION_MS: u128 = 110;

struct Animation {
    started: Instant,
    from_alpha: u8,
    to_alpha: u8,
    hide_when_done: bool,
}

struct ShelfFonts {
    header: Option<win::Font>,
    body: Option<win::Font>,
    small: Option<win::Font>,
}
const HISTORY_MAGIC: &[u8; 5] = b"WDCH1";

struct State {
    app: *mut App,
    chrome: Chrome,
    fonts: ShelfFonts,
    items: Vec<ClipboardItem>,
    page_start: usize,
    pointer_outside_since: Option<Instant>,
    animation: Option<Animation>,
    alpha: u8,
    pinned: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ClipboardItem {
    Text(String),
    Image { media_file: String, format: u32 },
    Files(Vec<String>),
}

impl ClipboardItem {
    fn display_label(&self) -> String {
        match self {
            Self::Text(text) => {
                let cleaned = text
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                let mut preview: String = cleaned.chars().take(180).collect();
                if cleaned.chars().count() > 180 {
                    preview.push('…');
                }
                if preview.is_empty() {
                    preview.push_str("Texto");
                }
                preview
            }
            Self::Image { .. } => "Captura de pantalla".to_string(),
            Self::Files(paths) => paths
                .first()
                .and_then(|p| std::path::Path::new(p).file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| format!("{} archivo(s)", paths.len())),
        }
    }
}

pub fn class() -> ui::ClassSpec {
    ui::ClassSpec {
        name: CLASS,
        proc: Some(proc),
        cursor: ui::CURSOR_ARROW,
        background: ui::SYS_BTNFACE,
    }
}

pub fn edge_class() -> ui::ClassSpec {
    ui::ClassSpec {
        name: EDGE_CLASS,
        proc: Some(edge_proc),
        cursor: ui::CURSOR_ARROW,
        background: windows::Win32::Graphics::Gdi::HBRUSH::default(),
    }
}

pub fn install_edge_trigger(app: &mut App) {
    if app.clothesline_edge_hwnd.is_some_and(win::is_window) {
        return;
    }
    let bounds = win::monitor_for_window(HWND::default())
        .map(|monitor| monitor.bounds)
        .unwrap_or_else(win::primary_work_area);
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            win::pcw(&win::wide(EDGE_CLASS)),
            win::pcw(&win::wide("")),
            WS_POPUP,
            bounds.left,
            bounds.top,
            bounds.right - bounds.left,
            3,
            None,
            None,
            Some(win::module_instance()),
            Some(app as *mut App as *mut c_void),
        )
    }
    .ok();
    if let Some(hwnd) = hwnd {
        app.clothesline_edge_hwnd = Some(hwnd);
        win::set_topmost(hwnd, true);
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
    }
}

pub fn remove_edge_trigger(app: &mut App) {
    if let Some(hwnd) = app.clothesline_edge_hwnd.take() {
        if win::is_window(hwnd) {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
    }
}

pub fn open(app: &mut App) {
    save_items(app);
    if let Some(hwnd) = app.clothesline_hwnd {
        if win::is_window(hwnd) {
            let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State };
            if !ptr.is_null() {
                unsafe {
                    (*ptr).pinned = true;
                }
            }
            show(hwnd);
            return;
        }
        app.clothesline_hwnd = None;
    }
    create_window(app, true);
}

fn reveal_from_edge(app: &mut App) {
    if let Some(hwnd) = app.clothesline_hwnd {
        if win::is_window(hwnd) {
            if !unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(hwnd).as_bool() }
            {
                let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State };
                if !ptr.is_null() {
                    unsafe {
                        (*ptr).pinned = false;
                    }
                }
                show(hwnd);
            }
            return;
        }
        app.clothesline_hwnd = None;
    }
    create_window(app, false);
}

fn create_window(app: &mut App, pinned: bool) {
    let instance = win::module_instance();
    let dpi = win::system_dpi();
    let state = Box::new(State {
        app: app as *mut App,
        chrome: ui::Chrome::new(crate::model::ThemeMode::System, dpi),
        fonts: ShelfFonts {
            header: win::create_font(crate::model::UI_FONT_FAMILY, 8.5, dpi, true),
            body: win::create_font(crate::model::UI_FONT_FAMILY, 9.0, dpi, false),
            small: win::create_font(crate::model::UI_FONT_FAMILY, 7.5, dpi, false),
        },
        items: app.clothesline_items.clone(),
        page_start: 0,
        pointer_outside_since: None,
        animation: None,
        alpha: 0,
        pinned,
    });
    let pointer = Box::into_raw(state);
    let work = win::primary_work_area();
    let slots = app.clothesline_items.len().clamp(1, MAX_VISIBLE_ITEMS) as i32;
    let desired_width = slots * 176 + 28 + (slots - 1) * 8;
    let minimum_width = 320;
    let width = desired_width
        .clamp(minimum_width, 640)
        .min(work.right - work.left);
    let x = work.left + (work.right - work.left - width) / 2;
    // Sit below the browser tab strip and address bar: the shelf must never
    // intercept tab clicks or navigation controls while it is visible.
    let y = (work.top + 88)
        .min(work.bottom - WINDOW_HEIGHT - 8)
        .max(work.top + 6);
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
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
        unsafe {
            let _ = SetLayeredWindowAttributes(
                hwnd,
                windows::Win32::Foundation::COLORREF(COLOR_KEY),
                0,
                LWA_COLORKEY | LWA_ALPHA,
            );
        }
        show(hwnd);
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
    save_items(app);
    if let Some(hwnd) = app.clothesline_hwnd {
        if win::is_window(hwnd) {
            unsafe {
                let visible =
                    windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(hwnd).as_bool();
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
                if visible {
                    if !ptr.is_null() {
                        (*ptr).pinned = false;
                    }
                    hide(hwnd);
                } else {
                    if !ptr.is_null() {
                        (*ptr).pinned = true;
                    }
                    show(hwnd);
                }
            }
            return;
        }
        app.clothesline_hwnd = None;
    }
    create_window(app, true);
}

fn save_items(app: &App) {
    let path = app.paths.root.join("clothesline.json");
    if let Ok(json) = serde_json::to_vec(&app.clothesline_items)
        && let Some(encrypted) = win::protect_for_user(&json)
    {
        let mut data = HISTORY_MAGIC.to_vec();
        data.extend_from_slice(&encrypted);
        let temp = path.with_extension("json.tmp");
        if std::fs::write(&temp, data).is_ok() {
            if std::fs::rename(temp, path).is_ok() {
                prune_media(app);
            }
        }
    }
}

fn prune_media(app: &App) {
    let keep: std::collections::HashSet<&str> = app
        .clothesline_items
        .iter()
        .filter_map(|item| {
            if let ClipboardItem::Image { media_file, .. } = item {
                Some(media_file.as_str())
            } else {
                None
            }
        })
        .collect();
    let dir = app.paths.root.join("clothesline-media");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !keep.contains(name.to_string_lossy().as_ref()) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn remove_item(state: &mut State, index: usize) {
    if index >= state.items.len() {
        return;
    }
    state.items.remove(index);
    state.page_start = state
        .page_start
        .min(state.items.len().saturating_sub(MAX_VISIBLE_ITEMS));
    if let Some(app) = unsafe { state.app.as_mut() } {
        app.clothesline_items = state.items.clone();
        save_items(app);
    }
}

fn clear_items(state: &mut State) {
    state.items.clear();
    state.page_start = 0;
    if let Some(app) = unsafe { state.app.as_mut() } {
        app.clothesline_items.clear();
        save_items(app);
    }
}

pub fn push_item(app: &mut App, text: String) {
    // Preserve clipboard contents byte-for-byte, including leading/trailing
    // whitespace. Reject pathological payloads instead of truncating them.
    if text.trim().is_empty() || text.len() > 10 * 1024 * 1024 || text.chars().count() > 2_000_000 {
        return;
    }
    push_item_kind(app, ClipboardItem::Text(text));
}

pub fn push_payload(app: &mut App, payload: win::ClipboardPayload) {
    match payload {
        win::ClipboardPayload::Text(text) => push_item(app, text),
        win::ClipboardPayload::Files(paths) => {
            if !paths.is_empty() {
                push_item_kind(app, ClipboardItem::Files(paths));
            }
        }
        win::ClipboardPayload::Image { format, bytes } => {
            if let Some(item) = store_image(app, format, &bytes) {
                push_item_kind(app, item);
            }
        }
    }
}

fn push_item_kind(app: &mut App, item: ClipboardItem) {
    app.clothesline_items.retain(|existing| existing != &item);
    app.clothesline_items.insert(0, item);
    if let Some(hwnd) = app.clothesline_hwnd {
        let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State };
        if !ptr.is_null() {
            unsafe {
                (*ptr).page_start = 0;
            }
        }
    }
    if app.clothesline_items.len() > MAX_ITEMS {
        app.clothesline_items.truncate(MAX_ITEMS);
    }
    save_items(app);
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

fn store_image(app: &App, format: u32, bytes: &[u8]) -> Option<ClipboardItem> {
    if bytes.len() < 40 || bytes.len() > 32 * 1024 * 1024 || unsafe { dib_info(bytes) }.is_none() {
        return None;
    }
    let encrypted = win::protect_for_user(bytes)?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    use std::hash::{Hash, Hasher};
    bytes.hash(&mut hasher);
    let media_file = format!("{:016x}.dib.dpapi", hasher.finish());
    let dir = app.paths.root.join("clothesline-media");
    std::fs::create_dir_all(&dir).ok()?;
    std::fs::write(dir.join(&media_file), encrypted).ok()?;
    Some(ClipboardItem::Image { media_file, format })
}

fn load_image(state: &State, media_file: &str) -> Option<Vec<u8>> {
    let app = unsafe { state.app.as_ref() }?;
    let path = app.paths.root.join("clothesline-media").join(media_file);
    if std::fs::metadata(&path).ok()?.len() > 40 * 1024 * 1024 {
        return None;
    }
    let encrypted = std::fs::read(path).ok()?;
    win::unprotect_for_user(&encrypted)
}

fn copy_item(item: &ClipboardItem, state: &State) {
    match item {
        ClipboardItem::Text(text) => {
            let _ = win::set_clipboard_text(text);
        }
        ClipboardItem::Files(paths) => {
            let _ = win::set_clipboard_files(paths);
        }
        ClipboardItem::Image { media_file, format } => {
            if let Some(bytes) = load_image(state, media_file) {
                let _ = win::set_clipboard_image(*format, &bytes);
            }
        }
    }
}

fn refresh_buttons(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
}

fn show(hwnd: HWND) {
    animate(hwnd, 0, 255, false);
}

fn hide(hwnd: HWND) {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State };
    let from = if ptr.is_null() {
        255
    } else {
        unsafe { (*ptr).alpha }
    };
    animate(hwnd, from, 0, true);
}

fn set_alpha(hwnd: HWND, alpha: u8) {
    unsafe {
        let _ = SetLayeredWindowAttributes(
            hwnd,
            windows::Win32::Foundation::COLORREF(COLOR_KEY),
            alpha,
            LWA_COLORKEY | LWA_ALPHA,
        );
    }
}

fn animate(hwnd: HWND, from_alpha: u8, to_alpha: u8, hide_when_done: bool) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = KillTimer(Some(hwnd), HIDE_TIMER);
        let _ = SetTimer(Some(hwnd), ANIMATE_TIMER, 10, None);
    }
    set_alpha(hwnd, from_alpha);
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State };
    if !ptr.is_null() {
        unsafe {
            (*ptr).alpha = from_alpha;
            (*ptr).animation = Some(Animation {
                started: Instant::now(),
                from_alpha,
                to_alpha,
                hide_when_done,
            });
            (*ptr).pointer_outside_since = None;
        }
    }
}

fn advance_animation(hwnd: HWND, state: &mut State) -> bool {
    let Some(animation) = state.animation.as_ref() else {
        return false;
    };
    let t = (animation.started.elapsed().as_millis() as f64 / ANIMATION_MS as f64).clamp(0.0, 1.0);
    let eased = 1.0 - (1.0 - t).powi(3);
    let alpha = animation.from_alpha as f64
        + (animation.to_alpha as f64 - animation.from_alpha as f64) * eased;
    let hide_when_done = animation.hide_when_done;
    state.alpha = alpha.round().clamp(0.0, 255.0) as u8;
    set_alpha(hwnd, state.alpha);
    if t >= 1.0 {
        state.animation = None;
        unsafe {
            let _ = KillTimer(Some(hwnd), ANIMATE_TIMER);
        }
        if hide_when_done {
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            state.pointer_outside_since = None;
        } else {
            unsafe {
                let _ = SetTimer(Some(hwnd), HIDE_TIMER, 100, None);
            }
        }
    }
    true
}

fn card_rect(index: usize, count: usize, width: i32) -> RECT {
    let gap = 8;
    let count = count.max(1) as i32;
    let card_width = ((width - 28 - gap * (count - 1)) / count).clamp(96, 220);
    let occupied = card_width * count + gap * (count - 1);
    let margin = (width - occupied) / 2;
    let left = margin + index as i32 * (card_width + gap);
    RECT {
        left,
        top: CARD_TOP,
        right: left + card_width,
        bottom: CARD_BOTTOM,
    }
}

fn interactive_at(hwnd: HWND, state: &State, screen: POINT) -> bool {
    let mut local = screen;
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut local);
    }
    let mut client = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut client);
    }
    let w = client.right;
    if local.y < 40 {
        if local.x >= w - 48 {
            return true;
        }
        if state.items.len() > MAX_VISIBLE_ITEMS
            && ((local.x >= w - 188 && local.x < w - 156)
                || (local.x >= w - 88 && local.x < w - 56))
        {
            return true;
        }
    }
    let visible = state.items.len().min(MAX_VISIBLE_ITEMS);
    (0..visible).any(|slot| {
        let r = card_rect(slot, visible, client.right);
        local.x >= r.left && local.x < r.right && local.y >= r.top && local.y < r.bottom
    })
}

unsafe fn paint_label(
    dc: windows::Win32::Graphics::Gdi::HDC,
    text: &str,
    rect: &mut RECT,
    color: windows::Win32::Foundation::COLORREF,
    flags: DRAW_TEXT_FORMAT,
) {
    let mut wide = win::wide(text);
    unsafe {
        let _ = SetTextColor(dc, color);
        let _ = SetBkMode(dc, TRANSPARENT);
        let _ = DrawTextW(dc, &mut wide, rect, flags);
    }
}

struct DibInfo {
    offset: usize,
    width: i32,
    height: i32,
    top_down: bool,
    bit_count: u16,
    compression: u32,
}

unsafe fn dib_info(bytes: &[u8]) -> Option<DibInfo> {
    if bytes.len() < std::mem::size_of::<BITMAPINFOHEADER>() {
        return None;
    }
    let header = unsafe { std::ptr::read_unaligned(bytes.as_ptr() as *const BITMAPINFOHEADER) };
    let width = header.biWidth.unsigned_abs() as i32;
    let height = header.biHeight.unsigned_abs() as i32;
    if header.biSize < 40
        || header.biSize as usize > bytes.len()
        || width == 0
        || height == 0
        || width > 60_000
        || height > 60_000
        || width as i64 * height as i64 > 60_000_000
    {
        return None;
    }
    let mut offset = header.biSize as usize;
    if header.biSize == 40 && header.biCompression == BI_BITFIELDS.0 {
        offset += 12;
    }
    if header.biBitCount <= 8 {
        let colors = if header.biClrUsed != 0 {
            header.biClrUsed as usize
        } else {
            1usize << header.biBitCount
        };
        offset = offset.checked_add(colors.checked_mul(4)?)?;
    }
    if offset >= bytes.len() {
        return None;
    }
    Some(DibInfo {
        offset,
        width,
        height,
        top_down: header.biHeight < 0,
        bit_count: header.biBitCount,
        compression: header.biCompression,
    })
}

/// Copies a 24/32-bit DIB into a clean bottom-up 32bpp `BI_RGB` buffer.
///
/// Clipboard producers disagree about negative heights, stride padding and
/// alpha, and `StretchDIBits` interprets each variant slightly differently.
/// Normalizing first keeps thumbnails free of stripe artifacts.
fn normalize_dib(bytes: &[u8], info: &DibInfo) -> Option<Vec<u8>> {
    if !matches!(info.bit_count, 24 | 32)
        || !(info.compression == 0 || info.compression == BI_BITFIELDS.0)
    {
        return None;
    }
    let pixel_bytes = usize::from(info.bit_count / 8);
    let width = info.width as usize;
    let height = info.height as usize;
    let source_stride = width.checked_mul(pixel_bytes)?.checked_add(3)? & !3;
    let source_len = info
        .offset
        .checked_add(source_stride.checked_mul(height)?)?;
    if bytes.len() < source_len {
        return None;
    }
    let target_stride = width.checked_mul(4)?;
    let image_len = target_stride.checked_mul(height)?;
    let mut out = vec![0u8; 40 + image_len];
    out[0..4].copy_from_slice(&40u32.to_le_bytes());
    out[4..8].copy_from_slice(&info.width.to_le_bytes());
    out[8..12].copy_from_slice(&(info.height).to_le_bytes());
    out[12..14].copy_from_slice(&1u16.to_le_bytes());
    out[14..16].copy_from_slice(&32u16.to_le_bytes());
    out[20..24].copy_from_slice(&(target_stride as u32).to_le_bytes());
    for y in 0..height {
        let source_row = if info.top_down { height - 1 - y } else { y };
        let source = info.offset + source_row * source_stride;
        let target = 40 + y * target_stride;
        for x in 0..width {
            let src = source + x * pixel_bytes;
            let dst = target + x * 4;
            out[dst] = bytes[src];
            out[dst + 1] = bytes[src + 1];
            out[dst + 2] = bytes[src + 2];
            out[dst + 3] = 0xFF;
        }
    }
    Some(out)
}

unsafe fn draw_dib(dc: windows::Win32::Graphics::Gdi::HDC, bytes: &[u8], target: RECT) -> bool {
    let Some(info) = (unsafe { dib_info(bytes) }) else {
        return false;
    };
    let normalized = normalize_dib(bytes, &info);
    let (bits_bytes, source_width, source_height, used_offset) = match normalized.as_deref() {
        Some(buffer) => (buffer, info.width, info.height, 40usize),
        None => (bytes, info.width, info.height, info.offset),
    };
    let available_width = (target.right - target.left).max(1);
    let available_height = (target.bottom - target.top).max(1);
    let scale = (available_width as f64 / source_width as f64)
        .min(available_height as f64 / source_height as f64);
    let draw_width = ((source_width as f64 * scale) as i32).max(1);
    let draw_height = ((source_height as f64 * scale) as i32).max(1);
    let x = target.left + (available_width - draw_width) / 2;
    let y = target.top + (available_height - draw_height) / 2;
    let mut aligned_info = vec![0u32; used_offset.div_ceil(std::mem::size_of::<u32>())];
    unsafe {
        std::ptr::copy_nonoverlapping(
            bits_bytes.as_ptr(),
            aligned_info.as_mut_ptr() as *mut u8,
            used_offset,
        );
        let bits = bits_bytes.as_ptr().add(used_offset);
        // Halftone downscaling keeps screenshot text legible instead of banded.
        let _ = SetStretchBltMode(dc, HALFTONE);
        let _ = SetBrushOrgEx(dc, x, y, None);
        let copied = StretchDIBits(
            dc,
            x,
            y,
            draw_width,
            draw_height,
            0,
            0,
            source_width,
            source_height,
            Some(bits as *const c_void),
            aligned_info.as_ptr() as *const BITMAPINFO,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
        copied > 0
    }
}

unsafe fn draw_file_glyph(dc: windows::Win32::Graphics::Gdi::HDC, target: RECT) {
    unsafe {
        let brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(0x00E5EAEC));
        let pen = CreatePen(
            PS_SOLID,
            1,
            windows::Win32::Foundation::COLORREF(0x0096A0A5),
        );
        let old_brush = SelectObject(dc, HGDIOBJ(brush.0));
        let old_pen = SelectObject(dc, HGDIOBJ(pen.0));
        let width = (target.right - target.left).min(42);
        let left = target.left + ((target.right - target.left) - width) / 2;
        let paper = RECT {
            left,
            top: target.top + 5,
            right: left + width,
            bottom: target.bottom - 5,
        };
        let _ = RoundRect(dc, paper.left, paper.top, paper.right, paper.bottom, 4, 4);
        let _ = SelectObject(dc, old_pen);
        let _ = SelectObject(dc, old_brush);
        let _ = DeleteObject(HGDIOBJ(pen.0));
        let _ = DeleteObject(HGDIOBJ(brush.0));
    }
}

fn paint(hwnd: HWND, state: &State) {
    const PAPER: u32 = 0x00F3FAFC;
    const PAPER_BORDER: u32 = 0x00C1D0D6;
    const SHADOW: u32 = 0x00CEDBE0;
    const HEADER: u32 = 0x00EFF8FB;
    const HEADER_BORDER: u32 = 0x00BBCBD2;
    const TEXT: u32 = 0x0034302A;
    const MUTED: u32 = 0x0080827D;
    const ACCENT: u32 = 0x009A793B;
    const ROPE: u32 = 0x00748B9A;
    const TEXT_STRIP: u32 = 0x00C8A88F;
    const IMAGE_STRIP: u32 = 0x008A9C5C;
    const FILES_STRIP: u32 = 0x00609ABF;

    let mut ps = PAINTSTRUCT::default();
    unsafe {
        let dc = windows::Win32::Graphics::Gdi::BeginPaint(hwnd, &mut ps);
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let width = client.right;

        // The unused canvas is color-keyed away; only the header plate and the
        // hanging paper slips should be visible over whatever is underneath.
        let bg = CreateSolidBrush(windows::Win32::Foundation::COLORREF(COLOR_KEY));
        FillRect(dc, &client, bg);
        let _ = DeleteObject(HGDIOBJ(bg.0));

        let old_font = SelectObject(
            dc,
            state
                .fonts
                .header
                .as_ref()
                .map(|font| font.obj())
                .unwrap_or_else(|| GetStockObject(DEFAULT_GUI_FONT)),
        );

        // Small paper header: title on the left, paging and close on the right.
        let header_shadow = CreateSolidBrush(windows::Win32::Foundation::COLORREF(0x00D3DCD6));
        let shadow_rect = RECT {
            left: 9,
            top: 6,
            right: width - 7,
            bottom: 34,
        };
        let _ = FillRect(dc, &shadow_rect, header_shadow);
        let _ = DeleteObject(HGDIOBJ(header_shadow.0));
        let header = CreateSolidBrush(windows::Win32::Foundation::COLORREF(HEADER));
        let header_pen = CreatePen(
            PS_SOLID,
            1,
            windows::Win32::Foundation::COLORREF(HEADER_BORDER),
        );
        let old_brush = SelectObject(dc, HGDIOBJ(header.0));
        let old_pen = SelectObject(dc, HGDIOBJ(header_pen.0));
        let _ = RoundRect(dc, 8, 4, width - 8, 32, 7, 7);
        let _ = SelectObject(dc, old_pen);
        let _ = SelectObject(dc, old_brush);
        let _ = DeleteObject(HGDIOBJ(header_pen.0));
        let _ = DeleteObject(HGDIOBJ(header.0));

        let has_pages = state.items.len() > MAX_VISIBLE_ITEMS;
        let mut title = RECT {
            left: 16,
            top: 6,
            right: 84,
            bottom: 30,
        };
        paint_label(
            dc,
            "TENDEDERO",
            &mut title,
            windows::Win32::Foundation::COLORREF(TEXT),
            DT_SINGLELINE | DT_VCENTER,
        );
        let _ = SelectObject(
            dc,
            state
                .fonts
                .small
                .as_ref()
                .map(|font| font.obj())
                .unwrap_or_else(|| GetStockObject(DEFAULT_GUI_FONT)),
        );
        let mut count = RECT {
            left: 84,
            top: 6,
            right: if has_pages { width - 200 } else { width - 56 },
            bottom: 30,
        };
        paint_label(
            dc,
            &format!("· {}", state.items.len()),
            &mut count,
            windows::Win32::Foundation::COLORREF(MUTED),
            DT_SINGLELINE | DT_VCENTER,
        );

        if has_pages {
            let mut previous = RECT {
                left: width - 184,
                top: 6,
                right: width - 160,
                bottom: 30,
            };
            let mut next = RECT {
                left: width - 84,
                top: 6,
                right: width - 60,
                bottom: 30,
            };
            let mut page = RECT {
                left: width - 160,
                top: 6,
                right: width - 88,
                bottom: 30,
            };
            let range = format!(
                "{}-{}/{}",
                state.page_start + 1,
                (state.page_start + MAX_VISIBLE_ITEMS).min(state.items.len()),
                state.items.len()
            );
            paint_label(
                dc,
                "<",
                &mut previous,
                windows::Win32::Foundation::COLORREF(TEXT),
                DT_SINGLELINE | DT_VCENTER | DT_CENTER,
            );
            paint_label(
                dc,
                &range,
                &mut page,
                windows::Win32::Foundation::COLORREF(MUTED),
                DT_SINGLELINE | DT_VCENTER | DT_RIGHT,
            );
            paint_label(
                dc,
                ">",
                &mut next,
                windows::Win32::Foundation::COLORREF(TEXT),
                DT_SINGLELINE | DT_VCENTER | DT_CENTER,
            );
        }
        let _ = SelectObject(
            dc,
            state
                .fonts
                .body
                .as_ref()
                .map(|font| font.obj())
                .unwrap_or_else(|| GetStockObject(DEFAULT_GUI_FONT)),
        );
        let mut close = RECT {
            left: width - 48,
            top: 5,
            right: width - 12,
            bottom: 31,
        };
        paint_label(
            dc,
            "×",
            &mut close,
            windows::Win32::Foundation::COLORREF(MUTED),
            DT_SINGLELINE | DT_VCENTER | DT_CENTER,
        );

        // Rope, then cards, then pegs on top so the clips overlap the paper.
        let count = state.items.len().min(MAX_VISIBLE_ITEMS);
        let rope = CreatePen(PS_SOLID, 1, windows::Win32::Foundation::COLORREF(ROPE));
        let old_pen = SelectObject(dc, HGDIOBJ(rope.0));
        let _ = MoveToEx(dc, 18, 39, None);
        for step in 1..=40 {
            let t = step as f64 / 40.0;
            let x = 18 + ((width - 36) as f64 * t) as i32;
            let sag = (std::f64::consts::PI * t).sin();
            let y = 39 + (sag * 3.5) as i32;
            let _ = LineTo(dc, x, y);
        }
        let _ = SelectObject(dc, old_pen);
        let _ = DeleteObject(HGDIOBJ(rope.0));

        if count == 0 {
            let card_width = (width - 120).clamp(180, 320);
            let left = (width - card_width) / 2;
            let empty_pen = CreatePen(
                PS_SOLID,
                1,
                windows::Win32::Foundation::COLORREF(PAPER_BORDER),
            );
            let old = SelectObject(dc, HGDIOBJ(empty_pen.0));
            let null_brush = GetStockObject(NULL_BRUSH);
            let old_brush = SelectObject(dc, null_brush);
            let _ = RoundRect(dc, left, CARD_TOP, left + card_width, CARD_BOTTOM, 6, 6);
            let _ = SelectObject(dc, old_brush);
            let _ = SelectObject(dc, old);
            let _ = DeleteObject(HGDIOBJ(empty_pen.0));
            let _ = SelectObject(
                dc,
                state
                    .fonts
                    .body
                    .as_ref()
                    .map(|font| font.obj())
                    .unwrap_or_else(|| GetStockObject(DEFAULT_GUI_FONT)),
            );
            let mut empty = RECT {
                left: left + 10,
                top: CARD_TOP,
                right: left + card_width - 10,
                bottom: CARD_BOTTOM,
            };
            paint_label(
                dc,
                "Copia texto, una captura o archivos",
                &mut empty,
                windows::Win32::Foundation::COLORREF(MUTED),
                DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_END_ELLIPSIS,
            );
        }

        for slot in 0..count {
            let item_index = state.page_start + slot;
            let Some(item) = state.items.get(item_index) else {
                continue;
            };
            let r = card_rect(slot, count, width);
            let shadow_brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(SHADOW));
            let old = SelectObject(dc, HGDIOBJ(shadow_brush.0));
            let _ = RoundRect(dc, r.left + 2, r.top + 3, r.right + 2, r.bottom + 3, 6, 6);
            let _ = SelectObject(dc, old);
            let _ = DeleteObject(HGDIOBJ(shadow_brush.0));

            let paper = CreateSolidBrush(windows::Win32::Foundation::COLORREF(PAPER));
            let border = CreatePen(
                PS_SOLID,
                1,
                windows::Win32::Foundation::COLORREF(PAPER_BORDER),
            );
            let old_brush = SelectObject(dc, HGDIOBJ(paper.0));
            let old_card_pen = SelectObject(dc, HGDIOBJ(border.0));
            let _ = RoundRect(dc, r.left, r.top, r.right, r.bottom, 6, 6);
            let _ = SelectObject(dc, old_card_pen);
            let _ = SelectObject(dc, old_brush);
            let _ = DeleteObject(HGDIOBJ(border.0));
            let _ = DeleteObject(HGDIOBJ(paper.0));

            let (strip_color, tag) = match item {
                ClipboardItem::Text(_) => (TEXT_STRIP, "TEXTO"),
                ClipboardItem::Image { .. } => (IMAGE_STRIP, "CAPTURA"),
                ClipboardItem::Files(paths) => (
                    FILES_STRIP,
                    if paths.len() == 1 {
                        "ARCHIVO"
                    } else {
                        "ARCHIVOS"
                    },
                ),
            };
            let strip = CreateSolidBrush(windows::Win32::Foundation::COLORREF(strip_color));
            let strip_rect = RECT {
                left: r.left + 1,
                top: r.top + 7,
                right: r.left + 4,
                bottom: r.bottom - 7,
            };
            let _ = FillRect(dc, &strip_rect, strip);
            let _ = DeleteObject(HGDIOBJ(strip.0));

            match item {
                ClipboardItem::Image { media_file, .. } => {
                    let image_rect = RECT {
                        left: r.left + 7,
                        top: r.top + 5,
                        right: r.right - 5,
                        bottom: r.bottom - 5,
                    };
                    if let Some(dib) = load_image(state, media_file) {
                        if !draw_dib(dc, &dib, image_rect) {
                            draw_file_glyph(dc, image_rect);
                        }
                    } else {
                        draw_file_glyph(dc, image_rect);
                    }
                }
                _ => {
                    let _ = SelectObject(
                        dc,
                        state
                            .fonts
                            .body
                            .as_ref()
                            .map(|font| font.obj())
                            .unwrap_or_else(|| GetStockObject(DEFAULT_GUI_FONT)),
                    );
                    let mut text = RECT {
                        left: r.left + 10,
                        top: r.top + 6,
                        right: r.right - 6,
                        bottom: r.bottom - 19,
                    };
                    let label = item.display_label();
                    paint_label(
                        dc,
                        &label,
                        &mut text,
                        windows::Win32::Foundation::COLORREF(TEXT),
                        DT_WORDBREAK | DT_END_ELLIPSIS,
                    );
                    let _ = SelectObject(
                        dc,
                        state
                            .fonts
                            .small
                            .as_ref()
                            .map(|font| font.obj())
                            .unwrap_or_else(|| GetStockObject(DEFAULT_GUI_FONT)),
                    );
                    let detail = match item {
                        ClipboardItem::Files(paths) => format!("{tag} · {}", paths.len()),
                        _ => tag.to_string(),
                    };
                    let mut tag_rect = RECT {
                        left: r.left + 10,
                        top: r.bottom - 18,
                        right: r.right - 6,
                        bottom: r.bottom - 4,
                    };
                    paint_label(
                        dc,
                        &detail,
                        &mut tag_rect,
                        windows::Win32::Foundation::COLORREF(MUTED),
                        DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
                    );
                }
            }

            let peg_brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(ACCENT));
            let old = SelectObject(dc, HGDIOBJ(peg_brush.0));
            let x = (r.left + r.right) / 2;
            let _ = RoundRect(dc, x - 3, 35, x + 3, 51, 3, 3);
            let _ = SelectObject(dc, old);
            let _ = DeleteObject(HGDIOBJ(peg_brush.0));
        }

        let _ = SelectObject(dc, old_font);
        let _ = EndPaint(hwnd, &ps);
    }
}

fn foreground_is_fullscreen() -> bool {
    let foreground = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    if foreground.0.is_null() {
        return false;
    }
    let Some(monitor) = win::monitor_for_window(foreground) else {
        return false;
    };
    let mut rect = RECT::default();
    unsafe {
        let _ = GetWindowRect(foreground, &mut rect);
    }
    let bounds = monitor.bounds;
    let tolerance = 2;
    rect.left <= bounds.left + tolerance
        && rect.top <= bounds.top + tolerance
        && rect.right >= bounds.right - tolerance
        && rect.bottom >= bounds.bottom - tolerance
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
            let _ = s;
            LRESULT(0)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_LBUTTONDOWN => {
            let x = (lparam.0 as u32 & 0xffff) as u16 as i16 as i32;
            let y = ((lparam.0 as u32 >> 16) & 0xffff) as u16 as i16 as i32;
            let mut client = RECT::default();
            unsafe {
                let _ = GetClientRect(hwnd, &mut client);
            }
            let st = unsafe { &mut *pointer };
            if st.items.len() > MAX_VISIBLE_ITEMS
                && y < 40
                && x >= client.right - 188
                && x < client.right - 156
            {
                st.page_start = st.page_start.saturating_sub(MAX_VISIBLE_ITEMS);
                unsafe {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            } else if st.items.len() > MAX_VISIBLE_ITEMS
                && y < 40
                && x >= client.right - 88
                && x < client.right - 56
            {
                st.page_start =
                    (st.page_start + MAX_VISIBLE_ITEMS).min(st.items.len() - MAX_VISIBLE_ITEMS);
                unsafe {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            } else if y < 40 && x >= client.right - 48 {
                hide(hwnd);
            } else {
                let mut card_hit = false;
                let visible = st.items.len().min(MAX_VISIBLE_ITEMS);
                for slot in 0..visible {
                    let i = st.page_start + slot;
                    let Some(item) = st.items.get(i) else {
                        continue;
                    };
                    let r = card_rect(slot, visible, client.right);
                    if x >= r.left && x < r.right && y >= r.top && y < r.bottom {
                        copy_item(item, st);
                        card_hit = true;
                        break;
                    }
                }
                // Clicking the translucent shelf backdrop means the user is
                // returning to the app underneath; tuck it away immediately.
                if !card_hit {
                    hide(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == ANIMATE_TIMER => {
            let st = unsafe { &mut *pointer };
            let _ = advance_animation(hwnd, st);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == HIDE_TIMER => {
            if unsafe { (*pointer).animation.is_some() || (*pointer).pinned } {
                return LRESULT(0);
            }
            if foreground_is_fullscreen() {
                hide(hwnd);
                return LRESULT(0);
            }
            let mut cursor = POINT::default();
            let mut r = RECT::default();
            unsafe {
                let _ = GetCursorPos(&mut cursor);
                let _ = GetWindowRect(hwnd, &mut r);
            }
            let st = unsafe { &mut *pointer };
            if interactive_at(hwnd, st, cursor) {
                st.pointer_outside_since = None;
            } else {
                let since = st.pointer_outside_since.get_or_insert_with(Instant::now);
                // Give the user time to move from the screen edge onto the shelf.
                if since.elapsed().as_millis() >= 850 {
                    hide(hwnd);
                    st.pointer_outside_since = None;
                }
            }
            LRESULT(0)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_NCHITTEST => {
            let screen = POINT {
                x: lparam.0 as u32 as u16 as i16 as i32,
                y: (lparam.0 as u32 >> 16) as u16 as i16 as i32,
            };
            if interactive_at(hwnd, unsafe { &*pointer }, screen) {
                LRESULT(windows::Win32::UI::WindowsAndMessaging::HTCLIENT as isize)
            } else {
                // Let browser tabs, app controls, and the desktop receive clicks
                // through the otherwise translucent shelf background.
                LRESULT(windows::Win32::UI::WindowsAndMessaging::HTTRANSPARENT as isize)
            }
        }
        windows::Win32::UI::WindowsAndMessaging::WM_PAINT => {
            paint(hwnd, unsafe { &*pointer });
            LRESULT(0)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_CONTEXTMENU => {
            unsafe {
                use windows::Win32::Graphics::Gdi::ScreenToClient;
                use windows::Win32::UI::WindowsAndMessaging::{
                    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, MF_SEPARATOR,
                    MF_STRING, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu,
                };
                let menu = CreatePopupMenu().unwrap_or_default();
                let mut point = POINT::default();
                if lparam.0 as i16 == -1 && (lparam.0 >> 16) as i16 == -1 {
                    let _ = GetCursorPos(&mut point);
                } else {
                    point.x = lparam.0 as u32 as u16 as i16 as i32;
                    point.y = (lparam.0 as u32 >> 16) as u16 as i16 as i32;
                }
                let mut local = point;
                let _ = ScreenToClient(hwnd, &mut local);
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                let st = &*pointer;
                let count = st.items.len().min(MAX_VISIBLE_ITEMS);
                let slot = (0..count).find(|slot| {
                    let r = card_rect(*slot, count, client.right);
                    local.x >= r.left && local.x < r.right && local.y >= r.top && local.y < r.bottom
                });
                let index = slot.map(|slot| st.page_start + slot);
                let clear_text = win::wide("Borrar todo el historial");
                if let Some(i) = index {
                    let copy_text = win::wide("Copiar");
                    let remove_text = win::wide("Quitar del tendedero");
                    let _ = AppendMenuW(menu, MF_STRING, 1, win::pcw(&copy_text));
                    let _ = AppendMenuW(menu, MF_STRING, 2, win::pcw(&remove_text));
                    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
                    let _ = AppendMenuW(menu, MF_STRING, 3, win::pcw(&clear_text));
                    let cmd = TrackPopupMenu(
                        menu,
                        TPM_RETURNCMD | TPM_RIGHTBUTTON,
                        point.x,
                        point.y,
                        None,
                        hwnd,
                        None,
                    );
                    let _ = DestroyMenu(menu);
                    if !win::is_window(hwnd) {
                        return LRESULT(0);
                    }
                    let current = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
                    if current.is_null() {
                        return LRESULT(0);
                    }
                    let st = &mut *current;
                    match cmd.0 {
                        1 => {
                            if let Some(item) = st.items.get(i).cloned() {
                                copy_item(&item, st);
                            }
                        }
                        2 => {
                            remove_item(st, i);
                            refresh_buttons(hwnd);
                        }
                        3 => {
                            clear_items(st);
                            refresh_buttons(hwnd);
                        }
                        _ => {}
                    }
                } else {
                    let _ = AppendMenuW(menu, MF_STRING, 3, win::pcw(&clear_text));
                    let cmd = TrackPopupMenu(
                        menu,
                        TPM_RETURNCMD | TPM_RIGHTBUTTON,
                        point.x,
                        point.y,
                        None,
                        hwnd,
                        None,
                    );
                    let _ = DestroyMenu(menu);
                    if !win::is_window(hwnd) {
                        return LRESULT(0);
                    }
                    let current = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
                    if current.is_null() {
                        return LRESULT(0);
                    }
                    let st = &mut *current;
                    if cmd.0 == 3 {
                        clear_items(st);
                        refresh_buttons(hwnd);
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
            let _ = KillTimer(Some(hwnd), HIDE_TIMER);
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

pub extern "system" fn edge_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == windows::Win32::UI::WindowsAndMessaging::WM_NCCREATE {
        let app = unsafe { (*(lparam.0 as *const CREATESTRUCTW)).lpCreateParams as *mut App };
        if !app.is_null() {
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, app as isize);
            }
        }
        return LRESULT(1);
    }
    let app = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App };
    if msg == windows::Win32::UI::WindowsAndMessaging::WM_MOUSEMOVE {
        if !app.is_null() && !foreground_is_fullscreen() {
            unsafe {
                reveal_from_edge(&mut *app);
            }
        }
        return LRESULT(0);
    }
    if msg == windows::Win32::UI::WindowsAndMessaging::WM_NCDESTROY {
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dib_header_is_bounded_before_thumbnail_rendering() {
        let mut dib = vec![0u8; 56];
        dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&2i32.to_le_bytes());
        dib[8..12].copy_from_slice(&2i32.to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&32u16.to_le_bytes());
        dib[16..20].copy_from_slice(&0u32.to_le_bytes());
        dib[20..24].copy_from_slice(&16u32.to_le_bytes());
        let info = unsafe { dib_info(&dib) }.expect("valid header");
        assert_eq!((info.offset, info.width, info.height), (40, 2, 2));
        assert!(!info.top_down);
        assert_eq!(info.bit_count, 32);
        dib[4..8].copy_from_slice(&i32::MAX.to_le_bytes());
        assert!(unsafe { dib_info(&dib) }.is_none());
    }

    #[test]
    fn top_down_dib_normalizes_to_bottom_up_with_opaque_alpha() {
        let mut dib = vec![0u8; 56];
        dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&2i32.to_le_bytes());
        dib[8..12].copy_from_slice(&(-2i32).to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&32u16.to_le_bytes());
        // Row 0 (top) is red, row 1 (bottom) is blue. Alpha bytes are zero on
        // purpose: opaque output must not depend on producer alpha.
        dib[40..48].copy_from_slice(&[0, 0, 255, 0, 0, 0, 255, 0]);
        dib[48..56].copy_from_slice(&[255, 0, 0, 0, 255, 0, 0, 0]);
        let info = unsafe { dib_info(&dib) }.expect("valid header");
        let out = normalize_dib(&dib, &info).expect("normalized");
        assert_eq!(&out[0..4], &40u32.to_le_bytes());
        assert_eq!(&out[8..12], &2i32.to_le_bytes());
        // Bottom-up: first stored row is the bottom (blue) and alpha is 255.
        assert_eq!(&out[40..44], &[255, 0, 0, 255]);
        assert_eq!(&out[48..52], &[0, 0, 255, 255]);
    }

    #[test]
    fn padded_24bpp_rows_are_copied_without_stripe_artifacts() {
        let width = 3usize;
        let height = 2usize;
        let stride = 12usize;
        let mut dib = vec![0u8; 40 + stride * height];
        dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&(width as i32).to_le_bytes());
        dib[8..12].copy_from_slice(&(height as i32).to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&24u16.to_le_bytes());
        for x in 0..width {
            dib[40 + x * 3] = (x as u8 + 1) * 10;
            dib[40 + stride + x * 3] = (x as u8 + 1) * 20;
        }
        let info = unsafe { dib_info(&dib) }.expect("valid header");
        let out = normalize_dib(&dib, &info).expect("normalized");
        let first = 40;
        assert_eq!(&out[first..first + 4], &[10, 0, 0, 255]);
        assert_eq!(&out[first + 4..first + 8], &[20, 0, 0, 255]);
        assert_eq!(&out[first + 8..first + 12], &[30, 0, 0, 255]);
        let second = first + 12;
        assert_eq!(&out[second..second + 4], &[20, 0, 0, 255]);
    }

    #[test]
    fn valid_dib_can_be_drawn_into_a_memory_surface() {
        let mut dib = vec![0u8; 56];
        dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&2i32.to_le_bytes());
        dib[8..12].copy_from_slice(&2i32.to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&32u16.to_le_bytes());
        dib[16..20].copy_from_slice(&0u32.to_le_bytes());
        dib[20..24].copy_from_slice(&16u32.to_le_bytes());
        dib[40..].copy_from_slice(&[
            0x20, 0x80, 0xE0, 0xFF, 0xE0, 0x80, 0x20, 0xFF, 0x40, 0x90, 0xE0, 0xFF, 0xE0, 0x90,
            0x40, 0xFF,
        ]);
        let surface = crate::win::MemoryDc::new(32, 32).expect("GDI memory surface");
        assert!(unsafe {
            draw_dib(
                surface.dc(),
                &dib,
                RECT {
                    left: 0,
                    top: 0,
                    right: 32,
                    bottom: 24,
                },
            )
        });
    }

    #[test]
    fn history_item_kinds_round_trip_through_json() {
        let items = vec![
            ClipboardItem::Text(" spaced text ".to_string()),
            ClipboardItem::Image {
                media_file: "image.dib.dpapi".to_string(),
                format: 17,
            },
            ClipboardItem::Files(vec![r"C:\notes\one.txt".to_string()]),
        ];
        let encoded = serde_json::to_vec(&items).unwrap();
        let decoded: Vec<ClipboardItem> = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, items);
        assert_eq!(decoded[1].display_label(), "Captura de pantalla");
    }
}

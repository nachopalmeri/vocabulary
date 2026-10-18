//! The compact card: a borderless, non-activating popup painted with GDI.
//!
//! The card never takes keyboard focus (`WS_EX_NOACTIVATE` + `SW_SHOWNOACTIVATE`),
//! is dragged with a manual mouse capture, and is repainted only when its
//! content, size, position or theme changes.

use crate::app::{Action, App};
use crate::model::Config;
use crate::theme;
use crate::win::{self, Font, MemoryDc, Pen};
use std::ffi::c_void;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreatePen, CreateSolidBrush, DT_END_ELLIPSIS, DT_NOPREFIX, DT_SINGLELINE,
    DT_VCENTER, DT_WORDBREAK, DrawTextW, EndPaint, FillRect, GetStockObject, HBRUSH, HDC,
    NULL_BRUSH, PAINTSTRUCT, PS_SOLID, Polygon, Polyline, RoundRect, SelectObject, SetBkMode,
    SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CreateWindowExW, DefWindowProcW, GWLP_USERDATA, GetWindowLongPtrW,
    GetWindowRect, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos, WINDOW_EX_STYLE,
    WM_CAPTURECHANGED, WM_CLOSE, WM_DPICHANGED, WM_ERASEBKGND, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_NCCREATE, WM_NCDESTROY, WM_PAINT, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

/// Height and width are expressed in device-independent pixels.
const PAD: i32 = 6;
const PAD_BOTTOM: i32 = 4;
const GAP: i32 = 2;
const TOOLBAR: i32 = 18;
const BUTTON: i32 = 18;
const BUTTON_GAP: i32 = 2;
const GROUP_GAP: i32 = 8;
const CORNER_DIP: i32 = 10;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CardButton {
    Previous,
    Next,
    Pause,
    Learned,
    Again,
    Details,
    Hide,
}

const BUTTONS: [CardButton; 7] = [
    CardButton::Previous,
    CardButton::Next,
    CardButton::Pause,
    CardButton::Learned,
    CardButton::Again,
    CardButton::Details,
    CardButton::Hide,
];

/// Device-pixel rectangles for the current card size.
#[derive(Clone, Debug, Default)]
pub struct CardLayout {
    pub word: RECT,
    pub meaning: RECT,
    pub meta: RECT,
    pub buttons: Vec<(CardButton, RECT)>,
}

#[derive(Default)]
pub struct CardState {
    pub hwnd: Option<HWND>,
    pub dpi: u32,
    pub fonts: Option<CardFonts>,
    pub buffer: Option<MemoryDc>,
    pub pressed: Option<CardButton>,
    pub dragging: bool,
    pub drag_anchor: (i32, i32),
    pub drag_origin: (i32, i32),
    pub layout: CardLayout,
}

pub struct CardFonts {
    word: Option<Font>,
    meaning: Option<Font>,
    meta: Option<Font>,
}

/// Card size in DIP for the current font size and width setting.
pub fn card_size_dip(config: &Config) -> (i32, i32) {
    let font = config.font_size_pt as i32;
    let word_height = font * 135 / 100;
    let meaning_pt = (font * 62 / 100).max(10);
    let meaning_height = (meaning_pt * 135 / 100) * 2; // up to two lines
    let meta_pt = (font * 45 / 100).max(8);
    let meta_height = meta_pt * 135 / 100;
    let height =
        PAD + word_height + GAP + meaning_height + GAP + meta_height + 3 + TOOLBAR + PAD_BOTTOM;
    (config.card_width_dip, height.max(60))
}

fn layout_for(width: i32, height: i32, dpi: u32) -> CardLayout {
    let scale = |value: i32| win::dip(value, dpi);
    let pad = scale(PAD);
    let word_height = scale(24);
    let meaning_height = scale(28);
    let meta_height = scale(10);

    let word_top = scale(PAD);
    let meaning_top = word_top + word_height + scale(GAP);
    let meta_top = meaning_top + meaning_height + scale(GAP);
    let toolbar_top = height - scale(PAD_BOTTOM) - scale(TOOLBAR);

    let mut buttons = Vec::with_capacity(BUTTONS.len());
    let mut x = pad;
    let mut previous: Option<CardButton> = None;
    for button in BUTTONS {
        if button == CardButton::Hide {
            break;
        }
        // Groups: [prev next] [pause] [learned again] [details]
        let grouped = matches!(
            (previous, button),
            (Some(CardButton::Next), CardButton::Pause)
                | (Some(CardButton::Pause), CardButton::Learned)
                | (Some(CardButton::Again), CardButton::Details)
        );
        if grouped {
            x += scale(GROUP_GAP);
        }
        buttons.push((
            button,
            RECT {
                left: x,
                top: toolbar_top,
                right: x + scale(BUTTON),
                bottom: toolbar_top + scale(BUTTON),
            },
        ));
        x += scale(BUTTON) + scale(BUTTON_GAP);
        previous = Some(button);
    }
    let hide_left = width - pad - scale(BUTTON);
    buttons.push((
        CardButton::Hide,
        RECT {
            left: hide_left,
            top: toolbar_top,
            right: hide_left + scale(BUTTON),
            bottom: toolbar_top + scale(BUTTON),
        },
    ));
    // The text column stops before the last button of the toolbar.
    let text_right = width - pad;

    CardLayout {
        word: RECT {
            left: pad,
            top: word_top,
            right: text_right,
            bottom: word_top + word_height,
        },
        meaning: RECT {
            left: pad,
            top: meaning_top,
            right: text_right,
            bottom: meaning_top + meaning_height,
        },
        meta: RECT {
            left: pad,
            top: meta_top,
            right: text_right,
            bottom: meta_top + meta_height,
        },
        buttons,
    }
}

/// Recreates the three card fonts (after a DPI or font-size change).
fn ensure_fonts(state: &mut CardState, config: &Config, dpi: u32) {
    let needs_rebuild = match &state.fonts {
        None => true,
        Some(_) => state.dpi != dpi,
    };
    if !needs_rebuild {
        return;
    }
    let family = crate::model::UI_FONT_FAMILY;
    let base = config.font_size_pt as f32;
    state.fonts = Some(CardFonts {
        word: win::create_font(family, base, dpi, true),
        meaning: win::create_font(family, (base * 0.62).max(10.0), dpi, false),
        meta: win::create_font(family, (base * 0.45).max(8.0), dpi, false),
    });
    state.dpi = dpi;
}

/// Drops the cached fonts so the next paint rebuilds them.
pub fn reset_fonts(state: &mut CardState) {
    state.fonts = None;
}

/// Creates the card window (hidden; `apply_placement` + `show` are separate).
pub fn create(app: &mut App, instance: HINSTANCE) -> Option<HWND> {
    let class = win::wide(crate::ui::card::CARD_CLASS);
    let title = win::wide("WordDock");
    let (width_dip, height_dip) = card_size_dip(&app.config);
    let dpi = app.card.dpi.max(96);
    // SAFETY: the class is registered by `register_class` before this call, and
    // `app` outlives the window (both are owned by main()).
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_NOACTIVATE
                | WS_EX_TOOLWINDOW
                | if app.config.always_on_top {
                    WS_EX_TOPMOST
                } else {
                    WINDOW_EX_STYLE(0)
                },
            win::pcw(&class),
            win::pcw(&title),
            WS_POPUP,
            0,
            0,
            win::dip(width_dip, dpi),
            win::dip(height_dip, dpi),
            None,
            None,
            Some(instance),
            Some(app as *mut App as *const c_void),
        )
    }
    .ok()?;
    app.card.hwnd = Some(hwnd);
    app.card.dpi = dpi;
    Some(hwnd)
}

pub const CARD_CLASS: &str = "WordDockCard";

pub fn class() -> crate::ui::ClassSpec {
    crate::ui::ClassSpec {
        name: CARD_CLASS,
        proc: Some(card_proc),
        cursor: crate::ui::CURSOR_ARROW,
        background: windows::Win32::Graphics::Gdi::HBRUSH::default(),
    }
}

/// Sizes, positions, clamps and re-themes the card.
pub fn apply_placement(app: &mut App) {
    let Some(hwnd) = app.card.hwnd else {
        return;
    };
    let dpi = win::dpi_of_window(hwnd).max(96);
    if app.card.dpi != dpi {
        reset_fonts(&mut app.card);
    }
    app.card.dpi = dpi;

    let (width_dip, height_dip) = card_size_dip(&app.config);
    let width = win::dip(width_dip, dpi);
    let height = win::dip(height_dip, dpi);

    // The stored monitor is used when it still exists; otherwise the monitor the
    // card is currently on, and finally the primary monitor.
    let stored = app
        .config
        .position
        .as_ref()
        .and_then(|position| win::monitor_by_device(&position.monitor));
    let monitor = stored
        .clone()
        .or_else(|| win::monitor_for_window(hwnd))
        .or_else(|| win::monitor_by_device(""));
    let (work, device) = match monitor {
        Some(info) => (info.work, info.device),
        None => (win::primary_work_area(), String::new()),
    };
    let stored_on_this_monitor = matches!(
        (&app.config.position, &stored),
        (Some(position), Some(info)) if !info.device.is_empty() && position.monitor == info.device
    );

    let (mut x, mut y) = if stored_on_this_monitor {
        let position = app.config.position.as_ref().expect("checked above");
        (
            work.left + win::dip(position.x_dip, dpi),
            work.top + win::dip(position.y_dip, dpi),
        )
    } else {
        // Default placement: centred horizontally, just above the taskbar.
        (
            work.left + (work.right - work.left - width) / 2,
            work.bottom - height - win::dip(8, dpi),
        )
    };

    // Always keep the card inside the work area of its monitor.
    x = x.clamp(work.left, (work.right - width).max(work.left));
    y = y.clamp(work.top, (work.bottom - height).max(work.top));

    win::set_window_rect_no_activate(hwnd, x, y, width, height);
    win::set_topmost(hwnd, app.config.always_on_top);
    win::set_rounded_window_region(hwnd, win::dip(CORNER_DIP, dpi));
    if app.config.position.is_none() {
        app.config.position = Some(crate::model::CardPosition {
            monitor: device,
            x_dip: win::dip_to_dip(x - work.left, dpi),
            y_dip: win::dip_to_dip(y - work.top, dpi),
        });
    }
    invalidate(hwnd);
}

pub fn invalidate(hwnd: HWND) {
    // SAFETY: invalidating an existing window; the repaint is coalesced by Windows.
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
    }
}

/// `WM_DPICHANGED` for the card window: resize and repaint at the new scale.
pub fn on_dpi_changed(app: &mut App, wparam: WPARAM, lparam: LPARAM) {
    let Some(hwnd) = app.card.hwnd else {
        return;
    };
    let dpi = (wparam.0 & 0xFFFF) as u32;
    let suggested = lparam.0 as *const RECT;
    if suggested.is_null() {
        return;
    }
    // SAFETY: Windows passes a valid RECT pointer with WM_DPICHANGED.
    let rect = unsafe { *suggested };
    reset_fonts(&mut app.card);
    app.card.dpi = dpi.max(96);
    apply_placement(app);
    // SAFETY: repositioning the card to the rectangle Windows suggested.
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOACTIVATE | SWP_NOZORDER,
        );
    }
    invalidate(hwnd);
}

fn hit_test(app: &App, point: POINT) -> Option<CardButton> {
    app.card
        .layout
        .buttons
        .iter()
        .find(|(_, rect)| contains(rect, point))
        .map(|(button, _)| *button)
}

fn contains(rect: &RECT, point: POINT) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}

fn cursor_point(lparam: LPARAM) -> POINT {
    POINT {
        x: (lparam.0 & 0xFFFF) as i16 as i32,
        y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
    }
}

// ---------------------------------------------------------------------------
// Painting
// ---------------------------------------------------------------------------

fn paint(app: &mut App, hwnd: HWND) {
    let mut paint = PAINTSTRUCT::default();
    // SAFETY: BeginPaint/EndPaint bracket drawing for an existing window.
    let hdc = unsafe { BeginPaint(hwnd, &mut paint) };
    let client = win::client_rect(hwnd);
    let width = client.right.max(1);
    let height = client.bottom.max(1);

    let dpi = app.card.dpi;
    ensure_fonts(&mut app.card, &app.config, dpi);
    let rebuild = match &app.card.buffer {
        Some(buffer) => buffer.width() != width || buffer.height() != height,
        None => true,
    };
    if rebuild {
        app.card.buffer = MemoryDc::new(width, height);
    }
    app.card.layout = layout_for(width, height, app.card.dpi);
    let layout = app.card.layout.clone();
    let content = app.card_content();
    let palette = theme::palette(app.config.theme);

    if let Some(buffer) = &app.card.buffer {
        let dc = buffer.dc();
        let fonts = app.card.fonts.as_ref();
        draw(
            dc,
            &layout,
            &content,
            palette,
            fonts,
            app.card.pressed,
            width,
            height,
        );
        buffer.blit_to(hdc);
    }
    // SAFETY: matches the BeginPaint above.
    unsafe {
        let _ = EndPaint(hwnd, &paint);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw(
    dc: HDC,
    layout: &CardLayout,
    content: &crate::app::CardContent,
    palette: theme::Palette,
    fonts: Option<&CardFonts>,
    pressed: Option<CardButton>,
    width: i32,
    height: i32,
) {
    // SAFETY: every GDI object created here is deleted before returning, and the
    // DC belongs to the back buffer for the duration of the call.
    unsafe {
        SetBkMode(dc, TRANSPARENT);

        // Background + border.
        let background = CreateSolidBrush(palette.bg);
        let border = CreatePen(PS_SOLID, 1, palette.border);
        let previous_brush = SelectObject(dc, windows::Win32::Graphics::Gdi::HGDIOBJ(background.0));
        let previous_pen = SelectObject(dc, windows::Win32::Graphics::Gdi::HGDIOBJ(border.0));
        let _ = RoundRect(
            dc,
            0,
            0,
            width,
            height,
            win::dip(CORNER_DIP, 96),
            win::dip(CORNER_DIP, 96),
        );
        SelectObject(dc, previous_pen);
        SelectObject(dc, previous_brush);
        let _ = windows::Win32::Graphics::Gdi::DeleteObject(
            windows::Win32::Graphics::Gdi::HGDIOBJ(border.0),
        );
        let _ = windows::Win32::Graphics::Gdi::DeleteObject(
            windows::Win32::Graphics::Gdi::HGDIOBJ(background.0),
        );

        // Text.
        let word_color = if content.paused {
            palette.muted
        } else {
            palette.word
        };
        if let Some(fonts) = fonts {
            if let Some(font) = &fonts.word {
                SelectObject(dc, font.obj());
                SetTextColor(dc, word_color);
                let mut text = win::wide(&content.word);
                let mut rect = layout.word;
                DrawTextW(
                    dc,
                    &mut text,
                    &mut rect,
                    DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX | DT_VCENTER,
                );
            }
            if let Some(font) = &fonts.meaning {
                SelectObject(dc, font.obj());
                SetTextColor(dc, palette.text);
                let mut text = win::wide(&content.meaning);
                let mut rect = layout.meaning;
                DrawTextW(
                    dc,
                    &mut text,
                    &mut rect,
                    DT_WORDBREAK | DT_END_ELLIPSIS | DT_NOPREFIX,
                );
            }
            if let Some(font) = &fonts.meta {
                SelectObject(dc, font.obj());
                SetTextColor(dc, palette.muted);
                let mut text = win::wide(&content.meta);
                let mut rect = layout.meta;
                DrawTextW(
                    dc,
                    &mut text,
                    &mut rect,
                    DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX | DT_VCENTER,
                );
            }
        }

        // Toolbar buttons.
        let color = if content.can_navigate {
            palette.button
        } else {
            palette.button_disabled
        };
        for (button, rect) in &layout.buttons {
            if pressed == Some(*button) {
                let highlight = CreateSolidBrush(palette.button_bg_pressed);
                let previous =
                    SelectObject(dc, windows::Win32::Graphics::Gdi::HGDIOBJ(highlight.0));
                let null_pen = SelectObject(dc, GetStockObject(NULL_BRUSH));
                let _ = RoundRect(dc, rect.left, rect.top, rect.right, rect.bottom, 6, 6);
                SelectObject(dc, null_pen);
                SelectObject(dc, previous);
                let _ = windows::Win32::Graphics::Gdi::DeleteObject(
                    windows::Win32::Graphics::Gdi::HGDIOBJ(highlight.0),
                );
            }
            let enabled = match button {
                CardButton::Previous | CardButton::Next => content.can_navigate,
                CardButton::Pause => content.can_navigate,
                CardButton::Learned => content.has_entry && !content.learned,
                CardButton::Again => content.has_entry || content.learned,
                CardButton::Details => content.has_entry,
                CardButton::Hide => true,
            };
            let tint = if enabled {
                color
            } else {
                palette.button_disabled
            };
            draw_glyph(dc, *button, rect, tint, content.paused);
        }
    }
}

fn draw_glyph(
    dc: HDC,
    button: CardButton,
    rect: &RECT,
    color: windows::Win32::Foundation::COLORREF,
    paused: bool,
) {
    let cx = (rect.left + rect.right) / 2;
    let cy = (rect.top + rect.bottom) / 2;
    let r = ((rect.right - rect.left) / 5).max(3);
    // SAFETY: the pen is deleted before returning; shapes only use the given DC.
    unsafe {
        let pen = Pen(CreatePen(PS_SOLID, 2, color));
        let brush = CreateSolidBrush(color);
        let previous_pen = SelectObject(dc, pen.obj());
        let previous_brush = SelectObject(dc, windows::Win32::Graphics::Gdi::HGDIOBJ(brush.0));

        match button {
            CardButton::Previous => {
                let points = [
                    POINT {
                        x: cx + r / 2,
                        y: cy - r,
                    },
                    POINT {
                        x: cx + r / 2,
                        y: cy + r,
                    },
                    POINT { x: cx - r, y: cy },
                ];
                let _ = Polygon(dc, &points);
            }
            CardButton::Next => {
                let points = [
                    POINT {
                        x: cx - r / 2,
                        y: cy - r,
                    },
                    POINT {
                        x: cx - r / 2,
                        y: cy + r,
                    },
                    POINT { x: cx + r, y: cy },
                ];
                let _ = Polygon(dc, &points);
            }
            CardButton::Pause if paused => {
                let points = [
                    POINT {
                        x: cx - r / 2,
                        y: cy - r,
                    },
                    POINT {
                        x: cx - r / 2,
                        y: cy + r,
                    },
                    POINT { x: cx + r, y: cy },
                ];
                let _ = Polygon(dc, &points);
            }
            CardButton::Pause => {
                let bar = (r / 2).max(2);
                let _ = FillRect(
                    dc,
                    &RECT {
                        left: cx - r + 2,
                        top: cy - r,
                        right: cx - r + 2 + bar,
                        bottom: cy + r,
                    },
                    HBRUSH(brush.0),
                );
                let _ = FillRect(
                    dc,
                    &RECT {
                        left: cx + r - 2 - bar,
                        top: cy - r,
                        right: cx + r - 2,
                        bottom: cy + r,
                    },
                    HBRUSH(brush.0),
                );
            }
            CardButton::Learned => {
                let points = [
                    POINT { x: cx - r, y: cy },
                    POINT {
                        x: cx - 2,
                        y: cy + r - 1,
                    },
                    POINT {
                        x: cx + r,
                        y: cy - r,
                    },
                ];
                let _ = Polyline(dc, &points);
            }
            CardButton::Again => {
                let previous = SelectObject(dc, GetStockObject(NULL_BRUSH));
                let _ = windows::Win32::Graphics::Gdi::Ellipse(dc, cx - r, cy - r, cx + r, cy + r);
                SelectObject(dc, previous);
                let points = [
                    POINT {
                        x: cx + r - 3,
                        y: cy - r - 2,
                    },
                    POINT {
                        x: cx + r + 2,
                        y: cy - 2,
                    },
                    POINT {
                        x: cx + r - 4,
                        y: cy - 2,
                    },
                ];
                let _ = Polygon(dc, &points);
            }
            CardButton::Details => {
                let previous = SelectObject(dc, GetStockObject(NULL_BRUSH));
                let _ = windows::Win32::Graphics::Gdi::Ellipse(dc, cx - r, cy - r, cx + r, cy + r);
                SelectObject(dc, previous);
                let _ = FillRect(
                    dc,
                    &RECT {
                        left: cx - 1,
                        top: cy - r / 2,
                        right: cx + 1,
                        bottom: cy + r - 1,
                    },
                    HBRUSH(brush.0),
                );
                let _ = FillRect(
                    dc,
                    &RECT {
                        left: cx - 1,
                        top: cy - r + 1,
                        right: cx + 1,
                        bottom: cy - r + 3,
                    },
                    HBRUSH(brush.0),
                );
            }
            CardButton::Hide => {
                let first = [
                    POINT {
                        x: cx - r + 1,
                        y: cy - r + 1,
                    },
                    POINT {
                        x: cx + r - 1,
                        y: cy + r - 1,
                    },
                ];
                let second = [
                    POINT {
                        x: cx + r - 1,
                        y: cy - r + 1,
                    },
                    POINT {
                        x: cx - r + 1,
                        y: cy + r - 1,
                    },
                ];
                let _ = Polyline(dc, &first);
                let _ = Polyline(dc, &second);
            }
        }

        SelectObject(dc, previous_brush);
        SelectObject(dc, previous_pen);
        let _ = windows::Win32::Graphics::Gdi::DeleteObject(
            windows::Win32::Graphics::Gdi::HGDIOBJ(brush.0),
        );
    }
}

// ---------------------------------------------------------------------------
// Window procedure
// ---------------------------------------------------------------------------

pub extern "system" fn card_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        // SAFETY: CREATESTRUCTW is valid for the duration of WM_NCCREATE and
        // lpCreateParams holds the App pointer passed to CreateWindowExW.
        let app = unsafe { (*(lparam.0 as *const CREATESTRUCTW)).lpCreateParams as *mut App };
        if !app.is_null() {
            // SAFETY: storing our own pointer in the window's user data slot.
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, app as isize);
            }
        }
        return LRESULT(1);
    }

    let app_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut App;
    if app_ptr.is_null() {
        // SAFETY: default handling for messages before/after our window exists.
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }

    match msg {
        WM_PAINT => {
            // Painting never pumps messages, so holding the borrow is safe here.
            let app = unsafe { &mut *app_ptr };
            paint(app, hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1), // fully painted in WM_PAINT
        WM_LBUTTONDOWN => {
            let point = cursor_point(lparam);
            let app = unsafe { &mut *app_ptr };
            match hit_test(app, point) {
                Some(button) => {
                    app.card.pressed = Some(button);
                    // Capture ensures we receive button-up even if the pointer
                    // leaves the compact card between down and up.
                    unsafe {
                        SetCapture(hwnd);
                    }
                }
                None => {
                    let mut origin = RECT::default();
                    let mut cursor = POINT::default();
                    // SAFETY: querying an existing window and the cursor position.
                    unsafe {
                        let _ = GetWindowRect(hwnd, &mut origin);
                        let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut cursor);
                        SetCapture(hwnd);
                    }
                    app.card.dragging = true;
                    app.card.drag_anchor = (cursor.x, cursor.y);
                    app.card.drag_origin = (origin.left, origin.top);
                }
            }
            invalidate(hwnd);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let app = unsafe { &mut *app_ptr };
            if app.card.dragging {
                let mut cursor = POINT::default();
                // SAFETY: pure query of the cursor position.
                unsafe {
                    let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut cursor);
                }
                let dx = cursor.x - app.card.drag_anchor.0;
                let dy = cursor.y - app.card.drag_anchor.1;
                let mut x = app.card.drag_origin.0 + dx;
                let mut y = app.card.drag_origin.1 + dy;
                let client = win::client_rect(hwnd);
                // Clamp to the work area of the monitor under the cursor so the
                // card can never be dragged out of reach.
                if let Some(monitor) = win::monitor_for_window(hwnd) {
                    x = x.clamp(
                        monitor.work.left,
                        (monitor.work.right - client.right).max(monitor.work.left),
                    );
                    y = y.clamp(
                        monitor.work.top,
                        (monitor.work.bottom - client.bottom).max(monitor.work.top),
                    );
                }
                win::set_window_rect_no_activate(hwnd, x, y, client.right, client.bottom);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let point = cursor_point(lparam);
            let app = unsafe { &mut *app_ptr };
            if app.card.dragging {
                app.card.dragging = false;
                // SAFETY: releasing the capture taken in WM_LBUTTONDOWN.
                unsafe {
                    let _ = ReleaseCapture();
                }
                let action = None;
                finish_drag(app, hwnd, action);
            } else if let Some(pressed) = app.card.pressed.take() {
                // SAFETY: releasing the capture if the mouse button came up inside.
                unsafe {
                    let _ = ReleaseCapture();
                }
                let hit = hit_test(app, point);
                invalidate(hwnd);
                if hit == Some(pressed) {
                    let action = match pressed {
                        CardButton::Previous => Some(Action::Previous),
                        CardButton::Next => Some(Action::Next),
                        CardButton::Pause => Some(Action::TogglePause),
                        CardButton::Learned => Some(Action::MarkLearned),
                        CardButton::Again => Some(Action::ReviewAgain),
                        CardButton::Details => Some(Action::Details),
                        CardButton::Hide => Some(Action::HideCard),
                    };
                    if let Some(action) = action {
                        app.handle(action);
                    }
                }
            }
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            let app = unsafe { &mut *app_ptr };
            if app.card.dragging {
                app.card.dragging = false;
                app.remember_card_position();
                app.save_config();
            }
            app.card.pressed = None;
            invalidate(hwnd);
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let app = unsafe { &mut *app_ptr };
            on_dpi_changed(app, wparam, lparam);
            LRESULT(0)
        }
        WM_CLOSE => {
            // Closing the card hides it; the tray icon keeps the app running.
            let app = unsafe { &mut *app_ptr };
            app.hide_card();
            LRESULT(0)
        }
        WM_NCDESTROY => {
            let app = unsafe { &mut *app_ptr };
            app.card.hwnd = None;
            app.card.buffer = None;
            app.card.fonts = None;
            // SAFETY: clearing the user data slot before the window goes away.
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        _ => {
            // SAFETY: default processing.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
    }
}

/// Shared tail of a drag: remember the position and stop the timer restart loop.
fn finish_drag(app: &mut App, hwnd: HWND, _action: Option<Action>) {
    app.remember_card_position();
    app.save_config();
    invalidate(hwnd);
}

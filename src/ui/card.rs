//! The compact card: a borderless, non-activating popup painted with GDI.
//!
//! The card never takes keyboard focus (`WS_EX_NOACTIVATE` + `SW_SHOWNOACTIVATE`),
//! is dragged with a manual mouse capture, and is repainted only when its
//! content, size, position or theme changes.

use crate::app::{Action, App};
use crate::model::{CardAnchor, Config, PresentationMode};
use crate::theme;
use crate::win::{self, Font, MemoryDc, Pen};
use std::ffi::c_void;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreatePen, CreateSolidBrush, DT_CALCRECT, DT_EDITCONTROL, DT_END_ELLIPSIS,
    DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK, DrawTextW, EndPaint, FillRect,
    GetStockObject, HBRUSH, HDC, NULL_BRUSH, PAINTSTRUCT, PS_SOLID, Polygon, Polyline, RestoreDC,
    RoundRect, SaveDC, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CreateWindowExW, DefWindowProcW, GWLP_USERDATA, GetWindowLongPtrW,
    GetWindowRect, SetWindowLongPtrW, WINDOW_EX_STYLE, WM_CAPTURECHANGED, WM_CLOSE, WM_DPICHANGED,
    WM_ERASEBKGND, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCREATE, WM_NCDESTROY, WM_PAINT,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

/// Height and width are expressed in device-independent pixels.
const PAD: i32 = 6;
const PAD_BOTTOM: i32 = 4;
const GAP: i32 = 2;
const TOOLBAR: i32 = 18;
pub const TOOLBAR_EXTRA_DIP: i32 = GAP + TOOLBAR;
const BUTTON: i32 = 18;
const BUTTON_GAP: i32 = 2;
const GROUP_GAP: i32 = 8;
const CORNER_DIP: i32 = 10;
const FIELD_GAP: i32 = 3;
const MAX_WORD_LINES: i32 = 3;
const MAX_MEANING_LINES: i32 = 5;
const MAX_EXAMPLE_LINES: i32 = 4;
const MAX_TEXT_CHARS: usize = 8192;
const WM_MOUSELEAVE_VALUE: u32 = 0x02A3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CardButton {
    Previous,
    Next,
    Pause,
    Learned,
    Again,
    Settings,
    ShowMore,
    Hide,
}

const BUTTONS: [CardButton; 7] = [
    CardButton::Previous,
    CardButton::Next,
    CardButton::Pause,
    CardButton::Learned,
    CardButton::Again,
    CardButton::Settings,
    CardButton::Hide,
];

/// Device-pixel rectangles for the current card size.
#[derive(Clone, Debug, Default)]
pub struct CardLayout {
    pub word: RECT,
    pub word_text: String,
    pub meaning: RECT,
    pub meaning_text: String,
    pub example: Option<RECT>,
    pub example_text: String,
    pub show_more: Option<RECT>,
    pub word_truncated: bool,
    pub meaning_truncated: bool,
    pub example_truncated: bool,
    pub height: i32,
    pub buttons: Vec<(CardButton, RECT)>,
}

#[derive(Default)]
pub struct CardState {
    pub hwnd: Option<HWND>,
    pub dpi: u32,
    pub fonts: Option<CardFonts>,
    pub buffer: Option<MemoryDc>,
    pub pressed: Option<CardButton>,
    pub hovered: bool,
    pub keyboard_revealed: bool,
    pub tracking_mouse: bool,
    pub dragging: bool,
    pub drag_anchor: (i32, i32),
    pub drag_origin: (i32, i32),
    pub layout: CardLayout,
}

pub struct CardFonts {
    pub(crate) word: Option<Font>,
    pub(crate) meaning: Option<Font>,
    pub(crate) example: Option<Font>,
    pub(crate) action: Option<Font>,
}

/// Creates the same fonts used by both the visible card and the Settings preview.
pub fn create_fonts(config: &Config, dpi: u32) -> CardFonts {
    let family = crate::model::UI_FONT_FAMILY;
    let base = config.font_size_pt as f32;
    CardFonts {
        word: win::create_font(family, base, dpi, true),
        meaning: win::create_font(family, (base * 0.62).max(10.0), dpi, false),
        example: win::create_font(family, (base * 0.5).max(9.0), dpi, false),
        action: win::create_font(family, 9.0, dpi, false),
    }
}

fn ensure_fonts(state: &mut CardState, config: &Config, dpi: u32) {
    let needs_rebuild = state.fonts.is_none() || state.dpi != dpi;
    if needs_rebuild {
        state.fonts = Some(create_fonts(config, dpi));
        state.dpi = dpi;
    }
}

fn break_long_tokens(text: &str) -> (String, bool) {
    let mut result = String::new();
    let mut token = String::new();
    let mut clamped = false;
    let flush = |token: &mut String, result: &mut String| {
        let count = token.chars().count();
        if count > 18 {
            for (index, ch) in token.chars().enumerate() {
                if index > 0 {
                    result.push('\u{200B}');
                }
                result.push(ch);
            }
        } else {
            result.push_str(token);
        }
        token.clear();
    };
    for ch in text.chars() {
        if ch.is_whitespace() {
            flush(&mut token, &mut result);
            result.push(ch);
        } else {
            token.push(ch);
        }
        if result.len() + token.len() > MAX_TEXT_CHARS {
            clamped = true;
            break;
        }
    }
    flush(&mut token, &mut result);
    if result.chars().count() > MAX_TEXT_CHARS {
        result = result.chars().take(MAX_TEXT_CHARS).collect();
        clamped = true;
    }
    if clamped {
        result.push('…');
    }
    (result, clamped)
}

fn measure_wrapped_height(text: &str, font: Option<&Font>, width: i32) -> i32 {
    if text.is_empty() || width <= 0 {
        return 0;
    }
    let Some(font) = font else {
        return win::dip(20, win::system_dpi());
    };
    let mut text = win::wide(text);
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: 0,
    };
    // SAFETY: the screen DC is released below and the selected font is restored.
    unsafe {
        let dc = windows::Win32::Graphics::Gdi::GetDC(None);
        if dc.0.is_null() {
            return 1;
        }
        let old = windows::Win32::Graphics::Gdi::SelectObject(dc, font.obj());
        let _ = DrawTextW(
            dc,
            &mut text,
            &mut rect,
            DT_CALCRECT | DT_WORDBREAK | DT_EDITCONTROL | DT_NOPREFIX,
        );
        windows::Win32::Graphics::Gdi::SelectObject(dc, old);
        let _ = windows::Win32::Graphics::Gdi::ReleaseDC(None, dc);
    }
    (rect.bottom - rect.top).max(1)
}

/// Measures and lays out text using the exact font handles, width and DPI that
/// the painter will use. Very long fields are capped by line count and expose a
/// visible Show more action instead of silently disappearing.
#[allow(clippy::too_many_arguments)] // each input is a rendering/layout constraint
pub fn measure_layout(
    config: &Config,
    content: &crate::app::CardContent,
    fonts: &CardFonts,
    width: i32,
    dpi: u32,
    hovered: bool,
    toolbar_visible: bool,
    available_height: i32,
) -> CardLayout {
    let scale = |value: i32| win::dip(value, dpi);
    let pad = scale(PAD);
    let text_width = (width - 2 * pad).max(1);
    let (word, word_input_clamped) = break_long_tokens(&content.word);
    let (meaning, meaning_input_clamped) = break_long_tokens(&content.meaning);
    let (example, example_input_clamped) = break_long_tokens(&content.example);
    let word_full = measure_wrapped_height(&word, fonts.word.as_ref(), text_width);
    let meaning_visible = match config.presentation {
        PresentationMode::Basic => true,
        PresentationMode::MeaningOnHover => hovered,
        PresentationMode::WithExample => true,
    };
    let example_visible =
        config.presentation == PresentationMode::WithExample && !content.example.trim().is_empty();
    let meaning_full = if meaning_visible {
        measure_wrapped_height(&meaning, fonts.meaning.as_ref(), text_width)
    } else {
        0
    };
    let example_full = if example_visible {
        measure_wrapped_height(&example, fonts.example.as_ref(), text_width)
    } else {
        0
    };
    let word_line = measure_wrapped_height("Ag", fonts.word.as_ref(), text_width).max(1);
    let meaning_line = measure_wrapped_height("Ag", fonts.meaning.as_ref(), text_width).max(1);
    let example_line = measure_wrapped_height("Ag", fonts.example.as_ref(), text_width).max(1);
    let action_height =
        measure_wrapped_height("Show more", fonts.action.as_ref(), text_width).max(1);
    let mut word_height = word_full.min(word_line * MAX_WORD_LINES).max(word_line);
    let mut meaning_height = meaning_full.min(meaning_line * MAX_MEANING_LINES);
    let mut example_height = example_full.min(example_line * MAX_EXAMPLE_LINES);
    let mut word_truncated = word_input_clamped || word_full > word_height;
    let mut meaning_truncated = meaning_input_clamped || meaning_full > meaning_height;
    let mut example_truncated = example_input_clamped || example_full > example_height;

    let toolbar_extra = if toolbar_visible {
        scale(GAP + TOOLBAR)
    } else {
        0
    };
    let max_text_height = (available_height - scale(PAD + PAD_BOTTOM) - toolbar_extra).max(0);
    let content_height = |word: i32, meaning: i32, example: i32| {
        word + meaning
            + example
            + if meaning > 0 { scale(FIELD_GAP) } else { 0 }
            + if example > 0 { scale(FIELD_GAP) } else { 0 }
    };
    while content_height(word_height, meaning_height, example_height) > max_text_height {
        if example_height > 0 {
            example_height = (example_height - example_line).max(0);
            example_truncated = true;
        } else if meaning_height > meaning_line {
            meaning_height = (meaning_height - meaning_line).max(meaning_line);
            meaning_truncated = true;
        } else if word_height > word_line {
            word_height = (word_height - word_line).max(word_line);
            word_truncated = true;
        } else {
            meaning_truncated |= meaning_height > 0;
            meaning_height = 0;
            word_truncated = true;
            break;
        }
    }
    let mut show_more = word_truncated || meaning_truncated || example_truncated;
    if show_more {
        let link_gap = scale(FIELD_GAP) + action_height;
        while content_height(word_height, meaning_height, example_height) + link_gap
            > max_text_height
        {
            if example_height > 0 {
                example_height = (example_height - example_line).max(0);
                example_truncated = true;
            } else if meaning_height > meaning_line {
                meaning_height = (meaning_height - meaning_line).max(meaning_line);
                meaning_truncated = true;
            } else if meaning_height > 0 {
                meaning_height = 0;
                meaning_truncated = true;
            } else if word_height > word_line {
                word_height = (word_height - word_line).max(word_line);
                word_truncated = true;
            } else {
                show_more = false;
                break;
            }
        }
    }

    let mut y = pad;
    let word_rect = RECT {
        left: pad,
        top: y,
        right: width - pad,
        bottom: y + word_height,
    };
    y = word_rect.bottom;
    let meaning_rect = if meaning_visible {
        y += scale(FIELD_GAP);
        let rect = RECT {
            left: pad,
            top: y,
            right: width - pad,
            bottom: y + meaning_height,
        };
        y = rect.bottom;
        rect
    } else {
        RECT::default()
    };
    let example_rect = if example_visible {
        y += scale(FIELD_GAP);
        let rect = RECT {
            left: pad,
            top: y,
            right: width - pad,
            bottom: y + example_height,
        };
        y = rect.bottom;
        Some(rect)
    } else {
        None
    };
    let show_more_rect = if show_more {
        y += scale(FIELD_GAP);
        let rect = RECT {
            left: pad,
            top: y,
            right: width - pad,
            bottom: y + action_height,
        };
        y = rect.bottom;
        Some(rect)
    } else {
        None
    };
    y += scale(PAD_BOTTOM);
    if toolbar_visible {
        y += toolbar_extra;
    }
    let height = y.max(scale(40));

    let mut buttons = Vec::with_capacity(if toolbar_visible { BUTTONS.len() } else { 0 });
    if toolbar_visible {
        let toolbar_top = height - scale(PAD_BOTTOM) - scale(TOOLBAR);
        let mut x = pad;
        let mut previous: Option<CardButton> = None;
        for button in BUTTONS
            .into_iter()
            .filter(|button| *button != CardButton::Hide)
        {
            let grouped = matches!(
                (previous, button),
                (Some(CardButton::Next), CardButton::Pause)
                    | (Some(CardButton::Pause), CardButton::Learned)
                    | (Some(CardButton::Again), CardButton::Settings)
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
            x += scale(BUTTON + BUTTON_GAP);
            previous = Some(button);
        }
        let left = width - pad - scale(BUTTON);
        buttons.push((
            CardButton::Hide,
            RECT {
                left,
                top: toolbar_top,
                right: left + scale(BUTTON),
                bottom: toolbar_top + scale(BUTTON),
            },
        ));
    }
    CardLayout {
        word: word_rect,
        word_text: word,
        meaning: meaning_rect,
        meaning_text: meaning,
        example: example_rect,
        example_text: example,
        show_more: show_more_rect,
        word_truncated,
        meaning_truncated,
        example_truncated,
        height,
        buttons,
    }
}

/// Drops the cached fonts so the next paint rebuilds them.
pub fn reset_fonts(state: &mut CardState) {
    state.fonts = None;
}

/// Computes an anchor origin in physical pixels. Kept pure for tests and used
/// after every DPI/content/size change.
pub fn anchor_origin(work: RECT, size: (i32, i32), margin: i32, anchor: CardAnchor) -> (i32, i32) {
    let left = work.left + margin;
    let top = work.top + margin;
    let right = work.right - size.0 - margin;
    let bottom = work.bottom - size.1 - margin;
    let center_x = work.left + (work.right - work.left - size.0) / 2;
    let center_y = work.top + (work.bottom - work.top - size.1) / 2;
    match anchor {
        CardAnchor::TopLeft => (left, top),
        CardAnchor::TopCenter => (center_x, top),
        CardAnchor::TopRight => (right, top),
        CardAnchor::CenterLeft => (left, center_y),
        CardAnchor::CenterRight => (right, center_y),
        CardAnchor::BottomLeft => (left, bottom),
        CardAnchor::BottomCenter => (center_x, bottom),
        CardAnchor::BottomRight | CardAnchor::Custom => (right, bottom),
    }
}

fn selected_monitor(config: &Config, hwnd: HWND) -> Option<win::MonitorInfo> {
    config
        .position_monitor
        .as_str()
        .trim()
        .is_empty()
        .then(|| {
            config
                .position
                .as_ref()
                .map(|position| position.monitor.as_str())
        })
        .flatten()
        .or_else(|| {
            (!config.position_monitor.is_empty()).then_some(config.position_monitor.as_str())
        })
        .and_then(win::monitor_by_device)
        .or_else(|| win::monitor_for_window(hwnd))
        .or_else(|| win::all_monitors().into_iter().next())
}

/// Creates the card window hidden; actual dimensions are measured before it is
/// first shown. The same measurement routine is also used by Settings preview.
pub fn create(app: &mut App, instance: HINSTANCE) -> Option<HWND> {
    let class = win::wide(CARD_CLASS);
    let title = win::wide("WordDock");
    let monitor = selected_monitor(&app.config, HWND::default())?;
    let dpi = win::system_dpi().max(96);
    app.card.dpi = dpi;
    app.card.fonts = Some(create_fonts(&app.config, dpi));
    let width = win::dip(app.config.card_width_dip, dpi)
        .min(monitor.work.right - monitor.work.left)
        .max(1);
    let layout = measure_layout(
        &app.config,
        &app.card_content(),
        app.card.fonts.as_ref().expect("fonts created above"),
        width,
        dpi,
        false,
        false,
        monitor.work.bottom - monitor.work.top,
    );
    app.card.layout = layout.clone();
    // SAFETY: the class is registered before this call and `app` outlives HWND.
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
            width,
            layout.height.max(1),
            None,
            None,
            Some(instance),
            Some(app as *mut App as *const c_void),
        )
    }
    .ok()?;
    app.card.hwnd = Some(hwnd);
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

/// Measures content, positions the card, updates its region and re-themes it.
pub fn apply_placement(app: &mut App) {
    let Some(hwnd) = app.card.hwnd else {
        return;
    };
    let dpi = win::dpi_of_window(hwnd).max(96);
    if app.card.dpi != dpi {
        reset_fonts(&mut app.card);
    }
    app.card.dpi = dpi;
    ensure_fonts(&mut app.card, &app.config, dpi);
    let Some(monitor) = selected_monitor(&app.config, hwnd) else {
        return;
    };
    let work = monitor.work;
    app.config.position_monitor = monitor.device.clone();
    let requested_margin = win::dip(app.config.margin_dip, dpi);
    let margin = requested_margin
        .min(((work.right - work.left - 1) / 2).max(0))
        .min(((work.bottom - work.top - 1) / 2).max(0));
    let available_width = (work.right - work.left - margin * 2).max(1);
    let available_height = (work.bottom - work.top - margin * 2).max(1);
    let width = win::dip(app.config.card_width_dip, dpi).clamp(1, available_width);
    let toolbar = toolbar_visible(&app.card);
    let layout = measure_layout(
        &app.config,
        &app.card_content(),
        app.card.fonts.as_ref().expect("fonts ensured above"),
        width,
        dpi,
        app.card.hovered,
        toolbar,
        available_height,
    );
    let height = layout.height.clamp(1, available_height);
    app.card.layout = layout;

    let (mut x, mut y) = if app.config.position_anchor == CardAnchor::Custom {
        app.config.position.as_ref().map_or_else(
            || anchor_origin(work, (width, height), margin, CardAnchor::BottomRight),
            |position| {
                (
                    work.left + win::dip(position.x_dip, dpi),
                    work.top + win::dip(position.y_dip, dpi)
                        - if toolbar {
                            win::dip(TOOLBAR_EXTRA_DIP, dpi)
                        } else {
                            0
                        },
                )
            },
        )
    } else {
        anchor_origin(work, (width, height), margin, app.config.position_anchor)
    };
    let clamp_left = work.left + margin;
    let clamp_top = work.top + margin;
    x = x.clamp(clamp_left, (work.right - width - margin).max(clamp_left));
    y = y.clamp(clamp_top, (work.bottom - height - margin).max(clamp_top));

    win::set_window_rect_no_activate(hwnd, x, y, width, height);
    win::set_topmost(hwnd, app.config.always_on_top);
    win::set_rounded_window_region(hwnd, win::dip(CORNER_DIP, dpi));
    invalidate(hwnd);
}

pub fn invalidate(hwnd: HWND) {
    // SAFETY: invalidating an existing window; the repaint is coalesced by Windows.
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
    }
}

pub fn toolbar_visible(state: &CardState) -> bool {
    state.hovered || state.keyboard_revealed
}

/// Expands/collapses the control strip while keeping the card's bottom edge
/// fixed. This lets the card remain anchored just above the taskbar.
fn resize_for_toolbar(app: &mut App) {
    apply_placement(app);
}

/// Toggles the controls using the application hotkey (Ctrl+Shift+Space).
pub fn toggle_keyboard_reveal(app: &mut App) {
    app.card.keyboard_revealed = !app.card.keyboard_revealed;
    resize_for_toolbar(app);
}

/// `WM_DPICHANGED` for the card window: resize and repaint at the new scale.
pub fn on_dpi_changed(app: &mut App, wparam: WPARAM, lparam: LPARAM) {
    let Some(hwnd) = app.card.hwnd else {
        return;
    };
    let dpi = (wparam.0 & 0xFFFF) as u32;
    reset_fonts(&mut app.card);
    app.card.dpi = dpi.max(96);
    apply_placement(app);
    invalidate(hwnd);
    let _ = lparam;
}

fn hit_test(app: &App, point: POINT) -> Option<CardButton> {
    if app
        .card
        .layout
        .show_more
        .as_ref()
        .is_some_and(|rect| contains(rect, point))
    {
        return Some(CardButton::ShowMore);
    }
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
    let content = app.card_content();
    let show_toolbar = toolbar_visible(&app.card);
    app.card.layout = measure_layout(
        &app.config,
        &content,
        app.card.fonts.as_ref().expect("fonts ensured above"),
        width,
        app.card.dpi,
        app.card.hovered,
        show_toolbar,
        height,
    );
    let layout = app.card.layout.clone();
    let palette = theme::palette_for_config(&app.config);

    if let Some(buffer) = &app.card.buffer {
        let dc = buffer.dc();
        let fonts = app.card.fonts.as_ref();
        draw_to_dc(
            dc,
            &layout,
            &content,
            palette,
            fonts,
            app.card.pressed,
            width,
            height,
            app.card.dpi,
        );
        buffer.blit_to(hdc);
    }
    // SAFETY: matches the BeginPaint above.
    unsafe {
        let _ = EndPaint(hwnd, &paint);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw_to_dc(
    dc: HDC,
    layout: &CardLayout,
    content: &crate::app::CardContent,
    palette: theme::Palette,
    fonts: Option<&CardFonts>,
    pressed: Option<CardButton>,
    width: i32,
    height: i32,
    dpi: u32,
) {
    // SAFETY: every GDI object created here is deleted before returning, and the
    // DC belongs to the back buffer for the duration of the call.
    unsafe {
        let saved_dc = SaveDC(dc);
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
            win::dip(CORNER_DIP, dpi),
            win::dip(CORNER_DIP, dpi),
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
                let mut text = win::wide(&layout.word_text);
                let mut rect = layout.word;
                let mut flags =
                    windows::Win32::Graphics::Gdi::DT_WORDBREAK | DT_NOPREFIX | DT_EDITCONTROL;
                if layout.word_truncated {
                    flags |= DT_END_ELLIPSIS;
                }
                DrawTextW(dc, &mut text, &mut rect, flags);
            }
            if let Some(font) = &fonts.meaning {
                SelectObject(dc, font.obj());
                SetTextColor(dc, palette.text);
                let mut text = win::wide(&layout.meaning_text);
                let mut rect = layout.meaning;
                let mut flags =
                    windows::Win32::Graphics::Gdi::DT_WORDBREAK | DT_NOPREFIX | DT_EDITCONTROL;
                if layout.meaning_truncated {
                    flags |= DT_END_ELLIPSIS;
                }
                DrawTextW(dc, &mut text, &mut rect, flags);
            }
            if let (Some(font), Some(example_rect)) = (&fonts.example, layout.example) {
                SelectObject(dc, font.obj());
                SetTextColor(dc, palette.muted);
                let mut text = win::wide(&layout.example_text);
                let mut rect = example_rect;
                let mut flags =
                    windows::Win32::Graphics::Gdi::DT_WORDBREAK | DT_NOPREFIX | DT_EDITCONTROL;
                if layout.example_truncated {
                    flags |= DT_END_ELLIPSIS;
                }
                DrawTextW(dc, &mut text, &mut rect, flags);
            }
            if let Some(more_rect) = layout.show_more {
                if let Some(font) = &fonts.action {
                    SelectObject(dc, font.obj());
                }
                SetTextColor(dc, palette.word);
                let mut text = win::wide("Show more");
                let mut rect = more_rect;
                DrawTextW(
                    dc,
                    &mut text,
                    &mut rect,
                    DT_SINGLELINE | DT_NOPREFIX | DT_VCENTER,
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
            let normal_bg = CreateSolidBrush(palette.button_bg);
            let _ = FillRect(dc, rect, normal_bg);
            let _ = windows::Win32::Graphics::Gdi::DeleteObject(
                windows::Win32::Graphics::Gdi::HGDIOBJ(normal_bg.0),
            );
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
                CardButton::Settings => true,
                CardButton::ShowMore => content.has_entry,
                CardButton::Hide => true,
            };
            let tint = if enabled {
                color
            } else {
                palette.button_disabled
            };
            draw_glyph(dc, *button, rect, tint, content.paused);
        }
        if saved_dc != 0 {
            let _ = RestoreDC(dc, saved_dc);
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
            CardButton::Settings => {
                let previous = SelectObject(dc, GetStockObject(NULL_BRUSH));
                let _ = windows::Win32::Graphics::Gdi::Ellipse(dc, cx - r, cy - r, cx + r, cy + r);
                SelectObject(dc, previous);
                let spokes = [
                    [
                        POINT {
                            x: cx,
                            y: cy - r - 2,
                        },
                        POINT { x: cx, y: cy - 1 },
                    ],
                    [
                        POINT { x: cx, y: cy + 1 },
                        POINT {
                            x: cx,
                            y: cy + r + 2,
                        },
                    ],
                    [
                        POINT {
                            x: cx - r - 2,
                            y: cy,
                        },
                        POINT { x: cx - 1, y: cy },
                    ],
                    [
                        POINT { x: cx + 1, y: cy },
                        POINT {
                            x: cx + r + 2,
                            y: cy,
                        },
                    ],
                ];
                for spoke in spokes {
                    let _ = Polyline(dc, &spoke);
                }
                let _ = windows::Win32::Graphics::Gdi::Ellipse(dc, cx - 2, cy - 2, cx + 2, cy + 2);
            }
            CardButton::ShowMore => {}
            CardButton::Hide => {
                // Familiar minimize glyph: a small window outline with a dash.
                let null_brush = SelectObject(dc, GetStockObject(NULL_BRUSH));
                let _ = RoundRect(dc, cx - r, cy - r, cx + r, cy + r, 3, 3);
                SelectObject(dc, null_brush);
                let _ = FillRect(
                    dc,
                    &RECT {
                        left: cx - r / 2,
                        top: cy + r / 2,
                        right: cx + r / 2 + 1,
                        bottom: cy + r / 2 + 2,
                    },
                    HBRUSH(brush.0),
                );
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
            if !app.card.tracking_mouse {
                app.card.tracking_mouse = true;
                let mut tracking = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                unsafe {
                    let _ = TrackMouseEvent(&mut tracking);
                }
            }
            if !app.card.hovered {
                app.card.hovered = true;
                resize_for_toolbar(app);
            }
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
                    let requested = win::dip(app.config.margin_dip as i32, app.card.dpi);
                    let mx = requested.min(
                        ((monitor.work.right - monitor.work.left - client.right - 1) / 2).max(0),
                    );
                    let my = requested.min(
                        ((monitor.work.bottom - monitor.work.top - client.bottom - 1) / 2).max(0),
                    );
                    let left = monitor.work.left + mx;
                    let top = monitor.work.top + my;
                    x = x.clamp(left, (monitor.work.right - client.right - mx).max(left));
                    y = y.clamp(top, (monitor.work.bottom - client.bottom - my).max(top));
                }
                win::set_window_rect_no_activate(hwnd, x, y, client.right, client.bottom);
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE_VALUE => {
            let app = unsafe { &mut *app_ptr };
            app.card.tracking_mouse = false;
            if app.card.hovered {
                app.card.hovered = false;
                resize_for_toolbar(app);
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
                        CardButton::Settings => Some(Action::Settings),
                        CardButton::ShowMore => Some(Action::Details),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_remain_inside_work_area_and_hold_the_selected_edge() {
        let work = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1032,
        };
        let size = (260, 120);
        let margin = 12;
        for anchor in CardAnchor::GRID {
            let (x, y) = anchor_origin(work, size, margin, anchor);
            assert!(x >= work.left && y >= work.top);
            assert!(x + size.0 <= work.right && y + size.1 <= work.bottom);
        }
        assert_eq!(
            anchor_origin(work, size, margin, CardAnchor::BottomRight),
            (1648, 900)
        );
        assert_eq!(
            anchor_origin(work, size, margin, CardAnchor::TopCenter),
            (830, 12)
        );
    }

    #[test]
    fn short_and_long_accented_entries_fit_at_common_dpis() {
        let config = Config {
            presentation: PresentationMode::WithExample,
            ..Config::default()
        };
        let long = crate::app::CardContent {
            word: "electroencefalografista extraordinariamente largo".repeat(4),
            meaning: "Hábil, diestro y cuidadoso al resolver problemas complejos.\nUna segunda línea con acentos: áéíóú ü ñ.".repeat(10),
            example: "The extraordinarily resourceful student kept detailed notes for every class. ".repeat(14),
            presentation: PresentationMode::WithExample,
            meta: String::new(), has_entry: true, learned: false, paused: false,
            can_navigate: true, empty_hint: None,
        };
        let short = crate::app::CardContent {
            word: "adroit".into(),
            meaning: "Hábil, diestro.".into(),
            example: "Una frase breve.".into(),
            presentation: PresentationMode::WithExample,
            meta: String::new(),
            has_entry: true,
            learned: false,
            paused: false,
            can_navigate: true,
            empty_hint: None,
        };
        for dpi in [96, 120, 144, 192] {
            let width = win::dip(config.card_width_dip, dpi);
            let available = win::dip(600, dpi);
            let fonts = create_fonts(&config, dpi);
            let long_layout =
                measure_layout(&config, &long, &fonts, width, dpi, false, true, available);
            assert!(
                long_layout.show_more.is_some(),
                "long text should expose Show more at {dpi} DPI"
            );
            assert!(long_layout.height <= available);
            for rect in [
                Some(long_layout.word),
                Some(long_layout.meaning),
                long_layout.example,
            ]
            .into_iter()
            .flatten()
            {
                assert!(
                    rect.left >= 0 && rect.right <= width,
                    "field escaped horizontally at {dpi} DPI: {rect:?}"
                );
                assert!(
                    rect.top >= 0 && rect.bottom <= long_layout.height,
                    "field escaped vertically at {dpi} DPI: {rect:?}"
                );
            }
            if let Some(more) = long_layout.show_more {
                assert!(more.bottom <= long_layout.height);
            }
            let short_layout =
                measure_layout(&config, &short, &fonts, width, dpi, false, false, available);
            assert!(short_layout.show_more.is_none());
            assert!(short_layout.example.is_some());
            assert!(short_layout.meaning.bottom > short_layout.meaning.top);
        }
    }
}

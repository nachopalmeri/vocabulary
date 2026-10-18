//! Native four-section configuration/setup window. Draft settings and preview
//! resources are owned by this window and discarded on Cancel/close.

use std::ffi::c_void;

use crate::app::App;
use crate::model::{
    CardAnchor, Config, MAX_CARD_WIDTH_DIP, MAX_FONT_SIZE_PT, MAX_INTERVAL_MINUTES,
    MIN_CARD_WIDTH_DIP, MIN_FONT_SIZE_PT, MIN_INTERVAL_MINUTES, Order, PresentationMode, ThemeMode,
};
use crate::theme;
use crate::ui::{self, Chrome, DipRect, id, rect};
use crate::win;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, HDC, HGDIOBJ, PAINTSTRUCT,
    RestoreDC, SaveDC, SetBkMode, SetViewportOrgEx, TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetClientRect, GetWindowLongPtrW,
    SetWindowLongPtrW, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_CTLCOLORBTN, WM_CTLCOLOREDIT,
    WM_CTLCOLORLISTBOX, WM_CTLCOLORSTATIC, WM_DPICHANGED, WM_ERASEBKGND, WM_NCCREATE, WM_NCDESTROY,
    WM_PAINT,
};

pub const CLASS: &str = "WordDockSettingsDialog";
const CLIENT: (i32, i32) = (900, 480);
const LEFT: i32 = 14;
const LEFT_W: i32 = 402;
const RIGHT: i32 = 430;
const RIGHT_W: i32 = 456;
const INTERVALS: [u32; 5] = [1, 5, 15, 30, 60];
const PREVIEW_DIP: (i32, i32) = (430, 266);
const PREVIEW_SIZE_DIP: (i32, i32) = (440, 112);

const POSITION_IDS: [u16; 8] = [
    id::POSITION_0,
    id::POSITION_1,
    id::POSITION_2,
    id::POSITION_3,
    id::POSITION_4,
    id::POSITION_5,
    id::POSITION_6,
    id::POSITION_7,
];
const POSITION_LABELS: [&str; 8] = ["TL", "Top", "TR", "Left", "Right", "BL", "Bottom", "BR"];
const CHILDREN: &[u16] = &[
    180,
    181,
    182,
    183,
    184,
    185,
    186,
    187,
    188,
    189,
    190,
    191,
    192,
    id::SET_INTERVAL,
    id::SET_CUSTOM_INTERVAL,
    id::SET_ORDER,
    id::SET_COLLECTION,
    id::SET_PRESENTATION,
    id::SET_THEME,
    id::SET_COLOR_BACKGROUND,
    id::SET_COLOR_TEXT,
    id::SET_COLOR_SECONDARY,
    id::SET_FONT,
    id::SET_WIDTH,
    id::SET_TOPMOST,
    id::SET_LAUNCH,
    id::SET_EXCLUDE_LEARNED,
    id::SET_HIDE_FULLSCREEN,
    id::SET_MONITOR,
    id::SET_MARGIN,
    id::SET_CONTENT_IMPORT,
    id::SET_CONTENT_MANAGE,
    id::SET_CONTENT_HINT,
    id::SET_POSITION_HINT,
    id::SET_CONTRAST,
    id::SET_HINT,
    id::APPLY,
    id::CANCEL,
    id::START_LEARNING,
    id::RESTORE_DEFAULTS,
    id::POSITION_0,
    id::POSITION_1,
    id::POSITION_2,
    id::POSITION_3,
    id::POSITION_4,
    id::POSITION_5,
    id::POSITION_6,
    id::POSITION_7,
];

#[derive(Clone)]
struct CollectionChoice {
    id: String,
    name: String,
    language_pair: String,
    eligible: usize,
    total: usize,
}

struct State {
    app: *mut App,
    chrome: Chrome,
    preview_fonts: crate::ui::card::CardFonts,
    draft: Config,
    collections: Vec<CollectionChoice>,
    /// One real eligible example per collection; cloned only while Settings is
    /// open so preview painting never dereferences App across WM_PAINT.
    preview_entries: Vec<(String, crate::model::Entry)>,
    monitors: Vec<win::MonitorInfo>,
    first_run: bool,
    loading_controls: bool,
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
    if let Some(hwnd) = app.settings_hwnd {
        if win::is_window(hwnd) {
            win::activate_window(hwnd);
            return;
        }
        app.settings_hwnd = None;
    }
    let reference = ui::reference_window(app);
    let dpi = win::dpi_of_window(reference).max(96);
    let mut draft = app.config.clone();
    let first_run = app.first_run_setup;
    draft
        .selected_collections
        .retain(|id| app.library.collection(id).is_some());
    if first_run {
        if let Some(active) = app
            .library
            .active_collection
            .as_ref()
            .filter(|id| app.library.collection(id).is_some())
        {
            draft.selected_collections = vec![active.clone()];
        } else if let Some(first) = app.library.collections.first() {
            draft.selected_collections = vec![first.id.clone()];
        }
        draft.selection_initialized = true;
        draft.card_visible = true;
        draft.always_on_top = true;
        draft.hide_during_fullscreen = true;
    }
    let collections = collection_choices(app, &draft);
    let preview_entries = preview_entries(app, &draft);
    let state = Box::new(State {
        app: app as *mut App,
        chrome: Chrome::new(draft.theme, dpi),
        preview_fonts: crate::ui::card::create_fonts(&draft, dpi),
        draft,
        collections,
        preview_entries,
        monitors: win::all_monitors(),
        first_run,
        loading_controls: false,
    });
    let pointer = Box::into_raw(state);
    let owner = app.card.hwnd.filter(|_| app.card_visible());
    if let Some(hwnd) = ui::create_dialog_window(
        owner,
        reference,
        win::module_instance(),
        CLASS,
        if first_run {
            "Welcome to WordDock"
        } else {
            "WordDock settings"
        },
        CLIENT,
        pointer as *mut c_void,
    ) {
        app.settings_hwnd = Some(hwnd);
    } else {
        unsafe {
            drop(Box::from_raw(pointer));
        }
    }
}

pub fn refresh(app: &mut App, hwnd: HWND) {
    let pointer = state(hwnd);
    if pointer.is_null() {
        return;
    }
    let state = unsafe { &mut *pointer };
    state.collections = collection_choices(app, &state.draft);
    state.preview_entries = preview_entries(app, &state.draft);
    state
        .draft
        .selected_collections
        .retain(|id| state.collections.iter().any(|c| &c.id == id));
    populate_collection_list(hwnd, state);
    update_status(hwnd, state);
    invalidate(hwnd);
}

fn collection_choices(app: &App, _config: &Config) -> Vec<CollectionChoice> {
    app.library
        .collections
        .iter()
        .map(|collection| {
            let pair = collection
                .entries
                .iter()
                .find_map(|entry| {
                    let source = entry.source_language.trim();
                    let target = entry.target_language.trim();
                    if source.is_empty() && target.is_empty() {
                        None
                    } else {
                        Some(format!(
                            "{}{}{}",
                            source,
                            if source.is_empty() || target.is_empty() {
                                ""
                            } else {
                                " -> "
                            },
                            target
                        ))
                    }
                })
                .unwrap_or_else(|| "Language pair not set".to_string());
            let eligible = collection
                .entries
                .iter()
                .filter(|entry| !entry.learned)
                .count();
            CollectionChoice {
                id: collection.id.clone(),
                name: collection.name.clone(),
                language_pair: pair,
                eligible,
                total: collection.entries.len(),
            }
        })
        .collect()
}

fn preview_entries(app: &App, config: &Config) -> Vec<(String, crate::model::Entry)> {
    app.library
        .collections
        .iter()
        .filter_map(|collection| {
            collection
                .entries
                .iter()
                .find(|entry| !config.exclude_learned || !entry.learned)
                .cloned()
                .map(|entry| (collection.id.clone(), entry))
        })
        .collect()
}

fn state(hwnd: HWND) -> *mut State {
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State }
}

fn create_controls(hwnd: HWND, state: &State) {
    let dpi = state.chrome.dpi;
    let font = state.chrome.font_handle();
    let create = |class: &str, text: &str, style: u32, control: u16, area: DipRect| {
        let _ = ui::create_child(hwnd, class, text, style, control, area, dpi, font);
    };
    create(
        "STATIC",
        if state.first_run {
            "Choose what to learn and how WordDock should look."
        } else {
            "Change your words, timing, appearance, and screen position."
        },
        ui::STYLE_STATIC,
        180,
        rect(14, 10, 850, 22),
    );

    create(
        "STATIC",
        "1  CONTENT",
        ui::STYLE_STATIC,
        181,
        rect(LEFT, 38, LEFT_W, 20),
    );
    create(
        "LISTBOX",
        "",
        ui::STYLE_LISTBOX_MULTI,
        id::SET_COLLECTION,
        rect(LEFT, 60, LEFT_W, 92),
    );
    create(
        "STATIC",
        "",
        ui::STYLE_STATIC,
        id::SET_CONTENT_HINT,
        rect(LEFT, 154, LEFT_W, 24),
    );
    create(
        "BUTTON",
        "Import CSV/JSON...",
        ui::STYLE_BUTTON,
        id::SET_CONTENT_IMPORT,
        rect(LEFT, 182, 150, 24),
    );
    create(
        "BUTTON",
        "Manage entries...",
        ui::STYLE_BUTTON,
        id::SET_CONTENT_MANAGE,
        rect(LEFT + 160, 182, 150, 24),
    );

    create(
        "STATIC",
        "2  TIMING",
        ui::STYLE_STATIC,
        182,
        rect(LEFT, 212, LEFT_W, 20),
    );
    create(
        "STATIC",
        "Rotate every",
        ui::STYLE_STATIC,
        id::SET_INTERVAL_LABEL,
        rect(LEFT, 239, 100, 20),
    );
    create(
        "COMBOBOX",
        "",
        ui::STYLE_COMBO,
        id::SET_INTERVAL,
        rect(LEFT + 104, 234, 144, 150),
    );
    create(
        "EDIT",
        "15",
        ui::STYLE_EDIT,
        id::SET_CUSTOM_INTERVAL,
        rect(LEFT + 315, 234, 68, 24),
    );
    create(
        "STATIC",
        "custom minutes",
        ui::STYLE_STATIC,
        183,
        rect(LEFT + 250, 239, 64, 18),
    );
    create(
        "STATIC",
        "Rotation order",
        ui::STYLE_STATIC,
        id::SET_ORDER_LABEL,
        rect(LEFT, 275, 100, 20),
    );
    create(
        "COMBOBOX",
        "",
        ui::STYLE_COMBO,
        id::SET_ORDER,
        rect(LEFT + 104, 270, 250, 140),
    );
    create(
        "BUTTON",
        "Pause rotation",
        ui::STYLE_CHECKBOX,
        id::SET_PAUSE,
        rect(LEFT + 104, 304, 200, 22),
    );
    create(
        "STATIC",
        "Minimize hides the card; pause stops word changes.",
        ui::STYLE_STATIC,
        184,
        rect(LEFT + 104, 329, 290, 20),
    );

    create(
        "STATIC",
        "4  POSITION",
        ui::STYLE_STATIC,
        185,
        rect(LEFT, 350, LEFT_W, 20),
    );
    for index in 0..POSITION_IDS.len() {
        let row = if index < 3 {
            0
        } else if index < 5 {
            1
        } else {
            2
        };
        let col = match index {
            0 | 3 | 5 => 0,
            1 | 6 => 1,
            2 | 4 | 7 => 2,
            _ => 0,
        };
        let x = LEFT + col * 38;
        let y = 374 + row * 26;
        create(
            "BUTTON",
            POSITION_LABELS[index],
            ui::STYLE_BUTTON,
            POSITION_IDS[index],
            rect(x, y, 34, 24),
        );
    }
    create(
        "STATIC",
        "Monitor",
        ui::STYLE_STATIC,
        186,
        rect(LEFT + 128, 376, 64, 18),
    );
    create(
        "COMBOBOX",
        "",
        ui::STYLE_COMBO,
        id::SET_MONITOR,
        rect(LEFT + 195, 371, 200, 150),
    );
    create(
        "STATIC",
        "Edge margin",
        ui::STYLE_STATIC,
        187,
        rect(LEFT + 128, 411, 70, 18),
    );
    create(
        "EDIT",
        "12",
        ui::STYLE_EDIT,
        id::SET_MARGIN,
        rect(LEFT + 202, 406, 60, 24),
    );
    create(
        "STATIC",
        "DIP",
        ui::STYLE_STATIC,
        188,
        rect(LEFT + 267, 411, 35, 18),
    );
    create(
        "STATIC",
        "Choose an edge; drag the card later for a custom spot.",
        ui::STYLE_STATIC,
        id::SET_POSITION_HINT,
        rect(LEFT + 128, 438, 280, 24),
    );

    create(
        "STATIC",
        "3  APPEARANCE",
        ui::STYLE_STATIC,
        189,
        rect(RIGHT, 38, RIGHT_W, 20),
    );
    create(
        "STATIC",
        "Card style",
        ui::STYLE_STATIC,
        id::SET_PRESENTATION_LABEL,
        rect(RIGHT, 66, 100, 20),
    );
    create(
        "COMBOBOX",
        "",
        ui::STYLE_COMBO,
        id::SET_PRESENTATION,
        rect(RIGHT + 112, 60, 310, 150),
    );
    create(
        "STATIC",
        "Theme",
        ui::STYLE_STATIC,
        id::SET_THEME_LABEL,
        rect(RIGHT, 100, 100, 20),
    );
    create(
        "COMBOBOX",
        "",
        ui::STYLE_COMBO,
        id::SET_THEME,
        rect(RIGHT + 112, 94, 180, 150),
    );
    create(
        "BUTTON",
        "Background...",
        ui::STYLE_BUTTON,
        id::SET_COLOR_BACKGROUND,
        rect(RIGHT, 126, 130, 24),
    );
    create(
        "BUTTON",
        "Main text...",
        ui::STYLE_BUTTON,
        id::SET_COLOR_TEXT,
        rect(RIGHT + 138, 126, 120, 24),
    );
    create(
        "BUTTON",
        "Secondary...",
        ui::STYLE_BUTTON,
        id::SET_COLOR_SECONDARY,
        rect(RIGHT + 266, 126, 140, 24),
    );
    create(
        "STATIC",
        "Font size",
        ui::STYLE_STATIC,
        id::SET_FONT_LABEL,
        rect(RIGHT, 160, 100, 20),
    );
    create(
        "EDIT",
        "18",
        ui::STYLE_EDIT,
        id::SET_FONT,
        rect(RIGHT + 112, 156, 64, 24),
    );
    create(
        "STATIC",
        "pt",
        ui::STYLE_STATIC,
        190,
        rect(RIGHT + 180, 161, 25, 18),
    );
    create(
        "STATIC",
        "Card width",
        ui::STYLE_STATIC,
        id::SET_WIDTH_LABEL,
        rect(RIGHT + 220, 160, 86, 20),
    );
    create(
        "EDIT",
        "260",
        ui::STYLE_EDIT,
        id::SET_WIDTH,
        rect(RIGHT + 310, 156, 64, 24),
    );
    create(
        "STATIC",
        "DIP",
        ui::STYLE_STATIC,
        191,
        rect(RIGHT + 378, 161, 34, 18),
    );
    create(
        "BUTTON",
        "Keep above other windows",
        ui::STYLE_CHECKBOX,
        id::SET_TOPMOST,
        rect(RIGHT, 188, 210, 22),
    );
    create(
        "BUTTON",
        "Start when I sign in",
        ui::STYLE_CHECKBOX,
        id::SET_LAUNCH,
        rect(RIGHT + 220, 188, 190, 22),
    );
    create(
        "BUTTON",
        "Hide during fullscreen apps",
        ui::STYLE_CHECKBOX,
        id::SET_HIDE_FULLSCREEN,
        rect(RIGHT, 214, 260, 22),
    );
    create(
        "BUTTON",
        "Skip learned words",
        ui::STYLE_CHECKBOX,
        id::SET_EXCLUDE_LEARNED,
        rect(RIGHT + 270, 214, 150, 22),
    );

    create(
        "STATIC",
        "LIVE PREVIEW",
        ui::STYLE_STATIC,
        192,
        rect(RIGHT, 244, RIGHT_W, 18),
    );
    create(
        "STATIC",
        "",
        ui::STYLE_STATIC,
        id::SET_CONTRAST,
        rect(RIGHT, 384, RIGHT_W, 28),
    );
    create(
        "STATIC",
        "Preview uses real stored words; no content is generated.",
        ui::STYLE_STATIC,
        id::SET_HINT,
        rect(RIGHT, 416, RIGHT_W, 22),
    );

    create(
        "BUTTON",
        "Restore defaults",
        ui::STYLE_BUTTON,
        id::RESTORE_DEFAULTS,
        rect(LEFT, 450, 130, 24),
    );
    create(
        "BUTTON",
        if state.first_run { "Not now" } else { "Cancel" },
        ui::STYLE_BUTTON,
        id::CANCEL,
        rect(LEFT + 475, 450, 88, 24),
    );
    create(
        "BUTTON",
        "Apply",
        ui::STYLE_BUTTON,
        id::APPLY,
        rect(LEFT + 575, 450, 88, 24),
    );
    create(
        "BUTTON",
        if state.first_run {
            "Start learning"
        } else {
            "Save & close"
        },
        ui::STYLE_BUTTON_DEFAULT,
        id::START_LEARNING,
        rect(LEFT + 675, 450, 170, 24),
    );
    if state.first_run
        && let Some(apply) = win::child(hwnd, id::APPLY)
    {
        win::hide_window(apply);
    }
}

fn place_controls(hwnd: HWND, dpi: u32) {
    let layout = [
        (180, rect(14, 10, 850, 22)),
        (181, rect(LEFT, 38, LEFT_W, 20)),
        (id::SET_COLLECTION, rect(LEFT, 60, LEFT_W, 92)),
        (id::SET_CONTENT_HINT, rect(LEFT, 154, LEFT_W, 24)),
        (id::SET_CONTENT_IMPORT, rect(LEFT, 182, 150, 24)),
        (id::SET_CONTENT_MANAGE, rect(LEFT + 160, 182, 150, 24)),
        (182, rect(LEFT, 212, LEFT_W, 20)),
        (id::SET_INTERVAL_LABEL, rect(LEFT, 239, 100, 20)),
        (id::SET_INTERVAL, rect(LEFT + 104, 234, 144, 150)),
        (183, rect(LEFT + 250, 239, 64, 18)),
        (id::SET_CUSTOM_INTERVAL, rect(LEFT + 315, 234, 68, 24)),
        (id::SET_ORDER_LABEL, rect(LEFT, 275, 100, 20)),
        (id::SET_ORDER, rect(LEFT + 104, 270, 250, 140)),
        (id::SET_PAUSE, rect(LEFT + 104, 304, 200, 22)),
        (184, rect(LEFT + 104, 329, 290, 20)),
        (185, rect(LEFT, 350, LEFT_W, 20)),
        (id::POSITION_0, rect(LEFT, 374, 34, 24)),
        (id::POSITION_1, rect(LEFT + 38, 374, 34, 24)),
        (id::POSITION_2, rect(LEFT + 76, 374, 34, 24)),
        (id::POSITION_3, rect(LEFT, 400, 34, 24)),
        (id::POSITION_4, rect(LEFT + 76, 400, 34, 24)),
        (id::POSITION_5, rect(LEFT, 426, 34, 24)),
        (id::POSITION_6, rect(LEFT + 38, 426, 34, 24)),
        (id::POSITION_7, rect(LEFT + 76, 426, 34, 24)),
        (186, rect(LEFT + 128, 376, 64, 18)),
        (id::SET_MONITOR, rect(LEFT + 195, 371, 200, 150)),
        (187, rect(LEFT + 128, 411, 70, 18)),
        (id::SET_MARGIN, rect(LEFT + 202, 406, 60, 24)),
        (188, rect(LEFT + 267, 411, 35, 18)),
        (id::SET_POSITION_HINT, rect(LEFT + 128, 438, 280, 20)),
        (189, rect(RIGHT, 38, RIGHT_W, 20)),
        (id::SET_PRESENTATION_LABEL, rect(RIGHT, 66, 100, 20)),
        (id::SET_PRESENTATION, rect(RIGHT + 112, 60, 310, 150)),
        (id::SET_THEME_LABEL, rect(RIGHT, 100, 100, 20)),
        (id::SET_THEME, rect(RIGHT + 112, 94, 180, 150)),
        (id::SET_COLOR_BACKGROUND, rect(RIGHT, 126, 130, 24)),
        (id::SET_COLOR_TEXT, rect(RIGHT + 138, 126, 120, 24)),
        (id::SET_COLOR_SECONDARY, rect(RIGHT + 266, 126, 140, 24)),
        (id::SET_FONT_LABEL, rect(RIGHT, 160, 100, 20)),
        (id::SET_FONT, rect(RIGHT + 112, 156, 64, 24)),
        (190, rect(RIGHT + 180, 161, 25, 18)),
        (id::SET_WIDTH_LABEL, rect(RIGHT + 220, 160, 86, 20)),
        (id::SET_WIDTH, rect(RIGHT + 310, 156, 64, 24)),
        (191, rect(RIGHT + 378, 161, 34, 18)),
        (id::SET_TOPMOST, rect(RIGHT, 188, 210, 22)),
        (id::SET_LAUNCH, rect(RIGHT + 220, 188, 190, 22)),
        (id::SET_HIDE_FULLSCREEN, rect(RIGHT, 214, 260, 22)),
        (id::SET_EXCLUDE_LEARNED, rect(RIGHT + 270, 214, 150, 22)),
        (192, rect(RIGHT, 244, RIGHT_W, 18)),
        (id::SET_CONTRAST, rect(RIGHT, 384, RIGHT_W, 28)),
        (id::SET_HINT, rect(RIGHT, 416, RIGHT_W, 22)),
        (id::RESTORE_DEFAULTS, rect(LEFT, 450, 130, 24)),
        (id::CANCEL, rect(LEFT + 475, 450, 88, 24)),
        (id::APPLY, rect(LEFT + 575, 450, 88, 24)),
        (id::START_LEARNING, rect(LEFT + 675, 450, 170, 24)),
    ];
    for (control, area) in layout {
        ui::place(hwnd, control, area, dpi);
    }
}

fn populate_collection_list(hwnd: HWND, state: &mut State) {
    if state.first_run
        && state.draft.selected_collections.is_empty()
        && let Some(first) = state.collections.first()
    {
        state.draft.selected_collections.push(first.id.clone());
        state.draft.selection_initialized = true;
    }
    let Some(list) = win::child(hwnd, id::SET_COLLECTION) else {
        return;
    };
    ui::list_reset(list);
    for item in &state.collections {
        ui::list_add(
            list,
            &format!(
                "{}  |  {}  |  {}/{} eligible",
                item.name,
                item.language_pair,
                if state.draft.exclude_learned {
                    item.eligible
                } else {
                    item.total
                },
                item.total
            ),
        );
    }
    let selected: Vec<usize> = state
        .collections
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            state
                .draft
                .selected_collections
                .contains(&item.id)
                .then_some(index)
        })
        .collect();
    ui::list_set_multi_selection(list, &selected);
}

fn populate_monitor_combo(hwnd: HWND, state: &State) {
    let Some(combo) = win::child(hwnd, id::SET_MONITOR) else {
        return;
    };
    ui::combo_reset(combo);
    for monitor in &state.monitors {
        let width = monitor.bounds.right - monitor.bounds.left;
        let height = monitor.bounds.bottom - monitor.bounds.top;
        ui::combo_add(
            combo,
            &format!("{} | {} Ã— {}", monitor.device, width, height),
        );
    }
    let selected = state.draft.position_monitor.as_str();
    let index = state
        .monitors
        .iter()
        .position(|monitor| monitor.device == selected)
        .or_else(|| {
            state.monitors.iter().position(|monitor| {
                state
                    .draft
                    .position
                    .as_ref()
                    .is_some_and(|p| p.monitor == monitor.device)
            })
        })
        .or(if state.monitors.is_empty() {
            None
        } else {
            Some(0)
        });
    ui::combo_select(combo, index);
}

fn populate_combo(hwnd: HWND, control: u16, values: &[&str], selected: usize) {
    if let Some(combo) = win::child(hwnd, control) {
        ui::combo_reset(combo);
        for value in values {
            ui::combo_add(combo, value);
        }
        ui::combo_select(combo, Some(selected.min(values.len().saturating_sub(1))));
    }
}

fn load_controls(hwnd: HWND, state: &mut State) {
    let was_loading = state.loading_controls;
    state.loading_controls = true;
    let custom_interval = !INTERVALS.contains(&state.draft.interval_minutes);
    populate_combo(
        hwnd,
        id::SET_INTERVAL,
        &[
            "1 minute",
            "5 minutes",
            "15 minutes",
            "30 minutes",
            "60 minutes",
            "Custom",
        ],
        INTERVALS
            .iter()
            .position(|value| *value == state.draft.interval_minutes)
            .unwrap_or(5),
    );
    ui::set_edit_text(
        hwnd,
        id::SET_CUSTOM_INTERVAL,
        &state.draft.interval_minutes.to_string(),
    );
    ui::set_enabled(
        hwnd,
        id::SET_CUSTOM_INTERVAL,
        custom_interval || combo_is_custom(hwnd),
    );
    populate_combo(
        hwnd,
        id::SET_ORDER,
        &["Sequential", "Shuffled (no repeat per cycle)"],
        if state.draft.order == Order::Sequential {
            0
        } else {
            1
        },
    );
    populate_combo(
        hwnd,
        id::SET_PRESENTATION,
        &[
            PresentationMode::Basic.label(),
            PresentationMode::MeaningOnHover.label(),
            PresentationMode::WithExample.label(),
        ],
        match state.draft.presentation {
            PresentationMode::Basic => 0,
            PresentationMode::MeaningOnHover => 1,
            PresentationMode::WithExample => 2,
        },
    );
    populate_combo(
        hwnd,
        id::SET_THEME,
        &["System", "Light", "Dark", "Custom colors"],
        match state.draft.theme {
            ThemeMode::System => 0,
            ThemeMode::Light => 1,
            ThemeMode::Dark => 2,
            ThemeMode::Custom => 3,
        },
    );
    ui::set_edit_text(hwnd, id::SET_FONT, &state.draft.font_size_pt.to_string());
    ui::set_edit_text(hwnd, id::SET_WIDTH, &state.draft.card_width_dip.to_string());
    ui::set_edit_text(hwnd, id::SET_MARGIN, &state.draft.margin_dip.to_string());
    populate_collection_list(hwnd, state);
    populate_monitor_combo(hwnd, state);
    win::set_checked(hwnd, id::SET_TOPMOST, state.draft.always_on_top);
    win::set_checked(hwnd, id::SET_LAUNCH, state.draft.launch_at_login);
    win::set_checked(hwnd, id::SET_EXCLUDE_LEARNED, state.draft.exclude_learned);
    win::set_checked(
        hwnd,
        id::SET_HIDE_FULLSCREEN,
        state.draft.hide_during_fullscreen,
    );
    win::set_checked(hwnd, id::SET_PAUSE, state.draft.paused);
    update_anchor_buttons(hwnd, state.draft.position_anchor);
    update_color_labels(hwnd, &state.draft);
    state.loading_controls = was_loading;
}

fn combo_is_custom(hwnd: HWND) -> bool {
    win::child(hwnd, id::SET_INTERVAL).and_then(ui::combo_selection) == Some(5)
}

fn update_anchor_buttons(hwnd: HWND, anchor: CardAnchor) {
    for index in 0..POSITION_IDS.len() {
        let text = if anchor.grid_index() == Some(index) {
            format!("{}  âœ“", POSITION_LABELS[index])
        } else {
            POSITION_LABELS[index].to_string()
        };
        if let Some(button) = win::child(hwnd, POSITION_IDS[index]) {
            win::set_window_text(button, &text);
        }
    }
}

fn hex_color(color: u32) -> String {
    format!(
        "#{:02X}{:02X}{:02X}",
        color & 0xff,
        (color >> 8) & 0xff,
        (color >> 16) & 0xff
    )
}

fn update_color_labels(hwnd: HWND, config: &Config) {
    ui::static_text(
        hwnd,
        id::SET_COLOR_BACKGROUND,
        &format!("Background {}...", hex_color(config.custom_background)),
    );
    ui::static_text(
        hwnd,
        id::SET_COLOR_TEXT,
        &format!("Text {}...", hex_color(config.custom_text)),
    );
    ui::static_text(
        hwnd,
        id::SET_COLOR_SECONDARY,
        &format!("Secondary {}...", hex_color(config.custom_secondary)),
    );
}

fn read_draft(hwnd: HWND, state: &State) -> Config {
    let mut config = state.draft.clone();
    let interval_kind = win::child(hwnd, id::SET_INTERVAL)
        .and_then(ui::combo_selection)
        .unwrap_or(2);
    config.interval_minutes = if interval_kind < INTERVALS.len() {
        INTERVALS[interval_kind]
    } else {
        win::control_text(hwnd, id::SET_CUSTOM_INTERVAL)
            .parse()
            .unwrap_or(config.interval_minutes)
    };
    config.order = if win::child(hwnd, id::SET_ORDER).and_then(ui::combo_selection) == Some(1) {
        Order::Shuffled
    } else {
        Order::Sequential
    };
    config.presentation = match win::child(hwnd, id::SET_PRESENTATION)
        .and_then(ui::combo_selection)
        .unwrap_or(0)
    {
        1 => PresentationMode::MeaningOnHover,
        2 => PresentationMode::WithExample,
        _ => PresentationMode::Basic,
    };
    config.theme = match win::child(hwnd, id::SET_THEME)
        .and_then(ui::combo_selection)
        .unwrap_or(0)
    {
        1 => ThemeMode::Light,
        2 => ThemeMode::Dark,
        3 => ThemeMode::Custom,
        _ => ThemeMode::System,
    };
    config.font_size_pt = win::control_text(hwnd, id::SET_FONT)
        .parse()
        .unwrap_or(config.font_size_pt);
    config.card_width_dip = win::control_text(hwnd, id::SET_WIDTH)
        .parse()
        .unwrap_or(config.card_width_dip);
    config.margin_dip = win::control_text(hwnd, id::SET_MARGIN)
        .parse()
        .unwrap_or(config.margin_dip);
    config.selected_collections = win::child(hwnd, id::SET_COLLECTION)
        .map(ui::list_selected_indices)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|index| state.collections.get(index).map(|item| item.id.clone()))
        .collect();
    config.selection_initialized = true;
    if let Some(index) = win::child(hwnd, id::SET_MONITOR).and_then(ui::combo_selection)
        && let Some(monitor) = state.monitors.get(index)
    {
        config.position_monitor = monitor.device.clone();
    }
    config.always_on_top = win::is_checked(hwnd, id::SET_TOPMOST);
    config.launch_at_login = win::is_checked(hwnd, id::SET_LAUNCH);
    config.exclude_learned = win::is_checked(hwnd, id::SET_EXCLUDE_LEARNED);
    config.hide_during_fullscreen = win::is_checked(hwnd, id::SET_HIDE_FULLSCREEN);
    config.paused = win::is_checked(hwnd, id::SET_PAUSE);
    config
}

fn update_status(hwnd: HWND, state: &State) {
    let draft = read_draft(hwnd, state);
    let selected: Vec<&CollectionChoice> = draft
        .selected_collections
        .iter()
        .filter_map(|id| state.collections.iter().find(|item| &item.id == id))
        .collect();
    let eligible: usize = selected
        .iter()
        .map(|item| {
            if draft.exclude_learned {
                item.eligible
            } else {
                item.total
            }
        })
        .sum();
    let message = if selected.is_empty() {
        "Click one or more collections to include them. Import or manage words below.".to_string()
    } else if eligible == 0 {
        "The selected collections have no eligible words. Import words or review learned entries."
            .to_string()
    } else {
        format!(
            "{} collection(s) selected | {eligible} eligible word(s).",
            selected.len()
        )
    };
    ui::static_text(hwnd, id::SET_CONTENT_HINT, &message);
    ui::set_enabled(hwnd, id::START_LEARNING, !state.first_run || eligible > 0);
    let ratio_main = theme::contrast_ratio(draft.custom_background, draft.custom_text);
    let ratio_secondary = theme::contrast_ratio(draft.custom_background, draft.custom_secondary);
    let contrast =
        if draft.theme != ThemeMode::Custom || (ratio_main >= 4.5 && ratio_secondary >= 4.5) {
            format!(
                "Text contrast: main {:.1}:1 | secondary {:.1}:1",
                ratio_main, ratio_secondary
            )
        } else {
            format!(
                "Low contrast: main {:.1}:1 | secondary {:.1}:1. Choose clearer colors.",
                ratio_main, ratio_secondary
            )
        };
    ui::static_text(hwnd, id::SET_CONTRAST, &contrast);
}

fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
    }
}

fn refresh_preview(hwnd: HWND, state: &mut State) {
    let next = read_draft(hwnd, state);
    if next.font_size_pt != state.draft.font_size_pt {
        state.preview_fonts = crate::ui::card::create_fonts(&next, state.chrome.dpi);
    }
    state.draft = next;
    if let Some(app) = unsafe { state.app.as_ref() } {
        state.collections = collection_choices(app, &state.draft);
        populate_collection_list(hwnd, state);
    }
    update_status(hwnd, state);
    update_color_labels(hwnd, &state.draft);
    invalidate(hwnd);
}

fn paint(hwnd: HWND, state: &State) {
    let mut ps = PAINTSTRUCT::default();
    unsafe {
        let dc = BeginPaint(hwnd, &mut ps);
        let palette = theme::palette_for_config(&state.draft);
        let background = CreateSolidBrush(palette.dialog_bg);
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        FillRect(dc, &client, background);
        let _ = DeleteObject(HGDIOBJ(background.0));

        let dpi = state.chrome.dpi;
        let preview_width = ui::px(PREVIEW_SIZE_DIP.0, dpi);
        let preview_height = ui::px(PREVIEW_SIZE_DIP.1, dpi);
        let width = ui::px(state.draft.card_width_dip, dpi).min(preview_width);
        let content = preview_content(state);
        let layout = crate::ui::card::measure_layout(
            &state.draft,
            &content,
            &state.preview_fonts,
            width,
            dpi,
            true,
            false,
            preview_height,
        );
        let preview_rect = RECT {
            left: ui::px(PREVIEW_DIP.0, dpi),
            top: ui::px(PREVIEW_DIP.1, dpi),
            right: ui::px(PREVIEW_DIP.0, dpi) + width,
            bottom: ui::px(PREVIEW_DIP.1, dpi) + layout.height.min(preview_height),
        };
        SetBkMode(dc, TRANSPARENT);
        let saved = SaveDC(dc);
        let _ = SetViewportOrgEx(dc, preview_rect.left, preview_rect.top, None);
        crate::ui::card::draw_to_dc(
            dc,
            &layout,
            &content,
            palette,
            Some(&state.preview_fonts),
            None,
            width,
            layout.height.min(preview_height),
            dpi,
        );
        if saved != 0 {
            let _ = RestoreDC(dc, saved);
        }
        let _ = EndPaint(hwnd, &ps);
    }
}

fn preview_content(state: &State) -> crate::app::CardContent {
    let entry = state.draft.selected_collections.iter().find_map(|id| {
        state
            .preview_entries
            .iter()
            .find(|(collection_id, _)| collection_id == id)
            .map(|(_, entry)| entry)
    });
    match entry {
        Some(entry) => crate::app::CardContent {
            word: entry.word.clone(),
            meaning: entry.meaning.clone(),
            example: entry.example.clone(),
            presentation: state.draft.presentation,
            meta: String::new(),
            has_entry: true,
            learned: entry.learned,
            paused: state.draft.paused,
            can_navigate: true,
            empty_hint: None,
        },
        None => crate::app::CardContent {
            word: "Choose a collection".to_string(),
            meaning: "Preview uses your stored words.".to_string(),
            example: String::new(),
            presentation: state.draft.presentation,
            meta: String::new(),
            has_entry: false,
            learned: false,
            paused: state.draft.paused,
            can_navigate: false,
            empty_hint: Some("empty".to_string()),
        },
    }
}

fn valid_draft(config: &Config, first_run: bool, eligible: usize) -> Result<(), String> {
    if !(MIN_INTERVAL_MINUTES..=MAX_INTERVAL_MINUTES).contains(&config.interval_minutes) {
        return Err(format!(
            "Choose an interval from {MIN_INTERVAL_MINUTES} to {MAX_INTERVAL_MINUTES} minutes."
        ));
    }
    if !(MIN_FONT_SIZE_PT..=MAX_FONT_SIZE_PT).contains(&config.font_size_pt) {
        return Err(format!(
            "Font size must be {MIN_FONT_SIZE_PT}â€“{MAX_FONT_SIZE_PT} points."
        ));
    }
    if !(MIN_CARD_WIDTH_DIP..=MAX_CARD_WIDTH_DIP).contains(&config.card_width_dip) {
        return Err(format!(
            "Card width must be {MIN_CARD_WIDTH_DIP}â€“{MAX_CARD_WIDTH_DIP} DIP."
        ));
    }
    if first_run && eligible == 0 {
        return Err(
            "Select a collection with eligible words, or import/manage entries first.".to_string(),
        );
    }
    if config.theme == ThemeMode::Custom {
        let main = theme::contrast_ratio(config.custom_background, config.custom_text);
        let secondary = theme::contrast_ratio(config.custom_background, config.custom_secondary);
        if main < 4.5 || secondary < 4.5 {
            return Err("The selected text colors are hard to read against the background. Adjust them until both contrast ratios are at least 4.5:1.".to_string());
        }
    }
    Ok(())
}

fn selected_eligible(state: &State, config: &Config) -> usize {
    config
        .selected_collections
        .iter()
        .filter_map(|id| state.collections.iter().find(|c| &c.id == id))
        .map(|choice| {
            if config.exclude_learned {
                choice.eligible
            } else {
                choice.total
            }
        })
        .sum()
}

fn save(hwnd: HWND, pointer: *mut State, start: bool, close: bool) -> bool {
    let state = unsafe { &mut *pointer };
    if state.first_run && !start {
        ui::static_text(
            hwnd,
            id::SET_HINT,
            "Press Start learning when you are ready. Nothing has been saved yet.",
        );
        return false;
    }
    let mut config = read_draft(hwnd, state);
    let eligible = selected_eligible(state, &config);
    if let Err(message) = valid_draft(&config, start && state.first_run, eligible) {
        ui::static_text(hwnd, id::SET_HINT, &message);
        return false;
    }
    if start {
        config.card_visible = true;
    }
    config.selection_initialized = true;
    let primary = config.selected_collections.first().cloned();
    state.draft = config.clone();
    let app_ptr = state.app;
    // All file dialogs are completed before borrowing App; this path only
    // applies already-collected values and posts any notices asynchronously.
    if let Some(app) = unsafe { app_ptr.as_mut() } {
        app.apply_config(config, hwnd, primary);
        if start {
            app.first_run_setup = false;
            app.show_card();
        }
    }
    if close {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    } else {
        invalidate(hwnd);
    }
    true
}

fn restore_defaults(hwnd: HWND, state: &mut State) {
    let mut defaults = Config::default();
    if let Some(first) = state.collections.first() {
        defaults.selected_collections = vec![first.id.clone()];
    }
    defaults.selection_initialized = true;
    if let Some(monitor) = state.monitors.first() {
        defaults.position_monitor = monitor.device.clone();
    }
    state.draft = defaults;
    state.preview_fonts = crate::ui::card::create_fonts(&state.draft, state.chrome.dpi);
    load_controls(hwnd, state);
    refresh_preview(hwnd, state);
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
    let pointer = state(hwnd);
    if pointer.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    match msg {
        WM_CREATE => {
            let s = unsafe { &mut *pointer };
            s.loading_controls = true;
            create_controls(hwnd, s);
            load_controls(hwnd, s);
            populate_monitor_combo(hwnd, s);
            place_controls(hwnd, s.chrome.dpi);
            update_status(hwnd, s);
            s.loading_controls = false;
            LRESULT(0)
        }
        WM_COMMAND => {
            if unsafe { (*pointer).loading_controls } {
                return LRESULT(0);
            }
            let command = ui::command_id(wparam);
            if command == id::CANCEL {
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
            } else if command == id::RESTORE_DEFAULTS {
                restore_defaults(hwnd, unsafe { &mut *pointer });
            } else if command == id::APPLY {
                let _ = save(hwnd, pointer, false, false);
            } else if command == id::START_LEARNING {
                let setup = unsafe { (*pointer).first_run };
                let _ = save(hwnd, pointer, setup, true);
            } else if command == id::SET_CONTENT_IMPORT {
                let selected = read_draft(hwnd, unsafe { &*pointer })
                    .selected_collections
                    .first()
                    .cloned();
                if let Some(target) = selected {
                    let filters = [
                        win::FileFilter {
                            label: "Vocabulary files",
                            pattern: "*.csv;*.json",
                        },
                        win::FileFilter {
                            label: "CSV",
                            pattern: "*.csv",
                        },
                        win::FileFilter {
                            label: "JSON",
                            pattern: "*.json",
                        },
                    ];
                    if let Some(path) = win::pick_open_file(hwnd, "Import vocabulary", &filters) {
                        let app_ptr = unsafe { (*pointer).app };
                        if let Some(app) = unsafe { app_ptr.as_mut() } {
                            app.start_import_into(path, target);
                        }
                    }
                } else {
                    ui::static_text(
                        hwnd,
                        id::SET_CONTENT_HINT,
                        "Select a collection before importing into it.",
                    );
                }
            } else if command == id::SET_CONTENT_MANAGE {
                let app_ptr = unsafe { (*pointer).app };
                if let Some(app) = unsafe { app_ptr.as_mut() } {
                    app.open_collections();
                }
            } else if matches!(
                command,
                id::SET_COLOR_BACKGROUND | id::SET_COLOR_TEXT | id::SET_COLOR_SECONDARY
            ) {
                let draft = read_draft(hwnd, unsafe { &*pointer });
                let current = match command {
                    id::SET_COLOR_BACKGROUND => draft.custom_background,
                    id::SET_COLOR_TEXT => draft.custom_text,
                    _ => draft.custom_secondary,
                };
                if let Some(color) = win::pick_color(hwnd, current) {
                    let s = unsafe { &mut *pointer };
                    s.draft = draft;
                    s.draft.theme = ThemeMode::Custom;
                    match command {
                        id::SET_COLOR_BACKGROUND => s.draft.custom_background = color,
                        id::SET_COLOR_TEXT => s.draft.custom_text = color,
                        _ => s.draft.custom_secondary = color,
                    }
                    load_controls(hwnd, s);
                    refresh_preview(hwnd, s);
                }
            } else {
                let s = unsafe { &mut *pointer };
                s.draft = read_draft(hwnd, s);
                if command == id::SET_COLLECTION {
                    s.draft.selected_collections = read_draft(hwnd, s).selected_collections;
                }
                if (id::POSITION_0..=id::POSITION_7).contains(&command) {
                    let index = (command - id::POSITION_0) as usize;
                    s.draft.position_anchor = CardAnchor::GRID[index];
                    update_anchor_buttons(hwnd, s.draft.position_anchor);
                }
                if command == id::SET_THEME && s.draft.theme == ThemeMode::Custom {
                    // The current custom colors are previewed immediately.
                }
                ui::set_enabled(hwnd, id::SET_CUSTOM_INTERVAL, combo_is_custom(hwnd));
                refresh_preview(hwnd, s);
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
            let s = unsafe { &*pointer };
            let palette = theme::palette_for_config(&s.draft);
            let brush = unsafe { CreateSolidBrush(palette.dialog_bg) };
            let mut client = RECT::default();
            unsafe {
                let _ = GetClientRect(hwnd, &mut client);
                FillRect(HDC(wparam.0 as *mut c_void), &client, brush);
                let _ = DeleteObject(HGDIOBJ(brush.0));
            }
            LRESULT(1)
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN => {
            let s = unsafe { &*pointer };
            s.chrome
                .ctl_color(msg, HDC(wparam.0 as *mut c_void))
                .unwrap_or_else(|| unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) })
        }
        WM_PAINT => {
            paint(hwnd, unsafe { &*pointer });
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let dpi = (wparam.0 & 0xffff) as u32;
            let s = unsafe { &mut *pointer };
            s.chrome.set_dpi(dpi);
            s.preview_fonts = crate::ui::card::create_fonts(&s.draft, dpi);
            ui::apply_font_to_children(hwnd, CHILDREN, s.chrome.font_handle());
            place_controls(hwnd, dpi);
            let suggested = lparam.0 as *const RECT;
            if !suggested.is_null() {
                let r = unsafe { *suggested };
                win::set_window_rect_no_activate(
                    hwnd,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                );
            }
            invalidate(hwnd);
            LRESULT(0)
        }
        WM_NCDESTROY => unsafe {
            if let Some(app) = (*pointer).app.as_mut() {
                app.settings_hwnd = None;
            }
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(Box::from_raw(pointer));
            DefWindowProcW(hwnd, msg, wparam, lparam)
        },
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_run_requires_real_eligible_content_and_custom_colors_must_be_readable() {
        let config = Config {
            interval_minutes: 1,
            ..Config::default()
        };
        assert!(valid_draft(&config, true, 0).is_err());
        assert!(valid_draft(&config, true, 20).is_ok());
        let unreadable = Config {
            theme: ThemeMode::Custom,
            custom_background: 0x00FF_FFFF,
            custom_text: 0x00FF_FFFF,
            custom_secondary: 0x00FF_FFFF,
            ..config
        };
        assert!(valid_draft(&unreadable, false, 0).is_err());
    }
}

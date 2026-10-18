//! Native collection and entry manager.

use std::ffi::c_void;

use crate::app::App;
use crate::model::{Collection, Entry};
use crate::ui::{self, Chrome, id, rect};
use crate::win;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::HDC;
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetWindowLongPtrW,
    SetWindowLongPtrW, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_CTLCOLORBTN, WM_CTLCOLOREDIT,
    WM_CTLCOLORLISTBOX, WM_CTLCOLORSTATIC, WM_DPICHANGED, WM_ERASEBKGND, WM_NCCREATE, WM_NCDESTROY,
    WM_PAINT,
};

pub const CLASS: &str = "WordDockCollectionsDialog";
pub const PROMPT_CLASS: &str = "WordDockTextPromptDialog";
const CLIENT: (i32, i32) = (740, 480);

struct State {
    app: *mut App,
    chrome: Chrome,
    collection_ids: Vec<String>,
    entry_ids: Vec<String>,
}

struct PromptState {
    app: *mut App,
    chrome: Chrome,
    kind: PromptKind,
    initial: String,
    initial_entry: Option<Entry>,
}

#[derive(Clone)]
enum PromptKind {
    NewCollection,
    RenameCollection(String),
    NewEntry(String),
    EditEntry(String, String),
}

pub fn class() -> ui::ClassSpec {
    ui::ClassSpec {
        name: CLASS,
        proc: Some(proc),
        cursor: ui::CURSOR_ARROW,
        background: ui::SYS_BTNFACE,
    }
}

pub fn prompt_class() -> ui::ClassSpec {
    ui::ClassSpec {
        name: PROMPT_CLASS,
        proc: Some(prompt_proc),
        cursor: ui::CURSOR_ARROW,
        background: ui::SYS_BTNFACE,
    }
}

pub fn open(app: &mut App) {
    if let Some(hwnd) = app.collections_hwnd {
        if win::is_window(hwnd) {
            win::activate_window(hwnd);
            return;
        }
        app.collections_hwnd = None;
    }
    let dpi = win::dpi_of_window(ui::reference_window(app)).max(96);
    let state = Box::new(State {
        app: app as *mut App,
        chrome: Chrome::new(app.config.theme, dpi),
        collection_ids: Vec::new(),
        entry_ids: Vec::new(),
    });
    let pointer = Box::into_raw(state);
    if let Some(hwnd) = ui::create_dialog_window(
        None,
        ui::reference_window(app),
        win::module_instance(),
        CLASS,
        "Manage collections",
        CLIENT,
        pointer as *mut c_void,
    ) {
        app.collections_hwnd = Some(hwnd);
    } else {
        // CreateWindowExW failed before WM_NCCREATE took ownership.
        unsafe {
            drop(Box::from_raw(pointer));
        }
    }
}

fn state(hwnd: HWND) -> *mut State {
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State }
}

fn create_controls(hwnd: HWND, state: &State) {
    let dpi = state.chrome.dpi;
    let font = state.chrome.font_handle();
    let create = |class: &str, text: &str, style: u32, control: u16, x, y, w, h| {
        ui::create_child(
            hwnd,
            class,
            text,
            style,
            control,
            rect(x, y, w, h),
            dpi,
            font,
        );
    };
    create(
        "STATIC",
        "Collections",
        ui::STYLE_STATIC,
        id::COL_HEADING,
        14,
        12,
        300,
        20,
    );
    create(
        "LISTBOX",
        "",
        ui::STYLE_LISTBOX,
        id::COL_LIST,
        14,
        38,
        260,
        300,
    );
    create(
        "BUTTON",
        "New",
        ui::STYLE_BUTTON,
        id::COL_NEW,
        14,
        348,
        78,
        26,
    );
    create(
        "BUTTON",
        "Rename",
        ui::STYLE_BUTTON,
        id::COL_RENAME,
        98,
        348,
        84,
        26,
    );
    create(
        "BUTTON",
        "Delete",
        ui::STYLE_BUTTON,
        id::COL_DELETE,
        188,
        348,
        86,
        26,
    );
    create(
        "BUTTON",
        "Make active",
        ui::STYLE_BUTTON,
        id::COL_ACTIVE,
        14,
        382,
        260,
        26,
    );
    create(
        "STATIC",
        "Entries",
        ui::STYLE_STATIC,
        id::ENTRY_HEADING,
        294,
        12,
        400,
        20,
    );
    create(
        "LISTBOX",
        "",
        ui::STYLE_LISTBOX,
        id::ENTRY_LIST,
        294,
        38,
        432,
        300,
    );
    create(
        "BUTTON",
        "Add",
        ui::STYLE_BUTTON,
        id::ENTRY_ADD,
        294,
        348,
        70,
        26,
    );
    create(
        "BUTTON",
        "Edit",
        ui::STYLE_BUTTON,
        id::ENTRY_EDIT,
        370,
        348,
        70,
        26,
    );
    create(
        "BUTTON",
        "Delete",
        ui::STYLE_BUTTON,
        id::ENTRY_DELETE,
        446,
        348,
        76,
        26,
    );
    create(
        "BUTTON",
        "Import…",
        ui::STYLE_BUTTON,
        id::ENTRY_IMPORT,
        528,
        348,
        92,
        26,
    );
    create(
        "BUTTON",
        "Export CSV…",
        ui::STYLE_BUTTON,
        id::ENTRY_EXPORT_CSV,
        294,
        382,
        132,
        26,
    );
    create(
        "BUTTON",
        "Export JSON…",
        ui::STYLE_BUTTON,
        id::ENTRY_EXPORT_JSON,
        434,
        382,
        138,
        26,
    );
    create(
        "STATIC",
        "",
        ui::STYLE_STATIC,
        id::ENTRY_SUMMARY,
        294,
        418,
        432,
        34,
    );
    create(
        "BUTTON",
        "Close",
        ui::STYLE_BUTTON_DEFAULT,
        id::CLOSE,
        640,
        440,
        86,
        26,
    );
}

pub fn repopulate(app: &mut App, hwnd: HWND) {
    let state_ptr = state(hwnd);
    if state_ptr.is_null() {
        return;
    }
    let state = unsafe { &mut *state_ptr };
    let selected_collection = win::child(hwnd, id::COL_LIST)
        .and_then(ui::list_selection)
        .and_then(|i| state.collection_ids.get(i).cloned())
        .or_else(|| app.library.active_collection.clone());
    let selected_entry = win::child(hwnd, id::ENTRY_LIST)
        .and_then(ui::list_selection)
        .and_then(|i| state.entry_ids.get(i).cloned());
    state.collection_ids.clear();
    state.entry_ids.clear();
    if let Some(list) = win::child(hwnd, id::COL_LIST) {
        ui::list_reset(list);
        for collection in &app.library.collections {
            state.collection_ids.push(collection.id.clone());
            ui::list_add(
                list,
                &format!(
                    "{}  ·  {} entries",
                    collection.name,
                    collection.entries.len()
                ),
            );
        }
        if let Some(index) = selected_collection
            .as_ref()
            .and_then(|id| state.collection_ids.iter().position(|item| item == id))
        {
            ui::list_select(list, Some(index));
        }
    }
    if let Some(list) = win::child(hwnd, id::ENTRY_LIST) {
        ui::list_reset(list);
        if let Some(collection) = selected_collection
            .as_deref()
            .and_then(|id| app.library.collection(id))
        {
            for entry in &collection.entries {
                state.entry_ids.push(entry.id.clone());
                ui::list_add(list, &entry.summary());
            }
            if let Some(index) = selected_entry
                .as_ref()
                .and_then(|id| state.entry_ids.iter().position(|item| item == id))
            {
                ui::list_select(list, Some(index));
            }
            ui::static_text(
                hwnd,
                id::ENTRY_SUMMARY,
                &format!(
                    "{} entries · {} learned",
                    collection.entries.len(),
                    collection.learned_count()
                ),
            );
        } else {
            ui::static_text(
                hwnd,
                id::ENTRY_SUMMARY,
                "Create or select a collection to manage entries.",
            );
        }
    }
}

fn selected_collection(state: &State, hwnd: HWND) -> Option<String> {
    win::child(hwnd, id::COL_LIST)
        .and_then(ui::list_selection)
        .and_then(|index| state.collection_ids.get(index).cloned())
}
fn selected_entry(state: &State, hwnd: HWND) -> Option<String> {
    win::child(hwnd, id::ENTRY_LIST)
        .and_then(ui::list_selection)
        .and_then(|index| state.entry_ids.get(index).cloned())
}

fn open_prompt(
    parent: HWND,
    app: *mut App,
    chrome_mode: crate::model::ThemeMode,
    dpi: u32,
    kind: PromptKind,
    initial: String,
    title: &str,
) {
    let initial_entry = match &kind {
        PromptKind::EditEntry(cid, eid) => unsafe { app.as_ref() }
            .and_then(|a| a.library.collection(cid))
            .and_then(|c| c.entries.iter().find(|e| e.id == *eid))
            .cloned(),
        _ => None,
    };
    let entry_mode = matches!(&kind, PromptKind::NewEntry(_) | PromptKind::EditEntry(_, _));
    let state = Box::new(PromptState {
        app,
        chrome: Chrome::new(chrome_mode, dpi),
        kind,
        initial,
        initial_entry,
    });
    let pointer = Box::into_raw(state);
    if ui::create_dialog_window(
        Some(parent),
        parent,
        win::module_instance(),
        PROMPT_CLASS,
        title,
        if entry_mode { (440, 360) } else { (390, 160) },
        pointer as *mut c_void,
    )
    .is_none()
    {
        unsafe {
            drop(Box::from_raw(pointer));
        }
    }
}

pub extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let pointer = unsafe { (*(lparam.0 as *const CREATESTRUCTW)).lpCreateParams as *mut State };
        if !pointer.is_null() {
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, pointer as isize);
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
            let s = unsafe { &*pointer };
            create_controls(hwnd, s);
            if !s.app.is_null() {
                repopulate(unsafe { &mut *(s.app) }, hwnd);
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let command = ui::command_id(wparam);
            let app_ptr = unsafe { (*pointer).app };
            let dpi = unsafe { (*pointer).chrome.dpi };
            let mode = unsafe { (*pointer).chrome.mode };
            if command == id::COL_NEW {
                open_prompt(
                    hwnd,
                    app_ptr,
                    mode,
                    dpi,
                    PromptKind::NewCollection,
                    String::new(),
                    "New collection",
                );
            } else if command == id::COL_RENAME {
                if let Some(cid) = selected_collection(unsafe { &*pointer }, hwnd) {
                    let name = unsafe { app_ptr.as_ref() }
                        .and_then(|a| a.library.collection(&cid))
                        .map(|c| c.name.clone())
                        .unwrap_or_default();
                    open_prompt(
                        hwnd,
                        app_ptr,
                        mode,
                        dpi,
                        PromptKind::RenameCollection(cid),
                        name,
                        "Rename collection",
                    );
                }
            } else if command == id::ENTRY_ADD {
                if let Some(cid) = selected_collection(unsafe { &*pointer }, hwnd) {
                    open_prompt(
                        hwnd,
                        app_ptr,
                        mode,
                        dpi,
                        PromptKind::NewEntry(cid),
                        String::new(),
                        "Add entry",
                    );
                }
            } else if command == id::ENTRY_EDIT {
                if let (Some(cid), Some(eid)) = (
                    selected_collection(unsafe { &*pointer }, hwnd),
                    selected_entry(unsafe { &*pointer }, hwnd),
                ) {
                    let word = unsafe { app_ptr.as_ref() }
                        .and_then(|a| a.library.collection(&cid))
                        .and_then(|c| c.entries.iter().find(|e| e.id == eid))
                        .map(|e| e.word.clone())
                        .unwrap_or_default();
                    open_prompt(
                        hwnd,
                        app_ptr,
                        mode,
                        dpi,
                        PromptKind::EditEntry(cid, eid),
                        word,
                        "Edit entry",
                    );
                }
            } else if let Some(app) = unsafe { app_ptr.as_mut() } {
                if command == id::COL_ACTIVE {
                    if let Some(cid) = selected_collection(unsafe { &*pointer }, hwnd) {
                        app.set_active_collection(&cid);
                    }
                } else if command == id::COL_DELETE {
                    if let Some(cid) = selected_collection(unsafe { &*pointer }, hwnd) {
                        if win::confirm(
                            hwnd,
                            "Delete this collection and all its entries? This cannot be undone.",
                            "WordDock",
                        ) {
                            app.library.collections.retain(|c| c.id != cid);
                            if app.library.active_collection.as_deref() == Some(&cid) {
                                app.library.active_collection =
                                    app.library.collections.first().map(|c| c.id.clone());
                                app.rotation.reset();
                            }
                            app.ensure_current();
                            app.save_library();
                            app.restart_timer();
                            app.refresh_card();
                        }
                    }
                } else if command == id::ENTRY_DELETE {
                    if let (Some(cid), Some(eid)) = (
                        selected_collection(unsafe { &*pointer }, hwnd),
                        selected_entry(unsafe { &*pointer }, hwnd),
                    ) {
                        if win::confirm(
                            hwnd,
                            "Delete this vocabulary entry? This cannot be undone.",
                            "WordDock",
                        ) {
                            if let Some(c) = app.library.collection_mut(&cid) {
                                c.entries.retain(|e| e.id != eid);
                            }
                            app.after_entries_changed();
                        }
                    }
                } else if command == id::ENTRY_IMPORT {
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
                        app.start_import(path);
                    }
                } else if command == id::ENTRY_EXPORT_CSV {
                    let _ = app.export_entries(crate::csvio::Format::Csv);
                } else if command == id::ENTRY_EXPORT_JSON {
                    let _ = app.export_entries(crate::csvio::Format::Json);
                } else if command == id::CLOSE {
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                }
            }
            if let Some(app) = unsafe { app_ptr.as_mut() } {
                repopulate(app, hwnd);
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
            if s.chrome.erase(hwnd, HDC(wparam.0 as *mut c_void)) {
                LRESULT(1)
            } else {
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN => {
            let s = unsafe { &*pointer };
            s.chrome
                .ctl_color(msg, HDC(wparam.0 as *mut c_void))
                .unwrap_or_else(|| unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) })
        }
        WM_PAINT => {
            unsafe { &*pointer }.chrome.paint(hwnd);
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let dpi = (wparam.0 & 0xffff) as u32;
            let s = unsafe { &mut *pointer };
            s.chrome.set_dpi(dpi);
            ui::apply_font_to_children(hwnd, &[], s.chrome.font_handle());
            LRESULT(0)
        }
        WM_NCDESTROY => unsafe {
            if let Some(app) = (*pointer).app.as_mut() {
                app.collections_hwnd = None;
            }
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(Box::from_raw(pointer));
            DefWindowProcW(hwnd, msg, wparam, lparam)
        },
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn prompt_controls(hwnd: HWND, state: &PromptState) {
    let dpi = state.chrome.dpi;
    let font = state.chrome.font_handle();
    match &state.kind {
        PromptKind::NewEntry(_) | PromptKind::EditEntry(_, _) => {
            let entry = state.initial_entry.as_ref();
            let add = |class, text, style, control, area| {
                ui::create_child(hwnd, class, text, style, control, area, dpi, font)
            };
            add(
                "STATIC",
                "Word",
                ui::STYLE_STATIC,
                id::FIELD_WORD_LABEL,
                rect(12, 12, 74, 16),
            );
            ui::create_child(
                hwnd,
                "EDIT",
                &state.initial,
                ui::STYLE_EDIT,
                id::FIELD_WORD,
                rect(92, 8, 332, 24),
                dpi,
                font,
            );
            add(
                "STATIC",
                "Meaning",
                ui::STYLE_STATIC,
                id::FIELD_MEANING_LABEL,
                rect(12, 44, 74, 16),
            );
            ui::create_child(
                hwnd,
                "EDIT",
                entry.map(|e| e.meaning.as_str()).unwrap_or(""),
                ui::STYLE_EDIT_MULTI,
                id::FIELD_MEANING,
                rect(92, 40, 332, 52),
                dpi,
                font,
            );
            add(
                "STATIC",
                "Example",
                ui::STYLE_STATIC,
                id::FIELD_EXAMPLE_LABEL,
                rect(12, 102, 74, 16),
            );
            add(
                "EDIT",
                entry.map(|e| e.example.as_str()).unwrap_or(""),
                ui::STYLE_EDIT_MULTI,
                id::FIELD_EXAMPLE,
                rect(92, 98, 332, 52),
            );
            add(
                "STATIC",
                "Source",
                ui::STYLE_STATIC,
                id::FIELD_SOURCE_LABEL,
                rect(12, 160, 74, 16),
            );
            add(
                "EDIT",
                entry.map(|e| e.source.as_str()).unwrap_or(""),
                ui::STYLE_EDIT,
                id::FIELD_SOURCE,
                rect(92, 156, 332, 24),
            );
            add(
                "STATIC",
                "From",
                ui::STYLE_STATIC,
                id::FIELD_SOURCE_LANG_LABEL,
                rect(12, 192, 74, 16),
            );
            add(
                "EDIT",
                entry.map(|e| e.source_language.as_str()).unwrap_or(""),
                ui::STYLE_EDIT,
                id::FIELD_SOURCE_LANG,
                rect(92, 188, 132, 24),
            );
            add(
                "STATIC",
                "To",
                ui::STYLE_STATIC,
                id::FIELD_TARGET_LANG_LABEL,
                rect(238, 192, 40, 16),
            );
            add(
                "EDIT",
                entry.map(|e| e.target_language.as_str()).unwrap_or(""),
                ui::STYLE_EDIT,
                id::FIELD_TARGET_LANG,
                rect(280, 188, 144, 24),
            );
            add(
                "BUTTON",
                "Learned",
                ui::STYLE_CHECKBOX,
                id::FIELD_LEARNED,
                rect(92, 224, 100, 20),
            );
            win::set_checked(hwnd, id::FIELD_LEARNED, entry.is_some_and(|e| e.learned));
        }
        _ => {
            ui::create_child(
                hwnd,
                "STATIC",
                "Name",
                ui::STYLE_STATIC,
                id::PROMPT_LABEL,
                rect(12, 16, 70, 18),
                dpi,
                font,
            );
            ui::create_child(
                hwnd,
                "EDIT",
                &state.initial,
                ui::STYLE_EDIT,
                id::PROMPT_VALUE,
                rect(84, 12, 286, 24),
                dpi,
                font,
            );
        }
    }
    let entry_mode = matches!(
        &state.kind,
        PromptKind::NewEntry(_) | PromptKind::EditEntry(_, _)
    );
    let button_y = if entry_mode { 296 } else { 122 };
    ui::create_child(
        hwnd,
        "BUTTON",
        "Cancel",
        ui::STYLE_BUTTON,
        id::CANCEL,
        rect(232, button_y, 82, 26),
        dpi,
        font,
    );
    ui::create_child(
        hwnd,
        "BUTTON",
        "Save",
        ui::STYLE_BUTTON_DEFAULT,
        id::OK,
        rect(322, button_y, 102, 26),
        dpi,
        font,
    );
}

pub extern "system" fn prompt_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCCREATE {
        let p = unsafe { (*(lparam.0 as *const CREATESTRUCTW)).lpCreateParams as *mut PromptState };
        if !p.is_null() {
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, p as isize);
            }
        }
        return LRESULT(1);
    }
    let p = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PromptState };
    if p.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    match msg {
        WM_CREATE => {
            prompt_controls(hwnd, unsafe { &*p });
            ui::focus(
                hwnd,
                if matches!(
                    unsafe { &(*p).kind },
                    PromptKind::NewEntry(_) | PromptKind::EditEntry(_, _)
                ) {
                    id::FIELD_WORD
                } else {
                    id::PROMPT_VALUE
                },
            );
            LRESULT(0)
        }
        WM_COMMAND if ui::command_id(wparam) == id::CANCEL => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_COMMAND if ui::command_id(wparam) == id::OK => {
            let kind = unsafe { (*p).kind.clone() };
            let app_ptr = unsafe { (*p).app };
            let entry_mode = matches!(kind, PromptKind::NewEntry(_) | PromptKind::EditEntry(_, _));
            let text = ui::edit_text(
                hwnd,
                if entry_mode {
                    id::FIELD_WORD
                } else {
                    id::PROMPT_VALUE
                },
            );
            if text.trim().is_empty() {
                win::warn(hwnd, "This value cannot be empty.", "WordDock");
                return LRESULT(0);
            }
            let mut close = true;
            if let Some(app) = unsafe { app_ptr.as_mut() } {
                match kind {
                    PromptKind::NewCollection => {
                        if app.library.name_taken(&text, None) {
                            win::warn(
                                hwnd,
                                "A collection with that name already exists.",
                                "WordDock",
                            );
                            close = false;
                        } else {
                            let collection = Collection::new(text.trim(), &mut app.rng);
                            let id = collection.id.clone();
                            app.library.collections.push(collection);
                            app.library.active_collection = Some(id);
                            app.rotation.reset();
                            app.save_library();
                            app.ensure_current();
                            app.refresh_card();
                        }
                    }
                    PromptKind::RenameCollection(cid) => {
                        if app.library.name_taken(&text, Some(&cid)) {
                            win::warn(
                                hwnd,
                                "A collection with that name already exists.",
                                "WordDock",
                            );
                            close = false;
                        } else if let Some(c) = app.library.collection_mut(&cid) {
                            c.name = text.trim().to_string();
                            app.save_library();
                            app.refresh_dialogs();
                        }
                    }
                    PromptKind::NewEntry(cid) => {
                        let meaning = ui::edit_text(hwnd, id::FIELD_MEANING);
                        if meaning.trim().is_empty() {
                            win::warn(hwnd, "Meaning cannot be empty.", "WordDock");
                            close = false;
                        } else if app
                            .library
                            .collection(&cid)
                            .is_some_and(|c| c.duplicate_index(&text, None).is_some())
                        {
                            win::warn(hwnd, "That word is already in this collection.", "WordDock");
                            close = false;
                        } else {
                            let mut e = Entry::new(text.trim(), meaning.trim(), &mut app.rng);
                            apply_entry_fields(hwnd, &mut e);
                            if let Some(c) = app.library.collection_mut(&cid) {
                                c.entries.push(e);
                                app.after_entries_changed();
                            }
                        }
                    }
                    PromptKind::EditEntry(cid, eid) => {
                        let meaning = ui::edit_text(hwnd, id::FIELD_MEANING);
                        if meaning.trim().is_empty() {
                            win::warn(hwnd, "Meaning cannot be empty.", "WordDock");
                            close = false;
                        } else if let Some(c) = app.library.collection_mut(&cid) {
                            if c.duplicate_index(&text, Some(&eid)).is_some() {
                                win::warn(
                                    hwnd,
                                    "That word is already in this collection.",
                                    "WordDock",
                                );
                                close = false;
                            } else if let Some(e) = c.entries.iter_mut().find(|e| e.id == eid) {
                                e.word = text.trim().to_string();
                                e.meaning = meaning.trim().to_string();
                                apply_entry_fields(hwnd, e);
                                app.after_entries_changed();
                            }
                        }
                    }
                }
            }
            if close {
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
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
        WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN => {
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
        WM_CLOSE => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_NCDESTROY => unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(Box::from_raw(p));
            DefWindowProcW(hwnd, msg, wparam, lparam)
        },
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn apply_entry_fields(hwnd: HWND, entry: &mut Entry) {
    entry.example = ui::edit_text(hwnd, id::FIELD_EXAMPLE);
    entry.source = ui::edit_text(hwnd, id::FIELD_SOURCE);
    entry.source_language = ui::edit_text(hwnd, id::FIELD_SOURCE_LANG);
    entry.target_language = ui::edit_text(hwnd, id::FIELD_TARGET_LANG);
    entry.learned = win::is_checked(hwnd, id::FIELD_LEARNED);
}

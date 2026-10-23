//! Application state, commands and the controller window procedure.
//!
//! # Ownership and re-entrancy
//!
//! `App` lives on the stack of `main()` and is passed to every window procedure
//! as a raw pointer (`CREATESTRUCT::lpCreateParams`, stored in `GWLP_USERDATA`).
//! The application is single-threaded: only the UI thread ever dereferences that
//! pointer, and only for the duration of one message. Handlers must therefore
//! never hold a reference to `App` across a call that can pump messages (message
//! boxes, file dialogs, popup menus, `DestroyWindow`); those calls are made at
//! the outermost level, or deferred through `queue_notice`.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::csvio::{self, Format, ImportReport};
use crate::model::{Collection, Config, Entry, PresentationMode, Rng, ThemeMode};
use crate::rotation::{Rotation, eligible_ids, eligible_ids_in_collections};
use crate::samples;
use crate::store::{self, Paths, Source};
use crate::theme;
use crate::win;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, DestroyWindow, KillTimer, PostMessageW, PostQuitMessage, SetTimer, WM_APP,
    WM_CLOSE, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ERASEBKGND, WM_NCCREATE, WM_SETTINGCHANGE,
    WM_TIMER, WM_WTSSESSION_CHANGE,
};

/// Tray icon callback message.
pub const WM_APP_TRAY: u32 = WM_APP + 1;
/// Posted by the import worker thread with a boxed `ImportReport` in lparam.
pub const WM_APP_IMPORT_DONE: u32 = WM_APP + 2;
/// Posted when a deferred notice should be shown.
pub const WM_APP_NOTICE: u32 = WM_APP + 3;
/// Posted by a second instance to reveal the card of the running instance.
pub const WM_APP_REVEAL: u32 = WM_APP + 4;

pub const CONTROLLER_CLASS: &str = "WordDockController";
const TIMER_ROTATE: usize = 1;

/// Session change codes from wtsapi32.h (`WM_WTSSESSION_CHANGE`).
const WTS_SESSION_LOCK: usize = 0x7;
const WTS_SESSION_UNLOCK: usize = 0x8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    ToggleCard,
    HideCard,
    Next,
    Previous,
    TogglePause,
    MarkLearned,
    ReviewAgain,
    Details,
    Settings,
    Collections,
    Clothesline,
    Exit,
}

/// What the card should render right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardContent {
    pub word: String,
    pub meaning: String,
    pub example: String,
    pub presentation: PresentationMode,
    /// meta
    pub meta: String,
    pub has_entry: bool,
    pub learned: bool,
    pub paused: bool,
    pub can_navigate: bool,
    /// Explains an empty card: no collection, empty collection, all learned…
    pub empty_hint: Option<String>,
}

pub struct App {
    pub paths: Paths,
    pub config: Config,
    pub library: crate::model::Library,
    pub rotation: Rotation,
    pub rng: Rng,
    pub controller: HWND,
    pub card: crate::ui::card::CardState,
    pub tray: crate::tray::TrayIcon,
    pub session_locked: bool,
    pub timer_active: bool,
    pub settings_hwnd: Option<HWND>,
    pub collections_hwnd: Option<HWND>,
    pub details_hwnd: Option<HWND>,
    pub import_hwnd: Option<HWND>,
    pub clothesline_hwnd: Option<HWND>,
    pub clothesline_edge_hwnd: Option<HWND>,
    pub clothesline_items: Vec<crate::clothesline::ClipboardItem>,
    pub pending_import: Option<ImportReport>,
    pub pending_import_target: Option<String>,
    pub notice: Option<(String, bool)>,
    pub save_error_reported: bool,
    pub startup_entry_ok: bool,
    /// Newer on-disk schema detected; startup must stop before any write.
    pub fatal_error: Option<String>,
    pub first_run_setup: bool,
}

impl App {
    pub fn new(controller: HWND, paths: Paths) -> App {
        let mut rng = Rng::new(Rng::seed());

        let config_load = store::load_config(&paths);
        let mut config = config_load.value.clone().unwrap_or_default();
        config.sanitize();
        if let Some(exe) = win::exe_path() {
            config.launch_at_login = crate::startup::is_enabled_for(&exe);
        }

        let library_load = store::load_library(&paths);
        let mut library = library_load.value.clone().unwrap_or_default();
        let first_run = library_load.source == Source::Default;
        if first_run {
            let mut collection = samples::sample_collection(&mut rng);
            library.collections.push(std::mem::take(&mut collection));
        }
        library.ensure_ids(&mut rng);

        let fatal_error = library_load
            .fatal
            .clone()
            .or_else(|| config_load.fatal.clone());
        let clothesline_path = paths.root.join("clothesline.json");
        let clothesline_bytes = std::fs::read(&clothesline_path).ok().and_then(|bytes| {
            if bytes.starts_with(b"WDCH1") {
                win::unprotect_for_user(&bytes[5..])
            } else {
                // One-time migration from the earlier unencrypted JSON scaffold.
                Some(bytes)
            }
        });
        let clothesline_items = clothesline_bytes
            .as_deref()
            .and_then(|bytes| {
                serde_json::from_slice::<Vec<crate::clothesline::ClipboardItem>>(bytes).ok()
            })
            .or_else(|| {
                clothesline_bytes
                    .as_deref()
                    .and_then(|bytes| serde_json::from_slice::<Vec<String>>(bytes).ok())
                    .map(|items| {
                        items
                            .into_iter()
                            .map(|text| {
                                if std::path::Path::new(&text).exists() {
                                    crate::clothesline::ClipboardItem::Files(vec![text])
                                } else {
                                    crate::clothesline::ClipboardItem::Text(text)
                                }
                            })
                            .collect()
                    })
            })
            .unwrap_or_default();
        let clothesline_items = clothesline_items
            .into_iter()
            .map(|item| match item {
                crate::clothesline::ClipboardItem::Text(path)
                    if std::path::Path::new(&path).is_file() =>
                {
                    crate::clothesline::ClipboardItem::Files(vec![path])
                }
                item => item,
            })
            .collect();
        let mut app = App {
            paths,
            config,
            library,
            rotation: Rotation::new(config_load.value.map(|c| c.order).unwrap_or_default()),
            rng,
            controller,
            card: crate::ui::card::CardState::default(),
            tray: crate::tray::TrayIcon::create(theme::is_dark(ThemeMode::System)),
            session_locked: false,
            timer_active: false,
            settings_hwnd: None,
            collections_hwnd: None,
            details_hwnd: None,
            import_hwnd: None,
            clothesline_hwnd: None,
            clothesline_edge_hwnd: None,
            clothesline_items,
            pending_import: None,
            pending_import_target: None,
            notice: None,
            save_error_reported: false,
            startup_entry_ok: true,
            fatal_error,
            first_run_setup: first_run,
        };
        app.rotation.set_order(app.config.order);
        app.ensure_current();
        if first_run && app.fatal_error.is_none() {
            // Persist the initial sample library now so it survives an exit
            // before the user first edits anything.
            app.save_library();
        }

        // Report load problems (and fatal ones) once the window exists.
        if let Some(fatal) = library_load.fatal.or(config_load.fatal) {
            app.queue_notice(fatal, true);
        } else {
            if let Some(warning) = library_load.warning.or(config_load.warning) {
                app.queue_notice(warning, false);
            }
        }
        if first_run {
            app.queue_notice(
                format!(
                    "WordDock created a starter collection with {} original sample entries.\n\n\
                     They are general-purpose examples, not extracted from any book. Rename the \
                     collection after your own book or topic and replace the entries with your own.",
                    app.library.collections.first().map(|c| c.entries.len()).unwrap_or(0)
                ),
                false,
            );
        }
        app
    }

    // -- data access -------------------------------------------------------

    pub fn active(&self) -> Option<&Collection> {
        self.library.active()
    }

    pub fn active_mut(&mut self) -> Option<&mut Collection> {
        self.library.active_mut()
    }

    pub fn eligible(&self) -> Vec<String> {
        eligible_ids_in_collections(&self.library, &self.config)
    }

    pub fn current_entry(&self) -> Option<&Entry> {
        let composite = self.rotation.current()?;
        let (collection_id, entry_id) = composite.split_once("::")?;
        let collection = self.library.collection(collection_id)?;
        collection.entries.iter().find(|entry| entry.id == entry_id)
    }

    pub fn can_navigate(&self) -> bool {
        !self.eligible().is_empty()
    }

    pub fn card_content(&self) -> CardContent {
        let paused = self.config.paused;
        let can_navigate = self.can_navigate();
        let Some(collection) = self.active() else {
            return CardContent {
                word: "No collection yet".to_string(),
                meaning: "Open Manage collections from the tray menu to create one, or import a CSV/JSON file."
                    .to_string(),
                example: String::new(),
                presentation: self.config.presentation,
                meta: String::new(),
                has_entry: false,
                learned: false,
                paused,
                can_navigate: false,
                empty_hint: Some("empty".to_string()),
            };
        };
        if collection.entries.is_empty() {
            return CardContent {
                word: format!("“{}” is empty", truncate(&collection.name, 28)),
                meaning: "Add words with Manage collections, or import a CSV/JSON vocabulary file."
                    .to_string(),
                example: String::new(),
                presentation: self.config.presentation,
                meta: String::new(),
                has_entry: false,
                learned: false,
                paused,
                can_navigate: false,
                empty_hint: Some("empty".to_string()),
            };
        }
        if !can_navigate {
            return CardContent {
                word: "All entries learned".to_string(),
                meaning:
                    "Use “Review again” to bring a word back into rotation, or add new entries."
                        .to_string(),
                example: String::new(),
                presentation: self.config.presentation,
                meta: format!(
                    "{} · {} entries",
                    truncate(&collection.name, 24),
                    collection.entries.len()
                ),
                has_entry: false,
                learned: true,
                paused,
                can_navigate: false,
                empty_hint: Some("all_learned".to_string()),
            };
        }
        match self.current_entry() {
            Some(entry) => CardContent {
                word: entry.word.clone(),
                meaning: entry.meaning.clone(),
                example: entry.example.clone(),
                presentation: self.config.presentation,
                meta: self
                    .rotation
                    .current()
                    .and_then(|id| id.split_once("::"))
                    .and_then(|(cid, _)| self.library.collection(cid))
                    .map(|c| meta_line(entry, c))
                    .unwrap_or_else(|| meta_line(entry, collection)),
                has_entry: true,
                learned: entry.learned,
                paused,
                can_navigate: true,
                empty_hint: None,
            },
            None => CardContent {
                word: "—".to_string(),
                meaning: "Press Next to show a word.".to_string(),
                example: String::new(),
                presentation: self.config.presentation,
                meta: truncate(&collection.name, 40),
                has_entry: false,
                learned: false,
                paused,
                can_navigate: true,
                empty_hint: Some("idle".to_string()),
            },
        }
    }

    // -- rotation ----------------------------------------------------------

    /// Picks the entry to show, preferring the stored selection if it is still
    /// eligible.
    pub fn ensure_current(&mut self) {
        let eligible = self.eligible();
        if eligible.is_empty() {
            self.rotation.set_current(None);
            return;
        }
        match self.rotation.current() {
            Some(id) if eligible.iter().any(|e| e == id) => {}
            _ => {
                self.rotation.set_current(None);
                let _ = self.rotation.next(&eligible, &mut self.rng);
            }
        }
    }

    pub fn advance(&mut self, forward: bool) {
        let eligible = self.eligible();
        if eligible.is_empty() {
            self.rotation.set_current(None);
        } else if forward {
            let _ = self.rotation.next(&eligible, &mut self.rng);
        } else {
            let _ = self.rotation.prev(&eligible, &mut self.rng);
        }
        self.refresh_card();
    }

    pub fn toggle_pause(&mut self) {
        self.config.paused = !self.config.paused;
        self.save_config();
        self.restart_timer();
        self.refresh_card();
    }

    pub fn mark_learned(&mut self) {
        let Some(composite) = self.rotation.current().map(|id| id.to_string()) else {
            return;
        };
        let Some((collection_id, entry_id)) = composite.split_once("::") else {
            return;
        };
        let collection_id = collection_id.to_string();
        let entry_id = entry_id.to_string();
        let mut changed = false;
        if let Some(collection) = self.library.collection_mut(&collection_id) {
            if let Some(entry) = collection.entries.iter_mut().find(|e| e.id == entry_id) {
                if !entry.learned {
                    entry.learned = true;
                    changed = true;
                }
            }
        }
        if changed {
            self.save_library();
            // Move on to the next word; the learned entry is no longer eligible.
            self.rotation.set_current(Some(composite.clone()));
            self.advance(true);
            self.restart_timer();
        }
        self.refresh_card();
    }

    pub fn review_again(&mut self) {
        let id = match self.rotation.current() {
            Some(id) => id.to_string(),
            None => {
                // Nothing shown (for example every entry is learned): take the
                // most recently learned entry back into rotation.
                let candidate = self.config.selected_collections.iter().find_map(|cid| {
                    self.library.collection(cid).and_then(|collection| {
                        collection
                            .entries
                            .iter()
                            .rev()
                            .find(|e| e.learned)
                            .map(|e| format!("{}::{}", cid, e.id))
                    })
                });
                match candidate {
                    Some(id) => id,
                    None => return,
                }
            }
        };
        if let Some((collection_id, entry_id)) = id.split_once("::") {
            if let Some(collection) = self.library.collection_mut(collection_id) {
                if let Some(entry) = collection.entries.iter_mut().find(|e| e.id == entry_id) {
                    entry.learned = false;
                }
            }
        }
        self.rotation.set_current(Some(id));
        self.save_library();
        self.restart_timer();
        self.refresh_card();
    }

    // -- card --------------------------------------------------------------

    pub fn show_card(&mut self) {
        let Some(hwnd) = self.card.hwnd else {
            return;
        };
        self.config.card_visible = true;
        self.ensure_current();
        crate::ui::card::apply_placement(self);
        win::show_without_activation(hwnd);
        self.save_config();
        self.restart_timer();
        self.refresh_card();
    }

    pub fn hide_card(&mut self) {
        let Some(hwnd) = self.card.hwnd else {
            return;
        };
        self.config.card_visible = false;
        win::hide_window(hwnd);
        self.save_config();
        // Nothing to rotate while the card is hidden: the timer is stopped
        // instead of running idle.
        self.restart_timer();
    }

    pub fn toggle_card(&mut self) {
        if self.card_visible() {
            self.hide_card();
        } else {
            self.show_card();
        }
    }

    pub fn card_visible(&self) -> bool {
        match self.card.hwnd {
            Some(hwnd) => {
                use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;
                // SAFETY: pure query for an existing window.
                unsafe { IsWindowVisible(hwnd).as_bool() }
            }
            None => false,
        }
    }

    pub fn refresh_card(&mut self) {
        if let Some(hwnd) = self.card.hwnd {
            // SAFETY: invalidating an existing window repaints it when idle.
            unsafe {
                let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
            }
        }
    }

    /// Remembers where the card is, in DIP relative to its monitor's work area.
    pub fn remember_card_position(&mut self) {
        let Some(hwnd) = self.card.hwnd else {
            return;
        };
        let Some(monitor) = win::monitor_for_window(hwnd) else {
            return;
        };
        let mut rect = windows::Win32::Foundation::RECT::default();
        // SAFETY: filling a RECT for an existing window.
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut rect);
        }
        let dpi = self.card.dpi.max(96);
        self.config.position = Some(crate::model::CardPosition {
            monitor: monitor.device,
            x_dip: to_dip(rect.left - monitor.work.left, dpi),
            y_dip: to_dip(rect.top - monitor.work.top, dpi),
        });
    }

    // -- timer -------------------------------------------------------------

    pub fn restart_timer(&mut self) {
        self.stop_timer();
        if self.session_locked || self.config.paused || !self.card_visible() {
            return;
        }
        if !self.can_navigate() {
            return;
        }
        // SAFETY: registering one timer on our own window; id 0 means failure.
        let id = unsafe {
            SetTimer(
                Some(self.controller),
                TIMER_ROTATE,
                self.config.interval_ms(),
                None,
            )
        };
        self.timer_active = id != 0;
    }

    pub fn stop_timer(&mut self) {
        if !self.timer_active {
            return;
        }
        // SAFETY: killing our own timer; a stale timestamp is ignored.
        unsafe {
            let _ = KillTimer(Some(self.controller), TIMER_ROTATE);
        }
        self.timer_active = false;
    }

    pub fn on_timer(&mut self) {
        self.advance(true);
    }

    pub fn on_session_change(&mut self, event: usize) {
        match event {
            WTS_SESSION_LOCK => {
                // Suspend rotation while the session is locked; nothing is
                // replayed on unlock.
                self.session_locked = true;
                self.stop_timer();
            }
            WTS_SESSION_UNLOCK => {
                self.session_locked = false;
                self.restart_timer();
            }
            _ => {}
        }
    }

    /// Responds to display/DPI/theme changes.
    pub fn on_display_change(&mut self) {
        crate::ui::card::apply_placement(self);
    }

    pub fn on_clipboard_update(&mut self) {
        if let Some(payload) = win::clipboard_payload() {
            crate::clothesline::push_payload(self, payload);
        }
    }

    pub fn on_theme_change(&mut self) {
        if self.config.theme == ThemeMode::System {
            self.refresh_card();
        }
    }

    // -- persistence -------------------------------------------------------

    pub fn save_config(&mut self) {
        let result = store::save_json(&self.paths, &self.paths.config, &self.config);
        self.report_save_result(result);
    }

    pub fn save_library(&mut self) {
        let result = store::save_json(&self.paths, &self.paths.library, &self.library);
        self.report_save_result(result);
    }

    fn report_save_result(&mut self, result: Result<(), String>) {
        if let Err(error) = result
            && !self.save_error_reported
        {
            self.save_error_reported = true;
            let paths = self.paths.root.display().to_string();
            self.queue_notice(
                format!(
                    "WordDock could not save your data.\n\n{error}\n\n\
                         Changes stay in memory until WordDock can write to {paths}."
                ),
                true,
            );
        }
    }

    /// Deferred notice: shown by the controller loop, outside any nested call.
    pub fn queue_notice(&mut self, text: impl Into<String>, is_error: bool) {
        self.notice = Some((text.into(), is_error));
        // SAFETY: posting to our own window; the pointer-free lparam is unused.
        unsafe {
            let _ = PostMessageW(Some(self.controller), WM_APP_NOTICE, WPARAM(0), LPARAM(0));
        }
    }

    pub fn take_notice(&mut self) -> Option<(String, bool)> {
        self.notice.take()
    }

    // -- settings ----------------------------------------------------------

    /// Applies a configuration coming from the settings dialog.
    pub fn apply_config(
        &mut self,
        config: Config,
        _owner: HWND,
        active_collection: Option<String>,
    ) {
        let previous_login = self.config.launch_at_login;
        self.config = config;
        self.config.sanitize();

        if self.config.launch_at_login != previous_login {
            let enabled = self.apply_startup_setting();
            if !enabled {
                self.config.launch_at_login = previous_login;
            }
        }

        self.rotation.set_order(self.config.order);
        let active_changed = self.library.active_collection != active_collection
            && active_collection
                .as_ref()
                .is_some_and(|id| self.library.collection(id).is_some());
        if active_changed {
            self.library.active_collection = active_collection;
            self.rotation.reset();
        }
        self.ensure_current();
        let library_changed = self.normalize_active_collection();
        self.save_config();
        if library_changed || active_changed {
            self.save_library();
        }
        crate::ui::card::reset_fonts(&mut self.card);
        crate::ui::card::apply_placement(self);
        self.restart_timer();
        self.refresh_card();
    }

    fn apply_startup_setting(&mut self) -> bool {
        let Some(exe) = win::exe_path() else {
            self.queue_notice(
                "WordDock could not determine its own executable path.",
                true,
            );
            return false;
        };
        match crate::startup::set_enabled(&exe, self.config.launch_at_login) {
            Ok(()) => {
                self.startup_entry_ok = true;
                true
            }
            Err(error) => {
                self.startup_entry_ok = false;
                self.queue_notice(
                    format!(
                        "WordDock could not update the “launch at login” entry.\n\n{error}\n\n\
                         WordDock continues to run; the setting was left unchanged."
                    ),
                    true,
                );
                false
            }
        }
    }

    /// Keeps `active_collection` pointing at an existing collection.
    fn normalize_active_collection(&mut self) -> bool {
        let before = self.library.active_collection.clone();
        if self.library.active().is_none() {
            self.library.active_collection = self.library.collections.first().map(|c| c.id.clone());
        }
        self.library.active_collection != before
    }

    pub fn set_active_collection(&mut self, id: &str) {
        if self.library.active_collection.as_deref() != Some(id) {
            self.library.active_collection = Some(id.to_string());
            self.rotation.reset();
            self.ensure_current();
            self.save_library();
            self.restart_timer();
            self.refresh_card();
        }
    }

    /// Called after the active collection's entries changed.
    pub fn after_entries_changed(&mut self) {
        self.rotation.retain_current(&self.eligible());
        self.ensure_current();
        self.save_library();
        self.restart_timer();
        self.refresh_card();
    }

    // -- import / export ---------------------------------------------------

    pub fn start_import(&mut self, path: PathBuf) {
        let existing: HashSet<String> = self
            .active()
            .map(|collection| collection.word_keys())
            .unwrap_or_default();
        // Raw Win32 handles are not `Send`; pass the numeric handle value and
        // reconstruct it only on the worker for PostMessageW.
        let controller = self.controller.0 as usize;
        // Parsing happens on a worker thread so the UI keeps responding; the
        // report is handed over through the window queue.
        std::thread::spawn(move || {
            let report = csvio::read_and_parse(&path, &existing);
            let boxed = Box::into_raw(Box::new(report));
            // SAFETY: `boxed` is a valid heap pointer; ownership is transferred to
            // the UI thread, which always takes it back with Box::from_raw.
            let posted = unsafe {
                PostMessageW(
                    Some(windows::Win32::Foundation::HWND(
                        controller as *mut core::ffi::c_void,
                    )),
                    WM_APP_IMPORT_DONE,
                    WPARAM(0),
                    LPARAM(boxed as isize),
                )
            };
            if posted.is_err() {
                // Nobody will take ownership: reclaim it here.
                unsafe {
                    drop(Box::from_raw(boxed));
                }
            }
        });
    }

    /// Takes ownership of the report passed in `lparam`.
    ///
    /// # Safety
    /// `lparam` must be a pointer produced by `Box::into_raw` of an
    /// `ImportReport`, and it must be taken exactly once.
    pub unsafe fn take_import_report(lparam: LPARAM) -> Option<ImportReport> {
        let pointer = lparam.0 as *mut ImportReport;
        if pointer.is_null() {
            return None;
        }
        // SAFETY: guaranteed by the caller (see the message producer in start_import).
        Some(unsafe { *Box::from_raw(pointer) })
    }

    pub fn start_import_into(&mut self, path: PathBuf, target: String) {
        if self.library.collection(&target).is_some() {
            self.pending_import_target = Some(target);
        }
        self.start_import(path);
    }

    pub fn finish_import(&mut self, report: ImportReport) {
        if let Some(error) = &report.error {
            let message = format!(
                "Nothing was imported from {}.\n\n{error}",
                report.path.display()
            );
            self.queue_notice(message, true);
            return;
        }
        if report.ready_count() == 0 && report.duplicate_count() == 0 {
            self.queue_notice(
                format!(
                    "{} contained {} rows, but none of them had both a word and a meaning.\n\n{}",
                    report.path.display(),
                    report.total(),
                    report.notes.join("\n")
                ),
                true,
            );
            return;
        }
        self.pending_import = Some(report);
        crate::ui::import::open(self);
    }

    /// Applies the reviewed import to the active collection.
    pub fn apply_import(&mut self, include_duplicates: bool) {
        let Some(report) = self.pending_import.take() else {
            return;
        };
        let entries = report.to_entries(include_duplicates, &mut self.rng);
        let added = entries.len();
        let target_id = self
            .pending_import_target
            .take()
            .or_else(|| self.library.active_collection.clone());
        if let Some(id) = target_id {
            if let Some(collection) = self.library.collection_mut(&id) {
                collection.entries.extend(entries);
            }
        }
        self.after_entries_changed();
        self.queue_notice(
            format!(
                "Imported {added} entr{} from {}.\n\n{}",
                if added == 1 { "y" } else { "ies" },
                report.path.display(),
                report.summary()
            ),
            false,
        );
    }

    pub fn export_entries(&mut self, format: Format) -> Option<PathBuf> {
        let collection = self.active()?;
        if collection.entries.is_empty() {
            self.queue_notice("There is nothing to export from this collection.", false);
            return None;
        }
        let default_name = format!(
            "{}.{}",
            sanitize_file_name(&collection.name),
            format.extension()
        );
        let owner = self.collections_hwnd.unwrap_or(self.controller);
        let filters = match format {
            Format::Csv => vec![win::FileFilter {
                label: "CSV file",
                pattern: "*.csv",
            }],
            Format::Json => vec![win::FileFilter {
                label: "JSON file",
                pattern: "*.json",
            }],
        };
        let title = format!("Export collection as {}", format.label());
        // File dialogs pump messages, so nothing borrowed from `self` is alive here.
        let path = win::pick_save_file(owner, &title, &filters, format.extension(), &default_name)?;
        let collection = self.active()?;
        let result = match format {
            Format::Csv => std::fs::write(&path, csvio::to_csv(&collection.entries))
                .map_err(|error| error.to_string()),
            Format::Json => csvio::to_json(&collection.entries)
                .map_err(|error| error.to_string())
                .and_then(|json| {
                    std::fs::write(&path, format!("{}\n", json)).map_err(|error| error.to_string())
                }),
        };
        match result {
            Ok(()) => self.queue_notice(
                format!(
                    "Exported {} entries to {}.",
                    collection.entries.len(),
                    path.display()
                ),
                false,
            ),
            Err(error) => self.queue_notice(
                format!("Could not write {}:\n\n{error}", path.display()),
                true,
            ),
        }
        Some(path)
    }

    // -- windows -----------------------------------------------------------

    pub fn open_settings(&mut self) {
        crate::ui::settings::open(self);
    }

    pub fn open_collections(&mut self) {
        crate::ui::collections::open(self);
    }

    pub fn open_clothesline(&mut self) {
        crate::clothesline::open(self);
    }
    pub fn toggle_clothesline(&mut self) {
        crate::clothesline::toggle(self);
    }

    pub fn open_details(&mut self) {
        crate::ui::details::open(self);
    }

    /// Refreshes any dialog that is currently open.
    pub fn refresh_dialogs(&mut self) {
        if let Some(hwnd) = self.collections_hwnd {
            crate::ui::collections::repopulate(self, hwnd);
        }
        if let Some(hwnd) = self.details_hwnd {
            crate::ui::details::refresh(self, hwnd);
        }
    }

    // -- lifecycle ---------------------------------------------------------

    pub fn handle(&mut self, action: Action) {
        match action {
            Action::ToggleCard => self.toggle_card(),
            Action::HideCard => self.hide_card(),
            Action::Next => self.advance(true),
            Action::Previous => self.advance(false),
            Action::TogglePause => self.toggle_pause(),
            Action::MarkLearned => self.mark_learned(),
            Action::ReviewAgain => self.review_again(),
            Action::Details => self.open_details(),
            Action::Settings => self.open_settings(),
            Action::Collections => self.open_collections(),
            Action::Clothesline => self.open_clothesline(),
            Action::Exit => self.quit(),
        }
    }

    pub fn quit(&mut self) {
        crate::clothesline::remove_edge_trigger(self);
        crate::clothesline::close(self);
        self.remember_card_position();
        self.save_config();
        self.stop_timer();
        self.tray.remove(self.controller);
        for hwnd in [
            self.import_hwnd,
            self.details_hwnd,
            self.collections_hwnd,
            self.settings_hwnd,
        ]
        .into_iter()
        .flatten()
        {
            // SAFETY: destroying windows owned by this application; each
            // WM_NCDESTROY handler releases its own state.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
        if let Some(hwnd) = self.card.hwnd.take() {
            // SAFETY: as above.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
        // SAFETY: ends the message loop in main().
        unsafe {
            PostQuitMessage(0);
        }
    }
}

/// Controller window procedure: receives tray callbacks, timers, broadcast
/// messages and deferred notices.
pub extern "system" fn controller_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCCREATE {
        // SAFETY: the CREATESTRUCT pointer is valid during WM_NCCREATE; the
        // lpCreateParams pointer is the App owned by main().
        let app = unsafe {
            let create = lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW;
            (*create).lpCreateParams as *mut App
        };
        if !app.is_null() {
            // SAFETY: storing the pointer in this window's user data slot.
            unsafe {
                windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(
                    hwnd,
                    windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA,
                    app as isize,
                );
            }
        }
        return LRESULT(1);
    }

    // SAFETY: retrieved pointer was stored in WM_NCCREATE; only the UI thread
    // reads it, and it is never held across messages.
    let app_ptr = unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(
            hwnd,
            windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA,
        )
    } as *mut App;

    match msg {
        WM_APP_TRAY => {
            if !app_ptr.is_null() {
                // SAFETY: valid for the duration of this message.
                let app = unsafe { &mut *app_ptr };
                let event = win::loword(lparam.0);
                use windows::Win32::UI::WindowsAndMessaging::{
                    WM_CONTEXTMENU, WM_LBUTTONUP, WM_RBUTTONUP,
                };
                if event == WM_LBUTTONUP {
                    app.toggle_card();
                } else if event == WM_RBUTTONUP || event == WM_CONTEXTMENU {
                    let visible = app.card_visible();
                    let paused = app.config.paused;
                    let has_entry = app.current_entry().is_some();
                    crate::tray::activate_for_menu(hwnd);
                    if let Some(command) = crate::tray::show_menu(hwnd, visible, paused, has_entry)
                    {
                        // `show_menu` pumped messages, so `app` is re-fetched.
                        let app = unsafe { &mut *app_ptr };
                        let action = match command {
                            crate::tray::TrayCommand::ToggleCard => Action::ToggleCard,
                            crate::tray::TrayCommand::Next => Action::Next,
                            crate::tray::TrayCommand::Previous => Action::Previous,
                            crate::tray::TrayCommand::TogglePause => Action::TogglePause,
                            crate::tray::TrayCommand::MarkLearned => Action::MarkLearned,
                            crate::tray::TrayCommand::ReviewAgain => Action::ReviewAgain,
                            crate::tray::TrayCommand::Details => Action::Details,
                            crate::tray::TrayCommand::Settings => Action::Settings,
                            crate::tray::TrayCommand::Collections => Action::Collections,
                            crate::tray::TrayCommand::Clothesline => Action::Clothesline,
                            crate::tray::TrayCommand::Exit => Action::Exit,
                        };
                        app.handle(action);
                    }
                }
            }
            LRESULT(0)
        }
        WM_APP_IMPORT_DONE => {
            if !app_ptr.is_null() {
                // SAFETY: the worker thread allocated this report with Box::into_raw
                // and transferred ownership to us.
                if let Some(report) = unsafe { App::take_import_report(lparam) } {
                    let app = unsafe { &mut *app_ptr };
                    app.finish_import(report);
                }
            }
            LRESULT(0)
        }
        WM_APP_NOTICE => {
            if !app_ptr.is_null() {
                let notice = unsafe { &mut *app_ptr }.take_notice();
                if let Some((text, is_error)) = notice {
                    if is_error {
                        win::error(hwnd, &text, "WordDock");
                    } else {
                        win::info(hwnd, &text, "WordDock");
                    }
                }
            }
            LRESULT(0)
        }
        WM_APP_REVEAL => {
            if !app_ptr.is_null() {
                let app = unsafe { &mut *app_ptr };
                if app.card_visible() {
                    let hwnd_card = app.card.hwnd;
                    if let Some(card) = hwnd_card {
                        win::set_topmost(card, true);
                        win::set_topmost(card, app.config.always_on_top);
                    }
                    app.refresh_card();
                } else {
                    app.show_card();
                }
            }
            LRESULT(0)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_HOTKEY => {
            if !app_ptr.is_null() {
                let app = unsafe { &mut *app_ptr };
                match wparam.0 as i32 {
                    1 => app.handle(Action::ToggleCard),
                    2 => app.toggle_clothesline(),
                    _ => {}
                }
            }
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == TIMER_ROTATE && !app_ptr.is_null() {
                unsafe { &mut *app_ptr }.on_timer();
            }
            LRESULT(0)
        }
        WM_WTSSESSION_CHANGE => {
            if !app_ptr.is_null() {
                unsafe { &mut *app_ptr }.on_session_change(wparam.0);
            }
            LRESULT(0)
        }
        WM_DISPLAYCHANGE => {
            if !app_ptr.is_null() {
                unsafe { &mut *app_ptr }.on_display_change();
            }
            LRESULT(0)
        }
        WM_SETTINGCHANGE => {
            if !app_ptr.is_null() {
                unsafe { &mut *app_ptr }.on_theme_change();
            }
            LRESULT(0)
        }
        WM_DPICHANGED => {
            if !app_ptr.is_null() {
                crate::ui::card::on_dpi_changed(unsafe { &mut *app_ptr }, wparam, lparam);
            }
            LRESULT(0)
        }
        0x031D => {
            // WM_CLIPBOARDUPDATE
            if !app_ptr.is_null() {
                unsafe { &mut *app_ptr }.on_clipboard_update();
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            // The controller window is hidden infrastructure: closing it means
            // quitting the application.
            if !app_ptr.is_null() {
                unsafe { &mut *app_ptr }.quit();
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(0),
        _ => {
            // SAFETY: default processing for unhandled messages.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
    }
}

fn meta_line(entry: &Entry, collection: &Collection) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !entry.source.trim().is_empty() {
        parts.push(truncate(entry.source.trim(), 28));
    } else {
        parts.push(truncate(&collection.name, 28));
    }
    let languages = match (
        entry.source_language.trim().is_empty(),
        entry.target_language.trim().is_empty(),
    ) {
        (false, false) => Some(format!(
            "{} → {}",
            entry.source_language.trim(),
            entry.target_language.trim()
        )),
        (false, true) => Some(entry.source_language.trim().to_string()),
        (true, false) => Some(entry.target_language.trim().to_string()),
        (true, true) => None,
    };
    if let Some(languages) = languages {
        parts.push(languages);
    }
    if entry.learned {
        parts.push("learned".to_string());
    }
    parts.join(" · ")
}

pub fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut result: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    result.push('…');
    result
}

fn to_dip(value: i32, dpi: u32) -> i32 {
    ((value as i64 * 96 + (dpi as i64) / 2) / dpi as i64) as i32
}

fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if r#"\/:*?"<>|"#.contains(c) { '-' } else { c })
        .collect();
    let cleaned = cleaned
        .trim()
        .trim_matches(|c| c == '.' || c == '-')
        .to_string();
    if cleaned.is_empty() {
        "worddock-collection".to_string()
    } else {
        truncate(&cleaned, 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_keeps_short_text_and_ellipsizes_long_text() {
        assert_eq!(truncate("adroit", 10), "adroit");
        let long = truncate("abcdefghijklmno", 6);
        assert_eq!(long.chars().count(), 6);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn dip_conversion_round_trips_at_common_scales() {
        assert_eq!(to_dip(300, 96), 300);
        assert_eq!(to_dip(450, 144), 300);
        assert_eq!(to_dip(600, 192), 300);
    }

    #[test]
    fn file_names_are_sanitized() {
        assert_eq!(sanitize_file_name(r#"Book: "One/Two""#), "Book- -One-Two");
        assert_eq!(sanitize_file_name("   "), "worddock-collection");
    }
}

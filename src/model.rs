//! Persistent data model: configuration, collections, entries and small helpers.
//!
//! Everything in this module is pure data logic (no Win32 calls) so that it can be
//! unit tested on any platform.

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Version of the on-disk JSON schema. Files with a *higher* version are never
/// overwritten; the application refuses to run against them.
pub const SCHEMA_VERSION: u32 = 1;

pub const CONFIG_FILE: &str = "config.json";
pub const LIBRARY_FILE: &str = "library.json";
pub const BACKUP_SUFFIX: &str = ".bak";

pub const MIN_INTERVAL_MINUTES: u32 = 1;
pub const MAX_INTERVAL_MINUTES: u32 = 1440;
pub const DEFAULT_INTERVAL_MINUTES: u32 = 15;

pub const MIN_FONT_SIZE_PT: u32 = 10;
pub const MAX_FONT_SIZE_PT: u32 = 36;
pub const DEFAULT_FONT_SIZE_PT: u32 = 18;

pub const MIN_CARD_WIDTH_DIP: i32 = 220;
pub const MAX_CARD_WIDTH_DIP: i32 = 800;
pub const DEFAULT_CARD_WIDTH_DIP: i32 = 300;

/// Font family used for the card and for the dialogs.
pub const UI_FONT_FAMILY: &str = "Segoe UI";

/// Rotation order for the card.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Order {
    Sequential,
    Shuffled,
}

impl Default for Order {
    fn default() -> Self {
        Order::Sequential
    }
}

impl Order {
    pub fn label(self) -> &'static str {
        match self {
            Order::Sequential => "Sequential",
            Order::Shuffled => "Shuffled (no repeat per cycle)",
        }
    }
}

/// Card theme selection. `System` follows the Windows "app mode" setting.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

impl Default for ThemeMode {
    fn default() -> Self {
        ThemeMode::System
    }
}

impl ThemeMode {
    pub fn label(self) -> &'static str {
        match self {
            ThemeMode::System => "System",
            ThemeMode::Light => "Light",
            ThemeMode::Dark => "Dark",
        }
    }
}

/// One vocabulary entry inside a collection.
///
/// All textual fields are plain strings; an empty string means "not set" which
/// keeps CSV and JSON round-tripping trivial.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(default)]
pub struct Entry {
    pub id: String,
    pub word: String,
    pub meaning: String,
    pub example: String,
    pub source: String,
    pub source_language: String,
    pub target_language: String,
    pub learned: bool,
}

impl Entry {
    /// Comparison key for duplicate detection: trimmed and case-insensitive.
    pub fn key(&self) -> String {
        word_key(&self.word)
    }

    pub fn new(word: &str, meaning: &str, rng: &mut Rng) -> Entry {
        Entry {
            id: new_id("e", rng),
            word: word.to_string(),
            meaning: meaning.to_string(),
            ..Entry::default()
        }
    }

    /// Short one-line summary used by the collection manager list.
    pub fn summary(&self) -> String {
        let meaning = self.meaning.trim();
        if meaning.is_empty() {
            self.word.clone()
        } else {
            format!("{}  —  {}", self.word.trim(), meaning)
        }
    }
}

/// Namespace for a group of entries. A collection may be named after a book,
/// a topic, a course, or anything else the user chooses.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(default)]
pub struct Collection {
    pub id: String,
    pub name: String,
    pub note: String,
    pub entries: Vec<Entry>,
}

impl Collection {
    pub fn new(name: &str, rng: &mut Rng) -> Collection {
        Collection {
            id: new_id("c", rng),
            name: name.to_string(),
            note: String::new(),
            entries: Vec::new(),
        }
    }

    pub fn learned_count(&self) -> usize {
        self.entries.iter().filter(|e| e.learned).count()
    }

    pub fn entry_index(&self, id: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.id == id)
    }

    /// Index of an entry whose word matches `word` (trimmed, case-insensitive),
    /// ignoring `ignore_id` (used when editing an existing entry).
    pub fn duplicate_index(&self, word: &str, ignore_id: Option<&str>) -> Option<usize> {
        let key = word_key(word);
        if key.is_empty() {
            return None;
        }
        self.entries
            .iter()
            .position(|e| e.key() == key && ignore_id.map(|id| e.id != id).unwrap_or(true))
    }

    pub fn word_keys(&self) -> std::collections::HashSet<String> {
        self.entries.iter().map(|e| e.key()).collect()
    }
}

/// Root object of `library.json`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct Library {
    pub schema_version: u32,
    pub active_collection: Option<String>,
    pub collections: Vec<Collection>,
}

impl Default for Library {
    fn default() -> Self {
        Library {
            schema_version: SCHEMA_VERSION,
            active_collection: None,
            collections: Vec::new(),
        }
    }
}

impl Library {
    pub fn is_empty(&self) -> bool {
        self.collections.is_empty()
    }

    pub fn active(&self) -> Option<&Collection> {
        let id = self.active_collection.as_ref()?;
        self.collections.iter().find(|c| &c.id == id)
    }

    pub fn active_mut(&mut self) -> Option<&mut Collection> {
        let id = self.active_collection.clone()?;
        self.collections.iter_mut().find(|c| c.id == id)
    }

    pub fn collection(&self, id: &str) -> Option<&Collection> {
        self.collections.iter().find(|c| c.id == id)
    }

    pub fn collection_mut(&mut self, id: &str) -> Option<&mut Collection> {
        self.collections.iter_mut().find(|c| c.id == id)
    }

    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.collections.iter().position(|c| c.id == id)
    }

    pub fn name_taken(&self, name: &str, ignore_id: Option<&str>) -> bool {
        let key = name.trim().to_lowercase();
        self.collections.iter().any(|c| {
            c.name.trim().to_lowercase() == key && ignore_id.map(|id| c.id != id).unwrap_or(true)
        })
    }

    /// Makes sure every collection and entry has a non-empty, unique id and that
    /// the active collection actually exists. Repairs hand-edited files.
    pub fn ensure_ids(&mut self, rng: &mut Rng) {
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for collection in &mut self.collections {
            if collection.id.trim().is_empty() || !seen.insert(collection.id.clone()) {
                collection.id = unique_id("c", rng, &mut seen);
            }
            for entry in &mut collection.entries {
                if entry.id.trim().is_empty() || !seen.insert(entry.id.clone()) {
                    entry.id = unique_id("e", rng, &mut seen);
                }
            }
        }
        if let Some(active) = &self.active_collection {
            if self.index_of(active).is_none() {
                self.active_collection = self.collections.first().map(|c| c.id.clone());
            }
        } else {
            self.active_collection = self.collections.first().map(|c| c.id.clone());
        }
    }

    pub fn entry_count(&self) -> usize {
        self.collections.iter().map(|c| c.entries.len()).sum()
    }
}

/// Persisted card position, stored in device-independent pixels relative to the
/// work area of the monitor identified by `monitor`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(default)]
pub struct CardPosition {
    /// Monitor device name such as `\\.\DISPLAY1`.
    pub monitor: String,
    pub x_dip: i32,
    pub y_dip: i32,
}

/// Root object of `config.json`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct Config {
    pub schema_version: u32,
    pub interval_minutes: u32,
    pub order: Order,
    pub theme: ThemeMode,
    pub font_size_pt: u32,
    pub card_width_dip: i32,
    pub always_on_top: bool,
    pub launch_at_login: bool,
    pub exclude_learned: bool,
    pub card_visible: bool,
    pub paused: bool,
    pub position: Option<CardPosition>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            schema_version: SCHEMA_VERSION,
            interval_minutes: DEFAULT_INTERVAL_MINUTES,
            order: Order::default(),
            theme: ThemeMode::default(),
            font_size_pt: DEFAULT_FONT_SIZE_PT,
            card_width_dip: DEFAULT_CARD_WIDTH_DIP,
            always_on_top: false,
            launch_at_login: false,
            exclude_learned: true,
            card_visible: true,
            paused: false,
            position: None,
        }
    }
}

impl Config {
    /// Clamps every numeric field into its supported range. Called after loading
    /// so that hand-edited files can never put the UI into an impossible state.
    pub fn sanitize(&mut self) {
        self.interval_minutes = self
            .interval_minutes
            .clamp(MIN_INTERVAL_MINUTES, MAX_INTERVAL_MINUTES);
        self.font_size_pt = self.font_size_pt.clamp(MIN_FONT_SIZE_PT, MAX_FONT_SIZE_PT);
        self.card_width_dip = self
            .card_width_dip
            .clamp(MIN_CARD_WIDTH_DIP, MAX_CARD_WIDTH_DIP);
        self.schema_version = SCHEMA_VERSION;
        if let Some(pos) = &mut self.position {
            // Guard against absurd values that would place the card off-world.
            pos.x_dip = pos.x_dip.clamp(-100_000, 100_000);
            pos.y_dip = pos.y_dip.clamp(-100_000, 100_000);
        }
    }

    pub fn interval_ms(&self) -> u32 {
        self.interval_minutes
            .clamp(MIN_INTERVAL_MINUTES, MAX_INTERVAL_MINUTES)
            .saturating_mul(60_000)
    }
}

/// Duplicate-detection key: trimmed, case-insensitive.
pub fn word_key(word: &str) -> String {
    word.trim().to_lowercase()
}

/// Small deterministic xorshift64* generator. Used for id suffixes and for
/// shuffling; avoiding a dependency for this is worth it.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    /// Seeds from the wall clock, the process id and a call counter.
    pub fn seed() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        nanos ^ ((std::process::id() as u64) << 32) ^ n.wrapping_mul(0x9E37_79B9_7F4A_7C15)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform-enough index in `0..n` (`n > 0`).
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }

    /// In-place Fisher-Yates shuffle.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        if items.len() < 2 {
            return;
        }
        for i in (1..items.len()).rev() {
            let j = self.below(i + 1);
            items.swap(i, j);
        }
    }
}

/// Creates an id such as `e-1a2b3c4d5e6f`.
pub fn new_id(prefix: &str, rng: &mut Rng) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    format!(
        "{prefix}-{:08x}{:04x}",
        (nanos as u32) ^ (rng.next_u64() as u32),
        (rng.next_u64() & 0xFFFF) as u16
    )
}

fn unique_id(prefix: &str, rng: &mut Rng, seen: &mut std::collections::HashSet<String>) -> String {
    loop {
        let id = new_id(prefix, rng);
        if seen.insert(id.clone()) {
            return id;
        }
    }
}

/// Compact UTC timestamp (`20261017T153012Z`) used for recovery file names.
/// Implemented with civil-from-days arithmetic to avoid a date dependency.
pub fn timestamp_utc() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_timestamp(secs)
}

fn format_timestamp(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    let rem = unix_secs % 86_400;
    let (hour, minute, second) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}Z")
}

/// Howard Hinnant's `civil_from_days` algorithm (days since 1970-01-01).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_key_trims_and_folds_case() {
        assert_eq!(word_key("  Adroit "), "adroit");
        assert_eq!(word_key("ADROIT"), word_key("adroit"));
    }

    #[test]
    fn config_sanitize_clamps_out_of_range_values() {
        let mut cfg = Config {
            interval_minutes: 0,
            font_size_pt: 400,
            card_width_dip: 10,
            ..Config::default()
        };
        cfg.sanitize();
        assert_eq!(cfg.interval_minutes, MIN_INTERVAL_MINUTES);
        assert_eq!(cfg.font_size_pt, MAX_FONT_SIZE_PT);
        assert_eq!(cfg.card_width_dip, MIN_CARD_WIDTH_DIP);
    }

    #[test]
    fn config_interval_ms_is_minutes_in_milliseconds() {
        let cfg = Config {
            interval_minutes: 15,
            ..Config::default()
        };
        assert_eq!(cfg.interval_ms(), 900_000);
        let cfg = Config {
            interval_minutes: 1440,
            ..Config::default()
        };
        assert_eq!(cfg.interval_ms(), 86_400_000);
    }

    #[test]
    fn partial_config_json_uses_defaults() {
        let cfg: Config = serde_json::from_str(r#"{"interval_minutes": 30}"#).unwrap();
        assert_eq!(cfg.interval_minutes, 30);
        assert_eq!(cfg.font_size_pt, DEFAULT_FONT_SIZE_PT);
        assert_eq!(cfg.order, Order::Sequential);
        assert!(cfg.exclude_learned);
        assert!(!cfg.always_on_top);
    }

    #[test]
    fn ensure_ids_repairs_missing_and_duplicate_ids() {
        let mut lib = Library {
            schema_version: SCHEMA_VERSION,
            active_collection: Some("gone".into()),
            collections: vec![
                Collection {
                    id: String::new(),
                    name: "A".into(),
                    note: String::new(),
                    entries: vec![
                        Entry {
                            id: "same".into(),
                            word: "a".into(),
                            ..Entry::default()
                        },
                        Entry {
                            id: "same".into(),
                            word: "b".into(),
                            ..Entry::default()
                        },
                    ],
                },
                Collection {
                    id: "same".into(),
                    name: "B".into(),
                    note: String::new(),
                    entries: Vec::new(),
                },
            ],
        };
        let mut rng = Rng::new(7);
        lib.ensure_ids(&mut rng);
        let ids: Vec<&str> = lib
            .collections
            .iter()
            .flat_map(|c| {
                c.entries
                    .iter()
                    .map(|e| e.id.as_str())
                    .chain([c.id.as_str()])
            })
            .collect();
        let unique: std::collections::HashSet<&&str> = ids.iter().collect();
        assert_eq!(ids.len(), unique.len(), "ids must be unique: {ids:?}");
        assert!(lib.collections.iter().all(|c| !c.id.is_empty()));
        // Active collection pointed at a missing id, so it falls back to the first one.
        assert_eq!(
            lib.active_collection.as_deref(),
            Some(lib.collections[0].id.as_str())
        );
    }

    #[test]
    fn duplicate_index_is_case_and_space_insensitive() {
        let mut rng = Rng::new(3);
        let mut collection = Collection::new("Test", &mut rng);
        collection
            .entries
            .push(Entry::new("  Adroit ", "Hábil", &mut rng));
        assert_eq!(collection.duplicate_index("adroit", None), Some(0));
        assert_eq!(collection.duplicate_index("ADROIT", Some("other")), Some(0));
        let id = collection.entries[0].id.clone();
        assert_eq!(collection.duplicate_index("adroit", Some(&id)), None);
    }

    #[test]
    fn timestamp_is_iso_shaped() {
        let ts = format_timestamp(1_769_000_000);
        assert_eq!(ts.len(), 16);
        assert!(ts.ends_with('Z'));
        assert_eq!(&ts[8..9], "T");
        assert_eq!(format_timestamp(0), "19700101T000000Z");
    }

    #[test]
    fn rng_shuffle_keeps_elements() {
        let mut rng = Rng::new(42);
        let mut items: Vec<u32> = (0..64).collect();
        rng.shuffle(&mut items);
        let mut sorted = items.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..64).collect::<Vec<u32>>());
        assert_ne!(items, sorted, "shuffle should change the order");
    }
}

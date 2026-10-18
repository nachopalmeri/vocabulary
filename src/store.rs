//! Persistence under `%LOCALAPPDATA%\WordDock`.
//!
//! Rules implemented here:
//! * writes are atomic (temporary file + replace with write-through);
//! * the previous good file is kept as `*.bak` before being replaced;
//! * a malformed file is *never* overwritten silently - it is moved to
//!   `recovery\` with a timestamp and the report tells the user what happened;
//! * a file written by a *newer* schema version is left untouched and reported
//!   as fatal, so an older build can never downgrade newer data.

use crate::model::{BACKUP_SUFFIX, CONFIG_FILE, Config, LIBRARY_FILE, Library, SCHEMA_VERSION};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Paths {
    pub root: PathBuf,
    pub config: PathBuf,
    pub library: PathBuf,
    pub backup_dir: PathBuf,
    pub recovery_dir: PathBuf,
}

impl Paths {
    /// `%LOCALAPPDATA%\WordDock`, falling back to a `data` folder next to the
    /// executable when the environment variable is unavailable.
    pub fn discover() -> Paths {
        let root = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|base| base.join("WordDock"))
            .or_else(|| {
                std::env::current_exe()
                    .ok()
                    .and_then(|exe| exe.parent().map(|p| p.join("WordDock-data")))
            })
            .unwrap_or_else(|| PathBuf::from("WordDock-data"));
        Paths::under(root)
    }

    pub fn under(root: PathBuf) -> Paths {
        Paths {
            config: root.join(CONFIG_FILE),
            library: root.join(LIBRARY_FILE),
            backup_dir: root.join("backup"),
            recovery_dir: root.join("recovery"),
            root,
        }
    }

    pub fn backup_of(&self, name: &str) -> PathBuf {
        self.backup_dir.join(format!("{name}{BACKUP_SUFFIX}"))
    }

    fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        std::fs::create_dir_all(&self.backup_dir)?;
        Ok(())
    }

    fn ensure_recovery_dir(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.recovery_dir)
    }
}

/// Where a loaded value came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// The main file was read successfully.
    File,
    /// The main file was unreadable, but its backup was good.
    Backup,
    /// Nothing usable on disk; the caller should use defaults.
    Default,
}

/// Result of a load attempt. `warning` is user-visible but not fatal;
/// `fatal` means the application must stop to avoid damaging data.
#[derive(Clone, Debug)]
pub struct Load<T> {
    pub value: Option<T>,
    pub source: Source,
    pub warning: Option<String>,
    pub fatal: Option<String>,
}

impl<T> Load<T> {
    fn defaults() -> Load<T> {
        Load {
            value: None,
            source: Source::Default,
            warning: None,
            fatal: None,
        }
    }

    fn from_file(value: T) -> Load<T> {
        Load {
            value: Some(value),
            source: Source::File,
            warning: None,
            fatal: None,
        }
    }
}

pub fn load_config(paths: &Paths) -> Load<Config> {
    load_json::<Config>(
        &paths.config,
        &paths.backup_of(CONFIG_FILE),
        paths,
        Config::default(),
    )
}

pub fn load_library(paths: &Paths) -> Load<Library> {
    load_json::<Library>(
        &paths.library,
        &paths.backup_of(LIBRARY_FILE),
        paths,
        Library::default(),
    )
}

/// Generic loader shared by config and library.
pub fn load_json<T: DeserializeOwned + Clone>(
    path: &Path,
    backup: &Path,
    paths: &Paths,
    defaults: T,
) -> Load<T> {
    match read_json::<T>(path) {
        Ok(Some(value)) => Load::from_file(value),
        Ok(None) => {
            // File missing: try the backup, otherwise start from defaults.
            match read_json::<T>(backup) {
                Ok(Some(value)) => Load {
                    value: Some(value),
                    source: Source::Backup,
                    warning: Some(format!(
                        "{} was missing; WordDock restored the last backup from {}.",
                        display(path),
                        display(backup)
                    )),
                    fatal: None,
                },
                _ => {
                    let mut load = Load::defaults();
                    load.value = Some(defaults);
                    load
                }
            }
        }
        Err(Problem::Newer { version }) => Load {
            value: None,
            source: Source::Default,
            warning: None,
            fatal: Some(format!(
                "{} was written by a newer version of WordDock (schema {version}; this build understands {SCHEMA_VERSION}).\n\n\
                 WordDock will not touch that file. Update WordDock, or move the file away to start fresh.",
                display(path)
            )),
        },
        Err(Problem::Malformed { message }) => {
            let quarantined = quarantine(paths, path).ok();
            let backup_result = read_json::<T>(backup);
            let (value, source, extra) = match backup_result {
                Ok(Some(value)) => (
                    Some(value),
                    Source::Backup,
                    format!("The last backup ({}) was loaded instead.", display(backup)),
                ),
                _ => (
                    Some(defaults),
                    Source::Default,
                    "No usable backup was found, so WordDock started from defaults.".to_string(),
                ),
            };
            let where_to = match &quarantined {
                Some(target) => format!(
                    "A copy of the unreadable file was kept at {}.",
                    display(target)
                ),
                None => "The unreadable file could not be moved aside; it was left untouched."
                    .to_string(),
            };
            Load {
                value,
                source,
                warning: Some(format!(
                    "{} could not be read: {message}\n\n{where_to}\n{extra}",
                    display(path)
                )),
                fatal: None,
            }
        }
    }
}

enum Problem {
    Newer { version: u32 },
    Malformed { message: String },
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, Problem> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(Problem::Malformed {
                message: error.to_string(),
            });
        }
    };
    if bytes.iter().all(|b| b.is_ascii_whitespace()) {
        return Err(Problem::Malformed {
            message: "the file is empty".to_string(),
        });
    }
    // Check the declared schema version before handing the bytes to the typed
    // deserializer: a file from a newer build must never be rewritten.
    if let Ok(document) = serde_json::from_slice::<serde_json::Value>(&bytes)
        && let Some(version) = document.get("schema_version").and_then(|v| v.as_u64())
        && version as u32 > SCHEMA_VERSION
    {
        return Err(Problem::Newer {
            version: version as u32,
        });
    }
    let value: T = serde_json::from_slice(&bytes).map_err(|error| Problem::Malformed {
        message: format!("invalid JSON ({error})"),
    })?;
    Ok(Some(value))
}

/// Checks the `schema_version` field of a raw JSON document without deserializing
/// it into the target type (used to detect files from a newer build).
fn peek_schema_version(path: &Path) -> Option<u32> {
    let bytes = std::fs::read(path).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value.get("schema_version")?.as_u64().map(|v| v as u32)
}

/// Moves an unreadable file to `recovery\<name>.<timestamp><ext>`.
fn quarantine(paths: &Paths, path: &Path) -> std::io::Result<PathBuf> {
    paths.ensure_recovery_dir()?;
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("data");
    let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("json");
    let mut target = paths.recovery_dir.join(format!(
        "{stem}.corrupt-{}.{extension}",
        crate::model::timestamp_utc()
    ));
    let mut attempt = 2;
    while target.exists() {
        target = paths.recovery_dir.join(format!(
            "{stem}.corrupt-{}-{attempt}.{extension}",
            crate::model::timestamp_utc()
        ));
        attempt += 1;
    }
    std::fs::rename(path, &target)?;
    Ok(target)
}

/// Saves `value` as pretty JSON, replacing `path` atomically and keeping the
/// previous content as `backup\<name>.bak`.
pub fn save_json<T: Serialize>(paths: &Paths, path: &Path, value: &T) -> Result<(), String> {
    paths
        .ensure_dirs()
        .map_err(|e| format!("cannot create {}: {e}", display(&paths.root)))?;
    let mut bytes =
        serde_json::to_vec_pretty(value).map_err(|e| format!("cannot serialize: {e}"))?;
    bytes.push(b'\n');

    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| format!("cannot write {}: {e}", display(&tmp)))?;
    {
        use std::io::Write;
        // Flush the temporary file to disk before it becomes the live file.
        if let Ok(mut file) = std::fs::OpenOptions::new().append(true).open(&tmp) {
            let _ = file.flush();
        }
    }

    // Keep the last good content as a backup before replacing it.
    if path.exists() {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("data");
        let backup = paths.backup_of(name);
        if let Some(parent) = backup.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::copy(path, &backup);
    }

    replace_file(&tmp, path).map_err(|e| format!("cannot replace {}: {e}", display(path)))
}

/// `MoveFileExW` with `REPLACE_EXISTING | WRITE_THROUGH`, so the swap is atomic
/// from the application's point of view.
fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    use windows::core::PCWSTR;

    let from_wide: Vec<u16> = from
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let to_wide: Vec<u16> = to
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: both buffers are NUL-terminated UTF-16 for the duration of the call.
    let result = unsafe {
        MoveFileExW(
            PCWSTR::from_raw(from_wide.as_ptr()),
            PCWSTR::from_raw(to_wide.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    match result {
        Ok(()) => Ok(()),
        Err(error) => Err(std::io::Error::other(error.to_string())),
    }
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

/// Returns the schema version stored in a file, when readable.
pub fn schema_version_of(path: &Path) -> Option<u32> {
    peek_schema_version(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Entry, Rng};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> TempDir {
            let path = std::env::temp_dir().join(format!(
                "worddock-test-{tag}-{}-{}",
                std::process::id(),
                crate::model::timestamp_utc()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn library_with(word: &str) -> Library {
        let mut rng = Rng::new(17);
        let mut library = Library::default();
        let mut collection = crate::model::Collection::new("Book", &mut rng);
        collection
            .entries
            .push(Entry::new(word, "significado", &mut rng));
        library.collections.push(collection.clone());
        library.active_collection = Some(collection.id);
        library
    }

    #[test]
    fn missing_files_report_defaults() {
        let dir = TempDir::new("missing");
        let paths = Paths::under(dir.0.clone());
        let load = load_library(&paths);
        assert_eq!(load.source, Source::Default);
        assert!(load.value.is_some(), "defaults are used on first run");
        assert!(load.warning.is_none(), "a fresh install should not warn");
        assert!(load.fatal.is_none());
    }

    #[test]
    fn save_then_load_round_trips_and_keeps_backup() {
        let dir = TempDir::new("roundtrip");
        let paths = Paths::under(dir.0.clone());
        let first = library_with("adroit");
        save_json(&paths, &paths.library, &first).unwrap();

        // First load: straight from the file, no backup yet.
        let load = load_library(&paths);
        assert_eq!(load.source, Source::File);
        assert_eq!(load.value.as_ref().unwrap().entry_count(), 1);
        assert!(!paths.backup_of(LIBRARY_FILE).exists());

        // Second save replaces the file and keeps the previous content as *.bak.
        let second = library_with("outlier");
        save_json(&paths, &paths.library, &second).unwrap();
        assert!(paths.backup_of(LIBRARY_FILE).exists());
        let load = load_library(&paths);
        assert_eq!(
            load.value.unwrap().collections[0].entries[0].word,
            "outlier"
        );

        let backup: Library =
            serde_json::from_slice(&std::fs::read(paths.backup_of(LIBRARY_FILE)).unwrap()).unwrap();
        assert_eq!(backup.collections[0].entries[0].word, "adroit");

        // No temporary files are left behind.
        assert!(!paths.library.with_extension("json.tmp").exists());
    }

    #[test]
    fn malformed_file_is_quarantined_and_backup_is_restored() {
        let dir = TempDir::new("recovery");
        let paths = Paths::under(dir.0.clone());
        let good = library_with("threshold");
        save_json(&paths, &paths.library, &good).unwrap();
        save_json(&paths, &paths.library, &good).unwrap(); // creates the .bak
        assert!(paths.backup_of(LIBRARY_FILE).exists());

        std::fs::write(&paths.library, b"{ this is not json").unwrap();
        let load = load_library(&paths);
        assert_eq!(load.source, Source::Backup, "warning: {:?}", load.warning);
        assert_eq!(
            load.value.unwrap().collections[0].entries[0].word,
            "threshold"
        );
        let warning = load.warning.unwrap();
        assert!(warning.contains("could not be read"));
        assert!(warning.contains("backup"));

        // The unreadable original was preserved, not overwritten.
        let recovered: Vec<_> = std::fs::read_dir(&paths.recovery_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(recovered.len(), 1, "expected exactly one quarantined file");
        let content = std::fs::read_to_string(recovered[0].path()).unwrap();
        assert!(content.contains("this is not json"));
    }

    #[test]
    fn malformed_file_without_backup_falls_back_to_defaults() {
        let dir = TempDir::new("nobackup");
        let paths = Paths::under(dir.0.clone());
        std::fs::create_dir_all(&paths.root).unwrap();
        std::fs::write(&paths.library, b"{ this is not json").unwrap();
        let load = load_library(&paths);
        assert_eq!(load.source, Source::Default);
        assert!(load.value.is_some(), "defaults are returned");
        assert!(load.warning.unwrap().contains("No usable backup"));
        assert!(
            !paths.library.exists(),
            "the unreadable file was moved aside"
        );
        assert!(paths.recovery_dir.exists());
    }

    #[test]
    fn empty_library_file_is_not_treated_as_first_run() {
        let dir = TempDir::new("emptylib");
        let paths = Paths::under(dir.0.clone());
        std::fs::create_dir_all(&paths.root).unwrap();
        std::fs::write(
            &paths.library,
            br#"{"schema_version": 1, "collections": []}"#,
        )
        .unwrap();
        let load = load_library(&paths);
        assert_eq!(load.source, Source::File);
        assert!(load.warning.is_none());
        assert!(load.value.unwrap().is_empty());
    }

    #[test]
    fn newer_schema_is_fatal_and_file_is_left_alone() {
        let dir = TempDir::new("newer");
        let paths = Paths::under(dir.0.clone());
        std::fs::create_dir_all(&paths.root).unwrap();
        let raw = format!(
            "{{\"schema_version\": {}, \"collections\": []}}",
            SCHEMA_VERSION + 1
        );
        std::fs::write(&paths.library, raw.as_bytes()).unwrap();
        let load = load_library(&paths);
        assert!(load.fatal.is_some());
        assert!(load.value.is_none());
        assert_eq!(std::fs::read_to_string(&paths.library).unwrap(), raw);
        assert_eq!(schema_version_of(&paths.library), Some(SCHEMA_VERSION + 1));
    }

    #[test]
    fn corrupt_config_does_not_prevent_defaults() {
        let dir = TempDir::new("badconfig");
        let paths = Paths::under(dir.0.clone());
        std::fs::create_dir_all(&paths.root).unwrap();
        std::fs::write(&paths.config, b"{\"interval_minutes\": \"soon\"}").unwrap();
        let load = load_config(&paths);
        assert_eq!(load.source, Source::Default);
        let mut config = load.value.unwrap();
        config.sanitize();
        assert_eq!(
            config.interval_minutes,
            crate::model::DEFAULT_INTERVAL_MINUTES
        );
        assert!(load.warning.unwrap().contains("invalid JSON"));
    }
}

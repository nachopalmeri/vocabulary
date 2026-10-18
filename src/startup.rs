//! Launch at login using the current-user `Run` key.
//!
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` is the documented
//! current-user startup mechanism; no administrator rights and no scheduled task
//! or service are involved.

use crate::win::registry;
use std::path::Path;

pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
pub const VALUE_NAME: &str = "WordDock";

/// Registry command line for the current executable.
fn command_line(exe: &Path) -> String {
    let path = exe.display().to_string();
    if path.contains(' ') {
        format!("\"{path}\"")
    } else {
        path
    }
}

/// Enables or disables the startup entry. Returns an error message on failure.
pub fn set_enabled(exe: &Path, enabled: bool) -> Result<(), String> {
    if enabled {
        registry::write_string(RUN_KEY, VALUE_NAME, &command_line(exe))
    } else {
        registry::delete_value(RUN_KEY, VALUE_NAME)
    }
}

/// Reads the current startup entry so the settings dialog shows the real state
/// (the user may have removed it by hand).
pub fn current_entry() -> Option<String> {
    registry::read_string(RUN_KEY, VALUE_NAME)
}

/// True when the startup entry points at this executable.
pub fn is_enabled_for(exe: &Path) -> bool {
    match current_entry() {
        Some(value) => {
            let value = value.trim().trim_matches('"');
            match (std::fs::canonicalize(value), std::fs::canonicalize(exe)) {
                (Ok(a), Ok(b)) => a == b,
                _ => value.eq_ignore_ascii_case(&exe.display().to_string()),
            }
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn command_line_quotes_paths_with_spaces() {
        assert_eq!(
            command_line(&PathBuf::from(r"C:\Program Files\WordDock\worddock.exe")),
            r#""C:\Program Files\WordDock\worddock.exe""#
        );
        assert_eq!(
            command_line(&PathBuf::from(r"D:\apps\worddock.exe")),
            r"D:\apps\worddock.exe"
        );
    }
}

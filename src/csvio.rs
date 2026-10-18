//! CSV and JSON import/export.
//!
//! The CSV reader is a small RFC 4180 implementation (quoted fields, escaped
//! quotes, embedded commas and newlines, optional UTF-8 BOM) written by hand so
//! that the application ships without an extra dependency and the exact
//! validation rules below stay under our control.

use crate::model::{Entry, Rng, SCHEMA_VERSION};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Hard limit for a single import file (10 MiB).
pub const MAX_IMPORT_BYTES: u64 = 10 * 1024 * 1024;
/// Hard limit for the number of data rows in a single import.
pub const MAX_IMPORT_ROWS: usize = 20_000;

/// How many rows the preview dialog lists before summarizing the rest.
pub const PREVIEW_ROW_LIMIT: usize = 2_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Csv,
    Json,
}

impl Format {
    pub fn from_path(path: &Path) -> Format {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref()
        {
            Some("json") => Format::Json,
            _ => Format::Csv,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::Json => "json",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Format::Csv => "CSV",
            Format::Json => "JSON",
        }
    }
}

/// Per-row validation outcome.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RowStatus {
    Ready,
    Duplicate,
    MissingWord,
    MissingMeaning,
    Invalid(String),
}

impl RowStatus {
    pub fn is_ready(&self) -> bool {
        matches!(self, RowStatus::Ready)
    }

    pub fn is_duplicate(&self) -> bool {
        matches!(self, RowStatus::Duplicate)
    }

    /// Short text shown in the preview list.
    pub fn label(&self) -> String {
        match self {
            RowStatus::Ready => "ok".to_string(),
            RowStatus::Duplicate => "duplicate".to_string(),
            RowStatus::MissingWord => "missing word".to_string(),
            RowStatus::MissingMeaning => "missing meaning".to_string(),
            RowStatus::Invalid(why) => format!("invalid: {why}"),
        }
    }
}

/// One parsed row plus its validation result.
#[derive(Clone, Debug)]
pub struct ImportRow {
    /// 1-based row number in the source (for CSV: the line where the record starts).
    pub row: usize,
    pub word: String,
    pub meaning: String,
    pub example: String,
    pub source: String,
    pub source_language: String,
    pub target_language: String,
    pub learned: bool,
    pub status: RowStatus,
}

impl ImportRow {
    fn from_fields<F: Fn(&str) -> String>(fields: F) -> ImportRow {
        ImportRow {
            row: 0,
            word: fields("word"),
            meaning: fields("meaning"),
            example: fields("example"),
            source: fields("source"),
            source_language: fields("source_language"),
            target_language: fields("target_language"),
            learned: parse_bool(&fields("learned")),
            status: RowStatus::Ready,
        }
    }

    pub fn to_entry(&self, rng: &mut Rng) -> Entry {
        Entry {
            id: crate::model::new_id("e", rng),
            word: self.word.trim().to_string(),
            meaning: self.meaning.trim().to_string(),
            example: self.example.trim().to_string(),
            source: self.source.trim().to_string(),
            source_language: self.source_language.trim().to_string(),
            target_language: self.target_language.trim().to_string(),
            learned: self.learned,
        }
    }

    /// One line for the preview list.
    pub fn preview_line(&self) -> String {
        let word = if self.word.trim().is_empty() {
            "(no word)"
        } else {
            self.word.trim()
        };
        let meaning: String = self.meaning.chars().take(90).collect();
        let meaning = if self.meaning.chars().count() > 90 {
            format!("{meaning}…")
        } else {
            meaning
        };
        if meaning.trim().is_empty() {
            format!("· {}  —  {}", word, self.status.label())
        } else {
            format!(
                "· {}  —  {}   [{}]",
                word,
                meaning.replace(['\r', '\n'], " "),
                self.status.label()
            )
        }
    }
}

/// Everything the preview dialog needs.
#[derive(Clone, Debug)]
pub struct ImportReport {
    pub path: PathBuf,
    pub format: Format,
    pub rows: Vec<ImportRow>,
    pub notes: Vec<String>,
    /// Set when the whole file must be rejected (too large, too many rows, unparsable).
    pub error: Option<String>,
}

impl ImportReport {
    pub fn ready_count(&self) -> usize {
        self.rows.iter().filter(|r| r.status.is_ready()).count()
    }

    pub fn duplicate_count(&self) -> usize {
        self.rows.iter().filter(|r| r.status.is_duplicate()).count()
    }

    pub fn invalid_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| !r.status.is_ready() && !r.status.is_duplicate())
            .count()
    }

    pub fn total(&self) -> usize {
        self.rows.len()
    }

    /// Rows that will be turned into entries. Duplicates are skipped unless the
    /// user unchecked "skip duplicates".
    pub fn accepted_rows(&self, include_duplicates: bool) -> Vec<&ImportRow> {
        self.rows
            .iter()
            .filter(|r| r.status.is_ready() || (include_duplicates && r.status.is_duplicate()))
            .collect()
    }

    pub fn to_entries(&self, include_duplicates: bool, rng: &mut Rng) -> Vec<Entry> {
        self.accepted_rows(include_duplicates)
            .into_iter()
            .map(|row| row.to_entry(rng))
            .collect()
    }

    pub fn summary(&self) -> String {
        format!(
            "{} rows · {} ready · {} duplicate · {} invalid",
            self.total(),
            self.ready_count(),
            self.duplicate_count(),
            self.invalid_count()
        )
    }
}

/// Reads a file, enforcing the size limit.
pub fn read_file_limited(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = std::fs::metadata(path).map_err(|e| format!("cannot read {path:?}: {e}"))?;
    if !metadata.is_file() {
        return Err(format!("{path:?} is not a regular file"));
    }
    if metadata.len() > MAX_IMPORT_BYTES {
        return Err(format!(
            "File is {:.1} MiB; the maximum import size is {} MiB. Split the file and import it in parts.",
            metadata.len() as f64 / (1024.0 * 1024.0),
            MAX_IMPORT_BYTES / (1024 * 1024)
        ));
    }
    std::fs::read(path).map_err(|e| format!("cannot read {path:?}: {e}"))
}

/// Reads and validates a file. `existing_keys` are duplicate-detection keys
/// (see [`crate::model::word_key`]) already present in the target collection.
pub fn read_and_parse(path: &Path, existing_keys: &HashSet<String>) -> ImportReport {
    let format = Format::from_path(path);
    match read_file_limited(path) {
        Ok(bytes) => parse_bytes(&bytes, format, existing_keys, path),
        Err(error) => ImportReport {
            path: path.to_path_buf(),
            format,
            rows: Vec::new(),
            notes: Vec::new(),
            error: Some(error),
        },
    }
}

/// Parses an in-memory buffer. Always returns a report: a structural problem
/// sets `error`, individual bad rows are reported per row and never discard the
/// rows that are valid.
pub fn parse_bytes(
    bytes: &[u8],
    format: Format,
    existing_keys: &HashSet<String>,
    path: &Path,
) -> ImportReport {
    let mut notes = Vec::new();
    if bytes.len() as u64 > MAX_IMPORT_BYTES {
        return ImportReport {
            path: path.to_path_buf(),
            format,
            rows: Vec::new(),
            notes,
            error: Some(format!(
                "File is larger than the {} MiB import limit.",
                MAX_IMPORT_BYTES / (1024 * 1024)
            )),
        };
    }

    let body = strip_bom(bytes);
    let text = match std::str::from_utf8(body) {
        Ok(text) => std::borrow::Cow::Borrowed(text),
        Err(_) => {
            notes.push("The file is not valid UTF-8; undecodable bytes were replaced.".to_string());
            std::borrow::Cow::Owned(String::from_utf8_lossy(body).into_owned())
        }
    };

    match format {
        Format::Csv => parse_csv_report(&text, existing_keys, path, notes),
        Format::Json => parse_json_report(&text, existing_keys, path, notes),
    }
}

fn strip_bom(bytes: &[u8]) -> &[u8] {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &bytes[3..]
    } else {
        bytes
    }
}

fn too_many_rows(path: &Path, format: Format, notes: Vec<String>, count: usize) -> ImportReport {
    ImportReport {
        path: path.to_path_buf(),
        format,
        rows: Vec::new(),
        notes,
        error: Some(format!(
            "The file contains {count} rows; the maximum is {MAX_IMPORT_ROWS}. Split it into smaller files."
        )),
    }
}

fn parse_csv_report(
    text: &str,
    existing_keys: &HashSet<String>,
    path: &Path,
    mut notes: Vec<String>,
) -> ImportReport {
    let mut records = parse_csv(text);
    if records.len() > MAX_IMPORT_ROWS + 1 {
        return too_many_rows(path, Format::Csv, notes, records.len());
    }
    if records.is_empty() {
        return ImportReport {
            path: path.to_path_buf(),
            format: Format::Csv,
            rows: Vec::new(),
            notes,
            error: Some("The file has no rows.".to_string()),
        };
    }

    let header = header_map(&records[0].fields);
    let (map, data) = match header {
        Some(map) => {
            notes.push(format!(
                "Header row detected (columns: {}).",
                records[0]
                    .fields
                    .iter()
                    .map(|f| f.trim())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            (map, records.split_off(1))
        }
        None => {
            notes.push(
                "No header row detected; columns are read as: word, meaning, example, source, source_language, target_language, learned."
                    .to_string(),
            );
            (HeaderMap::positional(), std::mem::take(&mut records))
        }
    };

    if data.len() > MAX_IMPORT_ROWS {
        return too_many_rows(path, Format::Csv, notes, data.len());
    }

    let mut seen: HashSet<String> = HashSet::new();
    let mut rows = Vec::with_capacity(data.len());
    for record in data {
        if record.fields.iter().all(|f| f.trim().is_empty()) {
            continue;
        }
        let fields = |name: &str| -> String {
            map.index(name)
                .and_then(|i| record.fields.get(i))
                .cloned()
                .unwrap_or_default()
        };
        let mut row = ImportRow::from_fields(&fields);
        row.row = record.line;
        row.status = validate_row(&row, existing_keys, &mut seen);
        rows.push(row);
    }

    ImportReport {
        path: path.to_path_buf(),
        format: Format::Csv,
        rows,
        notes,
        error: None,
    }
}

fn parse_json_report(
    text: &str,
    existing_keys: &HashSet<String>,
    path: &Path,
    mut notes: Vec<String>,
) -> ImportReport {
    let value: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(error) => {
            return ImportReport {
                path: path.to_path_buf(),
                format: Format::Json,
                rows: Vec::new(),
                notes,
                error: Some(format!("Invalid JSON: {error}")),
            };
        }
    };

    let items: Vec<(usize, Value)> = match value {
        Value::Array(items) => items.into_iter().enumerate().collect(),
        Value::Object(mut object) => match object.remove("entries") {
            Some(Value::Array(items)) => items.into_iter().enumerate().collect(),
            Some(_) => {
                return ImportReport {
                    path: path.to_path_buf(),
                    format: Format::Json,
                    rows: Vec::new(),
                    notes,
                    error: Some("The \"entries\" member must be an array.".to_string()),
                };
            }
            None => {
                return ImportReport {
                    path: path.to_path_buf(),
                    format: Format::Json,
                    rows: Vec::new(),
                    notes,
                    error: Some(
                        "Expected an array of entries or an object with an \"entries\" array."
                            .to_string(),
                    ),
                };
            }
        },
        _ => {
            return ImportReport {
                path: path.to_path_buf(),
                format: Format::Json,
                rows: Vec::new(),
                notes,
                error: Some(
                    "Expected an array of entries or an object with an \"entries\" array."
                        .to_string(),
                ),
            };
        }
    };

    if items.len() > MAX_IMPORT_ROWS {
        return too_many_rows(path, Format::Json, notes, items.len());
    }
    if items.is_empty() {
        notes.push("The file contains no entries.".to_string());
    }

    let mut seen: HashSet<String> = HashSet::new();
    let mut rows = Vec::with_capacity(items.len());
    for (index, item) in items {
        let mut row = match item {
            Value::Object(object) => {
                let invalid = std::cell::RefCell::new(None::<String>);
                let field = |name: &str| -> String {
                    match object.get(name) {
                        None | Some(Value::Null) => String::new(),
                        Some(Value::String(s)) => s.clone(),
                        // `learned` also accepts JSON booleans and 0/1 numbers.
                        Some(Value::Bool(b)) if name == "learned" => b.to_string(),
                        Some(Value::Number(n)) if name == "learned" => n.to_string(),
                        Some(other) => {
                            *invalid.borrow_mut() = Some(format!(
                                "field \"{name}\" must be a string, found {}",
                                kind(other)
                            ));
                            String::new()
                        }
                    }
                };
                let mut row = ImportRow::from_fields(&field);
                row.row = index + 1;
                if let Some(message) = invalid.into_inner() {
                    row.status = RowStatus::Invalid(message);
                    row.word.clear();
                    rows.push(row);
                    continue;
                }
                row
            }
            other => ImportRow {
                row: index + 1,
                word: String::new(),
                meaning: String::new(),
                example: String::new(),
                source: String::new(),
                source_language: String::new(),
                target_language: String::new(),
                learned: false,
                status: RowStatus::Invalid(format!("entry {} is not an object", kind(&other))),
            },
        };
        if row.status.is_ready() {
            row.status = validate_row(&row, existing_keys, &mut seen);
        }
        rows.push(row);
    }

    ImportReport {
        path: path.to_path_buf(),
        format: Format::Json,
        rows,
        notes,
        error: None,
    }
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

fn validate_row(
    row: &ImportRow,
    existing_keys: &HashSet<String>,
    seen: &mut HashSet<String>,
) -> RowStatus {
    if row.word.trim().is_empty() {
        return RowStatus::MissingWord;
    }
    if row.meaning.trim().is_empty() {
        return RowStatus::MissingMeaning;
    }
    let key = crate::model::word_key(&row.word);
    if existing_keys.contains(&key) || !seen.insert(key) {
        return RowStatus::Duplicate;
    }
    RowStatus::Ready
}

fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "y" | "si" | "sí" | "learned"
    )
}

// ---------------------------------------------------------------------------
// CSV reader
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
struct Record {
    /// 1-based line number where the record starts.
    line: usize,
    fields: Vec<String>,
}

/// RFC 4180 style parser: `"` quoting, `""` escapes, embedded commas and
/// newlines, CRLF or LF line endings. A trailing newline does not produce an
/// extra empty record.
fn parse_csv(text: &str) -> Vec<Record> {
    let mut records = Vec::new();
    let mut fields: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut record_started = false;
    let mut line = 1usize;
    let mut record_line = 1usize;
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if in_quotes {
            match c {
                '"' => {
                    if chars.peek() == Some(&'"') {
                        chars.next();
                        field.push('"');
                    } else {
                        in_quotes = false;
                    }
                }
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    field.push('\n');
                    line += 1;
                }
                '\n' => {
                    field.push('\n');
                    line += 1;
                }
                _ => field.push(c),
            }
            continue;
        }

        match c {
            '"' if field.trim().is_empty() => {
                field.clear();
                in_quotes = true;
                record_started = true;
            }
            ',' => {
                fields.push(std::mem::take(&mut field));
                record_started = true;
            }
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                fields.push(std::mem::take(&mut field));
                records.push(Record {
                    line: record_line,
                    fields: std::mem::take(&mut fields),
                });
                record_started = false;
                line += 1;
                record_line = line;
            }
            '\n' => {
                fields.push(std::mem::take(&mut field));
                records.push(Record {
                    line: record_line,
                    fields: std::mem::take(&mut fields),
                });
                record_started = false;
                line += 1;
                record_line = line;
            }
            _ => {
                field.push(c);
                record_started = true;
            }
        }
    }

    if record_started || !field.is_empty() || !fields.is_empty() {
        fields.push(field);
        records.push(Record {
            line: record_line,
            fields,
        });
    }
    records
}

/// Column-name → index mapping.
#[derive(Debug, Clone)]
struct HeaderMap(Vec<(String, usize)>);

impl HeaderMap {
    fn positional() -> HeaderMap {
        HeaderMap(vec![
            ("word".into(), 0),
            ("meaning".into(), 1),
            ("example".into(), 2),
            ("source".into(), 3),
            ("source_language".into(), 4),
            ("target_language".into(), 5),
            ("learned".into(), 6),
        ])
    }

    fn index(&self, name: &str) -> Option<usize> {
        self.0.iter().find(|(key, _)| key == name).map(|(_, i)| *i)
    }
}

fn normalize_header(name: &str) -> String {
    name.trim().to_ascii_lowercase().replace([' ', '-'], "_")
}

/// Returns a header map when the first row looks like a header (it must at least
/// contain `word` and `meaning`).
fn header_map(fields: &[String]) -> Option<HeaderMap> {
    let mut map: Vec<(String, usize)> = Vec::new();
    for (index, raw) in fields.iter().enumerate() {
        let name = normalize_header(raw);
        let name = match name.as_str() {
            "translation" | "definition" | "significado" => "meaning".to_string(),
            "word_" | "term" => "word".to_string(),
            other => other.to_string(),
        };
        if !name.is_empty() && !map.iter().any(|(key, _)| key == &name) {
            map.push((name, index));
        }
    }
    let has = |name: &str| map.iter().any(|(key, _)| key == name);
    if has("word") && has("meaning") {
        Some(HeaderMap(map))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Writers
// ---------------------------------------------------------------------------

pub const CSV_HEADER: &str = "word,meaning,example,source,source_language,target_language,learned";

/// Serializes entries to UTF-8 CSV (with BOM so that Excel detects UTF-8).
pub fn to_csv(entries: &[Entry]) -> String {
    let mut out = String::from("\u{FEFF}");
    out.push_str(CSV_HEADER);
    out.push_str("\r\n");
    for entry in entries {
        let fields = [
            entry.word.as_str(),
            entry.meaning.as_str(),
            entry.example.as_str(),
            entry.source.as_str(),
            entry.source_language.as_str(),
            entry.target_language.as_str(),
            if entry.learned { "true" } else { "false" },
        ];
        for (index, field) in fields.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str(&csv_field(field));
        }
        out.push_str("\r\n");
    }
    out
}

fn csv_field(value: &str) -> String {
    // Keep line breaks inside quotes so multiline fields survive a round trip.
    let needs_quotes = value.contains([',', '"', '\n', '\r']);
    if !needs_quotes {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        if c == '"' {
            out.push('"');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// Serializes entries to pretty JSON, including every entry field.
pub fn to_json(entries: &[Entry]) -> Result<String, serde_json::Error> {
    #[derive(serde::Serialize)]
    struct Document<'a> {
        schema_version: u32,
        entries: &'a [Entry],
    }
    serde_json::to_string_pretty(&Document {
        schema_version: SCHEMA_VERSION,
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Collection, Entry, Rng};

    fn keys(collection: &Collection) -> HashSet<String> {
        collection.word_keys()
    }

    fn p(csv: &str, existing: &HashSet<String>) -> ImportReport {
        parse_bytes(csv.as_bytes(), Format::Csv, existing, Path::new("test.csv"))
    }

    fn pj(json: &str, existing: &HashSet<String>) -> ImportReport {
        parse_bytes(
            json.as_bytes(),
            Format::Json,
            existing,
            Path::new("test.json"),
        )
    }

    #[test]
    fn csv_parses_quoted_commas_and_multiline_fields() {
        let csv = "word,meaning,example\n\
                   \"to weigh, carefully\",\"Sopesar, comparar\",\"Line one\nLine two\"\n";
        let report = p(csv, &HashSet::new());
        assert_eq!(report.error, None);
        assert_eq!(report.total(), 1);
        let row = &report.rows[0];
        assert_eq!(row.word, "to weigh, carefully");
        assert_eq!(row.meaning, "Sopesar, comparar");
        assert_eq!(row.example, "Line one\nLine two");
        assert_eq!(row.status, RowStatus::Ready);
        assert_eq!(report.ready_count(), 1);
    }

    #[test]
    fn csv_handles_bom_escaped_quotes_and_crlf() {
        let csv = "\u{FEFF}word,meaning\r\n\"say \"\"hello\"\"\",\"decir hola\"\r\n";
        let report = p(csv, &HashSet::new());
        assert_eq!(report.error, None);
        assert_eq!(report.total(), 1);
        assert_eq!(report.rows[0].word, "say \"hello\"");
        assert_eq!(report.rows[0].meaning, "decir hola");
    }

    #[test]
    fn csv_header_mapping_accepts_any_column_order() {
        let csv = "meaning,word,source language,learned\n\
                   \"insignificante\",\"negligible\",\"English\",yes\n";
        let report = p(csv, &HashSet::new());
        assert_eq!(report.total(), 1);
        let row = &report.rows[0];
        assert_eq!(row.word, "negligible");
        assert_eq!(row.meaning, "insignificante");
        assert_eq!(row.source_language, "English");
        assert!(row.learned);
        assert!(
            report
                .notes
                .iter()
                .any(|n| n.starts_with("Header row detected"))
        );
    }

    #[test]
    fn csv_without_header_uses_positional_columns() {
        let csv = "adroit,Hábil,,WordDock sample,English,Spanish,false\n";
        let report = p(csv, &HashSet::new());
        assert_eq!(report.total(), 1);
        let row = &report.rows[0];
        assert_eq!(row.word, "adroit");
        assert_eq!(row.meaning, "Hábil");
        assert_eq!(row.source, "WordDock sample");
        assert_eq!(row.target_language, "Spanish");
        assert!(!row.learned);
        assert!(report.notes.iter().any(|n| n.contains("No header row")));
    }

    #[test]
    fn invalid_rows_are_reported_without_discarding_valid_ones() {
        let csv = "word,meaning\n\
                   adroit,Hábil\n\
                   ,sin palabra\n\
                   outlier,\n\
                   threshold,Umbral\n";
        let report = p(csv, &HashSet::new());
        assert_eq!(report.total(), 4);
        assert_eq!(report.ready_count(), 2);
        assert_eq!(report.invalid_count(), 2);
        assert_eq!(report.rows[0].status, RowStatus::Ready);
        assert_eq!(report.rows[1].status, RowStatus::MissingWord);
        assert_eq!(report.rows[2].status, RowStatus::MissingMeaning);
        assert_eq!(report.rows[3].status, RowStatus::Ready);
        // Row numbers are 1-based and point at the source line.
        assert_eq!(report.rows[0].row, 2);
        assert_eq!(report.rows[2].row, 4);
    }

    #[test]
    fn duplicates_are_detected_against_collection_and_file() {
        let mut rng = Rng::new(5);
        let mut collection = Collection::new("Existing", &mut rng);
        collection
            .entries
            .push(Entry::new("adroit", "Hábil", &mut rng));
        collection
            .entries
            .push(Entry::new("outlier", "Valor atípico", &mut rng));

        let csv = "word,meaning\n\
                   ADROIT,otra definición\n\
                   adroit ,otra más\n\
                   new word,nueva palabra\n\
                   outlier, duplicado\n";
        let report = p(csv, &keys(&collection));
        assert_eq!(report.ready_count(), 1);
        assert_eq!(report.duplicate_count(), 3);
        assert_eq!(report.rows[0].status, RowStatus::Duplicate);
        assert_eq!(
            report.rows[1].status,
            RowStatus::Duplicate,
            "case and spaces are ignored"
        );
        assert_eq!(report.rows[2].status, RowStatus::Ready);
        assert_eq!(report.rows[3].status, RowStatus::Duplicate);

        // Applying with duplicates skipped adds exactly one entry.
        let entries = report.to_entries(false, &mut rng);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].word, "new word");
        // ... and the user can opt to keep them.
        let all = report.to_entries(true, &mut rng);
        assert_eq!(all.len(), 4);
    }

    #[test]
    fn too_many_rows_is_a_clear_error() {
        let mut csv = String::from("word,meaning\n");
        for i in 0..(MAX_IMPORT_ROWS + 5) {
            csv.push_str(&format!("word{i},meaning{i}\n"));
        }
        let report = p(&csv, &HashSet::new());
        assert!(report.error.as_deref().unwrap_or("").contains("maximum"));
        assert!(report.rows.is_empty());
    }

    #[test]
    fn oversized_files_are_rejected() {
        let mut bytes = vec![b'x'; (MAX_IMPORT_BYTES + 1) as usize];
        bytes[0] = b'a';
        let report = parse_bytes(&bytes, Format::Csv, &HashSet::new(), Path::new("big.csv"));
        assert!(report.error.is_some());
        assert!(report.rows.is_empty());
    }

    #[test]
    fn csv_round_trip_preserves_every_field() {
        let mut rng = Rng::new(9);
        let entries = vec![
            Entry {
                id: "e-1".into(),
                word: "to stall".into(),
                meaning: "Dar largas, postergar.".into(),
                example: "He stalled,\nand the deadline passed.".into(),
                source: "WordDock sample".into(),
                source_language: "English".into(),
                target_language: "Spanish".into(),
                learned: true,
            },
            Entry::new("adroit", "Hábil, diestro.", &mut rng),
        ];
        let csv = to_csv(&entries);
        let report = p(&csv, &HashSet::new());
        assert_eq!(report.error, None);
        assert_eq!(report.total(), 2);
        for (original, row) in entries.iter().zip(report.rows.iter()) {
            assert_eq!(row.word, original.word);
            assert_eq!(row.meaning, original.meaning);
            assert_eq!(row.example, original.example);
            assert_eq!(row.source, original.source);
            assert_eq!(row.source_language, original.source_language);
            assert_eq!(row.target_language, original.target_language);
            assert_eq!(row.learned, original.learned);
            assert_eq!(row.status, RowStatus::Ready);
        }
    }

    #[test]
    fn json_import_validates_per_row() {
        let json = r#"[
            {"word": "adroit", "meaning": "Hábil", "source": "Book X", "learned": true},
            {"word": "", "meaning": "sin palabra"},
            {"word": "outlier"},
            {"meaning": "sin palabra"},
            {"word": "threshold", "meaning": 12},
            "not an object"
        ]"#;
        let report = pj(json, &HashSet::new());
        assert_eq!(report.error, None);
        assert_eq!(report.total(), 6);
        assert_eq!(report.ready_count(), 1);
        assert_eq!(report.rows[0].status, RowStatus::Ready);
        assert!(report.rows[0].learned);
        assert_eq!(report.rows[1].status, RowStatus::MissingWord);
        assert_eq!(report.rows[2].status, RowStatus::MissingMeaning);
        assert_eq!(report.rows[3].status, RowStatus::MissingWord);
        assert!(matches!(report.rows[4].status, RowStatus::Invalid(_)));
        assert!(matches!(report.rows[5].status, RowStatus::Invalid(_)));
        // Every row that will not be imported counts as invalid (4 bad rows
        // plus the two rows with a missing required field).
        assert_eq!(report.invalid_count(), 5);
    }

    #[test]
    fn json_export_contains_all_fields_and_reimports() {
        let mut rng = Rng::new(13);
        let mut entry = Entry::new("trade-off", "Compensación", &mut rng);
        entry.example = "A trade-off between time and money.".into();
        entry.source = "My notes".into();
        entry.source_language = "English".into();
        entry.target_language = "Spanish".into();
        entry.learned = true;
        let entries = vec![entry.clone()];

        let json = to_json(&entries).unwrap();
        for field in [
            "word",
            "meaning",
            "example",
            "source",
            "source_language",
            "target_language",
            "learned",
            "id",
            "schema_version",
        ] {
            assert!(json.contains(field), "export must contain {field}: {json}");
        }
        let report = pj(&json, &HashSet::new());
        assert_eq!(report.error, None);
        assert_eq!(report.total(), 1);
        assert_eq!(report.rows[0].word, entry.word);
        assert_eq!(report.rows[0].example, entry.example);
        assert!(report.rows[0].learned);
    }

    #[test]
    fn json_accepts_wrapper_object_and_reports_bad_structures() {
        let wrapped = pj(
            r#"{"schema_version": 1, "entries": [{"word":"a","meaning":"b"}]}"#,
            &HashSet::new(),
        );
        assert_eq!(wrapped.ready_count(), 1);

        let broken = pj("{ not json", &HashSet::new());
        assert!(
            broken
                .error
                .as_deref()
                .unwrap_or("")
                .starts_with("Invalid JSON")
        );

        let wrong = pj(r#"{"entries": 3}"#, &HashSet::new());
        assert!(wrong.error.is_some());
    }

    #[test]
    fn format_detection_uses_extension() {
        assert_eq!(Format::from_path(Path::new("a/b.JSON")), Format::Json);
        assert_eq!(Format::from_path(Path::new("a/b.csv")), Format::Csv);
        assert_eq!(Format::from_path(Path::new("a/b.txt")), Format::Csv);
    }

    #[test]
    fn bundled_sample_csv_and_json_each_validate_twenty_rows() {
        let csv = parse_bytes(
            include_bytes!("../samples/sample_vocabulary.csv"),
            Format::Csv,
            &HashSet::new(),
            Path::new("sample_vocabulary.csv"),
        );
        let json = parse_bytes(
            include_bytes!("../samples/sample_vocabulary.json"),
            Format::Json,
            &HashSet::new(),
            Path::new("sample_vocabulary.json"),
        );
        for report in [csv, json] {
            assert_eq!(report.error, None);
            assert_eq!(report.total(), 20);
            assert_eq!(report.ready_count(), 20);
        }
    }

    #[test]
    fn csv_parser_skips_blank_lines() {
        let csv = "word,meaning\n\nadroit,Hábil\n\n";
        let report = p(csv, &HashSet::new());
        assert_eq!(report.total(), 1);
        assert_eq!(report.ready_count(), 1);
    }
}

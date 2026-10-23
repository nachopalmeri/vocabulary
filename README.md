# WordDock

WordDock is a small, native Windows 11 vocabulary companion written in Rust and Win32. It displays one entry in a compact card above the taskbar; it is not embedded into Explorer or the taskbar. The card does not take focus, and closing it hides it while the tray app keeps running.

The first run creates a collection containing 20 original English→Spanish sample entries. They were written for WordDock and are not extracted from a book. Rename the collection for your own book or topic, then edit or replace the entries.

## Requirements

- Windows 11 x64
- Rust stable, MSVC target (`stable-x86_64-pc-windows-msvc`)
- Visual Studio 2022 Build Tools with the **Desktop development with C++** workload and a Windows 10/11 SDK

## Build and run

From this repository in PowerShell:

```powershell
cargo fmt --check
cargo test --offline
cargo clippy --offline --all-targets
cargo build --release --offline
.
target\release\worddock.exe
```

The portable release executable is `target\release\worddock.exe`; no installer is required. Rust dependencies are locked in `Cargo.lock`.

## Use

- Left-click the tray icon to show/hide the card; right-click for navigation, pause, learned/review, word details, settings, collection management, and Exit.
- Clothesline opens from the tray menu or `Home`; resting the pointer at the top edge also reveals it. It is a compact three-card shelf placed below the browser tab strip, fades in/out without taking focus, and tucks away after the pointer leaves. Use `<` / `>` to page through up to ten items. Click a paper card to copy its content; right-click a card to copy, remove it, or clear the shelf. Image thumbnails are normalized for correct stride, orientation, and aspect ratio.
- Drag the card to reposition it. The tray Exit command terminates WordDock.
- Settings control the 1–1440 minute interval, sequential/shuffled order, active collection, theme, font size, card width, topmost behavior, learned filtering, and optional launch-at-login (off by default).
- Manage collections to add, rename, or delete collections and entries. The manager also imports and exports CSV/JSON. Its entry list has a live filter over words, meanings, and examples, and reports how many entries match plus the learned percentage.
- Word details shows the full meaning, example, source, and languages. **Pronounce** reads the word aloud with the built-in Windows speech engine, preferring an installed English voice.
- Imports are limited to 10 MiB and 20,000 data rows. CSV accepts a header or positional columns (`word, meaning, example, source, source_language, target_language, learned`). Duplicate words are case/whitespace-insensitive and skipped by default. Exported CSV is UTF-8 with BOM for spreadsheet compatibility; JSON uses UTF-8.
- Data is stored in `%LOCALAPPDATA%\WordDock` (`config.json`, `library.json`, `backup\`, and `recovery\`). Writes replace atomically and preserve the prior file as a backup. Corrupt files are quarantined; a file from a newer schema prevents startup without being overwritten.

Sample import files: [`samples/sample_vocabulary.csv`](samples/sample_vocabulary.csv) and [`samples/sample_vocabulary.json`](samples/sample_vocabulary.json).

Optional English collections adapted from public Wikipedia glossaries are in [`vocabularies/`](vocabularies/) (computer science, artificial intelligence, philosophy). Definitions retain attribution in each CSV source field. Wikipedia text is available under [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/); these CSVs are community-maintained imports, not book excerpts or an official glossary edition. [`ThinkingFastAndSlow.csv`](vocabularies/ThinkingFastAndSlow.csv) is a 613-entry study set of words and phrases grouped by book topic, with contextual Spanish translations and study examples, not text copied from the book. Import them through Manage collections.

## Idle measurement

1. Build and start `target\release\worddock.exe`; leave the card visible and do not interact with it.
2. In another PowerShell window, run `powershell -ExecutionPolicy Bypass -File .\scripts\measure-worddock.ps1 -Seconds 60`.
3. Record Private Bytes, working set, normalized CPU percentage, handles, and deltas. CPU is process CPU time divided by elapsed wall time and logical processor count. This is a measurement procedure, not a performance guarantee.

## Manual interaction checklist

- Tray click toggles the card; tray Exit terminates; card close button hides it without stealing focus.
- Next/previous, pause/resume, mark learned, review again, and learned filtering.
- Create/rename/delete a collection; add/edit/delete entries; select an active collection.
- In Manage collections, filter by a word, a Spanish meaning, and part of an example; confirm the "Showing N of M" count and that add/edit/delete still target the filtered row.
- Open Word details from the tray and click Pronounce; confirm the word is spoken and a second click restarts it instead of queueing.
- Import both sample formats; inspect validation/duplicate preview; test cancel and export both formats.
- Restart and verify saved settings, collection, learned state, and card position.
- Move the card between monitors with different DPI scales; disconnect/reconnect a monitor.
- Lock and unlock the Windows session; verify rotation suspends during lock.
- Enable and disable launch-at-login; confirm the Run-key entry changes as expected.
- With a disposable data directory, test malformed JSON recovery and confirm a newer schema is left untouched.

## Scope and limitations

WordDock has no network access, accounts, telemetry, background polling, embedded browser, Explorer injection, cloud sync, updater, or installer. Standard Windows push buttons retain system theme styling; dark backgrounds and text are custom-painted. Pronunciation uses the offline Windows SAPI voices already installed on the machine; it never contacts a service. Performance figures must be measured on the target system using the included script.

Clothesline keeps up to 10 distinct text, DIB screenshot/image, or copied-file-path items. Image bytes and history metadata are encrypted with Windows DPAPI for the current account; copied files remain references to their original paths and are never duplicated. It respects Windows' `ExcludeClipboardContentFromMonitorProcessing` privacy marker. HTML/RTF preservation, drag-and-drop, screenshot capture interception, and screenshot-folder watching are not implemented. The top-edge trigger currently targets the primary monitor.

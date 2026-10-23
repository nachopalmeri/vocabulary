# WordDock

**A native vocabulary card and clipboard shelf for Windows 11, built with Rust and Win32.**

WordDock lives in the notification area. It rotates entries from your own vocabulary collections in a compact card and keeps recent clipboard content on a small clothesline shelf called the **Tendedero**. Everything is native: no webview, no Explorer injection, no account, no telemetry. WordDock is open source under the MIT license.

![Platform](https://img.shields.io/badge/platform-Windows%2011%20x64-0078D4)
[![License](https://img.shields.io/badge/license-MIT-green)](LICENSE)

> **Status:** early development. There is no installer or packaged release yet; build from source.

## Features

- **Vocabulary card** — rotates words and phrases from one or more collections. Choose word + meaning, word only (reveal the meaning on hover), or word + meaning + example.
- **Study controls** — next/previous, pause, mark learned, review again, learned filtering, sequential or shuffled order, and a 1–1440 minute interval.
- **Collections** — create, rename, and delete collections; add, edit, and delete entries; filter the entry list live across words, meanings, and examples; see match and learned counts.
- **Pronunciation** — the details window can speak the current word with the offline Windows SAPI voice, preferring an installed English voice.
- **Tendedero clipboard shelf** — a compact, non-activating clothesline for copied text, screenshots/images, and file references. Three cards are visible at a time, up to ten are kept. Click a card to put its content back on the clipboard; right-click to copy, remove, or clear.
- **Native and small** — plain Win32/GDI windows, DPI-aware, multi-monitor positioning, optional always-on-top, and a single running instance. No Electron, WebView2, or embedded browser.
- **Local-first** — no network access, accounts, telemetry, cloud sync, or background polling. Data lives in `%LOCALAPPDATA%\WordDock`; shelf history and image bytes are protected with Windows DPAPI.
- **Your own data** — import and export CSV or JSON, with atomic writes, backups, and quarantine recovery for malformed files.

## Requirements

- Windows 11 x64 (the tested target)
- Rust 1.85 or newer with the MSVC target (`stable-x86_64-pc-windows-msvc`)
- Visual Studio 2022 Build Tools with the **Desktop development with C++** workload and a Windows 10/11 SDK

## Build from source

```powershell
git clone https://github.com/nachopalmeri/vocabulary.git
cd vocabulary
cargo build --release
.\target\release\worddock.exe
```

`--offline` works after the first dependency fetch. The result is a standalone executable: `target\release\worddock.exe` needs no installer and can be placed anywhere.

Before sending a change, run the same checks the project uses:

```powershell
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## First run

WordDock creates a collection with 20 original English–Spanish sample entries. It is a starting point, not a book extract: rename it, edit the entries, or import your own vocabulary through **Manage collections**.

The tray icon controls the app. The card appears near the bottom-right corner, above the taskbar; drag it wherever you like. Open **Settings** from the tray to change the interval, order, active collections, theme, font size, card width, topmost behavior, learned filtering, and launch-at-login.

## Usage

### Vocabulary card

- Left-click the tray icon to show or hide the card.
- Hover the card to reveal its controls, or press **Ctrl+Shift+Space** to show them from anywhere.
- Drag the card to reposition it. The close button hides the card without taking focus; the tray app keeps running.
- **Show more** opens the details window with the full meaning, example, source, languages, a learned/review toggle, and **Pronounce**.

### Tendedero clipboard shelf

- Press **Ctrl+Alt+Space**, rest the pointer at the top edge of the screen, or choose **Tendedero** from the tray menu.
- The shelf fades in without taking focus, so typing and browser navigation continue normally.
- Use `<` and `>` to page through up to ten items; it tucks away after the pointer leaves.
- Click a paper card to restore its content. Right-click a card to copy, remove it, or clear the shelf.
- Images are normalized for correct stride, orientation, and aspect ratio. Copied files remain references to their original paths and are never duplicated.

### Collections and study tools

- **Manage collections** adds, renames, and deletes collections and entries, imports CSV/JSON, and exports both formats.
- The entry filter matches words, meanings, and examples as you type. It reports `Showing N of M` and the learned percentage for the selected collection.
- Learned entries can be excluded from rotation. Marking an entry learned or reviewing it again updates the card immediately.

### Global hotkeys

| Shortcut | Action |
|---|---|
| `Ctrl+Shift+Space` | Reveal the card's controls |
| `Ctrl+Alt+Space` | Show or hide the Tendedero |

A hotkey may be unavailable if another application already owns it; the tray menu and the top-edge trigger remain available.

## Data and privacy

WordDock stores its data in `%LOCALAPPDATA%\WordDock`:

- `config.json` — preferences, selected collections, position, and presentation mode.
- `library.json` — collections and entries.
- `clothesline.json` — shelf history.
- `backup\` — previous copies of files.
- `recovery\` — quarantined files that could not be read.

Writes replace files atomically and keep the previous version as a backup. A file from a newer schema stops startup without being overwritten. Shelf metadata and copied image bytes are encrypted with DPAPI for the current Windows account. WordDock makes no network requests and includes no telemetry.

Pronunciation uses the offline SAPI voices already installed on the machine; it never contacts a service. If no English voice is installed, WordDock falls back to the system default voice.

## Vocabulary files

Sample imports: [`samples/sample_vocabulary.csv`](samples/sample_vocabulary.csv) and [`samples/sample_vocabulary.json`](samples/sample_vocabulary.json).

Additional collections are in [`vocabularies/`](vocabularies/):

- `ComputerScience.csv`, `ArtificialIntelligence.csv`, and `Philosophy.csv` are adapted from public Wikipedia glossaries. Definitions retain attribution in each CSV source field. Wikipedia text is available under [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/); these are community-maintained imports, not official glossary editions.
- `ThinkingFastAndSlow.csv` is a 613-entry study set of words and phrases grouped by book topic, with contextual Spanish translations and original study sentences. It is a vocabulary study aid, not text copied from the book.

Import limits are 10 MiB and 20,000 data rows. CSV accepts a header or positional columns (`word, meaning, example, source, source_language, target_language, learned`). Duplicate words are case- and whitespace-insensitive and skipped by default. Exported CSV is UTF-8 with a BOM for spreadsheet compatibility; JSON is UTF-8.

## Development

| File | Responsibility |
|---|---|
| `src/main.rs` | Single-instance startup, window classes, hotkeys, clipboard listener, and message loop. |
| `src/app.rs` | Owns `App`, routes messages, updates the card, and integrates the Tendedero. |
| `src/model.rs` | Serializable configuration and vocabulary types. |
| `src/rotation.rs` | Eligibility and sequential/shuffled traversal across selected collections. |
| `src/ui/card.rs` | Card measurement, painting, positioning, and pointer controls. |
| `src/ui/settings.rs` | Settings window and live preview. |
| `src/ui/collections.rs` | Collection/entry manager, import/export, and live filtering. |
| `src/ui/details.rs` | Details window and SAPI pronunciation button. |
| `src/speech.rs` | Lazy COM SAPI voice wrapper with English-first SSML and a plain fallback. |
| `src/clothesline.rs` | Top-edge sensor, shelf rendering, DIB normalization, history, and item actions. |
| `src/win.rs` | Win32 helpers for clipboard/DIB, DPAPI, monitors, and owned GDI resources. |
| `src/store.rs` | JSON storage, backups, and recovery. |

The detailed manual test checklist and the idle-measurement procedure live in [`docs/manual-testing.md`](docs/manual-testing.md).

Idle footprint is small, but measure it on your own machine rather than trusting a number from another desktop:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\measure-worddock.ps1 -Seconds 60
```

## Limitations

- Windows 11 x64 is the supported target; older Windows versions are not currently tested.
- The top-edge trigger targets the primary monitor only.
- HTML/RTF clipboard preservation, drag-and-drop, screenshot-folder watching, and a rich Quick Look-style preview are not implemented.
- There is no installer, auto-updater, cloud sync, or account system.
- Standard Windows push buttons keep the system theme; custom colors apply to the card and the Tendedero.

## Contributing

Issues and pull requests are welcome. Keep the app native (Win32/GDI) and local-first: avoid webviews, accounts, telemetry, and new network dependencies. Run `cargo fmt --check`, strict Clippy, and `cargo test` before opening a pull request, and describe the manual verification you performed.

By contributing, you agree that your changes are licensed under the MIT license.

## License

MIT — see [LICENSE](LICENSE).

# WordDock configuration overhaul

- [x] Add backward-compatible content selection, presentation, appearance, and placement preferences.
- [x] Extend rotation and card rendering to selected collections and presentation modes.
- [x] Replace the small settings form with a four-section native setup/settings window and live preview.
- [x] Add first-run setup, import/manage actions, monitor/anchor selection, and color pickers.
- [x] Verify state changes with tests; run fmt, strict Clippy, tests, and release build.
- [x] Test isolated first-run setup and checkbox/collection defaults.
- [x] Test live one-minute rotation, card minimize/restore, and Show more opening full details.
- [x] Confirm no stale process was active, launch the final release on the normal profile, and leave Settings plus the card open.
- [x] Visually inspect short and long entries at the available 100% desktop scale; 125/150/200% use automated measurement/layout tests only.

# Clothesline and Rust learning guide follow-up

- [x] Replace native rectangle controls with GDI paper/image/file cards, clips, sagging rope, layered opacity, edge reveal, click-through backdrop, and eased tuck animation.
- [x] Add text, DIB/DIBV5 image, and CF_HDROP file-reference clipboard items; restore the copied format on click.
- [x] Protect shelf metadata and copied image media with current-user DPAPI; honor Windows' private clipboard monitoring marker.
- [x] Verify DIB bounds/render path, item serialization, glossary CSV import validation, fmt, strict Clippy, tests, and release build.
- [x] Normalize DIB stride, top-down rows, and alpha before rendering; screenshot thumbnails no longer show black stripes.
- [x] Test clipboard image capture and thumbnail rendering on the desktop.
- [ ] Test a copied multi-file bundle, file restore, and clipboard persistence after restart.
- [ ] Add rich HTML/RTF preservation and drag-and-drop only after the text/image/file shelf behavior is visually approved.
- [x] Create the WordDock Rust/architecture learning guide and two source SVGs in the UADE Obsidian project folder.
- [x] Redesign the shelf to three compact cards with a small header, paging, click-through canvas, and a 110 ms alpha fade.
- [x] Import the 22-entry "Thinking, Fast and Slow" study collection and select it for rotation.
- [x] Expand the study collection to the full 613-entry list and re-select it for rotation.
- [x] Replace every template study example with a natural, topic-specific sentence.
- [x] Check launch-at-login entry: `Run` points at the release, and the app was already running after the next sign-in (PID 13072, 09:46).

# Study workflow features

- [x] Add a live filter to Manage collections across word, meaning, and example, with match count and learned percentage.
- [x] Add `Word details…` to the tray menu and a `Pronounce` button backed by Windows SAPI, preferring an installed en-US voice.
- [x] Verify the filter on the 619-entry collection (2 matches for `wary`) and the details/pronounce layout on the debug build.
- [x] Run `cargo fmt --check`, strict Clippy, tests, and the release build.

# Public repository polish

- [x] Rewrite the README for a public open-source audience: features, requirements, build, usage, privacy, vocabularies, contributing, and license.
- [x] Add the MIT `LICENSE` file.
- [x] Move the manual test checklist and idle-measurement procedure to `docs/manual-testing.md`.
- [x] Replace the plain `Home` shelf hotkey with `Ctrl+Alt+Space`; a plain global hotkey swallowed Home in every app.
- [x] Re-run fmt, strict Clippy, 62 tests, and the release build after the hotkey change.
- [x] Re-check `Ctrl+Alt+Space` and normal `Home` behavior in Notepad on the release build; the filter was visually verified on the debug build and is covered by tests.

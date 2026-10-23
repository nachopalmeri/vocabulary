# Manual testing

These checks cover behavior that unit tests cannot see. Run them against a release build on Windows 11, ideally at 100% and 125% display scaling.

Start clean:

```powershell
cargo build --release
.\target\release\worddock.exe
```

Close any older WordDock instance first. The app is single-instance, so a second launch only reveals the existing card.

## Card and tray

- Left-click the tray icon: the card hides and shows again without taking focus from the active window.
- Hover the card: the controls appear without moving the card. Move the pointer away and confirm they hide again.
- Next, previous, pause/resume, mark learned, and review again all update the card immediately.
- Drag the card to another monitor and restart the app; the position is restored.
- Closing the card leaves the tray app running.

## Global hotkeys

- Press `Ctrl+Shift+Space` while another app has focus: the card controls appear.
- Press `Ctrl+Alt+Space`: the Tendedero toggles.
- Open Notepad, type two lines, and press `Home`. The caret must move to the start of the line and the shelf must **not** open. This guards against a plain-key global hotkey swallowing normal typing keys.
- If a hotkey is unavailable, the tray menu and the top-edge trigger must still work.

## Tendedero clipboard shelf

- Copy a sentence, a screenshot, and one or more files. Each should appear as a distinct paper card.
- Screenshot thumbnails must keep the correct aspect ratio, with no black stripes or upside-down rows.
- Rest the pointer at the top edge: the shelf fades in without focus. Leave it: it fades out.
- Use `<` and `>` to page when more than three items exist.
- Click a text card, an image card, and a file card in turn; paste into Notepad, Paint, and Explorer to confirm the original format is restored.
- Right-click a card: copy, remove, and clear must behave as labeled.
- Delete a copied file before clicking its card; WordDock should report the missing reference without crashing.
- Restart after copying an image; the encrypted shelf item must still render.

## Collections and study tools

- Create, rename, and delete a collection.
- Add, edit, and delete entries; duplicate words are skipped case-insensitively.
- Import both sample formats from `samples\`; inspect the validation/duplicate preview and cancel once.
- Export CSV and JSON and re-import them.
- Select the 619-entry `ThinkingFastAndSlow` collection and filter for `wary`. The list should show two matches and the summary should read `Showing 2 of 619`.
- Clear the filter, then filter by part of a Spanish meaning and by part of an example.
- With a filter active, add, edit, and delete the selected entry; the operation must target the filtered row, not the original index.
- Mark entries learned and confirm the learned percentage changes. Delete the last matching entry and confirm the empty result state is stable.

## Details and pronunciation

- Open **Word details** from the tray or the card's **Show more**.
- The meaning, example, source, languages, and learned state must match the card.
- Click **Pronounce**: the word is spoken. Click twice quickly; the second utterance should replace the first instead of queueing.
- Mark learned/review again from the details window; the card must update.
- On a Windows install without an English voice, pronunciation must fall back to the default voice rather than showing an error.

## Settings and startup

- Change interval, order, theme, font size, card width, topmost, and learned filtering; the preview and card update.
- Change the active collection and confirm rotation switches to it.
- Enable launch-at-login, sign out and back in, and confirm WordDock starts. Disable it and confirm the current-user `Run` entry is removed.
- Lock and unlock the session; rotation must pause while locked and resume afterwards.
- Test dark and light themes; standard push buttons keep the system theme, while the card and shelf use custom colors.

## Persistence and recovery

Use a disposable data directory, not your real one:

1. Close WordDock.
2. Rename `%LOCALAPPDATA%\WordDock` to `WordDock.backup`.
3. Start WordDock, let it create fresh data, then close it.
4. Corrupt `library.json` with invalid JSON.
5. Start WordDock: it must quarantine the bad file under `recovery\`, restore the last backup if possible, and explain what happened.
6. Restore `WordDock.backup` to `%LOCALAPPDATA%\WordDock` when finished.

Also confirm that a `schema_version` newer than the app supports stops startup without overwriting the file.

## Idle measurement

1. Start the release build and leave the card visible without interacting with it.
2. In another PowerShell window:

   ```powershell
   powershell -ExecutionPolicy Bypass -File .\scripts\measure-worddock.ps1 -Seconds 60
   ```

3. Record Private Bytes, working set, normalized CPU percentage, handles, and deltas. CPU is process CPU time divided by elapsed wall time and logical processor count.

This is a measurement procedure, not a performance guarantee. Compare results on the same machine and workload.

# Changelog

## 1.0.0 — first stable release

**Tell your Mac what to do with your files, in plain English. Review exactly what will happen. Undo anything.**

### Highlights
- **Chat that actually does things.** Create, move, rename, delete, sort, de-duplicate and change extensions, with fuzzy folder names, lists (“delete A, B and C”) and follow-ups (“delete all of them”).
- **Several tasks in one message — as many as you like.** “Delete A and B. Then rename movie.mp4 to Film.mp4 and move it into Archive.” Tidy splits your message into tasks and shows **one plan** listing all of them; anything it can't understand is named under *Not included* instead of being silently dropped.
- **Two-step jobs in one go.** “Rename X to Y and move it into Z”, “move X into Z and rename it”, or “create a folder Z and move X into it” are a single reviewed step for files and whole folders.
- **Names understood the way people say them.** “Life and hell” is one name, not two; each name can be a folder *or* a file; a single typo is forgiven (“corssover” → Crossover); a name with an extension (`movie.mp4`) is always a file; loose guesses are flagged for you to uncheck.
- **A disk explorer that tells the truth.** Every folder with its real allocated size, cached so it opens instantly and refreshed in the background.
- **History with Put back.** Every change is journaled; undo a batch of moves or put a trashed folder back exactly where it was.
- **Bring your own model (optional).** An instant built-in engine handles everyday requests; a local model handles unusual ones. Add any Hugging Face `.gguf`, pinned by checksum.
- **Small always-on-top assistant** window, plus light and dark themes.

### Safe by design
- Nothing changes until you approve a plan that lists the exact files, folders and sizes.
- You choose which folders Tidy can see; everything else is invisible to it.
- No overwrites and no permanent deletion, anywhere: removal is the native Trash with its recovery location recorded.
- Fully local: no account, no cloud, no telemetry. The only network use is you pressing *Download model*.

### Also in 1.0
- Read-only folders can be trashed or moved (permissions are restored, also on Put back).
- The chat remembers the last listing (“only the Rust ones”, “biggest first”, “delete them”).
- *Folders Tidy knows about* with **Forget**, a Stop button for scans, native **Add folder**.
- Repositories, `/Applications` and safe parts of `~/Library` can be managed; `.git` internals stay untouchable.
- 150+ automated tests, including end-to-end scenarios that run the real safety engine on real files; CI checks versions, secrets, formatting, lints and tests on every push.

### Fixed
- Dates in “organize by date” were wrong for many timestamps.
- Case-only renames on case-insensitive volumes; “find …” being mistaken for a folder listing.
- “Move the Old Stuff folder” was read as an age filter (“old files”).

Earlier history (0.5–0.8) lives in git.

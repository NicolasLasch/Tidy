# Changelog

## 1.0.0 — first stable release
- **Name lists resolve properly:** in “delete A, life and hell, beer and plunder and B”, “and” may separate items or sit inside a name, so Tidy tries every split and keeps the one where the most names resolve; each name can be a folder **or a file** (`beer-and-plunder-mac-universal.zip`), weak guesses are flagged for review, and names Tidy can't find are listed.
- **Unlimited tasks per message:** split on sentences, “then”, “after that” and “and” + a new verb (“delete A and rename B to C and move D into E”); “rename X to Y and move **it** into Z” still stays one two-step job.
- **Several tasks in one message:** “delete A, B and C. Then rename movie.mp4 to Film.mp4” becomes one plan listing every task; a task Tidy can't understand is named instead of silently dropped. Trash steps are approved first, then the remaining steps follow in the chat. Names with an extension (`movie.mp4`) are always treated as files and never fuzzy-matched to a folder, and a single typo in a folder name (“corssover” → Crossover) is forgiven when nothing else matches.
- **Multi-step requests:** “rename X to Y and move it into Z” (or “move X into Z and rename it to Y”) and “create a folder Z and move X into it” now work as one reviewed plan with a single approval, for files and whole folders. Nothing is overwritten, and Put back restores the original name and place in one step. A folder named explicitly (“the Old Stuff folder”) is no longer mistaken for an age filter when moving.
- **Read-only folders can be trashed or moved:** Tidy makes the approved folder writable just for the move and restores its permissions (also on Put back); partial failures say what already ran and drop finished items from the plan.
- **Chat remembers the last listing:** “how come some have no size?”, “measure them”, “only the Rust ones”, “biggest first”, “delete them”. Project sizes missing from the index are read from disk.
- Names prefer a project over a same-named folder deep inside its build output (“chef mod”).
- Plain-language explanations when a folder can't be managed; `/Users/Shared`, `/usr/local` and more `~/Library` subfolders are manageable; the Data-volume path is normalized everywhere.
- **Repository & pipeline cleanup:** removed the legacy JSON-planning agent, unused commands and old design docs; CI now checks versions, secrets, fmt, clippy and tests on macOS and builds the full bundle; releases are gated on the same checks and publish a DMG, a zip and SHA-256 checksums with build provenance.
- **Settings:** *Folders Tidy knows about* with **Forget** (purges the saved index, never touches files); Stop button for scans; native **Add folder**.
- Whole-disk **Storage** explorer (real allocated sizes, cached and refreshed in the background), files and folders in one place.
- Request engine: create / move / rename folders and files, bulk renames, extension changes, duplicate removal, launcher-instance filtering by game version, projects list, folder cards.
- **History** with Put back from the Trash; case-only renames; whole-folder Trash and moves with tree fingerprints.
- Model picker in the chat and “add your own model” from Hugging Face (pinned by commit + SHA-256).
- Small always-on-top assistant window; light/dark design with the Tidy logo.
- Fixed: calendar math in “organize by date” (dates were wrong for many timestamps), case-only renames on case-insensitive volumes, “find …” being mistaken for a folder listing.
- Large-scale end-to-end tests (1,000 messy files) and benchmarks (1k/10k/50k).
- Repositories, `/Applications` and safe parts of `~/Library` can now be managed; `.git` internals stay untouchable.

Earlier history (0.5–0.8) lives in git.

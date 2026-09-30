# Architecture

Tidy is a **Tauri 2** desktop app: a Rust backend that owns every file operation, and a React/TypeScript front end that renders and asks. A small **llama.cpp worker** runs optional local AI.

```
┌───────────── React + TypeScript UI ─────────────┐
│ Chat · Storage (disk explorer) · History · AI   │  ← never touches files
└──────────────┬──────────────────────────────────┘
               │ Tauri IPC (typed commands; each re-checks scope + state)
┌──────────────▼──────────────────────────────────┐
│ apps/desktop/src-tauri                          │
│  planning.rs  request → Investigation           │
│  safety_ipc.rs approval / execute / undo / restore
│  disk.rs      whole-disk sizes, cached to disk  │
│  selection.rs which folders the assistant may see
└──────┬───────────┬───────────┬──────────────────┘
       │           │           │
 agent-runtime  file-indexer  safety ─────────► the ONLY code that mutates files
 (intent.rs,    (scan, SQLite (validate → approve → journal
  model client)  + FTS index)  → execute → verify → undo/restore)
       │                          │
 organization / storage (pure)  platform (authorized roots, no-follow handles)
```

## The request path

1. **Understand.** `agent-runtime/src/intent.rs` parses everyday phrasing deterministically: folder names (fuzzy, comma/“and” lists, launcher synonyms), file criteria (type, age, size, name, “biggest N”, “except …”), structural changes (create/move/rename), bulk renames, extension changes, duplicates, projects, listings. It returns an `Investigation`: proposed *actions* (typed, by index ID or path), folder targets, structured *sections* for cards, and an explanation. It never asks unnecessary questions — ambiguity is resolved sensibly and disclosed.
2. **Optional AI fallback** when the engine finds nothing: (a) a local model restates the request as a plain command using real folder names, which goes back through the same engine; (b) for folder-building requests, the model emits *matching rules* (validated JSON grammar) that are applied to every indexed file; (c) for questions, the model answers from index statistics and the closest matches. See [HOW-AI-IS-USED.md](HOW-AI-IS-USED.md).
3. **Review.** The UI shows exactly what will change (checkboxes, sizes, versions). Nothing has happened yet.
4. **Approve.** `safety` validates every action against the *live* filesystem, fingerprints sources (file: dev/inode/size/times; folder: identity + entry count + bytes), journals a prepared transaction and issues a one-use, 5-minute token.
5. **Execute.** Each step re-checks its fingerprint, then uses handle-relative, no-follow, no-replace operations (or the native Trash), records evidence, and verifies the result.
6. **History.** Undo (moves, renames, folder moves) and **Put back** (from the recorded Trash location, never overwriting) are journaled transactions too.

## Safety model

* Scope: a folder must be authorized (native picker or a click) *and* switched on for the assistant. Protected areas (system roots, `.ssh`, `.gnupg`, Trash, most of `~/Library`, `.git` internals) can never be authorized or touched.
* Path safety: `openat`-style traversal with `O_NOFOLLOW`, device checks (no crossing mounts), `RENAME_EXCL` (no replace), case-only rename handling for case-insensitive volumes.
* Approval binds exact actions; a folder that changes after review (new file, different size) refuses to execute.
* No permanent delete exists in the code base. Removal = `NSFileManager trashItemAtURL`, with the resulting location stored for Put back.
* The UI, the request engine and the model hold **no executor**. They can only produce proposals.

## Data

* SQLite (WAL) index of authorized folders: metadata, optional bounded text excerpts (FTS5), optional SHA-256 for duplicates. Stored in the app data directory; “Forget folder” purges it.
* Safety journal (SQLite): transactions, steps, evidence (before/after stamps, Trash path), states, recovery on startup.
* `disk_cache.json`: last known sizes for the disk explorer (allocated bytes via `st_blocks`), refreshed in the background.
* `ai_selection.json`, `ai_models.json`: which folders the assistant may see; chosen and custom models.

## Whole-disk explorer

`disk.rs` walks the Data volume on worker threads (allocated bytes like Finder, hard links counted once, no symlink following, no other volumes), streams results while scanning, caches every listing up to three levels deep, and persists the cache so the next visit is instant while a background pass refreshes only what changed. Folders macOS blocks are reported as *Protected* with a shortcut to grant Full Disk Access.

## Testing strategy

* Unit tests beside the code (safety refusals, parsers, tree fingerprints, calendar math…).
* Contract tests (`tests/*.rs`) for scanner, storage analysis, organization and safety journeys.
* End-to-end scenarios on mock trees: `tests/e2e_mock_folder.rs` (small) and `tests/e2e_large_scale.rs` (~1,000 messy files: organize, rename, extensions, de-duplicate, undo, Put back).
* Benchmarks: `crates/agent-runtime/examples/benchmark.rs` → [BENCHMARKS.md](BENCHMARKS.md).


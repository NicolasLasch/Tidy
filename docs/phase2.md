# Phase 2 delivery and verification

## What works

A self-contained macOS Tauri application with a native folder picker, multiple remembered folders, scan/cancel, filename and optional content search, size/name sorting, 100-row pagination, excerpt details, exclusions, and explicit index removal. It remains useful without a model. The React UI and Rust core require no Python and use no external APIs or remote assets at runtime.

The application offers no source-file modification operation. Forgetting a scope modifies only Tidy's own cache. Read-only here means the app does not move, rename or delete source files; an OS may still update access metadata on reads.

## Scanning and confinement

The desktop's Unix scanner holds directory descriptors, inspects entries relative to them, and uses `openat` with `O_NOFOLLOW`. It opens the selected root component by component and compares the recorded device/inode. Child directory/file opens compare expected identities and refuse device changes. Directory links, app bundles, protected locations, Git repositories (including worktree markers), and special files are excluded. macOS `SF_DATALESS` placeholders are excluded before opening content. This flag is defined in the installed macOS SDK's `sys/stat.h`.

Content is opt-in, strict UTF-8 plain text. Changed files are rejected if identity, size, nanosecond modification or change timestamps differ before/after reading. Hashing happens after metadata grouping by size, through newly validated root/descendant handles; identical hashes are evidence for later storage analysis, not permission to delete. Hard links retain separate path rows with a shared identity.

The scan has one worker, a depth limit and entry/time budgets. Content excerpts have an independent memory/read cap; hashes have per-file and total-read caps. Polling reports metadata entry progress. A blocking OS call cannot be forcibly interrupted; cancellation is cooperative. Policy omissions are visible. Git exclusion currently marks coverage partial conservatively, even if every other reachable directory was enumerated.

This is not a mutation-ready filesystem security proof. Same-device bind mounts, concurrent changes in ancestor repository policy, adversarial cloud-provider state changes, and directories renamed while descriptors are held need further platform work before execution. The existing CLI intentionally remains metadata-only and path-based. Windows content indexing is disabled; Windows metadata and root identity policies remain less strong than Unix.

## Persistence and incremental semantics

SQLite schema version 1, WAL, foreign keys, full synchronous transactions, FTS5 and an 8 MiB page cache. Unknown future schemas are rejected. The DB and its sidecar files live in owner-only app-data directory. All content and query strings remain local. No automatic scans on launch; users can search their saved snapshot offline.

Each committed scan increments a generation atomically. Known paths are upserted. A unique identity at a new path can retain its row ID when its previous path is absent; hard-link ambiguity avoids that shortcut. Complete scans remove unseen metadata rows. Partial scans retain old metadata for reconciliation but hide it from current results. Old content/hashes are cleared so previously indexed text does not remain searchable through a newly excluded subtree. Cached content is reused only when the fingerprint matches. Search results are snapshots, not live existence guarantees.

A scan result cannot recreate a forgotten scope: database foreign keys and scope existence checks reject late commits. Job completion and revocation share lock ordering. Forget performs a logical purge, FTS optimization and WAL checkpoint. This is not a claim of forensic erasure from SSDs, filesystem snapshots or backups.

Current reconciliation is on-demand; automatic filesystem watchers are deferred. A new scan still enumerates metadata to detect changes. Partial snapshots do not silently claim whole-folder completeness.

## Local verification

- Rust core: previous 16 tests plus Phase 2 contract tests for persistent reopen/search, literal FTS inputs, partial-scan retention, rename identity, hard links, revoked scopes/late commits, schema version handling, bounded pages/queries, malformed text, content opt-in/hash grouping, excluded apps/links/repositories, replaced files/roots, parent traversal, cancellation/limits, changed content, replaced child directories and oversized text.
- `cargo test --workspace --locked --offline`, formatting and strict Clippy run on this Apple silicon Mac. Frontend TypeScript and Vite production build run locally.
- Native smoke test: packaged app opens; native folder selection leads to indexed fixture rows; app-bundle exclusion is applied. Downloads also displayed 61,592 files, 93 exclusions and 15.5 seconds in the live app. This is a single observed run, not an 8 GB performance benchmark.
- Browser and packaged UI make no application network requests by design; testing with the physical network disabled has not been performed. Installed dependencies are sufficient for offline builds. Full inference benchmarks belong to Phase 3/7.
- Linux/Windows core CI and macOS app build are configured, not claimed executed remotely.

Development bundle is arm64 macOS, unsigned/unnotarized for local use. Distribution signing, universal/x64 builds and Windows packaging remain Phase 7. No LLM benchmark results or undo functionality are claimed in this phase.

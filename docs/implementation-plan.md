# Incremental implementation plan

Each phase ends with runnable code and its own acceptance checks. Phases 1–5 are implemented for macOS; see docs/phase2.md, docs/phase3.md, docs/phase4.md, and docs/phase5.md for verified scope and platform limits.

1. **Architecture and foundation (complete):** architecture/risk review, module contracts, six-crate workspace, explicit read grants, bounded metadata scanner and CLI, portable test layout. No execution or inferred authorization. Acceptance: offline Rust build/test, strict Clippy and scanner boundary tests.
2. **Persistent scanning and desktop scaffold (macOS implemented):** Tauri 2, React, TypeScript, Tailwind, native folder picker and read-only results; Tokio workers; SQLite migrations and incremental reconciliation. Implement handle-based containment and mount/placeholder policy before bounded text extraction; plain text first, sandbox document extractors later. Staged hashing, hard-link identity, cancellation and resource budgets. Acceptance: restart, partial scan, rename reconciliation, revoked scope, malformed text and race tests.
3. **Local inference (macOS implemented):** pin llama.cpp, native worker lifecycle, explicit model installation with license/checksum/size, Q4 baseline and CPU/Metal paths. Network disabled after install; handle missing/corrupt model and worker crashes. Acceptance: inference smoke tests offline, timeout, context and RAM bounds. No proposals executed.
4. **Planning and storage (complete):** deterministic filename/category/date/project proposals, large-file and installer/artifact evidence, exact duplicate confirmation. Add bounded read-only agent tools and strict JSON schema. Acceptance: adversarial text and malformed output rejected; deterministic fallback covers all journeys.
5. **Safety (complete):** validation and preview, one-use approval, durable journal, no-replace moves/renames, native Trash support, verification, startup recovery and explicitly approved undo. No permanent deletion, overwrite or cross-volume move. Acceptance: full fault-injection matrix before enabling any UI execution control.
6. **Floating UI and journey integration:** accessible review, per-action selection, approval, progress, omissions, errors and history/undo. Model-unavailable status with useful rules and search. No inference chat box substitutes for the three workflows.
7. **Benchmarking and packaging:** execute models/README.md protocol, tune measured budgets, release builds on macOS first and Windows second; installer, signing/notarization process, dependency/license inventory and offline smoke tests. No claims of measured support until tested on target hardware. Desktop icon repositioning remains post-MVP.

## Complete journey acceptance criteria (Phases 4–6)

**Organize Downloads:** user selects Downloads -> bounded authorized scan -> category/date/project rule choice -> proposed destinations with evidence -> user selects exact moves -> safety preview and explicit approval -> revalidation/execution -> verified result and approved undo. Missing model uses deterministic extension/date rules; ambiguous files remain unassigned.

**Recover storage:** user selects folders -> partial/completed scan clearly labeled -> large files, exact duplicates, old installers and development artifacts with evidence -> user keeps at least one chosen duplicate -> preview individual Trash actions -> approval -> native Trash with recovery metadata -> verified results. Aging thresholds are user-visible heuristics; Git content remains excluded. No automatic emptying of Trash; display potential savings separately from actually freed bytes.

**Group a project:** user enters project name and selects folders -> filename/metadata and bounded-text search -> evidence-backed candidate list -> user corrects membership -> relative grouping proposal -> approval -> verified reversible moves. Existing Git repositories are excluded and explained. Missing model still supports literal filename search and manual membership; no embedding or vision requirement.

## Current boundary

Phases 1–5 are complete with full integration testing, safety guarantees, atomic executions, durable SQLite transaction journal, one-use approval tokens, native OS Trash integration, and history/undo UI. Phase 6 refines floating notifications and journey integrations. Windows hardening, signing/notarization and release packaging remain later work.

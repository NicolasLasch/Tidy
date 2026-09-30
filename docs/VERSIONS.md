# Tidy Versions & Evolutionary Roadmap (V1 – V7)

This document tracks the precise evolutionary roadmap of **Tidy**, from initial architectural primitives (V1) to the distributed, multi-package desktop application (V7).

Each milestone introduces concrete, isolated workspace packages and strict safety boundaries.

---

## Evolution Summary Matrix

| Version / Milestone | Core Packages & Crates | Primary Capabilities | Verification & Contract | Status |
| :--- | :--- | :--- | :--- | :--- |
| **V1** | `tidy-platform`, `tidy-file-indexer` | 6-crate workspace, explicit read grants, bounded metadata scanner, CLI (`tidy-scan`) | `tests/scan_contract.rs` | Complete |
| **V2** | `tidy-desktop`, `tidy-file-indexer` (SQLite) | Tauri 2 desktop app, SQLite WAL indexer, handle-based traversal, bounded plain-text & SHA-256 | `tests/phase2_contract.rs` | Complete |
| **V3** | `native/inference`, `tidy-agent-runtime` | Pinned llama.cpp worker, Metal/CPU offline inference, verified model catalog, zero-network runtime | `scripts/smoke-inference.mjs` | Complete |
| **V4** | `tidy-organization`, `tidy-storage` | Deterministic rules (category/date/project), duplicate detection, storage cleanup analyzer | `tests/phase4_contract.rs` | Complete |
| **V5** | `tidy-safety` | One-use approval tokens, durable SQLite transaction journal, native OS Trash, atomic undo | `tests/phase5_contract.rs` | Complete |
| **V6** | `tidy-agent-runtime`, `tidy-desktop` | Unified Ask Tidy chat, 39 workflow skills, live investigation timeline, instant exact fast-paths | `tests/phase4_contract.rs` | Complete |
| **V7** | Full Workspace & GitHub CI/CD | Self-contained `.app` packaging, cross-platform CI matrix, automated CD release pipeline | `.github/workflows/` | Complete |

---

## Detailed Version Breakdown

### Version 1 (V1) — Architecture & Core Foundation
- **Milestone Focus**: Establishing foundational contracts, memory safety guarantees, and zero-privilege execution.
- **Workspace Packages**:
  - `crates/platform`: Platform abstractions, OS error diagnostics, device & inode identity tracking.
  - `crates/file-indexer` (CLI binary `tidy-scan`): Explicit read grants (`AuthorizedRoot`), path canonicalization, budget-bounded recursive metadata traversal.
- **Core Guarantees**:
  - Read-only: zero mutation or deletion capabilities.
  - Strict containment: refuses symlinks pointing outside the granted boundary, skips Git internals, junctions, and device mount transitions.
  - Bounded resource consumption: maximum 10,000 entries and 30-second budget in CLI mode.
- **Verification**:
  ```sh
  cargo test --test scan_contract
  cargo run --bin tidy-scan -- --root "$HOME/Downloads"
  ```

---

### Version 2 (V2) — Persistent Scanning & Desktop Scaffold
- **Milestone Focus**: Persistent local indexing and modern native desktop presentation.
- **Workspace Packages**:
  - `apps/desktop`: Tauri 2 host, React 19, TypeScript, and Tailwind CSS.
  - `crates/file-indexer`: SQLite integration with Write-Ahead Logging (WAL) and automated migrations.
- **Core Guarantees**:
  - Handle-based containment on Unix (`openat`, `O_NOFOLLOW`) preventing TOCTOU file-swap attacks.
  - Incremental metadata reconciliation: tracks file identity across renames using inode/device pairs.
  - Bounded plain-text extraction: strict UTF-8 validation (rejects NUL bytes), limited to 64 KiB/file and 8 MiB/scan.
  - Candidate deduplication hashing: staged SHA-256 computation exclusively for files of identical byte size.
- **Verification**:
  ```sh
  cargo test --test phase2_contract
  cd apps/desktop && npm run build
  ```

---

### Version 3 (V3) — Local Offline Inference Engine
- **Milestone Focus**: Completely private, local-first artificial intelligence running directly on user hardware.
- **Workspace Packages**:
  - `native/inference`: Pinned `llama.cpp` C++ runtime compiled via CMake with Metal GPU and CPU backends.
  - `crates/agent-runtime`: Child-process supervisor managing worker lifecycle, stdin/stdout IPC, and budget guards.
- **Core Guarantees**:
  - Zero network telemetry: network access is prohibited during inference; model downloads are authenticated by cryptographic SHA-256 hashes.
  - Fault tolerance: worker crashes, timeouts, and truncated output fail closed without destabilizing the desktop app.
  - Memory bounds: enforces context window limits and execution timeouts (90 seconds).
- **Verification**:
  ```sh
  npm run worker:build --prefix apps/desktop
  node scripts/smoke-inference.mjs
  ```

---

### Version 4 (V4) — Planning & Storage Intelligence
- **Milestone Focus**: Intelligent organization strategies and disk reclamation heuristics.
- **Workspace Packages**:
  - `crates/organization`: Deterministic rule engines for Categorization (file extensions), Civil Date (`YYYY/MM`), and Project grouping.
  - `crates/storage`: Heuristic analyzers detecting stale installers, build artifacts (`node_modules`, `target`, `dist`), and exact duplicates.
  - `crates/agent-runtime`: Bounded read-only agent inspection tools with strict JSON schemas.
- **Core Guarantees**:
  - Model proposals cannot execute directly; all suggestions are strictly data proposals.
  - Preserves Git repositories and system bundles from relocation.
  - In absence of a local AI model, deterministic rule fallbacks guarantee complete operational parity.
- **Verification**:
  ```sh
  cargo test --test phase4_contract
  cargo test -p tidy-organization
  cargo test -p tidy-storage
  ```

---

### Version 5 (V5) — Safety System, Journaling & Reversibility
- **Milestone Focus**: Fail-safe operational execution and verifiable reversibility.
- **Workspace Packages**:
  - `crates/safety`: The centralized execution authority for all file mutations.
- **Core Guarantees**:
  - Three-stage gate: Validation -> Impact Preview -> Single-Use Cryptographic Approval Token.
  - Durable Transaction Journal: Every step is recorded in SQLite before execution begins; recovers automatically on startup after crashes.
  - Non-destructive execution:
    - Never permanently deletes files: moves files to the native OS Trash (`trash-rs`).
    - Collision rejection: never overwrites existing target files.
    - Preserves Unix permissions and verifies destination byte sizes.
  - Approved Undo: Any executed transaction can be safely reverted with full verification.
- **Verification**:
  ```sh
  cargo test --test phase5_contract
  cargo test -p tidy-safety
  ```

---

### Version 6 (V6) — Conversational Assistant & Guided Workflows
- **Milestone Focus**: Seamless human-in-the-loop interaction and transparent agent reasoning.
- **Workspace Packages**:
  - `crates/agent-runtime`: Multi-turn conversational session manager and skill router.
  - `skills/`: Catalog of 39 domain-specific workflow skills guiding the agent's investigation.
  - `apps/desktop`: Single-pane Ask Tidy chat interface with embedded proposal previews and approval controls.
- **Core Guarantees**:
  - Live investigation timeline: displays real-time tool execution (searches, inspections) rather than black-box AI reasoning.
  - Instant non-AI shortcuts: common batch actions (e.g. extension changes, plain-text cleanup) execute via verified deterministic code paths without loading model weights.
  - Conversational context remains strictly local and ephemeral (not stored across restarts).
- **Verification**:
  ```sh
  cargo test -p tidy-agent-runtime
  cd apps/desktop && npm run build
  ```

---

### Version 7 (V7) — Benchmarking, Packaging & Continuous Distribution
- **Milestone Focus**: Release engineering, cross-platform compilation, and automated packaging.
- **Workspace Packages**:
  - Full Cargo workspace (`crates/*`, `apps/desktop/src-tauri`).
  - `.github/workflows/ci.yml`: Automated CI matrix on macOS, Windows, and Ubuntu.
  - `.github/workflows/cd.yml`: Continuous Delivery pipeline building self-contained application bundles and publishing GitHub Releases.
- **Core Guarantees**:
  - Self-contained packaging: `Tidy.app` bundles the pinned native worker and all UI assets into an offline-ready bundle.
  - Strict CI enforcement: zero Clippy warnings (`-D warnings`), 100% formatted code (`cargo fmt --check`), and all unit & contract tests pass offline.
- **Verification & Packaging**:
  ```sh
  # Development bundle
  cd apps/desktop && npm run tauri -- build --debug --bundles app
  # Release package
  cd apps/desktop && npm run package
  ```

---

## Package Structure

```
Tidy/
├── .github/workflows/       # CI/CD pipelines (V7)
│   ├── ci.yml               # Automated build & test across macOS, Windows, Ubuntu
│   └── cd.yml               # Release bundle packager and GitHub release publisher
├── apps/
│   └── desktop/             # Tauri 2 + React 19 desktop GUI application (V2, V6, V7)
├── crates/
│   ├── platform/            # OS primitives & identity tracking (V1)
│   ├── file-indexer/        # Scoped scanner & SQLite WAL indexer (V1, V2)
│   ├── agent-runtime/       # LLM inference & conversational planner (V3, V6)
│   ├── organization/        # Rule-based organization engine (V4)
│   ├── storage/             # Space reclamation & duplicate analyzer (V4)
│   └── safety/              # Transaction journal, OS Trash & reversibility (V5)
├── native/
│   └── inference/           # C++ worker wrapping pinned llama.cpp (V3)
├── models/                  # Curated offline model manifests & benchmark protocol (V3, V7)
├── skills/                  # 39 guided workflow skills (V6)
└── tests/                   # Contract tests for all verification gates (V1–V5)
```

# Phase 4: planning and storage analysis

TIDY now provides deterministic planning and storage recovery analysis across authorized file indices. The planning agent uses bounded read-only tools and strict schema validation, while deterministic algorithms guarantee full functionality across all three user journeys even when the local AI model is unavailable. No autonomous file modifications or execution occur in this phase.

## Architecture and boundary

* **No execution authority:** Neither `tidy-organization` nor `tidy-agent-runtime` holds filesystem mutation references or shell access. Proposals use opaque `FileId` index references and normalized relative destinations (`destination_relative`). Execution, durable journaling, approvals, and undo remain strictly deferred to Phase 5.
* **Deterministic planning (`tidy-organization`):**
  - **Category mode:** Groups files into standard folders (`Documents`, `Images`, `Audio`, `Video`, `Archives`, `Code`, `Spreadsheets`, `Presentations`, `Design`) based on lowercase extensions. Files with unknown or missing extensions remain unassigned. Existing files at destination and potential destination collisions are detected to prevent collisions.
  - **Date mode:** Organizes files by modification timestamp into `YYYY/MM` directory hierarchies using pure civil calendar math. Files with missing or invalid timestamps remain unassigned.
  - **Project mode:** Groups files matching a project name by filename, directory name, or saved text excerpts into `<ProjectName>/`. Files inside Git repositories (`.git` markers) are strictly excluded and protected.
  - **Storage cleanup proposals:** Redundant copies of exact duplicates (preserving at least one user-chosen copy), old installers, and development artifacts are proposed as `ProposedAction::Trash { source: FileId }`. Large files are surfaced for individual review and never automatically proposed for Trash.
* **Storage analyzer (`tidy-storage`):**
  - **Large files:** Files exceeding configurable thresholds (default: 50 MiB) sorted descending by size.
  - **Exact duplicates:** Candidates grouped by SHA-256 hash. Inode identities are inspected: hard links sharing the same inode report 0 physical reclaimable space with clear explanations, while distinct physical files calculate potential savings as `size * (distinct_copies - 1)`.
  - **Old installers:** Identifies package files (`.dmg`, `.pkg`, `.iso`, `.exe`, `.msi`, `.deb`, `.rpm`) modified past an aging threshold (default: 30 days).
  - **Development artifacts:** Identifies build and dependency directories (`target`, `node_modules`, `dist`, `.gradle`, `__pycache__`, `.next`, etc.) and artifact extensions (`.pyc`, `.o`, `.class`, `.pdb`), respecting Git exclusions.
  - **Reclaim accounting:** Displays potential savings distinctly from physically freed bytes.

## Bounded agent tools and schema validation (`tidy-agent-runtime`)

* **Bounded read-only tools:**
  - `Search { query, limit }`: Query capped at 256 UTF-8 bytes; limit capped at 50 items.
  - `Metadata { file_id }`: Lookups restricted to authorized scope records.
  - `TextExcerpt { file_id, max_bytes }`: Excerpts bounded to at most 4,096 bytes.
  - `StorageFindings { limit }`: Findings capped at 50 items.
  - No write, execute, delete, or shell tools exist in the tool enum.
* **Strict proposal schema:**
  - `ModelPlanProposal` enforces `version: 1` and `#[serde(deny_unknown_fields)]`.
  - Any model output attempting to inject unauthorized keys (e.g. `"shell"`, `"command"`, `"delete"`, `"sudo"`) is rejected at deserialization.
  - Bounded action count (1–100).
  - `source_file_id` must match a verified file ID in the authorized scope; unknown/hallucinated IDs are rejected.
  - `destination_relative` must be a clean relative path; absolute paths (`/etc/passwd`, `C:\Windows`), parent directory traversals (`..`), null bytes, and paths targeting `.git` are rejected.
  - Destination collisions within the proposal are rejected.
* **Deterministic fallback:**
  - If the local model is unavailable, times out, crashes, or produces malformed/invalid JSON, the system safely falls back to deterministic planning. The user experience remains uninterrupted.

## Desktop IPC integration (`apps/desktop`)

Exposes three new Tauri commands in `src-tauri/src/planning.rs`:
- `analyze_storage_scope(scope_id)`: Runs storage analyzer on current index generation, returning structured findings and reclaim summary.
- `propose_organization(scope_id, mode, project_name, use_ai)`: Generates organization proposal (attempting local inference with timeout/validation if `use_ai` is enabled, falling back to deterministic planning).
- `propose_storage_cleanup(scope_id, keep_duplicate_ids)`: Generates trash proposal for redundant duplicates, old installers, and development artifacts.

## Acceptance and verification

- **Contract tests (`tests/phase4_contract.rs`):**
  - Journey 1 (Downloads): Category and Date mode proposals verified; ambiguous files remain unassigned; zero source file mutation.
  - Journey 2 (Storage Recovery): Large files, exact duplicate confirmation, hard link zero-reclaim accounting, old installers, and development artifacts verified.
  - Journey 3 (Group Project): Project name matching with text excerpts and Git repository exclusion verified.
  - Agent tools and adversarial rejection: Rejection of prompt injection, path traversal, absolute paths, and hallucinated IDs verified.
- **Test suite:** 75 tests passing across the workspace (49 unit/contract tests across `tidy-storage`, `tidy-organization`, `tidy-agent-runtime`, `tidy-file-indexer`, `tidy-platform`, `tidy-desktop`, plus 26 existing contract tests).
- **Code quality:** `cargo clippy --workspace --all-targets -- -D warnings` passed with 0 warnings. `cargo fmt --check` passed cleanly.

## Next

Phase 5: Deterministic safety engine (validation, preview, one-use approval tokens, durable journal, no-replace atomic moves/renames, native Trash support, verification, startup crash recovery, and reversible undo).

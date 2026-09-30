# Architecture review and decisions

Status: Phase 2 adds the macOS desktop, SQLite persistence and handle-based read-only scanning. See docs/phase2.md for current implementation and remaining gates. The design below retains the original risk decisions; future mutation/inference interfaces remain contracts.

## Simplest architecture

One Tauri 2 desktop process owns authorization, SQLite, workers and safety. React/TypeScript/Tailwind renders a floating window and review views. A single optional, pinned llama.cpp worker handles local GGUF inference through a fixed, application-owned process interface; prefer pipes to a listening server. No shell intermediary, caller-provided executable or model-provided process arguments. If pipes prove impractical, evaluate authenticated loopback with no remote binding before implementation. Metal first on macOS; CPU fallback; Windows GPU backend selected after measurements. Do not introduce a vector database, background daemon, mandatory vision model or Python runtime.

Tokio coordinates cancellation and bounded queues; blocking scans and SQLite access use dedicated bounded workers. One SQLite writer with WAL, short transactions and schema migrations. Index metadata first; SQLite FTS5 over bounded text and filenames second; staged SHA-256 hashes only when useful for duplicate confirmation. Filesystem watchers are hints followed by reconciliation. A partial scan must never tombstone unseen rows. Removal of a scope revokes tools and purges its index/text on explicit user request. Store the DB in private application data; do not send names, content, telemetry or crash attachments remotely.

Read-only data path: user-selected scope -> platform grant -> indexer -> SQLite -> bounded retrieval -> deterministic or AI-assisted proposal -> review UI.

Modification path, only after Phase 5: selected proposal actions -> safety validation -> explicit user approval -> durable journal -> platform operation -> verification -> undo record. Neither agent-runtime nor organization holds an executor reference. Only safety can invoke platform mutation APIs. The webview receives no generic filesystem or shell capability. Every custom Rust command independently enforces scope and state; Tauri capabilities alone are insufficient domain authorization.

## Risk review

| Risk | Design decision / release gate |
| --- | --- |
| Path races, symlink swaps, junctions and mount changes | Phase 1 uses path checks only and reads metadata. Before content reads, use handle-relative no-follow traversal, retained root identity and mount/device checks. Before mutations, use OS no-replace semantics and verify identities at the operation boundary. Fail closed where guarantees are unavailable. |
| Prompt injection in file text or filenames | Treat all retrieved content as untrusted data, separate it from instructions, cap bytes, accept only a strict schema and a fixed read-tool enum. Never resolve model-supplied absolute paths. |
| Hallucinated or malformed plans | Reject unknown fields, IDs, actions, traversal, excessive lengths and budgets. Never repair into executable actions silently. Deterministic validation remains authoritative. |
| 8 GB memory pressure | Initial experimental budgets: 4K context, 512 output tokens, one request, bounded excerpts, 2 scan workers, 64 MB index queue, 32 MB SQLite cache. Target combined peak RSS below 3 GB; measure unified memory and GPU separately. These are targets, not benchmark results. |
| File changed since preview | Approval binds immutable action IDs, source identity, size, mtime, content hash where required, destination and plan digest. Recheck before each action; any change invalidates that action's approval. |
| Collision, case folding, normalization | Compare actual filesystem identity; never overwrite. Case-only rename requires a journaled intermediate step. Undo also refuses collisions and requests new user review. |
| Crash or partial batch | Flush prepared journal entry before each mutation; persist outcome and verification. Startup recovery inspects identities and source/destination, never blindly replays an operation. Partial results remain visible. |
| Trash and undo | Only OS Trash/Recycle Bin APIs, with restore identifiers persisted. If no supported reversible trash API exists, disable trash. Never fall back to unlink. External emptying of Trash can make restore impossible; report this honestly. |
| Cross-volume move | Initially reject. Later copy/verify/trash requires its own approval and recovery design; do not disguise copy/delete as rename. |
| Git and system damage | Default excludes entire repositories including .git files, symlink markers and repository ancestors. OS known-folder and mount policies must be hardened before execution. No broad full-disk access by default. |
| Misleading storage savings | Logical size is not allocated/reclaimable space. Hard links, clones, sparse files and Trash retention require separate accounting. Duplicate hashes do not automatically justify removing all copies. |
| Cloud placeholders / hidden network access | Before text reads, detect offline placeholders and avoid hydration. No runtime network clients, auto-update, model auto-download or remote asset loading after installation. Verify with networking disabled. |
| Permissions / unbounded trees | Report omissions and partial status, bound visited entries/depth/time; cancellation between OS calls. One stalled filesystem call can exceed the time budget; isolate it in a worker. |

## Module interfaces

| Module | Input -> output | Authority |
| --- | --- | --- |
| platform | selected path -> AuthorizedRoot; later native identity/open/Trash adapters | OS-specific boundary; no mutations implemented |
| file-indexer | AuthorizedRoot + ScanLimits + cancellation -> ScanReport | read metadata only in Phase 1 |
| organization | indexed evidence + Project/Category/Date mode -> Proposal | pure planning; FileId references, relative destinations |
| storage | authorized indexed snapshots -> Finding[] | evidence only; reclaim estimate may be unknown |
| agent-runtime | user goal + bounded ReadTool responses -> untrusted proposal | no executor, shell or write tool; model unavailable in Phase 1 |
| safety | proposal + current evidence -> validation; later approval -> journaled results/undo | sole future execution authority; lifecycle enums only today |
| apps/desktop | native folder choice + explicit intents -> views and Rust IPC | owns orchestration; model cannot create approval tokens |

Current Rust contracts reside in each crate's src/lib.rs. Do not expose FileId values from one scope/session in another. Future serialization uses versioned envelopes and deny-unknown-fields schemas. Paths stay lossless internally; UI displays escaped strings, while operations use stored native path bytes and opaque IDs.

Planned IPC: authorize_folder(native selection), scan(scope_id), cancel_scan(job_id), search(scope_id, query, limit), propose(scope_id, goal), preview(plan_id), approve(plan_id, digest, selected_action_ids), execute(approval_id), undo(operation_id). Execution and undo commands are not introduced until validated safety infrastructure exists. Approval is one-use, expires and binds exact selected actions. Undo is itself a reviewed modification.

Planned SQLite tables: scopes(id, native_path, root_identity, revoked), scans(id, scope_id, state), files(id, scope_id, native_relative_path, identity, size, mtime, hash, last_complete_generation), excerpts(file_id, extractor_version, text), proposals(id, version, digest, body), approvals(id, digest, selection, expiry, consumed), operations(id, approval_id, before, destination, state, restore_token, error). Unique constraints and transactions enforce state transitions; the filesystem and DB are not one atomic transaction.

## Upstream basis

- [Tauri runtime authority](https://v2.tauri.app/security/runtime-authority/) describes command/capability enforcement; domain checks stay in Rust.
- [llama.cpp](https://github.com/ggml-org/llama.cpp) supports GGUF and CPU/Metal and other acceleration backends. Pin a reviewed revision at integration time.
- [Qwen3 1.7B official GGUF](https://huggingface.co/Qwen/Qwen3-1.7B-GGUF) establishes the baseline family; the listed official quantization is Q8_0, so do not assume an official Q4 asset. Produce and checksum Q4_K_M with the pinned native quantizer or review a reproducible derivative.

These sources support the component choices, not TIDY performance or safety guarantees.

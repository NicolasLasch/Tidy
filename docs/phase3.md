# Phase 3: local inference

The desktop can install a local Qwen model explicitly and explain whole-index statistics and retrieved evidence from an authorized index. Scanning/search remain useful with no model. This phase does not generate executable plans or mutate source files.

## Runtime boundary

llama.cpp is pinned to b6500 commit `a7a98e0fffed794396b3fbad4dcdbbc184963645`. `scripts/build-worker.mjs` verifies the source archive SHA-256, builds a static C++ worker and writes its digest manifest. CMake disables curl, server/examples/tests, dynamic backends and OpenMP. The installed app does not compile anything. It invokes only its bundled worker with fixed arguments and an empty environment; model output cannot select a program, path, shell or tool. Frontend CSP blocks external connections. The Rust model installer is the only network client and runs only after an explicit install/repair action.

The one-request worker reads JSON on stdin and emits one validated JSON envelope. Native parser and Rust response parser enforce byte/token limits. GPU layers are 0 for CPU and 99 for Metal, with embedded Metal shaders. Both use four threads and greedy non-thinking Qwen ChatML. All model memory is released on process exit. On macOS the worker also monitors parent death, including abrupt desktop exit. Windows shutdown and packaging require platform validation.

An app job mutex prevents concurrent model loads/downloads. Cancel/90-second timeout kills and reaps the child. Stdin writing and bounded stdout draining are concurrent so a blocked input pipe cannot defeat the timeout. Child stderr is discarded by the desktop. Forgetting a scope cancels and discards its pending answer; completion checks authorization again. Model and worker hashes are verified before every inference. This is integrity checking, not a sandbox against another malicious process running as the same OS user replacing files between verification and open.

## Budgets and installation

* Models: pinned 1.28 GB 1.7B Q4_K_M and 0.43 GB 0.6B Q4_0 GGUF files, Apache-2.0. Explicit HTTPS download with a restricted redirect allowlist, exact size, GGUF header and SHA-256 verification. No remote inference. Startup never downloads.
* Installer: 64 KiB streaming buffer; 15-second connect/read waits; cancel checks between reads. Verified partial files publish by rename. Failure removes only app-created partial downloads; an existing final file survives failed replacement. A successful repair retains its previous file as an app-owned backup. Abrupt termination can leave a partial/backup in app data; automatic pruning is not implemented.
* Evidence (v0.3.1): streaming metadata aggregation and keyword retrieval across every row of the authorized scope’s current generation, using one SQLite read transaction. Exact totals and 12 extension groups plus remaining totals cover the whole index. Up to 16 distinct question terms search indexed paths and saved text; filename matches rank above text matches, then size and ID break ties. At most 20 examples enter the prompt, with relevant matches first and large files filling remaining slots. Path/excerpt lengths are bounded; the prompt builder remains capped at 9,000 UTF-8 bytes. Quoted evidence is untrusted data. Native control-token delimiters are neutralized; semantic prompt injection is still possible, but has no execution authority.
* Worker: 12,000-byte input prompt, 3,500 input tokens, 4,096 context, 384 output tokens, 16 KiB answer and 32 KiB response envelope. At most 90 seconds after launch; model checksum time precedes this and supports cancellation.
* Memory: fixed catalog/context limits, no hard OS RAM ceiling. An 8 GB deployment claim requires measurement on actual target hardware. The current host has 16 GB.

## Development and checks

Run `npm run worker:build` in `apps/desktop` before packaging. Requires CMake, C++17 toolchain and initial access to the pinned GitHub source archive. Cached builds work offline. Bundled notices cover llama.cpp/ggml and nlohmann/json.

Core tests exercise valid/corrupt/truncated/oversized downloads, cancellation, preservation of an existing model, symlink/missing models, bounded Unicode evidence, strict response parsing, worker crash, blocked stdin, timeout, cancellation and stdout overflow. A child-process fixture is intentionally ignored as a standalone test and invoked by the supervisor tests. Desktop tests cover job serialization and cancellation state. No paid API or mandatory Python runtime is used.

`scripts/smoke-inference.mjs MODEL_DIR WORKER OUTPUT_JSON` runs two synthetic cases on both models/backends under a macOS network-denying sandbox and records `/usr/bin/time` worker RSS. These are single-run, 128-token smoke checks, not the full 90-case planning benchmark. OS file cache and shader compilation affect latency; worker RSS is not combined desktop/GPU memory. The full protocol remains in models/README.md.

## Next

Phase 4: deterministic organization and storage evidence, bounded read-only agent tools and strict proposal schemas. Phase 5: approved, journaled, reversible operations. No file modification controls are exposed yet.

## Verified on 2026-09-29

50 Rust tests passed (49 core + 1 desktop); one subprocess fixture is intentionally ignored directly. Formatting, strict workspace Clippy, frontend production build and macOS arm64 debug packaging passed. The packaged worker digest matches its compiled manifest. The desktop displayed an actual Metal answer over a 20-file indexed sample; no source files were modified.

All eight synthetic CPU/Metal runs succeeded with network denied. On this Apple M4 / 16 GB / macOS 26.6.2 host, 0.6B took approximately 0.6–1.5 seconds and 1.7B approximately 1.0–3.2 seconds for short answers. Peak worker RSS stayed below 2 GB in these cases. See phase3-smoke-results.json for exact raw measurements; these exclude combined app/GPU accounting and are not 8 GB validation or cold-cache benchmarks.

Both models identified the two project filenames. Under the injection fixture, 0.6B fabricated that a file had been deleted; 1.7B correctly said the sample lacked duplicate evidence. Keep 1.7B as the default candidate, but do not treat this tiny comparison as proof of safe or accurate planning. No model output has execution authority. Full quality evaluation and a different-family alternative remain Phase 7.

## v0.3.1 coverage correction

The original 20-largest-files prompt could not describe the folder accurately. The fix computes statistics across the whole current index and searches all indexed paths/saved text before choosing model examples. Exact counts, byte totals, extension distribution, saved-text coverage, snapshot date and exclusions are displayed independently from generated prose. The model is instructed never to use example count as folder count.

This does not load 61,592 full files into a 4,096-token model or claim a semantic reading of every file. It provides complete indexed metadata coverage and bounded lexical retrieval. Unsupported/binary/unindexed contents remain unread. The existing scanner’s exclusions and limits still apply, and a partial scan is explicitly labeled. General semantic retrieval, deterministic project/category/date proposals and storage analysis belong to Phase 4.

Regression coverage includes 61,592 rows, retrieval of a small file beyond the first page, saved-text matching beyond the excerpt prefix, exclusion of old generations and other scopes, cancellation and category remainder accounting. Prompt tests preserve the whole-index count under Unicode/context truncation. No schema migration or source-file modification is required.

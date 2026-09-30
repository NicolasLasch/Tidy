Phase 2 results and additional contracts are documented in `docs/phase2.md`. Core checks now require one initial `cargo fetch --locked`; desktop checks also require `npm ci` and a built frontend.

# Verification and remaining gates

Run `cargo fmt --all -- --check`, `cargo test --workspace --offline`, and `cargo clippy --workspace --all-targets --offline -- -D warnings`.

Phase 1 has 14 macOS scanner integration tests: nested metadata and unchanged content, empty folders, invalid roots, Git repositories/worktrees/ancestors, Git marker added after grant, entry and depth limits, cancellation and zero timeout, changed-file rescans, symlinks/cycles/broken links, symlink ancestors, unreadable directories, protected roots and scope escape. Linux adds a native non-UTF-8 filename test because the macOS test volume rejects that filename. Permissions assertions apply when the test account cannot bypass mode bits. Tests create disposable fixtures outside the checkout so a Git checkout does not invalidate authorization.

The runtime scanner has no file mutation API. Test fixture creation and cleanup are test-only. The no-model CLI is smoke-tested separately. Windows and Linux execution are not locally verified; the CI matrix requests those platforms. Windows junction and known-folder tests must be added during platform hardening.

Required future integration/fault tests before execution ships:

| Scenario | Expected behavior |
| --- | --- |
| Existing destination, case/Unicode-equivalent name, case-only rename | No overwrite; explicit collision or safely journaled approved rename |
| Permission denied before/after preview | Action stops with precise error; no silent alternate operation |
| Symlink/junction or parent replaced during traversal/execution | Handle-bound scope rejection, no access outside grant |
| Source size/content/identity changes or disappears | Stale approval rejected, new preview required |
| Crash before journal, after journal, after operation, before verification | Startup reconciliation by file identity; never blind replay |
| Malformed JSON, unknown fields/tools/IDs, model shell text, huge output | Reject, bounded error/fallback, zero filesystem changes |
| Model unavailable/corrupt, worker timeout/crash | Rules/search remain usable; no automatic network request |
| Undo after collision, source edit, permission change or Trash emptying | Preserve both files, refuse unsafe restore, explain unrecoverable state |
| Multiple clicks, replayed/expired approval, partial batch | One-use digest-bound authorization and per-action outcomes |
| Duplicate hard links, APFS clones/sparse files | Honest savings; hashes alone do not imply physical space recovery |
| Interrupted incremental scan | Keep previous complete generation; do not delete unseen entries |
| All three journeys offline | Review, explicit approval, verification and undo with/without model |

Path-based Phase 1 checks do not pass adversarial race tests by construction. These are explicit gates for later phases, not claimed coverage today.

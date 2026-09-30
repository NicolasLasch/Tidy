# Tidy 0.8.2 — say it, get a plan

**Chat that decides.** A new instant intent engine (`crates/agent-runtime/src/intent.rs`) understands everyday requests without loading a model and without clarifying questions: delete a folder by (fuzzy) name, several folders at once, files by type/age/size/name/“biggest N”, “except …” exclusions, dev artifacts (node_modules, caches, Cargo target) as whole folders, organize by type/date, “what’s taking space”, and “find …”. Ambiguity is resolved with the most sensible reading and disclosed in the reply. Anything it cannot map falls through to the local model; if that is missing or fails, the reply is a friendly list of examples instead of an error. Typing “yes / do it / go ahead” opens the approval sheet for the pending plan.

**Whole-folder Trash.** New `TrashDir` safety action: a folder moves to the native Trash as one journaled, one-use-approved step (up to 50 per approval, no overlaps). The tree is inspected with no-follow handle-relative opens; Git repositories, mount points, cross-volume folders, the scope root and protected paths are refused. The approval is bound to the tree’s identity, file count and byte total, so a folder that changes after review will not execute. Undo is via Finder’s Trash (Put Back). Folders have a trash button in **Folders**.

**Folders you choose.** The Folders tab lists every indexed folder heaviest-first with a switch. Only switched-on folders are visible to planning, questions and AI proposals (enforced in the backend, persisted in `ai_selection.json`). Nothing is selected by default.

**iOS-style interface.** New shell with bottom tab bar, message bubbles, bottom sheets and light/dark support. The expand/shrink button turns the window into a small always-on-top assistant at the top-right of the screen and back.

Verification: `cargo test --workspace`, clippy `-D warnings`, `tsc -b` and `vite build` pass. Folder-Trash validation is unit-tested (tree totals, changed-after-review, Git/root/file/overlap refusals); the native Trash step itself is exercised by a manual disposable-folder check.

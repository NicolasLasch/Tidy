# Tidy 0.6.0 — model-directed investigation

Ask Tidy replaces the category/date/custom mode cards. The active workflow no longer calls the rule translator or rewrites model destinations using photo/extension heuristics.

The local model receives an index overview and your request. It chooses read-only search calls (including paging, filename/text filters, file extensions and size ordering), can inspect saved metadata/text, then proposes exact destinations for discovered file IDs. Folder names, arbitrary nesting and destination filenames come from the model. It may ask a clarification question. Only cached text is available; filenames do not prove contents.

The runtime validates every proposal against discovered IDs, relative paths, protected paths and destination collisions. Application bundles and common dependency/build directories cannot be reorganized through this workflow. Approval and execution remain entirely in the separate safety engine. No shell or filesystem mutation tool is exposed to the model.

A session keeps model weights loaded while it investigates, then releases the process at completion, cancellation or error. Bounds: 24 model steps, four minutes after startup, 90 seconds per step, 500 proposed actions per review batch. Responses use a constrained tool JSON grammar. Model quality and speed are not certified by compilation.

Search operates over the full authorized index. The model sees paginated evidence. The UI distinguishes searchable files, examined files and unexamined matches. A finish with unexamined matches is labeled partial. Continue investigation retains the current proposed batch; new requests replace it. A complete investigation label does not guarantee the model interpreted your intent correctly or read every file's contents.

The interface centers on a request composer, live tool activity, clarification, proposed folder tree, selectable source/destination rows and an exact approval sheet. Files, storage analysis, history and local model installation remain available through sidebar navigation when inference is unavailable.

Trash is still a separate Storage recovery flow with native Trash receipts and manual Finder recovery. Ask Tidy proposes moves/renames through full destinations; it does not issue cleanup deletion actions.

## User-run acceptance checks

Quit all older Tidy processes and open the repository bundle. Confirm v0.6.0 and Ask Tidy as the first screen.

1. Create a disposable folder containing several png/jpg images, one txt file and a nested folder. Index it. Optional text indexing enables saved-text investigation.
2. Request: “Put my images in Memories, then subfolders by file format. Don't touch text files.” Watch Search/Propose steps. Review exact paths; expected hierarchy should be chosen by the AI, with no text-file action.
3. Change the request to “Use Album/Originals/By format with another nested folder for each extension.” Verify this is a new AI investigation and its exact destinations reflect your request; no Photos-specific transformation should run.
4. Test project discovery using a project name present in filenames or saved text. Verify shown evidence and paths.
5. Ask an ambiguous request such as “Organize the files for my project.” The assistant should investigate or ask which project, rather than apply a default category layout.
6. Use more than 20 matching disposable files. Check paginated searches or partial coverage, and Continue investigation. Unexamined matches must not be presented as full review.
7. Cancel an investigation. Verify the activity stops and no files moved.
8. Select only two proposed moves. Review approval: the modal must show exactly those source/destination pairs. Cancel once, then generate approval again and apply only after checking the paths.
9. Verify moves in Finder, then Review undo, inspect reverse paths and approve. Changed-source/collision protections should still refuse unsafe operations.
10. If execution fails, check History for earlier verified steps before retrying.
11. Check Explore files and Storage with the model unavailable; metadata workflows should remain useful.

Added regression cases for arbitrary AI nesting, unseen IDs, traversal rejection and paginated coverage. They were type-checked, not executed:
cargo test --offline -p tidy-agent-runtime investigation::tests

Functional model inference, native file operations, responsiveness on your hardware and visual UI interaction have not been tested by Codex in this revision, per your preference. Use disposable files for the first approved changes.

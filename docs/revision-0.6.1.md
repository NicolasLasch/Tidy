# Tidy 0.6.1 — request-aware moves and Trash

The reported removal failure had two concrete causes: Ask Tidy's tool protocol supported only moves, and its prompt supplied a literal `Any/Folder/Subfolder/file.png` example. The model copied that example and misrepresented a move plan as a completed removal investigation.

The agent now has a separate constrained `trash` proposal tool. Workflow instructions distinguish organizing, renaming, removing, and read-only investigation. Real indexed IDs must be discovered through search before either move or Trash proposals are accepted. Folder names and nesting remain model-selected. No default folder layout is imposed.

A conservative request boundary recognizes affirmative English removal verbs (`remove`, `delete`, `trash`), negation, explicitly named indexed targets, and named exclusions. A named removal cannot broaden to other IDs, and moves cannot substitute for removals. Missing or ambiguous named targets fail clearly. Known example/placeholder destinations are rejected. Read-only requests cannot propose changes. For ambiguous or combined removal/organization instructions, use separate requests. This boundary is deliberately conservative, not a complete natural-language parser.

A removal finish with no valid Trash proposal or an uncovered named target gets up to two correction attempts using discovered IDs. Persistent failure becomes an incomplete clarification, rather than a success claim. Accepted removal summaries are generated from the actual proposals, not the model's claim of deletion. Duplicate cleanup without named targets is routed to Storage's hash-based duplicate analysis; names/sizes alone are insufficient.

The preview and approval sheet display **Move to Trash** explicitly. Apply still uses the existing approval-bound, rechecking, journaled native Trash engine. Nothing is permanently deleted. Trash recovery is through Finder using History's receipt; automatic undo is offered for moves only. No changes occur during planning.

## User-run checks

Quit older Tidy instances, open the updated app, and confirm **v0.6.1**. Use disposable files first. Model execution and native file-operation tests were left to you as requested.

1. Create a fresh disposable folder with `project/copy.txt` and `project/notes.txt`. Index it in Tidy.
2. Ask: **Remove project/copy.txt. Don't touch any other files.** The only acceptable change is `project/copy.txt → Move to Trash`. `notes.txt` must have no action. No example folder should appear.
3. Cancel the approval sheet once; both files must remain. Then review again and approve only after checking that exact source/action.
4. Verify only `copy.txt` is in Finder Trash. In History, reveal its recovery location and restore it through Finder. Refresh the index afterward.
5. Ask: **Remove missing.txt.** Expect a missing-target error and no substitute action.
6. Add another `copy.txt` in a different subfolder and refresh. Ask **Remove copy.txt.** Expect an ambiguity error requiring the relative path.
7. Ask: **Find project/notes.txt; do not delete anything.** Expect findings without change proposals.
8. Add a PNG/JPG, refresh, and ask **Put photos into Memories/Originals, then one subfolder per format. Leave text files untouched.** Review the exact AI destinations and confirm no text actions; arbitrary nesting remains available.
9. Change a disposable source after generating its plan, then try approval/application. The existing changed-file checks must refuse it.

Focused regression suite (does not use your real indexed folders):

```sh
cd "/Users/nlasch/Coding Projects/Tidy"
cargo test --offline -p tidy-agent-runtime investigation::tests
```

Added regressions cover wrong operation/extra named targets, truthful incomplete removal, exclusions/negation/read-only requests, ambiguous/missing names, and placeholder rejection while retaining arbitrary nesting. Compilation is not proof of model quality: repeat the prompts above with your installed model and report any incorrect proposed source/action before approving.

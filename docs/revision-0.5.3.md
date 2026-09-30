# Tidy 0.5.3

The local model could return a flat Photos destination despite a request for extension subfolders. The planner now supports Photos/{EXT}, expands it deterministically to uppercase extension folders, and enforces the hierarchy for explicit requests mentioning extension/type subfolders. Photo-only hierarchy requests filter out non-photo formats. Files directly inside the requested parent (for example Photos/a.png from an earlier flat plan) can also be regrouped; unrelated nested folders remain untouched.

The preview now displays destination folders and file counts before the per-file table, making a missing hierarchy visible before approval. The approval batch remains capped at 500.

The interface has shared glass surfaces, lighter sidebar navigation, segmented controls, consistent blue primary actions, updated file tables, destination cards, history panels and approval sheets. Text and error states retain opaque backgrounds for readability. Reduced transparency and reduced motion preferences are supported.

## Check it yourself

Quit old Tidy, reopen the exact repository app bundle, and confirm v0.5.3.

In a disposable folder create a.png, b.jpg, c.jpeg, keep.txt, and Photos/old.png. Index that folder. Ask:

“Put all Photos into one folder with sub folder for PNG, JPG etc. Don't touch other files.”

Check the preview:
- a.png → Photos/PNG/a.png
- b.jpg → Photos/JPG/b.jpg
- c.jpeg → Photos/JPEG/c.jpeg
- Photos/old.png → Photos/PNG/old.png
- keep.txt has no action.

Review the folder cards, then cancel once. Nothing should move. If the preview is correct, generate again, approve the disposable files, and test Undo. Existing destination collisions should be omitted rather than overwritten.

Regression tests were added and type-checked, not executed. To run them:
cargo test --offline -p tidy-organization rules::tests

Only builds/type checks were run by Codex, per your testing preference. Model interpretation and the visual layout still need your check.

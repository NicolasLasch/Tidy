# Tidy 0.7.0 — guided local skills and storage contents

## Local skills

Tidy now includes 39 application workflow skills. The local model selects the most specific workflow before investigating your request. You can also choose one in the workflow picker. The chosen workflow and its steps are visible in Ask Tidy. Folder names, nesting and requested new names still come from the request and model reasoning; the skills do not prescribe a folder layout.

The catalog covers Downloads, types, image formats/dates, videos, audio, documents, archives, dates, projects, photo consolidation, existing hierarchy, flattening, custom hierarchy, related documents, work/personal contexts, screenshots, descriptive names, normalized names, affixes, numbering, dated names, filename/text/project search, folder/file explanation, storage overview, large files, heavy folders, verified duplicates, installers, development artifacts, named/filtered Trash, undo, Trash recovery and clarification.

`skills/catalog.json` is the embedded runtime source of truth. Each entry has a matching `skills/<id>/SKILL.md` document describing context, steps, evidence and stopping conditions. These are Tidy skills, not external plugins. Catalog changes require rebuilding the app; changing a descriptive SKILL.md alone does not alter runtime behavior.

The runtime checks discovered file IDs, workflow action type, image/video/audio type boundaries, rename parent/extension, request exclusions, named Trash targets, collision/path policy and placeholders. A wrong workflow or unsupported evidence produces a refusal or clarification, never a silent organization fallback. Removal and name editing have separate workflows. Storage and recovery skills guide you to Storage/History rather than inventing hash evidence or recovery paths.

A read-only folder tool reports recursively summed indexed file contents and supports drill-down. AI evidence pages are bounded by serialized byte size; pagination and examined counts include only records actually delivered. Budgets and model limitations still apply. Workflow guidance improves control but does not prove every model interpretation is correct; review the exact proposal before approval.

## Storage overview

Storage shows OS capacity/used/available figures for volumes associated with authorized folders, plus a global overview of indexed authorized roots. Overlapping roots are deduplicated by absolute file path for the global indexed total, using the newest saved snapshot for matching paths. Root cards retain their own totals and may overlap; do not sum them manually. Capacity reporting is implemented for macOS/Unix; unavailable platforms/roots are labeled rather than assigned invented values.

Each folder's contents size is the sum of all indexed regular-file lengths recursively, including files below the large-file threshold. It never uses directory metadata size. Drill-down separates subfolder totals from files directly in the current folder. Empty directories are not indexed.

These are logical contents totals, not physical allocation or guaranteed reclaimable bytes. Hard-link references count as separate logical paths. APFS compression/clones/snapshots, sparse allocation, unscanned areas and scan exclusions may differ from these totals. Scans with exclusions or incomplete coverage are labeled as known indexed contents, potentially smaller than the real folder. Refresh the index to update contents; refreshing the storage overview reads the saved index and current volume capacity.

Select contents on a folder to replace the cleanup selection with its largest eligible indexed files, up to 500 per batch. Duplicate keepers and protected bundles are skipped. Review the exact individual files in the cleanup proposal/approval sheet. The directory itself is never a Trash action. Nothing changes until explicit approval; existing source rechecks, native Trash receipts and journal safeguards remain in force. Trash does not immediately free disk space.

## User-run acceptance checks

Quit older Tidy instances. Open the repository app and confirm **v0.7.0**. Compilation and packaging were performed; model inference, visual interactions and native changes were left to you as requested.

1. Open Ask Tidy's workflow picker. Confirm **39 skills** and an Automatic option. Ask for photo format subfolders: inspect the chosen workflow, evidence trace and exact PNG/JPG destinations. Non-image files must have no proposal.
2. Ask **Remove project/copy.txt. Don't touch other files.** Expect a named-file Trash workflow and only that file proposed for Trash. Cancel approval first to verify no changes occur.
3. Ask **Remove the draft_ prefix from filenames.** Expect filename editing, with the same parent and extension, never Trash.
4. Ask **Find notes.txt; do not delete anything.** Expect findings with no modification proposals.
5. Ask **Find heavy folders.** Expect folder-contents investigation and an Open Storage button. Ask about duplicate recovery and verify it directs to hash-based Storage review rather than claiming duplicate evidence from names/sizes.
6. Manually choose Custom folder hierarchy and request unusual nesting. The model should retain your arbitrary folder names rather than apply default categories.
7. In Storage, check all authorized roots, scan timestamps/exclusions and OS volume figures. Authorizing another folder must not turn into an unapproved whole-disk scan.
8. Verify exact contents totals with this disposable fixture (run these commands yourself):

```sh
tidy_check_root="$(mktemp -d /private/tmp/tidy-skill-check.XXXXXX)"
mkdir -p "$tidy_check_root/Small/deep" "$tidy_check_root/Large"
printf '1234567890' > "$tidy_check_root/Small/one.txt"
printf '12345678901234567890' > "$tidy_check_root/Small/deep/two.txt"
printf '12345' > "$tidy_check_root/root.txt"
printf 'abcd' > "$tidy_check_root/Large/four.txt"
printf '%s\n' "$tidy_check_root"
open "$tidy_check_root"
```

Authorize and scan that printed folder in Tidy. Expected indexed root: **39 B / 4 files**; **Small: 30 B / 2 files**, including **deep: 20 B / 1 file**; **Large: 4 B / 1 file**; root direct files: **5 B / 1 file**. Small must rank above Large despite their names. Directory inode sizes and the 50 MiB findings threshold must not affect these totals.

9. Select Small's contents. Expect two selected files, then preview only those file paths. Cancel approval and verify all fixture files remain. Use disposable files for any approved Trash check; recover through History/Finder.
10. Add/remove a fixture file yourself, refresh the index, and verify folder/global totals update. Switch roots or refresh during loading; stale results must not replace the newer view.
11. If roots overlap, verify the global indexed path total does not double-count their common files. Root-specific cards may legitimately overlap.

Focused suites for you to run:

```sh
cd "/Users/nlasch/Coding Projects/Tidy"
cargo test --offline -p tidy-agent-runtime workflows::tests
cargo test --offline -p tidy-agent-runtime investigation::tests
cargo test --offline -p tidy-storage folders::tests
```

New regressions cover catalog completeness, model workflow routing/type guards, name-affix removal versus Trash, rename parent/extension protection, read-only storage investigation, bounded evidence, recursive contents and root/direct-file totals. They were compiled, not executed in this revision.

# Tidy 0.5.1: user-run verification

Quit every older Tidy window/process before opening the new build. Use only a disposable folder for these checks, not your main Downloads folder. Do not empty Trash during testing. No operation below requires administrator privileges.

## 1. Start the updated app

```sh
cd "/Users/nlasch/Coding Projects/Tidy"
open target/debug/bundle/macos/Tidy.app
```

The sidebar should show v0.5.1. If not, quit the old process and reopen this exact bundle.

## 2. Create disposable inputs

Run in Terminal; keep the printed folder path:

```sh
TIDY_CHECK=$(mktemp -d "$HOME/Downloads/Tidy-check.XXXXXX")
mkdir -p "$TIDY_CHECK/Documents" "$TIDY_CHECK/Apollo" "$TIDY_CHECK/protected/.git"
printf 'do not replace\n' > "$TIDY_CHECK/Documents/report.txt"
printf 'collision source\n' > "$TIDY_CHECK/report.txt"
printf 'notes for testing\n' > "$TIDY_CHECK/notes.txt"
printf 'image fixture\n' > "$TIDY_CHECK/photo.jpg"
printf 'duplicate fixture\n' > "$TIDY_CHECK/copy-a.txt"
cp "$TIDY_CHECK/copy-a.txt" "$TIDY_CHECK/copy-b.txt"
printf 'Apollo brief\n' > "$TIDY_CHECK/Apollo/brief.txt"
printf 'old installer fixture\n' > "$TIDY_CHECK/setup.dmg"
touch -t 202001010000 "$TIDY_CHECK/setup.dmg"
printf 'protected project\n' > "$TIDY_CHECK/protected/source.rs"
ln -s "$HOME/Documents" "$TIDY_CHECK/external-link"
printf '%s\n' "$TIDY_CHECK"
```

## 3. Scan, links and safe previews

1. Add that folder through Tidy's picker. Enable optional content indexing and rescan so the tiny duplicate fixtures receive hashes.
2. Confirm the Git folder and symlink are excluded. No linked Documents content should appear.
3. Open Organize & Group with local AI off. Generate Category.
4. `notes.txt` and `photo.jpg` should receive destinations. The existing `Documents/report.txt` must not be overwritten. Nested `Apollo/brief.txt` must remain intact in category mode.
5. Click a preview filename. Finder should reveal the correct original file.
6. Open approval, then Cancel. Verify no files moved.

## 4. Changed-file rejection

1. Open approval for a fresh category plan, but do not execute yet.
2. In the same Terminal run `printf 'changed after approval\n' >> "$TIDY_CHECK/notes.txt"`.
3. Confirm execution. It must reject the changed file. The batch may already have applied earlier steps; check History. No file should be overwritten or permanently removed.
4. Rescan before generating another plan. Inspect any `needs_recovery` steps rather than repeatedly clicking Execute.

## 5. Move and undo

1. Generate a new category plan and approve the exact preview.
2. Confirm successful items appear at their destinations and disappear from their original paths immediately in Files & Search.
3. Use Undo Plan (or expand History, then Undo) and approve it. Unchanged moved files should return. Empty category directories may remain.
4. To check undo protection, move a disposable file again, edit it at its destination, then request undo. It must refuse the changed file, preserving the new contents.

## 6. Storage cleanup and manual Trash restore

1. Rescan and run Storage Recovery. Selection should start empty.
2. The two tiny copies should form a duplicate group. Keep one and select only the other. Select All must not allow removal of every known duplicate copy.
3. Approve one selected file for Trash. Completion should not wait for a full rescan. The retained copy must stay.
4. Expand its History transaction. Click Reveal in Finder Trash. Check the recorded item exists; restore it using Finder's Put Back where available, or drag it to the fixture folder. Tidy does not offer automatic Trash undo.
5. Rescan after restoring. A trashed filename link should report that the original no longer exists rather than open an unrelated file.
6. Selecting nothing must never propose removal of everything.

## 7. Project, custom rules and responsiveness

1. Project mode with `Apollo` should find the matching indexed path. Review before approving; unknown or protected files remain untouched.
2. Test custom mapping on a second disposable folder, for example `Documents: txt | Images: jpg`.
3. Compare planning with AI off and on. AI-on can take longer and must disclose bounded review or fallback. Cancel AI should return a rules-based plan. It must not reduce whole-folder deterministic coverage to 20 files.
4. Switching selected folders must clear old approval/selection state. Storage lists initially display at most 100 groups/items per category and provide Show 100 more.

## 8. Automated checks you can run

```sh
cd "/Users/nlasch/Coding Projects/Tidy"
cargo test --workspace --offline
cargo clippy --workspace --all-targets --offline -- -D warnings
```

Tests include native Trash of disposable fixtures. They may leave fixtures in Trash; restore/inspect them manually as needed. Four added regressions cover edits after approval, a different root, a swapped destination symlink and changed-file undo. These tests were added and type-checked, but not run by Codex in this revision.

If anything fails, send the exact error, tab/mode, whether AI was enabled, and the History step status. Stop testing against real files until that failure is resolved. Crash/permission/large-directory checks and target-hardware performance still need validation; a successful build alone does not certify Phase 5.

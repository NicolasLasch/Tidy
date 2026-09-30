# Ideas & roadmap

Everything here is open for anyone to pick up. Comment on (or open) an issue first if it's big. **GFI** = good first issue.

## Requests Tidy should understand (in `intent.rs`)
- [ ] **GFI** “Delete empty folders” (needs an index of empty dirs or a disk walk)
- [ ] **GFI** More category mappings and synonyms (`concept_names`, `kind_exts`): ebooks, fonts, 3D models, design files
- [ ] “Sort photos into Photos/YEAR/MONTH” without an AI model (compose date + kind)
- [x] Multi-step requests: “rename X to Y and move it into Z”, “move X into Z and rename it”, “create a folder Z and move X into it” (one plan, one approval; see `multi_step` in `intent.rs`)
- [ ] Longer chains and other verb pairs: “move X into Z, then organize Z by type”, “copy X to Z and rename the copy”, “rename X to Y and trash the old copies”
- [ ] **GFI** Multi-step with lists and filters: “rename these three folders to A, B, C and move them into Z”; “move all PDFs into Invoices and rename them by date”
- [ ] **GFI** Multi-step requests that mix folders and files in one sentence (“move the Old Stuff folder and setup.dmg into Archive”)
- [ ] Show multi-step plans as a before → after tree in the review sheet instead of one line per action
- [ ] Undo by chat: “undo the last change” / “put back the folder I deleted”
- [ ] Localization of request phrases (French, Spanish, German…)

## Understanding files better
- [ ] Extract text from PDF / DOCX / EPUB for search and “find the invoice about roof repair”
- [ ] Near-duplicate photos (perceptual hashing) and “burst” groups
- [ ] Detect “versions” (`final`, `final2`, `copy`) and offer to keep the newest
- [ ] Embedding search over indexed text (local model)

## Storage
- [ ] Windows/Linux disk explorer (`disk.rs` uses `st_blocks`; port allocation size)
- [ ] Treemap view; “what changed since last week”
- [ ] Purgeable space / APFS snapshot awareness; Full Disk Access walkthrough
- [ ] Docker / Xcode / Homebrew cache cleaners as first-class tips

## Safety & platform
- [ ] Windows execution (handle-based ops + Recycle Bin with restore IDs)
- [ ] Cross-volume moves (copy → verify → trash) with their own approval
- [ ] Signed + notarized builds and auto-update (opt-in)
- [ ] Fuzz the request parser and the journal recovery path

## UX
- [ ] **GFI** Keyboard shortcuts (⌘K focus chat, ⌘1–4 tabs) and a global “summon” hotkey
- [ ] Drag a folder onto the window to add it
- [ ] Onboarding tour; localized UI strings
- [ ] Accessibility audit (VoiceOver labels, reduced motion)

## AI
- [ ] A small public benchmark of real requests → expected plans, to compare models
- [ ] Better prompts / few-shot examples for the *restate* step
- [ ] Optional vision model for “photos of my dog”, receipts, screenshots

## Community
- [ ] Share “recipes”: named, reviewed request macros (“monthly downloads cleanup”)
- [ ] A gallery of before/after folder structures

Have a different idea? Open an **Idea** issue — even a rough one.

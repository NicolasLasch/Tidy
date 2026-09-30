# Tidy 0.5.2 — repair and request planning

The prior journal contains 4,140 verified Trash steps whose original paths no longer exist. The interrupted legacy cleanup did not reconcile them into the index. On startup this revision removes only missing index entries associated with verified Trash steps. It does not alter disk files or restore Trash. Review the older transaction in History and Finder Trash if you want those files back.

Approval now reports the affected filename when a source is missing, removes that stale index entry, and refuses to create an approval. Refresh results and review again. Failed execution closes the consumed approval rather than offering to execute it twice.

Custom requests now translate into bounded filename/extension rules, using grammar-constrained local JSON generation and up to 1,024 output tokens. Rules run over all indexed candidates; top-level matches receive move proposals. Nested folders remain intact. Destination collisions are skipped. Each approval remains limited to 500 actions. Rules and counts appear in the preview. The model never executes changes.

Example: “Move PDFs whose filename contains invoice or receipt into Invoices.”
Expected rule: PDF extension AND either filename substring. Other PDFs must not be selected.

Content-based classification, arbitrary renaming and recursive custom moves are not implemented by this rule interpreter. Unsupported or failed requests show an error; generic sorting is never substituted. Category/date/project modes retain deterministic planning. Explicit mapping syntax remains available with AI off.

Visual changes: supplied broom/leaf logo in the sidebar and macOS app icon; translucent light panels; blue primary controls; an animated SVG leaf mascot inside the app that opens organization planning. This is not a separate always-on-top desktop pet.

## Quick verification (user-run)
1. Quit all old Tidy processes. Open the repository's exact target/debug/bundle/macos/Tidy.app. Check v0.5.2, broom logo and mascot.
2. Use the disposable fixture folder from manual-verification.md, not real Downloads for mutation checks.
3. Add invoice.pdf, receipt.pdf and unrelated.pdf (plain text fixtures are enough). In Custom, with AI enabled, use the example request. Only the first two should receive Invoices destinations.
4. Cancel once: no files move. Generate again, review and approve; then approve Undo. Original paths should return.
5. In Storage, select one disposable file, review and approve Trash. Refresh results: the missing source should not be offered again. Restore through Finder Trash.
6. Ask for an unsupported content-based request. Expect an explicit limitation/error, never a generic extension plan.
7. Click the mascot; it should open organization. Click a source filename; Finder should reveal it.
8. Optional automated check: cargo test --workspace --offline

Only compilation/build checks were run by Codex. No model inference, functional file-operation tests, or visual UI automation were run in this revision. Model intent accuracy, native Trash behavior and user-visible layout still need these checks.

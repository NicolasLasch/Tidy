## What does this change, in one sentence a user would understand?

## How was it tested?
- [ ] `cargo fmt --all` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo test --workspace`
- [ ] `cd apps/desktop && npx tsc -b`
- [ ] Added or updated a test (for new actions: refusal cases too — collisions, `.git`, outside scope, changed after review)
- [ ] Tried it on made-up data (`make_mock`)

## Safety checklist (if it touches files)
- [ ] Only `crates/safety` mutates files; no permanent delete; no overwrite
- [ ] Undo story (or an honest “not undoable”)

## Notes for reviewers / screenshots

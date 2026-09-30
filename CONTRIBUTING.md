# Contributing to Tidy

Thanks for helping! Tidy is a small, focused codebase and welcomes contributions of every size — a typo, a new request phrase, a whole feature.

## Quick start

```sh
git clone https://github.com/NicolasLasch/Tidy.git && cd Tidy
cargo test --workspace            # ~1 minute, no network needed after `cargo fetch`
cd apps/desktop && npm ci && npm run build   # type-checks the UI
```

Run the app: `npm run worker:build` (once) then `npm run desktop`. Try changes on made-up data:
`cargo run --example make_mock -p tidy-agent-runtime -- ~/TidyMock`.

## Where to add things

| I want to… | Look at |
|---|---|
| teach Tidy a new kind of request | `crates/agent-runtime/src/intent.rs` (add a parser + a test next to the others) |
| add a file category / extension mapping | `crates/organization/src/lib.rs` (`extension_category`) |
| add a new *change* Tidy can make | `crates/safety` (new `ValidatedAction`, validator, executor, undo) — needs tests for refusal cases |
| improve a screen | `apps/desktop/src/*.tsx`, styles in `app.css` (plain CSS variables, light/dark) |
| add a model to the catalog | `crates/agent-runtime/src/catalog.rs` (pinned URL, size, SHA-256) — or just paste a Hugging Face link in the app |
| add an end-to-end scenario | `tests/e2e_large_scale.rs` (uses the shared mock in `tests/support/`) |

## Ground rules (they keep users' files safe)

1. **Only `crates/safety` changes files.** The request engine, model and UI only *propose*.
2. **Never overwrite, never permanently delete.** Use no-replace operations and the native Trash.
3. **Every new action needs**: validation, a journal entry, verification, an undo story (or an honest “not undoable”), and tests that prove the refusal cases (collisions, `.git`, outside-scope, changed-after-review).
4. **The request engine must be deterministic and explain itself.** If a parse is ambiguous, pick the sensible reading and say so in the reply; never silently widen scope.
5. Keep the UI accessible: labels on controls, keyboard support, sheets close with Esc.

## Before you open a PR

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
(cd apps/desktop && npx tsc -b)
```

Add or update tests, note user-visible changes in `CHANGELOG.md`, and describe *what a user will notice*. Small PRs are easier to review than big ones.

## Ideas welcome

Open a *Discussion* or an *Idea* issue — no code required. The [ideas list](docs/IDEAS.md) has starter tasks tagged by difficulty.

By contributing you agree your work is released under the MIT license. Please follow the [Code of Conduct](CODE_OF_CONDUCT.md).

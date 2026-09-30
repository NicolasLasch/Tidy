# Testing

```sh
cargo test --workspace --locked                                  # everything (macOS; ~1–2 min)
cargo test -p tidy-agent-runtime --test e2e_large_scale          # 1,000-file workflows
cargo run --release --example benchmark -p tidy-agent-runtime -- --write docs/BENCHMARKS.md
cargo clippy --workspace --all-targets --locked -- -D warnings
cd apps/desktop && npx tsc -b && npm run build
node scripts/check-version.mjs                                   # version consistency
```

* **Unit / contract tests** live beside the crates and in `tests/` (scanner, storage findings, organization, safety journeys).
* **End-to-end:** `tests/e2e_mock_folder.rs` and `tests/e2e_large_scale.rs` generate mock trees (`tests/support/`), scan them with the real indexer, run the real request engine, and execute through the real safety engine, then check the disk. They move items to the macOS Trash and put them back so your Trash stays clean; they only run on macOS.
* **Try it by hand:** `cargo run --example make_mock -p tidy-agent-runtime -- ~/TidyMock --large`.
* **CI** (`.github/workflows/ci.yml`): version consistency, secret scan (gitleaks), fmt + clippy + tests on macOS, and a full app-bundle build with worker on every push to `main`. **Releases** (`release.yml`) re-run the whole gate before packaging.

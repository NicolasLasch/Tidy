# Tidy desktop app

Tauri 2 (Rust) backend + React/TypeScript front end. See the [root README](../../README.md) and [docs/ARCHITECTURE.md](../../docs/ARCHITECTURE.md).

```sh
npm ci
npm run worker:build   # once: builds the pinned llama.cpp worker for local AI
npm run desktop        # development app with hot reload
npm run build          # type-check + production front end
npm run package        # release bundle → ../../target/release/bundle/macos/Tidy.app
```

* `src/` — UI (`ChatView`, `StorageView`, `HistoryView`, `ModelView`, `App`), styles in `app.css`.
* `src-tauri/src/` — IPC commands: `planning.rs` (chat pipeline), `safety_ipc.rs` (approve / execute / undo / put back), `disk.rs` (disk explorer), `ai.rs` (models), `selection.rs` (folders the assistant may see).

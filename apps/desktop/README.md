# Tidy desktop — Phase 2

Tauri 2 host and React/TypeScript/Tailwind UI. Run `npm ci`, then `npm run desktop`. Build macOS with `npm run package`; output is in the workspace's `target/release/bundle/macos/` directory. Development app bundle: `npm run tauri -- build --debug --bundles app`.

Only native folder selection can add a scope. The webview cannot pass arbitrary paths for file reads; commands take stored scope IDs. It has no filesystem/shell plugin capability and no mutation commands. CSP blocks remote assets/connections in the packaged app. SQLite lives under the OS application-data directory (`~/Library/Application Support/org.tidy.desktop/index.sqlite3` on macOS), in a directory restricted to its owner. Cached text is optional and not encrypted separately from the user's disk.

The frontend's browser preview intentionally has no filesystem access or fake data. Use the native application to test scanning. Phase 2 keeps a normal resizable window; tray, global shortcut, always-on-top floating behavior and operation approval views remain later UI work.

Read `../../docs/phase2.md` for limits and known platform gaps.

# Benchmarks

Measured on **Apple M4**, 10 cores, macOS 26.6.2, release build. Mock data is generated deterministically (`tests/support/large_mock.rs`): one messy `Inbox/` dump with photos, documents, screenshots, installers, logs, temp files, odd names and exact duplicates, plus projects, client folders, launcher instances and an archive. Every operation is executed by the real safety engine and verified on disk; nothing here uses an AI model (the instant engine answers these requests).

| Operation | 978 files | 9978 files | 49978 files |
|---|---:|---:|---:|
| Build mock tree + first scan/index | 248.6 ms | 2.56 s | 13.87 s |
| Rescan (incremental, unchanged tree) | 45.2 ms (21650 files/s) | 457.4 ms (21812 files/s) | 3.05 s (16369 files/s) |
| Understand “what's taking the most space?” | 1.0 ms | 6.6 ms | 30.8 ms |
| Understand “find invoice acme” | 1.1 ms | 7.8 ms | 37.0 ms |
| Understand “delete all .log files older than 3 months” | 1.2 ms | 8.2 ms | 38.6 ms |
| Understand “delete my screenshots” | 1.2 ms | 9.0 ms | 45.8 ms |
| Understand “clean up node_modules and build artifacts” | 1.2 ms | 7.7 ms | 37.0 ms |
| Understand “remove the curseforge instances that are not the 26.2 version” | 1.0 ms | 5.4 ms | 23.9 ms |
| Understand “replace spaces with underscores in file names” | 1.9 ms | 15.2 ms | 75.8 ms |
| Understand “change all .txt files to .md” | 1.3 ms | 9.3 ms | 45.6 ms |
| Understand “delete duplicate files” (hashes contents) | 12.8 ms | 244.4 ms | 1.75 s |
| Trash one reviewed batch (approve, execute, journal, rescan) | 476.5 ms (500 files, 1049/s) | 846.5 ms (500 files, 591/s) | 2.91 s (500 files, 172/s) |
| Put that batch back from the Trash (incl. rescan) | 487.2 ms (500 files, 1026/s) | 929.3 ms (500 files, 538/s) | 2.92 s (500 files, 171/s) |
| Review a whole-folder Trash (tree walk + fingerprint) | 3.4 ms (748 files inside) | 71.8 ms (9748 files inside) | 405.2 ms (49748 files inside) |
| Execute it (one atomic move to the Trash) | 2.9 ms | 40.6 ms | 226.7 ms |
| Put the whole folder back | 0.3 ms | 0.4 ms | 0.5 ms |
| Move all PDFs into a new folder | 86.1 ms (67 files, 778/s) | 730.4 ms (500 files, 685/s) | 2.74 s (500 files, 183/s) |
| Undo that move | 40.1 ms (67 files) | 306.4 ms (500 files) | 314.9 ms (500 files) |

Reproduce: `cargo run --release --example benchmark -- --write docs/BENCHMARKS.md`. Timings vary by machine and disk; ratios between tiers are the useful signal.

## Reading the results

* **Understanding a request is effectively free.** The instant engine answers in about 1 ms at 1,000 files and about 30–75 ms at 50,000, scaling linearly with the index. The only slow request is duplicate detection because it reads file contents (it hashes only files that share a size, and stops at a 1 GiB read budget).
* **Executing changes is bounded by the filesystem and the journal, not by Tidy.** A reviewed batch is capped at 500 actions; roughly 1–2 ms per action at small scale. The larger-tier “Trash/Put back” times include a full rescan of the tree to refresh the index, which dominates at 50,000 files (~3 s); the app instead updates its index in place, so the interactive cost is closer to the 1k row.
* **Whole-folder actions are cheap regardless of size.** Reviewing a 50,000-file folder (walk + fingerprint) takes about 0.4 s, and moving it to the Trash — or putting it back — is a single atomic rename (well under a second).
* **Nothing is trusted between review and execution.** Each executed step re-fingerprints its source, which is why per-file throughput is in the hundreds to a thousand files per second rather than tens of thousands.

Scenario tests behind these numbers: `tests/e2e_large_scale.rs` (10 workflows on ~1,000 files) and `tests/e2e_mock_folder.rs` (11 workflows on a small tree).

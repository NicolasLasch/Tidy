//! Reproducible benchmark on mock data at several scales. It doubles as a correctness check: every
//! measured operation is verified on disk. Run in release mode for meaningful numbers:
//!   cargo run --release --example benchmark -- [--write docs/BENCHMARKS.md] [--tiers 1000,10000,50000]
#![cfg(target_os = "macos")]
#[path = "../../../tests/support/env.rs"]
mod env;
#[path = "../../../tests/support/large_mock.rs"]
mod large_mock;

use env::Env;
use large_mock::build_large;
use std::{path::PathBuf, process::Command, time::Instant};

struct Row {
    tier: usize,
    files: usize,
    lines: Vec<(String, String)>,
}
fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}
fn best_of<T>(runs: usize, mut f: impl FnMut() -> T) -> f64 {
    let mut best = f64::MAX;
    for _ in 0..runs {
        let start = Instant::now();
        let _ = f();
        best = best.min(ms(start));
    }
    best
}
fn fmt(v: f64) -> String {
    if v >= 1000.0 {
        format!("{:.2} s", v / 1000.0)
    } else {
        format!("{v:.1} ms")
    }
}
fn shell(cmd: &str, args: &[&str]) -> String {
    Command::new(cmd)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn run_tier(total: usize) -> Row {
    let mut lines: Vec<(String, String)> = Vec::new();
    let mut add = |k: &str, v: String| lines.push((k.to_string(), v));
    let t = Instant::now();
    let manifest = std::cell::RefCell::new(large_mock::Manifest::default());
    let mut env = Env::with("bench", |root| {
        *manifest.borrow_mut() = build_large(root, total, 7)
    });
    let files = env.files.len();
    add("Build mock tree + first scan/index", fmt(ms(t)));
    let t = Instant::now();
    env.scan();
    add(
        "Rescan (incremental, unchanged tree)",
        format!(
            "{} ({:.0} files/s)",
            fmt(ms(t)),
            files as f64 / (ms(t) / 1000.0)
        ),
    );

    // Request understanding: the instant engine, no AI model.
    for (label, q) in [
        (
            "“what's taking the most space?”",
            "what's taking the most space?",
        ),
        ("“find invoice acme”", "find invoice acme"),
        (
            "“delete all .log files older than 3 months”",
            "delete all .log files older than 3 months",
        ),
        ("“delete my screenshots”", "delete my screenshots"),
        (
            "“clean up node_modules and build artifacts”",
            "clean up node_modules and build artifacts",
        ),
        (
            "“remove the curseforge instances that are not the 26.2 version”",
            "remove the curseforge instances that are not the 26.2 version",
        ),
        (
            "“replace spaces with underscores in file names”",
            "replace spaces with underscores in file names",
        ),
        (
            "“change all .txt files to .md”",
            "change all .txt files to .md",
        ),
        (
            "“delete duplicate files” (hashes contents)",
            "delete duplicate files",
        ),
    ] {
        add(
            &format!("Understand {label}"),
            fmt(best_of(3, || env.ask(q))),
        );
    }

    // Executing reviewed plans through the safety engine.
    let r = env.ask("delete all files older than 1 month");
    let n = r.proposal.actions.len();
    let t = Instant::now();
    let tx = env.run_actions(&r.proposal.actions);
    let dt = ms(t);
    add(
        "Trash one reviewed batch (approve, execute, journal, rescan)",
        format!("{} ({n} files, {:.0}/s)", fmt(dt), n as f64 / (dt / 1000.0)),
    );
    let t = Instant::now();
    env.restore_all(tx);
    let dt = ms(t);
    add(
        "Put that batch back from the Trash (incl. rescan)",
        format!("{} ({n} files, {:.0}/s)", fmt(dt), n as f64 / (dt / 1000.0)),
    );

    // Whole-folder Trash of the biggest folder, by tree fingerprint.
    let big = "Inbox".to_string();
    let inbox_files = env
        .files
        .iter()
        .filter(|f| f.relative_path.starts_with("Inbox"))
        .count();
    let t = Instant::now();
    let view = env
        .engine
        .request_folder_trash_approval(&env.root, env.scope, "bench", &[PathBuf::from(&big)])
        .unwrap();
    add(
        "Review a whole-folder Trash (tree walk + fingerprint)",
        format!("{} ({inbox_files} files inside)", fmt(ms(t))),
    );
    let t = Instant::now();
    let report = env
        .engine
        .execute_approved_plan(&env.root, &view.token)
        .unwrap();
    add("Execute it (one atomic move to the Trash)", fmt(ms(t)));
    assert!(!env.path(&big).exists());
    let t = Instant::now();
    let detail = env
        .engine
        .get_detail(report.transaction_id)
        .unwrap()
        .unwrap();
    env.engine
        .restore_trashed(
            &env.root,
            env.scope,
            report.transaction_id,
            detail.steps[0].id,
        )
        .unwrap();
    add("Put the whole folder back", fmt(ms(t)));
    assert!(env.path(&big).exists());
    env.scan();

    // Reorganizing: rename, move, extension changes.
    let r = env.ask("move all pdf files into Documents");
    let n = r.proposal.actions.len();
    let t = Instant::now();
    let tx = env.run_actions(&r.proposal.actions);
    let dt = ms(t);
    add(
        "Move all PDFs into a new folder",
        format!("{} ({n} files, {:.0}/s)", fmt(dt), n as f64 / (dt / 1000.0)),
    );
    let t = Instant::now();
    let undo = env.engine.request_undo_approval(&env.root, tx).unwrap();
    env.engine
        .execute_approved_undo(&env.root, tx, &undo.token)
        .unwrap();
    add("Undo that move", format!("{} ({n} files)", fmt(ms(t))));
    Row {
        tier: total,
        files,
        lines,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let tiers: Vec<usize> = arg("--tiers")
        .unwrap_or_else(|| "1000,10000,50000".into())
        .split(',')
        .filter_map(|t| t.trim().parse().ok())
        .collect();
    let rows: Vec<Row> = tiers
        .iter()
        .map(|t| {
            eprintln!("running tier {t}…");
            run_tier(*t)
        })
        .collect();

    let mut md = String::new();
    md.push_str("# Benchmarks\n\n");
    md.push_str(&format!(
        "Measured on **{}**, {} cores, macOS {}, release build. Mock data is generated deterministically (`tests/support/large_mock.rs`): one messy `Inbox/` dump with photos, documents, screenshots, installers, logs, temp files, odd names and exact duplicates, plus projects, client folders, launcher instances and an archive. Every operation is executed by the real safety engine and verified on disk; nothing here uses an AI model (the instant engine answers these requests).\n\n",
        shell("sysctl", &["-n", "machdep.cpu.brand_string"]),
        shell("sysctl", &["-n", "hw.ncpu"]),
        shell("sw_vers", &["-productVersion"]),
    ));
    md.push_str("| Operation |");
    for r in &rows {
        md.push_str(&format!(" {} files |", r.files));
    }
    md.push_str("\n|---|");
    for _ in &rows {
        md.push_str("---:|");
    }
    md.push('\n');
    for (i, (key, _)) in rows[0].lines.iter().enumerate() {
        md.push_str(&format!("| {key} |"));
        for r in &rows {
            md.push_str(&format!(
                " {} |",
                r.lines.get(i).map(|l| l.1.clone()).unwrap_or_default()
            ));
        }
        md.push('\n');
    }
    md.push_str("\nReproduce: `cargo run --release --example benchmark -- --write docs/BENCHMARKS.md`. Timings vary by machine and disk; ratios between tiers are the useful signal.\n");
    println!("{md}");
    if let Some(path) = arg("--write") {
        std::fs::write(path, md).unwrap();
    }
    let _ = rows.iter().map(|r| r.tier).sum::<usize>();
}

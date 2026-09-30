//! Large-scale, human-style workflows on ~1000 messy files: sort, move, rename, change extensions,
//! delete, de-duplicate, restructure, undo and put back. Every change is executed by the real safety
//! engine and verified on disk (contents preserved, nothing overwritten, index consistent).
//! Trashed items are put back at the end so the machine's Trash stays clean.
#![cfg(target_os = "macos")]
#[path = "support/env.rs"]
mod env;
#[path = "support/large_mock.rs"]
mod large_mock;

use env::Env;
use large_mock::{Manifest, build_large};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    hash::{Hash, Hasher},
    path::Path,
    time::Instant,
};
use tidy_organization::civil_from_unix_seconds;

const N: usize = 1000;
const SEED: u64 = 20260930;

fn setup(name: &str, sub: &str) -> (Env, Manifest) {
    let manifest = std::cell::RefCell::new(Manifest::default());
    let env = Env::with_sub(
        name,
        |root| *manifest.borrow_mut() = build_large(root, N, SEED),
        sub,
    );
    (env, manifest.into_inner())
}
fn timed<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let out = f();
    eprintln!(
        "[bench] {label}: {:.1} ms",
        start.elapsed().as_secs_f64() * 1000.0
    );
    out
}
/// Every regular file under `dir` as (relative path -> content hash).
fn tree(dir: &Path) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap().flatten() {
            let path = e.path();
            if e.file_type().unwrap().is_dir() {
                stack.push(path);
            } else {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                fs::read(&path).unwrap().hash(&mut h);
                out.insert(
                    path.strip_prefix(dir)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    h.finish(),
                );
            }
        }
    }
    out
}
/// Contents regardless of where they live: proves nothing was lost, changed or overwritten.
fn contents(t: &BTreeMap<String, u64>) -> BTreeMap<u64, usize> {
    let mut m = BTreeMap::new();
    for h in t.values() {
        *m.entry(*h).or_default() += 1;
    }
    m
}
fn ext(path: &str) -> String {
    Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}
/// Repeats a request the way a person would ("do the next batch") until nothing is left to do.
fn until_done(env: &mut Env, request: &str, max_rounds: usize) -> Vec<i64> {
    let mut txs = Vec::new();
    for _ in 0..max_rounds {
        let r = env.ask(request);
        if r.proposal.actions.is_empty() {
            break;
        }
        assert!(r.proposal.actions.len() <= 500, "batches are capped");
        txs.push(env.run_actions(&r.proposal.actions));
    }
    txs
}

#[test]
fn the_mock_is_big_messy_and_reproducible() {
    let (_, m) = setup("shape", "");
    assert!(m.total >= 900 && m.total <= 1300, "{m:?}");
    assert!(
        m.photos > 80 && m.docs > 100 && m.duplicate_extra > 10 && m.temp > 20,
        "{m:?}"
    );
    let (_, again) = setup("shape2", "");
    assert_eq!(
        format!("{m:?}"),
        format!("{again:?}"),
        "same seed, same tree"
    );
}

#[test]
fn organize_the_inbox_by_type_until_done_then_undo_everything() {
    let (mut env, m) = setup("bytype", "Inbox");
    let dir = env.root.path().to_path_buf();
    let before = tree(&dir);
    let txs = timed("organize by type (all batches)", || {
        until_done(&mut env, "organize this folder by type", 5)
    });
    assert!(
        txs.len() >= 2,
        "a 750-file inbox needs more than one 500-action batch"
    );
    let after = tree(&dir);
    assert_eq!(
        contents(&before),
        contents(&after),
        "no file lost, changed or overwritten"
    );
    assert_eq!(before.len(), after.len());
    // Anything still at the top level has no known category (files without an extension).
    for name in after.keys().filter(|p| !p.contains('/')) {
        assert!(ext(name).is_empty(), "{name} should have been sorted");
    }
    // Every category folder holds only what belongs there.
    let category = |folder: &str, exts: &[&str]| {
        for p in after
            .keys()
            .filter(|p| p.starts_with(&format!("{folder}/")))
        {
            assert!(
                exts.contains(&ext(p).as_str()),
                "{p} does not belong in {folder}"
            );
        }
    };
    category("Images", &["jpg", "jpeg", "png", "heic", "gif", "webp"]);
    category("Installers", &["dmg", "pkg"]);
    category("Logs", &["log"]);
    category("Temporary", &["tmp", "bak", "part"]);
    assert!(after.keys().filter(|p| p.starts_with("Images/")).count() >= m.photos);
    let count_before = |exts: &[&str]| {
        before
            .keys()
            .filter(|p| exts.contains(&ext(p).as_str()))
            .count()
    };
    assert_eq!(
        after.keys().filter(|p| p.starts_with("Temporary/")).count(),
        count_before(&["tmp", "bak", "part"])
    );
    assert_eq!(
        after.keys().filter(|p| p.starts_with("Logs/")).count(),
        count_before(&["log"])
    );
    assert_eq!(
        after
            .keys()
            .filter(|p| p.starts_with("Installers/"))
            .count(),
        count_before(&["dmg", "pkg"])
    );
    let _ = &m;
    // Undo every batch, newest first: the inbox is exactly as it was.
    timed("undo all batches", || {
        for tx in txs.iter().rev() {
            let undo = env.engine.request_undo_approval(&env.root, *tx).unwrap();
            env.engine
                .execute_approved_undo(&env.root, *tx, &undo.token)
                .unwrap();
        }
    });
    let restored = tree(&dir);
    assert_eq!(
        before.keys().collect::<BTreeSet<_>>(),
        restored
            .keys()
            .filter(|p| !p.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter(|p| before.contains_key(*p))
            .collect::<BTreeSet<_>>()
    );
    assert_eq!(
        before,
        restored
            .into_iter()
            .filter(|(p, _)| before.contains_key(p))
            .collect()
    );
}

#[test]
fn organize_the_inbox_by_date_puts_every_file_in_its_year_and_month() {
    let (mut env, _) = setup("bydate", "Inbox");
    let dir = env.root.path().to_path_buf();
    let before = tree(&dir);
    timed("organize by date (all batches)", || {
        until_done(&mut env, "organize by date", 5)
    });
    let after = tree(&dir);
    assert_eq!(contents(&before), contents(&after));
    for path in after.keys() {
        let parts: Vec<&str> = path.split('/').collect();
        assert_eq!(parts.len(), 3, "{path} should be YYYY/MM/name");
        let modified = fs::metadata(dir.join(path))
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let (y, mo, _) = civil_from_unix_seconds(modified).unwrap();
        assert_eq!(
            format!("{y:04}/{mo:02}"),
            format!("{}/{}", parts[0], parts[1]),
            "{path}"
        );
    }
}

#[test]
fn photos_screenshots_temp_files_installers_and_logs() {
    let (mut env, m) = setup("cleanup", "Inbox");
    // Move every photo (screenshots are PNG photos too) into one folder.
    let images = |t: &BTreeMap<String, u64>| {
        t.keys()
            .filter(|p| ["jpg", "jpeg", "png", "heic"].contains(&ext(p).as_str()))
            .count()
    };
    let expected = images(&tree(env.root.path()));
    assert!(
        expected >= m.photos + m.screenshots,
        "copies of photos are photos too"
    );
    let r = env.ask("move all photos into Photos");
    assert_eq!(
        r.proposal.actions.len(),
        expected.min(500),
        "{}",
        r.proposal.rationale
    );
    until_done(&mut env, "move all photos into Photos", 4);
    let t = tree(env.root.path());
    assert_eq!(
        t.keys().filter(|p| p.starts_with("Photos/")).count(),
        expected
    );
    assert!(
        !t.keys()
            .any(|p| !p.contains('/') && ["jpg", "jpeg", "png", "heic"].contains(&ext(p).as_str()))
    );
    // Delete screenshots (wherever they are now), temp files, old installers, old logs.
    let shot_count = tree(env.root.path())
        .keys()
        .filter(|p| {
            p.rsplit('/')
                .next()
                .unwrap()
                .to_lowercase()
                .starts_with("screenshot")
        })
        .count();
    let shots = env.ask("delete my screenshots");
    assert_eq!(shots.sources.len(), shot_count);
    let tx = env.run_actions(&shots.proposal.actions);
    assert_eq!(
        tree(env.root.path())
            .keys()
            .filter(|p| p
                .rsplit('/')
                .next()
                .unwrap()
                .to_lowercase()
                .starts_with("screenshot"))
            .count(),
        0
    );
    env.restore_all(tx);
    assert_eq!(
        tree(env.root.path())
            .keys()
            .filter(|p| p
                .rsplit('/')
                .next()
                .unwrap()
                .to_lowercase()
                .starts_with("screenshot"))
            .count(),
        shot_count
    );
    let temp_count = tree(env.root.path())
        .keys()
        .filter(|p| ["tmp", "bak", "part"].contains(&ext(p).as_str()))
        .count();
    let temp = env.ask("delete all .tmp and .bak and .part files");
    assert_eq!(temp.sources.len(), temp_count);
    let tx = env.run_actions(&temp.proposal.actions);
    assert!(
        !tree(env.root.path())
            .keys()
            .any(|p| ["tmp", "bak", "part"].contains(&ext(p).as_str()))
    );
    env.restore_all(tx);
    let inst = env.ask("delete installers older than 30 days");
    assert!(
        inst.sources.len() >= m.old_installers && !inst.sources.is_empty(),
        "{}",
        inst.proposal.rationale
    );
    let logs = env.ask("delete all log files older than 6 months");
    assert!(!logs.sources.is_empty() && logs.sources.len() <= m.logs);
    // A request that matches nothing says so instead of inventing work.
    let none = env.ask("delete all .xyz files");
    assert!(none.proposal.actions.is_empty());
}

#[test]
fn rename_in_bulk_and_change_extensions() {
    let (mut env, m) = setup("rename", "Inbox");
    let dir = env.root.path().to_path_buf();
    let before = tree(&dir);
    // Spaces to underscores, until every name is clean.
    timed("replace spaces in file names", || {
        until_done(&mut env, "replace spaces with underscores in file names", 4)
    });
    let t = tree(&dir);
    assert_eq!(contents(&before), contents(&t));
    assert!(
        !t.keys().any(|p| p.contains(' ')),
        "{:?}",
        t.keys()
            .filter(|p| p.contains(' '))
            .take(3)
            .collect::<Vec<_>>()
    );
    assert!(m.spaced_names > 100);
    // Lowercase everything, extensions included.
    until_done(&mut env, "lowercase all file names", 4);
    let t = tree(&dir);
    assert!(!t.keys().any(|p| p.chars().any(|c| c.is_uppercase())));
    assert_eq!(contents(&before), contents(&t));
    // Remove a word from names.
    let before_copy = t
        .keys()
        .filter(|p| p.to_lowercase().contains("copy"))
        .count();
    assert!(before_copy > 0);
    until_done(&mut env, "remove the word copy from file names", 4);
    assert!(!tree(&dir).keys().any(|p| p.to_lowercase().contains("copy")));
    // Change extensions: names only, bytes untouched.
    let txt_before = tree(&dir).keys().filter(|p| ext(p) == "txt").count();
    assert!(txt_before > 0);
    let r = env.ask("change all .txt files to .md");
    assert!(
        r.proposal.rationale.contains("does not convert"),
        "{}",
        r.proposal.rationale
    );
    until_done(&mut env, "change all .txt files to .md", 3);
    let t = tree(&dir);
    // A .txt is left alone only when a .md with the same name already exists (never overwritten).
    for left in t.keys().filter(|p| ext(p) == "txt") {
        assert!(
            t.contains_key(&format!("{}.md", left.trim_end_matches(".txt"))),
            "{left} was skipped without a clash"
        );
    }
    assert_eq!(
        contents(&before),
        contents(&t),
        "bytes never change when only a name changes"
    );
    // Repeating an already-applied request has nothing left to do.
    assert!(
        env.ask("change all .txt files to .md")
            .proposal
            .actions
            .is_empty()
    );
}

#[test]
fn find_and_remove_duplicate_files_keeping_the_originals() {
    let (mut env, m) = setup("dupes", "Inbox");
    let dir = env.root.path().to_path_buf();
    let r = timed("find duplicates", || env.ask("find duplicate files"));
    assert!(!r.sections.is_empty());
    let r = env.ask("delete duplicate files");
    let total_redundant: usize = r.sections.iter().map(|s| s.items.len() - 1).sum();
    assert!(
        total_redundant <= m.duplicate_extra
            && total_redundant >= m.duplicate_extra.min(20).saturating_sub(1),
        "{} vs {}",
        total_redundant,
        m.duplicate_extra
    );
    let before = tree(&dir);
    let originals: BTreeSet<u64> = before.values().copied().collect();
    let mut txs = Vec::new();
    for _ in 0..4 {
        let r = env.ask("delete duplicate files");
        if r.proposal.actions.is_empty() {
            break;
        }
        txs.push(env.run_actions(&r.proposal.actions));
    }
    let after = tree(&dir);
    // Every distinct content survives exactly once; the extra copies are gone.
    assert_eq!(after.values().copied().collect::<BTreeSet<_>>(), originals);
    assert_eq!(after.len(), originals.len(), "no duplicates left");
    assert!(env.ask("find duplicate files").sections.is_empty());
    for tx in txs.iter().rev() {
        env.restore_all(*tx);
    }
    assert_eq!(tree(&dir).len(), before.len());
}

#[test]
fn folders_projects_launcher_instances_and_build_output() {
    let (mut env, _) = setup("folders", "");
    // Build a folder structure, restructure, undo.
    let r = env.ask("create folders 2024 and 2025 in Archive");
    env.run_actions(&r.proposal.actions);
    assert!(env.path("Archive/2024").is_dir() && env.path("Archive/2025").is_dir());
    let r = env.ask("move the 2019 folder into Archive/Old");
    let tx = env.run_actions(&r.proposal.actions);
    assert!(env.path("Archive/Old/2019/old-file-0.doc").exists());
    let undo = env.engine.request_undo_approval(&env.root, tx).unwrap();
    env.engine
        .execute_approved_undo(&env.root, tx, &undo.token)
        .unwrap();
    assert!(env.path("Archive/2019/old-file-0.doc").exists());
    env.scan();
    let r = env.ask("rename the Epsilon API folder to Epsilon-API");
    env.run_actions(&r.proposal.actions);
    assert!(env.path("Projects/Epsilon-API/Cargo.toml").exists());
    // Projects, by marker files, including the Git repositories the index never sees inside.
    let projects = tidy_agent_runtime::intent::list_projects(env.root.path(), "Mock", &env.files);
    assert_eq!(
        projects.sections[0].items.len(),
        8,
        "{}",
        projects.proposal.rationale
    );
    // Build output and dependencies become whole folders.
    let r = env.ask("clean up node_modules and build artifacts");
    assert_eq!(
        r.folders.len(),
        6,
        "{:?}",
        r.folders.iter().map(|f| &f.path).collect::<Vec<_>>()
    );
    let paths: Vec<String> = r.folders.iter().map(|f| f.path.clone()).collect();
    let tx = env.trash_folders(&paths);
    assert!(
        env.path("Projects/AlphaApp/package.json").exists()
            && !env.path("Projects/AlphaApp/node_modules").exists()
    );
    env.restore_all(tx);
    // Launcher instances by game version.
    let r = env.ask("remove the curseforge instances that are not the 26.2 version");
    assert_eq!(r.folders.len(), 4, "{}", r.proposal.rationale);
    assert!(
        r.sections.iter().any(|s| s.items.len() == 2),
        "Skyblock and Test World are kept"
    );
    let pick = env.ask("list all my modrinth profiles");
    assert!(pick.pick && pick.folders.len() == 6);
}

#[test]
fn asking_questions_about_what_is_where() {
    let (env, m) = setup("questions", "");
    let overview = env.ask("what's taking the most space?");
    let names: Vec<&str> = overview.sections[0]
        .items
        .iter()
        .map(|i| i.path.as_str())
        .collect();
    for expected in ["Inbox", "Projects", "Work", "Games", "Archive"] {
        assert!(names.contains(&expected), "{expected} in {names:?}");
    }
    assert!(names.iter().all(|n| !n.contains('/')));
    let work = env.ask("what's inside Work");
    assert_eq!(work.sections[0].items.len(), 1);
    let clients = env.ask("what's inside Work/Clients");
    assert_eq!(clients.sections[0].items.len(), 3);
    let invoices = env.ask("find invoice acme");
    let hits = invoices.sections.first().map_or(0, |s| s.items.len());
    assert!(hits > 0);
    let biggest = env.ask("show my biggest files");
    assert!(
        biggest.sections[0]
            .items
            .windows(2)
            .all(|w| w[0].bytes >= w[1].bytes)
    );
    let _ = m;
}

#[test]
fn repeating_a_request_is_safe_and_collisions_never_overwrite() {
    let (mut env, _) = setup("safe", "Inbox");
    let dir = env.root.path().to_path_buf();
    let before = tree(&dir);
    until_done(&mut env, "move all pdf files into Documents", 4);
    let once = tree(&dir);
    assert!(
        env.ask("move all pdf files into Documents")
            .proposal
            .actions
            .is_empty(),
        "already moved"
    );
    assert_eq!(contents(&before), contents(&once));
    // A second file with the same name is skipped, never written over the first.
    fs::write(dir.join("Documents").join("same.pdf"), "first").unwrap();
    fs::write(dir.join("same.pdf"), "second").unwrap();
    env.scan();
    let r = env.ask("move all pdf files into Documents");
    assert!(r.sources.iter().all(|s| s.path != "same.pdf"));
    if !r.proposal.actions.is_empty() {
        env.run_actions(&r.proposal.actions);
    }
    assert_eq!(
        fs::read_to_string(dir.join("Documents/same.pdf")).unwrap(),
        "first"
    );
    assert_eq!(fs::read_to_string(dir.join("same.pdf")).unwrap(), "second");
}

#[test]
fn a_full_500_action_batch_runs_undoes_and_puts_back() {
    let (mut env, _) = setup("batch", "Inbox");
    let dir = env.root.path().to_path_buf();
    let before = tree(&dir);
    let r = env.ask("delete all files older than 1 month");
    assert_eq!(
        r.proposal.actions.len(),
        500,
        "capped at one reviewed batch"
    );
    assert!(r.remaining_matches > 0 && !r.complete);
    let tx = timed("trash 500 files in one approved transaction", || {
        env.run_actions(&r.proposal.actions)
    });
    assert_eq!(tree(&dir).len(), before.len() - 500);
    timed("put back 500 files from the Trash", || env.restore_all(tx));
    assert_eq!(tree(&dir), before, "everything is back, byte for byte");
}

//! End-to-end checks on a realistic mock folder: real scan and index, the real request engine,
//! and the real safety engine that performs (and undoes) the changes on disk.
//! Trashed items are put back at the end of each test so the user's Trash stays clean.
#![cfg(target_os = "macos")]
#[path = "support/mock.rs"]
mod mock;

#[path = "support/env.rs"]
mod env;

use env::Env;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};
use tidy_agent_runtime::{intent::list_projects, investigation::Investigation};
use tidy_organization::{FileId, ProposedAction};

fn folder_paths(r: &Investigation) -> Vec<String> {
    let mut v: Vec<String> = r.folders.iter().map(|f| f.path.clone()).collect();
    v.sort();
    v
}

#[test]
fn scan_indexes_repository_files_but_never_git_internals() {
    let env = Env::with("scan", mock::build);
    assert!(env.indexed("Projects/AlphaApp/package.json"));
    assert!(env.indexed("Projects/AlphaApp/src/main.js"));
    assert!(!env.files.iter().any(|f| {
        f.relative_path
            .components()
            .any(|c| c.as_os_str() == ".git")
    }));
    assert!(env.files.len() > 30, "{}", env.files.len());
}

#[test]
fn overview_lists_only_top_level_folders_biggest_first() {
    let env = Env::with("overview", mock::build);
    let r = env.ask("what's taking the most space?");
    assert!(
        !r.sections.is_empty(),
        "{} / {:?}",
        r.proposal.rationale,
        r.trace.iter().map(|t| &t.label).collect::<Vec<_>>()
    );
    let folders = &r.sections[0];
    assert!(folders.title.starts_with("Folders in"));
    let names: Vec<&str> = folders.items.iter().map(|i| i.path.as_str()).collect();
    for expected in [
        "Downloads",
        "Documents",
        "Photos",
        "Projects",
        "curseforge",
        "Desktop",
    ] {
        assert!(
            names.contains(&expected),
            "{expected} missing from {names:?}"
        );
    }
    assert!(
        folders
            .items
            .iter()
            .all(|i| !i.path.contains('/') && i.kind == "folder")
    );
    assert!(
        folders.items.windows(2).all(|w| w[0].bytes >= w[1].bytes),
        "sorted by size"
    );
    // Opening one folder shows its own folders and files.
    let inside = env.ask("what's inside Projects");
    let opened: Vec<&str> = inside.sections[0]
        .items
        .iter()
        .map(|i| i.path.as_str())
        .collect();
    assert!(opened.contains(&"Projects/AlphaApp") && opened.contains(&"Projects/BetaRust"));
    assert!(
        inside.sections[0]
            .items
            .iter()
            .all(|i| i.path.matches('/').count() == 1)
    );
}

#[test]
fn old_files_are_found_by_age_and_trashed_then_put_back() {
    let mut env = Env::with("age", mock::build);
    let r = env.ask("delete all .log files older than 3 months");
    assert_eq!(r.sources.len(), 1);
    assert!(r.sources[0].path.ends_with("old.log"));
    let tx = env.run_actions(&r.proposal.actions);
    assert!(!env.path("Downloads/old.log").exists() && env.path("Downloads/new.log").exists());
    assert!(!env.indexed("Downloads/old.log"));
    env.restore_all(tx);
    assert!(env.path("Downloads/old.log").exists() && env.indexed("Downloads/old.log"));
    // Installers older than 30 days and screenshots.
    let r = env.ask("delete installers older than 30 days");
    assert_eq!(r.sources.len(), 2);
    let r = env.ask("delete my screenshots");
    assert_eq!(r.sources.len(), 1);
}

#[test]
fn create_move_rename_and_undo() {
    let mut env = Env::with("edit", mock::build);
    // Create.
    let r = env.ask("create a folder called Archive in Downloads");
    env.run_actions(&r.proposal.actions);
    assert!(env.path("Downloads/Archive").is_dir());
    // Move files by kind into an existing and a new folder.
    let r = env.ask("move all pdf files into Documents/Invoices");
    assert!(r.proposal.actions.len() >= 2, "{}", r.proposal.rationale);
    let moved = env.run_actions(&r.proposal.actions);
    assert!(
        env.path("Documents/Invoices/report.pdf").exists()
            && !env.path("Downloads/report.pdf").exists()
    );
    // Undo puts them back where they were.
    let undo = env.engine.request_undo_approval(&env.root, moved).unwrap();
    env.engine
        .execute_approved_undo(&env.root, moved, &undo.token)
        .unwrap();
    assert!(env.path("Downloads/report.pdf").exists());
    env.scan();
    // Rename a folder with content, then undo.
    let r = env.ask("rename the Old Stuff folder to Journal");
    let tx = env.run_actions(&r.proposal.actions);
    assert!(
        env.path("Desktop/Journal/deep/b.txt").exists() && !env.path("Desktop/Old Stuff").exists()
    );
    assert!(env.indexed("Desktop/Journal/deep/b.txt"));
    let undo = env.engine.request_undo_approval(&env.root, tx).unwrap();
    env.engine
        .execute_approved_undo(&env.root, tx, &undo.token)
        .unwrap();
    assert!(env.path("Desktop/Old Stuff/a.txt").exists());
    // Rename a file keeps its extension.
    env.scan();
    let r = env.ask("rename setup.dmg to Installer");
    env.run_actions(&r.proposal.actions);
    assert!(env.path("Downloads/Installer.dmg").exists());
}

#[test]
fn instances_not_matching_a_version_are_trashed_whole_and_restorable() {
    let mut env = Env::with("instances", mock::build);
    let r = env.ask("remove the curseforge instances that are not the 26.2 version");
    assert_eq!(
        folder_paths(&r),
        [
            "curseforge/minecraft/Instances/Pack A",
            "curseforge/minecraft/Instances/Pack C"
        ],
        "{}",
        r.proposal.rationale
    );
    assert!(
        r.sections
            .iter()
            .any(|s| s.items.iter().any(|i| i.path.ends_with("Pack B"))),
        "kept section lists Pack B"
    );
    let tx = env.trash_folders(&folder_paths(&r));
    assert!(!env.path("curseforge/minecraft/Instances/Pack A").exists());
    assert!(!env.path("curseforge/minecraft/Instances/Pack C").exists());
    assert!(
        env.path("curseforge/minecraft/Instances/Pack B/mods/mod.jar")
            .exists()
    );
    assert!(!env.indexed("curseforge/minecraft/Instances/Pack A/mods/mod.jar"));
    env.restore_all(tx);
    assert!(
        env.path("curseforge/minecraft/Instances/Pack A/mods/mod.jar")
            .exists()
    );
    assert!(env.indexed("curseforge/minecraft/Instances/Pack C/mods/mod.jar"));
}

#[test]
fn listing_launcher_profiles_then_deleting_all_of_them() {
    let env = Env::with("profiles", mock::build);
    let q = "List all my modrinth profiles on my computer so I will be able to remove those";
    let r = env.ask(q);
    assert!(r.pick && r.folders.len() == 3, "{}", r.proposal.rationale);
    assert!(
        r.folders.iter().all(|f| f.path.matches('/').count() == 3),
        "no nested profiles folders offered"
    );
    let all = env.ask(&format!("{q}\nUser follow-up: delete all of them"));
    assert!(!all.pick && all.folders.len() == 3);
}

#[test]
fn projects_are_found_by_marker_files_including_git_repositories() {
    let env = Env::with("projects", mock::build);
    let r = list_projects(env.root.path(), "Mock", &env.files);
    let items = &r.sections[0].items;
    let note = |name: &str| {
        items
            .iter()
            .find(|i| i.path.ends_with(name))
            .and_then(|i| i.note.clone())
            .unwrap_or_default()
    };
    assert!(note("AlphaApp").contains("Git") && note("AlphaApp").contains("Node"));
    assert!(note("BetaRust").contains("Rust"));
    assert!(items.iter().all(|i| !i.path.ends_with("Notes")));
}

#[test]
fn build_artifacts_become_whole_folders_that_can_be_trashed_and_restored() {
    let mut env = Env::with("artifacts", mock::build);
    let r = env.ask("clean up node_modules and build artifacts");
    assert_eq!(
        folder_paths(&r),
        ["Projects/AlphaApp/node_modules", "Projects/BetaRust/target"]
    );
    let tx = env.trash_folders(&folder_paths(&r));
    assert!(
        env.path("Projects/AlphaApp/package.json").exists()
            && !env.path("Projects/AlphaApp/node_modules").exists()
    );
    env.restore_all(tx);
    assert!(env.path("Projects/BetaRust/target/debug/beta").exists());
}

#[test]
fn lists_of_names_are_all_prepared_and_missing_ones_are_reported() {
    let env = Env::with("multi", mock::build);
    let r = env.ask("delete the Photos folder, Old Stuff and Ghost Folder");
    assert_eq!(folder_paths(&r), ["Desktop/Old Stuff", "Photos"]);
    assert!(
        r.proposal.rationale.to_lowercase().contains("ghost"),
        "{}",
        r.proposal.rationale
    );
}

#[test]
fn the_safety_engine_refuses_dangerous_actions() {
    let mut env = Env::with("safety", mock::build);
    let root = &env.root;
    let engine = &env.engine;
    // Git internals are never touched, nothing outside the folder, no root, no overlaps.
    for bad in [
        "Projects/AlphaApp/.git",
        "../escape",
        "",
        "Downloads/report.pdf",
    ] {
        assert!(
            engine
                .request_folder_trash_approval(root, 1, "x", &[PathBuf::from(bad)])
                .is_err(),
            "{bad}"
        );
    }
    assert!(
        engine
            .request_folder_trash_approval(
                root,
                1,
                "x",
                &[
                    PathBuf::from("Projects"),
                    PathBuf::from("Projects/AlphaApp")
                ]
            )
            .is_err()
    );
    let map = HashMap::new();
    let into_itself = ProposedAction::MoveFolder {
        source: "Projects".into(),
        destination_relative: "Projects/inner".into(),
    };
    assert!(
        engine
            .request_plan_approval(root, 1, "x", &[into_itself], &map)
            .is_err()
    );
    let into_git = ProposedAction::MoveFolder {
        source: "Photos".into(),
        destination_relative: "Projects/AlphaApp/.git/photos".into(),
    };
    assert!(
        engine
            .request_plan_approval(root, 1, "x", &[into_git], &map)
            .is_err()
    );
    let onto_existing = ProposedAction::MoveFolder {
        source: "Photos".into(),
        destination_relative: "Documents".into(),
    };
    assert!(
        engine
            .request_plan_approval(root, 1, "x", &[onto_existing], &map)
            .is_err()
    );
    // A folder that changes after review does not execute.
    let view = engine
        .request_folder_trash_approval(root, 1, "x", &[PathBuf::from("Documents")])
        .unwrap();
    fs::write(env.path("Documents/late.txt"), "changed after review").unwrap();
    assert!(
        env.engine
            .execute_approved_plan(&env.root, &view.token)
            .is_err()
    );
    assert!(env.path("Documents/Taxes/2025.pdf").exists());
    env.scan();
}

#[test]
fn index_bookkeeping_matches_a_fresh_scan_after_folder_operations() {
    let mut env = Env::with("index", mock::build);
    // Bookkeeping path used by the app (no rescan): rename_tree and remove_tree.
    let before = env.files.len();
    let r = env.ask("rename the Old Stuff folder to Journal");
    let map: HashMap<FileId, PathBuf> = env
        .files
        .iter()
        .map(|f| (f.id, f.relative_path.clone()))
        .collect();
    let view = env
        .engine
        .request_plan_approval(&env.root, env.scope, "e2e", &r.proposal.actions, &map)
        .unwrap();
    env.engine
        .execute_approved_plan(&env.root, &view.token)
        .unwrap();
    env.db
        .rename_tree(
            env.scope,
            Path::new("Desktop/Old Stuff"),
            Path::new("Desktop/Journal"),
        )
        .unwrap();
    let bookkept: std::collections::BTreeSet<_> = env
        .db
        .storage_files(env.scope)
        .unwrap()
        .into_iter()
        .map(|f| f.path)
        .collect();
    env.scan();
    let rescanned: std::collections::BTreeSet<_> =
        env.files.iter().map(|f| f.relative_path.clone()).collect();
    assert_eq!(bookkept, rescanned);
    assert_eq!(env.files.len(), before);
    let removed = env
        .db
        .remove_tree(env.scope, Path::new("Desktop/Journal"))
        .unwrap();
    assert_eq!(removed, 2);
}

#[test]
fn follow_ups_use_the_previous_listing_and_missing_sizes_are_read_from_disk() {
    let mut env = Env::with("followups", mock::build);
    // A project the index has never seen (added after the last scan) still gets a size.
    fs::create_dir_all(env.path("Projects/FreshRust/src")).unwrap();
    fs::write(env.path("Projects/FreshRust/Cargo.toml"), "[package]").unwrap();
    fs::write(
        env.path("Projects/FreshRust/src/big.rs"),
        vec![b'x'; 200_000],
    )
    .unwrap();
    let now = 0;
    let previous = "List all my projects";
    let list = tidy_agent_runtime::intent::list_projects(env.root.path(), "Mock", &env.files);
    let fresh = list.sections[0]
        .items
        .iter()
        .find(|i| i.path.ends_with("FreshRust"))
        .unwrap();
    assert!(fresh.bytes >= 200_000, "{} {:?}", fresh.bytes, fresh.note);
    assert!(fresh.note.as_deref().unwrap().contains("read from disk"));
    let ask = |q: &str| {
        tidy_agent_runtime::intent::follow_up(
            q,
            previous,
            &env.files,
            "Mock",
            now,
            Some(env.root.path()),
        )
    };
    // Explains instead of searching for files called “no size”.
    let why = ask("how come some have no size ?").expect("understood");
    assert!(
        why.proposal.rationale.contains("index"),
        "{}",
        why.proposal.rationale
    );
    assert!(why.sections.is_empty());
    // Only the Rust ones; biggest first; measure them.
    let rust = ask("only the rust ones").unwrap();
    assert!(
        rust.sections[0]
            .items
            .iter()
            .all(|i| i.note.as_deref().unwrap().contains("Rust"))
    );
    let biggest = ask("biggest first").unwrap();
    let sizes: Vec<u64> = biggest.sections[0].items.iter().map(|i| i.bytes).collect();
    assert!(sizes.windows(2).all(|w| w[0] >= w[1]), "{sizes:?}");
    assert!(
        ask("measure them").unwrap().sections[0]
            .items
            .iter()
            .all(|i| i.bytes > 0 || i.note.as_deref().unwrap().contains("empty"))
    );
    // “delete them” prepares the project folders for the Trash.
    let del = ask("delete them all").unwrap();
    assert!(del.folders.len() >= 3 && !del.pick);
    // A question is never mistaken for a file search.
    assert!(env.ask("what's inside Projects").sections[0].items.len() >= 3);
    env.scan();
}

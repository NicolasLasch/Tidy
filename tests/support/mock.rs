//! A realistic mock "Documents" tree for end-to-end tests and manual trials in the app.
//! Everything here is small; mtimes are set so age-based requests have something to find.
use std::{
    fs::{self, File},
    io::Write,
    path::Path,
    time::{Duration, SystemTime},
};

fn write(root: &Path, rel: &str, bytes: usize) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut f = File::create(&path).unwrap();
    let chunk = vec![b'x'; 8192];
    let mut left = bytes;
    while left > 0 {
        let n = left.min(chunk.len());
        f.write_all(&chunk[..n]).unwrap();
        left -= n;
    }
}
fn text(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}
fn age(root: &Path, rel: &str, days: u64) {
    let when = SystemTime::now() - Duration::from_secs(days * 86_400);
    File::options()
        .write(true)
        .open(root.join(rel))
        .unwrap()
        .set_modified(when)
        .unwrap();
}
pub fn build(root: &Path) {
    // Downloads: installers, logs, screenshots, duplicates.
    write(root, "Downloads/setup.dmg", 40_000);
    write(root, "Downloads/installer-old.pkg", 30_000);
    age(root, "Downloads/setup.dmg", 400);
    age(root, "Downloads/installer-old.pkg", 200);
    write(root, "Downloads/old.log", 2_000);
    age(root, "Downloads/old.log", 300);
    write(root, "Downloads/new.log", 2_000);
    write(root, "Downloads/report.pdf", 5_000);
    write(root, "Downloads/Report Final.pdf", 6_000);
    write(
        root,
        "Downloads/Screenshot 2026-01-01 at 10.00.00.png",
        9_000,
    );
    write(root, "Downloads/photo.jpg", 12_000);
    text(root, "Downloads/notes.txt", "remember the milk");
    text(root, "Downloads/dup-a.txt", "same bytes");
    text(root, "Downloads/dup-b.txt", "same bytes");
    write(root, "Downloads/big.bin", 3_000_000);
    // Documents.
    write(root, "Documents/Invoices/invoice-2026-01.pdf", 7_000);
    write(root, "Documents/Invoices/invoice-2026-02.pdf", 7_500);
    write(root, "Documents/Taxes/2025.pdf", 20_000);
    text(root, "Documents/todo.txt", "ship it");
    write(root, "Photos/2024/a.jpg", 15_000);
    write(root, "Photos/2024/b.jpg", 16_000);
    write(root, "Photos/2025/c.png", 18_000);
    // Projects: a Git repo with dependencies, a Rust crate with build output, plain notes.
    text(root, "Projects/AlphaApp/.git/HEAD", "ref: refs/heads/main");
    text(root, "Projects/AlphaApp/.git/config", "[core]");
    text(
        root,
        "Projects/AlphaApp/package.json",
        "{\"name\":\"alpha\"}",
    );
    text(root, "Projects/AlphaApp/src/main.js", "console.log(1)");
    write(
        root,
        "Projects/AlphaApp/node_modules/left-pad/index.js",
        30_000,
    );
    write(
        root,
        "Projects/AlphaApp/node_modules/lodash/lodash.js",
        90_000,
    );
    text(
        root,
        "Projects/BetaRust/Cargo.toml",
        "[package]\nname=\"beta\"",
    );
    text(root, "Projects/BetaRust/src/main.rs", "fn main(){}");
    write(root, "Projects/BetaRust/target/debug/beta", 500_000);
    text(root, "Projects/Notes/readme.md", "# notes");
    // A launcher with instances of different game versions.
    for (name, version, mb) in [
        ("Pack A", "1.20.1", 900_000),
        ("Pack B", "26.2", 700_000),
        ("Pack C", "1.21.1", 800_000),
    ] {
        text(
            root,
            &format!("curseforge/minecraft/Instances/{name}/minecraftinstance.json"),
            &format!("{{\"name\":\"{name}\",\"gameVersion\": \"{version}\"}}"),
        );
        write(
            root,
            &format!("curseforge/minecraft/Instances/{name}/mods/mod.jar"),
            mb,
        );
        text(
            root,
            &format!("curseforge/minecraft/Instances/{name}/config/jade/profiles/1/x.json"),
            "{}",
        );
    }
    // A folder to rename and one to leave alone.
    text(root, "Desktop/Old Stuff/a.txt", "a");
    text(root, "Desktop/Old Stuff/deep/b.txt", "b");
}

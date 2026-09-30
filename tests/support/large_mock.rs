//! A deliberately messy, reproducible "life on a laptop" tree for large-scale tests and benchmarks.
//! `Inbox/` is one big unsorted dump (photos, documents, screenshots, installers, logs, temp files,
//! odd names and case, exact duplicates); the rest is a lived-in home: projects, clients, games.
use std::{
    fs::{self, File},
    io::Write,
    path::Path,
    time::{Duration, SystemTime},
};

pub struct Rng(pub u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        // xorshift64*: small, fast and identical on every machine.
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545F4914F6CDD1D)
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}
#[derive(Default, Debug, Clone)]
pub struct Manifest {
    pub total: usize,
    pub inbox: usize,
    pub photos: usize,
    pub screenshots: usize,
    pub docs: usize,
    pub txt: usize,
    pub installers: usize,
    pub logs: usize,
    pub temp: usize,
    pub upper_ext: usize,
    pub spaced_names: usize,
    pub copy_named: usize,
    pub duplicate_extra: usize,
    pub old_installers: usize,
    pub projects: usize,
    pub instances: usize,
}
fn put(root: &Path, rel: &str, size: usize, seed: u64, age_days: u64) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut f = File::create(&path).unwrap();
    // Unique, incompressible-ish content so only intended duplicates share bytes.
    let mut r = Rng(seed | 1);
    let mut left = size.max(16);
    while left > 0 {
        let chunk: Vec<u8> = (0..left.min(4096))
            .map(|_| (r.next() >> 24) as u8)
            .collect();
        f.write_all(&chunk).unwrap();
        left -= chunk.len();
    }
    f.set_modified(SystemTime::now() - Duration::from_secs(age_days * 86_400 + 3_600))
        .unwrap();
}
fn text(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}
const CLIENTS: &[&str] = &["Acme", "Globex", "Initech", "Umbrella", "Hooli"];
const TOPICS: &[&str] = &[
    "budget",
    "roadmap",
    "meeting notes",
    "todo list",
    "ideas",
    "recipe",
    "trip plan",
    "workout",
    "shopping list",
    "draft",
];

pub fn build_large(root: &Path, total: usize, seed: u64) -> Manifest {
    let mut m = Manifest::default();
    let mut rng = Rng(seed);
    let mut n = 0u64;
    let mut seq = || {
        n += 1;
        n
    };
    // ---- lived-in folders (fixed size) ----
    for p in 0..8 {
        let name = [
            "AlphaApp",
            "BetaRust",
            "Gamma-site",
            "Delta_tool",
            "Epsilon API",
            "Zeta",
            "Eta-cli",
            "Theta",
        ][p];
        let base = format!("Projects/{name}");
        match p % 3 {
            0 => {
                text(root, &format!("{base}/.git/HEAD"), "ref: refs/heads/main");
                text(root, &format!("{base}/package.json"), "{\"name\":\"x\"}");
                for i in 0..12 {
                    put(
                        root,
                        &format!("{base}/node_modules/dep{i}/index.js"),
                        2000,
                        seq() + seed,
                        30,
                    );
                }
            }
            1 => {
                text(root, &format!("{base}/Cargo.toml"), "[package]\nname=\"x\"");
                for i in 0..12 {
                    put(
                        root,
                        &format!("{base}/target/debug/artifact{i}"),
                        3000,
                        seq() + seed,
                        10,
                    );
                }
            }
            _ => text(
                root,
                &format!("{base}/pyproject.toml"),
                "[project]\nname='x'",
            ),
        }
        for i in 0..6 {
            put(
                root,
                &format!("{base}/src/module{i}.rs"),
                800,
                seq() + seed,
                (i * 20) as u64,
            );
        }
        m.projects += 1;
    }
    for client in CLIENTS.iter().take(3) {
        for sub in ["contracts", "invoices"] {
            for i in 0..6 {
                put(
                    root,
                    &format!(
                        "Work/Clients/{}/{sub}/{}-{i}.pdf",
                        client,
                        if sub == "invoices" {
                            "invoice"
                        } else {
                            "contract"
                        }
                    ),
                    4000,
                    seq() + seed,
                    40 * i as u64,
                );
            }
        }
    }
    for (i, (name, version)) in [
        ("Vanilla Plus", "1.20.1"),
        ("Tech Pack", "1.21.1"),
        ("Skyblock", "26.2"),
        ("Cobbleverse", "1.21.1"),
        ("Old Legacy", "1.12.2"),
        ("Test World", "26.2"),
    ]
    .iter()
    .enumerate()
    {
        text(
            root,
            &format!("Games/curseforge/minecraft/Instances/{name}/minecraftinstance.json"),
            &format!("{{\"gameVersion\": \"{version}\"}}"),
        );
        for j in 0..6 {
            put(
                root,
                &format!("Games/curseforge/minecraft/Instances/{name}/mods/mod{j}.jar"),
                5000,
                seq() + seed,
                (i * 3 + j) as u64,
            );
        }
        m.instances += 1;
    }
    for (y, year) in [2019, 2020, 2021].iter().enumerate() {
        for i in 0..8 {
            put(
                root,
                &format!("Archive/{year}/old-file-{i}.doc"),
                1500,
                seq() + seed,
                1500 + y as u64 * 365 + i as u64,
            );
        }
    }
    let fixed = 12 * 8 + 6 * 8 + 3 * 2 * 6 + 6 * 7 + 3 * 8 + 6; // approximate; the inbox tops up to `total`
    let inbox_target = total.saturating_sub(fixed).max(50);
    // ---- the messy inbox ----
    let mut made: Vec<String> = Vec::new();
    let mut unique = 0usize;
    while m.inbox < inbox_target {
        let roll = rng.below(100);
        let age = 1 + rng.below(1400) as u64;
        let id = seq() + seed;
        let name;
        let size;
        match roll {
            0..=21 => {
                let ext = *rng.pick(&["jpg", "JPG", "jpeg", "png", "heic", "PNG"]);
                name = match rng.below(3) {
                    0 => format!("IMG_{:04}.{ext}", rng.below(9999)),
                    1 => format!(
                        "Photo 20{:02}-{:02}-{:02} {:02}.{:02}.{:02}.{ext}",
                        19 + rng.below(8),
                        1 + rng.below(12),
                        1 + rng.below(28),
                        rng.below(24),
                        rng.below(60),
                        rng.below(60)
                    ),
                    _ => format!("vacation pic {}.{ext}", rng.below(500)),
                };
                size = 3000 + rng.below(20000);
                m.photos += 1;
                if ext.chars().any(|c| c.is_uppercase()) {
                    m.upper_ext += 1;
                }
            }
            22..=26 => {
                name = format!(
                    "Screenshot 202{}-{:02}-{:02} at {:02}.{:02}.{:02}.png",
                    3 + rng.below(4),
                    1 + rng.below(12),
                    1 + rng.below(28),
                    rng.below(24),
                    rng.below(60),
                    rng.below(60)
                );
                size = 4000 + rng.below(9000);
                m.screenshots += 1;
            }
            27..=45 => {
                let ext = *rng.pick(&["pdf", "pdf", "pdf", "docx", "xlsx", "pptx", "PDF"]);
                let base = match rng.below(4) {
                    0 => format!("Report FINAL v{} (copy)", 1 + rng.below(5)),
                    1 => format!(
                        "Invoice {} 202{}-{:02}",
                        rng.pick(CLIENTS),
                        3 + rng.below(4),
                        1 + rng.below(12)
                    ),
                    2 => format!("Contract {}", rng.pick(CLIENTS)),
                    _ => format!("{} {}", rng.pick(TOPICS), rng.below(100)),
                };
                if base.contains("copy") {
                    m.copy_named += 1;
                }
                name = format!("{base}.{ext}");
                size = 2000 + rng.below(25000);
                m.docs += 1;
                if ext == "PDF" {
                    m.upper_ext += 1;
                }
            }
            46..=53 => {
                name = format!(
                    "{} {}.{}",
                    rng.pick(TOPICS),
                    rng.below(1000),
                    rng.pick(&["txt", "txt", "md"])
                );
                size = 200 + rng.below(3000);
                m.txt += 1;
            }
            54..=57 => {
                name = format!("clip {}.{}", rng.below(1000), rng.pick(&["mp4", "mov"]));
                size = 20000 + rng.below(40000);
            }
            58..=61 => {
                name = format!("song {}.{}", rng.below(1000), rng.pick(&["mp3", "wav"]));
                size = 10000 + rng.below(30000);
            }
            62..=65 => {
                name = format!(
                    "backup {}.{}",
                    rng.below(1000),
                    rng.pick(&["zip", "tar.gz"])
                );
                size = 8000 + rng.below(20000);
            }
            66..=69 => {
                name = format!("Installer {}.{}", rng.below(500), rng.pick(&["dmg", "pkg"]));
                size = 30000 + rng.below(30000);
                m.installers += 1;
                if age > 30 {
                    m.old_installers += 1;
                }
            }
            70..=76 => {
                name = format!(
                    "script{}.{}",
                    rng.below(1000),
                    rng.pick(&["js", "py", "rs", "json"])
                );
                size = 300 + rng.below(4000);
            }
            77..=82 => {
                name = format!("app {}.log", rng.below(1000));
                size = 500 + rng.below(6000);
                m.logs += 1;
            }
            83..=89 => {
                name = format!(
                    "download {}.{}",
                    rng.below(1000),
                    rng.pick(&["tmp", "bak", "part"])
                );
                size = 500 + rng.below(5000);
                m.temp += 1;
            }
            90..=91 => {
                name = format!("README{}", rng.below(1000));
                size = 300 + rng.below(1000);
            }
            _ => {
                // An exact duplicate of something already in the inbox, under a different name.
                if made.is_empty() {
                    continue;
                }
                let original = made[rng.below(made.len())].clone();
                let stem = Path::new(&original)
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                let ext = Path::new(&original)
                    .extension()
                    .map(|e| format!(".{}", e.to_string_lossy()))
                    .unwrap_or_default();
                let dup = if rng.below(2) == 0 {
                    format!("{stem} (1){ext}")
                } else {
                    format!("Copy of {stem}{ext}")
                };
                if root.join("Inbox").join(&dup).exists() {
                    continue;
                }
                fs::copy(
                    root.join("Inbox").join(&original),
                    root.join("Inbox").join(&dup),
                )
                .unwrap_or_default();
                m.duplicate_extra += 1;
                m.inbox += 1;
                continue;
            }
        }
        if name.contains(' ') {
            m.spaced_names += 1;
        }
        if root.join("Inbox").join(&name).exists() {
            continue;
        }
        put(root, &format!("Inbox/{name}"), size, id, age);
        made.push(name);
        unique += 1;
        m.inbox += 1;
    }
    let _ = unique;
    m.total = walk_count(root);
    m
}
fn walk_count(root: &Path) -> usize {
    let mut n = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(d).unwrap().flatten() {
            let t = e.file_type().unwrap();
            if t.is_dir() {
                if e.file_name() != ".git" {
                    stack.push(e.path());
                }
            } else {
                n += 1;
            }
        }
    }
    n
}

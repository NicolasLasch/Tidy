//! Instant understanding of everyday requests. No model is loaded and nothing is ever executed:
//! the result is a reviewable preview like every other plan. Ambiguity is resolved with the most
//! sensible reading and disclosed in the reply instead of turning into a question.

#![allow(clippy::all)]

use crate::investigation::{FolderTarget, Investigation, ListItem, Section, Source, Trace};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
};
use tidy_organization::{FileCandidate, OrganizationMode, Proposal, ProposedAction};
use tidy_storage::AnalyzableFile;

const BATCH: usize = 500;
const FOLDER_BATCH: usize = 50;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Verb {
    Trash,
    Organize,
    Show,
}

pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if n < 1024 {
        return format!("{n} B");
    }
    let i = ((n as f64).ln() / 1024f64.ln()).floor().min(4.0) as usize;
    let value = n as f64 / 1024f64.powi(i as i32);
    if i > 1 {
        format!("{value:.1} {}", UNITS[i])
    } else {
        format!("{value:.0} {}", UNITS[i])
    }
}
fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}
fn tokens(text: &str) -> Vec<String> {
    raw_tokens(text)
        .into_iter()
        .map(|w| w.to_lowercase())
        .collect()
}
/// Same splitting as `tokens`, but keeps the user's capitalization for names they want created.
fn raw_tokens(text: &str) -> Vec<String> {
    // Commas and semicolons survive as "," tokens so lists of names can be split later.
    text.replace(['’', '‘', '“', '”'], "'")
        .replace([',', ';'], " , ")
        .split(|c: char| c.is_whitespace() || matches!(c, '!' | '?' | '(' | ')' | '"'))
        .map(|w| {
            w.trim_matches(|c: char| {
                matches!(c, '\'' | ':' | '.') && !w.starts_with('.') || c == '\'' || c == ':'
            })
            .to_string()
        })
        .filter(|w| !w.is_empty())
        .collect()
}
fn alnum(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}
fn name_tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect()
}
const FILLER: &[&str] = &[
    "the",
    "a",
    "an",
    "my",
    "this",
    "that",
    "these",
    "those",
    "all",
    "every",
    "whole",
    "entire",
    "complete",
    "please",
    "pls",
    "folder",
    "folders",
    "directory",
    "directories",
    "dir",
    "called",
    "named",
    "for",
    "me",
    "now",
    "thanks",
    "thank",
    "you",
    "can",
    "could",
    "would",
    "will",
    "just",
    "to",
    "of",
    "it",
    "them",
    "and",
    "also",
    "then",
    "permanently",
    "forever",
    "completely",
    "totally",
    "everything",
    "inside",
    "in",
    "stuff",
    "want",
    "need",
    "i",
    "let's",
    "lets",
    "go",
    "ahead",
    "there",
    "is",
    "are",
    "be",
    "should",
    "i'd",
    "like",
    "id",
    "kindly",
    "instantly",
];

struct Folder {
    path: PathBuf,
    name_tokens: Vec<String>,
    name_norm: String,
    bytes: u64,
    files: usize,
}
fn folders_of(files: &[FileCandidate]) -> Vec<Folder> {
    let analyzable: Vec<_> = files
        .iter()
        .map(|f| AnalyzableFile {
            id: f.id.0,
            path: f.relative_path.clone(),
            size: f.size,
            modified: f.modified,
            hash: None,
            identity: String::new(),
        })
        .collect();
    tidy_storage::folder_usage(&analyzable)
        .into_iter()
        .filter(|f| !f.path.as_os_str().is_empty())
        .map(|f| {
            let name = f
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            Folder {
                name_tokens: name_tokens(&name),
                name_norm: alnum(&name),
                path: f.path,
                bytes: f.logical_bytes,
                files: f.file_count,
            }
        })
        .collect()
}
/// Best-matching folders for a spoken name; equal-quality matches are ordered by size.
fn resolve<'a>(phrase: &[String], folders: &'a [Folder]) -> Vec<&'a Folder> {
    resolve_q(phrase, folders).0
}
/// Like `resolve`, plus how good the match is (0 = nothing, 10 = the exact name).
fn resolve_q<'a>(phrase: &[String], folders: &'a [Folder]) -> (Vec<&'a Folder>, i32) {
    if let [only] = phrase
        && only.contains('/')
    {
        let wanted = only.trim_matches('/').to_lowercase();
        let mut exact: Vec<&Folder> = folders
            .iter()
            .filter(|f| {
                let path = f.path.to_string_lossy().to_lowercase();
                path == wanted || path.ends_with(&format!("/{wanted}"))
            })
            .collect();
        exact.sort_by_key(|f| (f.path.components().count(), std::cmp::Reverse(f.bytes)));
        let tier = if exact.is_empty() { 0 } else { 10 };
        return (exact, tier);
    }
    // A name that itself contains “and” (“life and hell”) is tried whole before the filler is dropped.
    let whole_norm: Option<String> = phrase
        .iter()
        .any(|w| matches!(w.as_str(), "and" | "&"))
        .then(|| phrase.iter().map(|w| alnum(w)).collect());
    let phrase: Vec<&String> = phrase
        .iter()
        .filter(|w| !FILLER.contains(&w.as_str()))
        .collect();
    if phrase.is_empty() {
        return (vec![], 0);
    }
    let phrase_norm: String = phrase.iter().map(|w| alnum(w)).collect();
    if phrase_norm.len() < 2 {
        return (vec![], 0);
    }
    // (effective score, tier, folder)
    let mut scored: Vec<(i32, i32, &Folder)> = folders
        .iter()
        .filter_map(|f| {
            if f.name_norm.is_empty() {
                return None;
            }
            let (score, tier): (u8, i32) = if f.name_norm == phrase_norm
                || whole_norm.as_deref() == Some(f.name_norm.as_str())
            {
                (4, 10)
            } else if whole_norm
                .as_deref()
                .is_some_and(|w| w.len() >= 6 && f.name_norm.contains(w))
            {
                (3, 8)
            } else if phrase.iter().all(|p| {
                f.name_tokens
                    .iter()
                    .any(|t| t == *p || (p.len() >= 3 && t.starts_with(p.as_str())))
            }) {
                (3, 7)
            } else if phrase_norm.len() >= 4 && f.name_norm.contains(&phrase_norm) {
                (2, 5)
            } else if f.name_norm.len() >= 4 && phrase_norm.contains(&f.name_norm) {
                (1, 3)
            } else {
                return None;
            };
            // Folders inside build output are rarely what a person means by a name.
            // Deeper folders count for less (a project beats a same-named folder six levels down),
            // and build output counts for least.
            let depth = f.path.components().count() as i32;
            let mut effective = i32::from(score) * 2 - (depth - 2).max(0);
            if in_build_output(&f.path) {
                effective -= 6;
            }
            Some((effective, tier, f))
        })
        .collect();
    if scored.is_empty() {
        // Looser match for names typed from memory ("background animated web" -> Background-animated):
        // every word of the folder name appears in the request, or a distinctive request word
        // (rare across folder names) matches one. Common words like "game" never decide alone.
        let mut frequency: HashMap<&str, usize> = HashMap::new();
        for f in folders {
            for t in &f.name_tokens {
                *frequency.entry(t.as_str()).or_default() += 1;
            }
        }
        scored = folders
            .iter()
            .filter_map(|f| {
                if f.name_norm.len() < 4 {
                    return None;
                }
                let covers = !f.name_tokens.is_empty()
                    && f.name_tokens
                        .iter()
                        .all(|t| phrase.iter().any(|p| p.as_str() == t));
                let distinctive = phrase.iter().any(|p| {
                    p.len() >= 3
                        && f.name_tokens.iter().any(|t| {
                            (t == *p || t.starts_with(p.as_str()))
                                && frequency.get(t.as_str()).copied().unwrap_or(0) <= 2
                        })
                });
                if covers {
                    Some((1, 5, f))
                } else if distinctive {
                    Some((0, 3, f))
                } else {
                    None
                }
            })
            .collect();
    }
    if scored.is_empty() && phrase_norm.len() >= 5 {
        // Last resort, for a typo (“corssover” → Crossover): one slip in a long-enough name.
        let limit = if phrase_norm.len() >= 9 { 2 } else { 1 };
        scored = folders
            .iter()
            .filter(|f| {
                edit_distance(&f.name_norm, &phrase_norm) <= limit
                    || phrase.len() == 1
                        && f.name_tokens
                            .iter()
                            .any(|t| t.len() >= 5 && edit_distance(t, &phrase_norm) <= 1)
            })
            .map(|f| (0, 4, f))
            .collect();
    }
    let best = scored.iter().map(|(s, _, _)| *s).max().unwrap_or(0);
    scored.retain(|(s, _, _)| *s == best);
    scored.sort_by(|a, b| b.2.bytes.cmp(&a.2.bytes).then(a.2.path.cmp(&b.2.path)));
    let tier = scored.first().map(|(_, t, _)| *t).unwrap_or(0);
    (scored.into_iter().map(|(_, _, f)| f).collect(), tier)
}
/// Edit distance counting a swap of two neighbouring letters as one slip (“corssover” ↔ “crossover”).
fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.len().abs_diff(b.len()) > 2 {
        return 3;
    }
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for j in 0..=b.len() {
        d[0][j] = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}
fn in_build_output(path: &Path) -> bool {
    path.components().any(|c| {
        matches!(
            c.as_os_str().to_string_lossy().as_ref(),
            "build"
                | "dist"
                | "target"
                | "node_modules"
                | ".gradle"
                | "out"
                | ".next"
                | "Pods"
                | "DerivedData"
                | "__pycache__"
                | ".cache"
        )
    })
}
fn outermost<'a>(found: Vec<&'a Folder>) -> Vec<&'a Folder> {
    let all: Vec<&Path> = found.iter().map(|f| f.path.as_path()).collect();
    found
        .into_iter()
        .filter(|f| {
            !all.iter()
                .any(|other| *other != f.path.as_path() && f.path.starts_with(other))
        })
        .collect()
}
fn target(f: &Folder) -> FolderTarget {
    FolderTarget {
        path: f.path.to_string_lossy().into_owned(),
        files: f.files,
        bytes: f.bytes,
        note: None,
    }
}
fn folder_item(f: &Folder) -> ListItem {
    ListItem {
        kind: "folder",
        path: f.path.to_string_lossy().into_owned(),
        bytes: f.bytes,
        files: f.files,
        note: None,
    }
}
fn file_item(f: &FileCandidate) -> ListItem {
    ListItem {
        kind: "file",
        path: f.relative_path.to_string_lossy().into_owned(),
        bytes: f.size,
        files: 1,
        note: None,
    }
}

fn contains_seq(words: &[String], seq: &[&str]) -> bool {
    words
        .windows(seq.len())
        .any(|w| w.iter().zip(seq).all(|(a, b)| a == b))
}
fn has(words: &[String], any: &[&str]) -> bool {
    words.iter().any(|w| any.contains(&w.as_str()))
}
fn negated(words: &[String], at: usize) -> bool {
    words[at.saturating_sub(3)..at].iter().any(|w| {
        matches!(
            w.as_str(),
            "don't" | "dont" | "not" | "never" | "without" | "no" | "won't" | "shouldn't"
        )
    })
}
fn detect_verb(words: &[String]) -> Option<Verb> {
    const TRASH: &[&str] = &[
        "delete",
        "remove",
        "trash",
        "erase",
        "discard",
        "wipe",
        "purge",
        "uninstall",
        "bin",
        "destroy",
        "dump",
        "nuke",
        "drop",
    ];
    const ORGANIZE: &[&str] = &[
        "organize",
        "organise",
        "sort",
        "tidy",
        "group",
        "arrange",
        "categorize",
        "categorise",
        "declutter",
        "structure",
        "classify",
    ];
    const SHOW: &[&str] = &[
        "find",
        "show",
        "list",
        "what",
        "which",
        "where",
        "how",
        "search",
        "analyze",
        "analyse",
        "summarize",
        "summarise",
        "overview",
        "describe",
        "tell",
        "whats",
        "what's",
        "explain",
        "count",
        "look",
        "see",
        "check",
        "inspect",
        "give",
        "display",
    ];
    for (at, w) in words.iter().enumerate() {
        let w = w.as_str();
        if negated(words, at) {
            continue;
        }
        if TRASH.contains(&w)
            || (w == "get" && words.get(at + 1).is_some_and(|n| n == "rid"))
            || (w == "throw" && words.get(at + 1).is_some_and(|n| n == "away" || n == "out"))
            || ((w == "clean" || w == "clear") && words.get(at + 1).is_some_and(|n| n == "out"))
        {
            return Some(Verb::Trash);
        }
        if ORGANIZE.contains(&w) {
            return Some(Verb::Organize);
        }
        if (w == "clean" || w == "cleanup") && !words.iter().any(|x| x == "out") {
            return Some(Verb::Organize);
        }
        if SHOW.contains(&w) {
            return Some(Verb::Show);
        }
    }
    if has(
        words,
        &["biggest", "largest", "heaviest", "space", "storage"],
    ) {
        return Some(Verb::Show);
    }
    None
}

#[derive(Default, Debug)]
struct Criteria {
    exts: Vec<String>,
    screenshots: bool,
    junk: bool,
    dev: bool,
    name_terms: Vec<String>,
    older_days: Option<i64>,
    min_bytes: Option<u64>,
    top_n: Option<usize>,
    in_folder: Option<PathBuf>,
    excludes: Vec<String>,
    label: String,
}
impl Criteria {
    fn targets_files(&self) -> bool {
        !self.exts.is_empty()
            || self.screenshots
            || self.junk
            || !self.name_terms.is_empty()
            || self.older_days.is_some()
            || self.min_bytes.is_some()
            || self.top_n.is_some()
    }
}
fn kind_exts(word: &str) -> Option<(&'static [&'static str], &'static str)> {
    Some(match word {
        "photo" | "photos" | "image" | "images" | "picture" | "pictures" | "pics" | "pic" => (
            &[
                "jpg", "jpeg", "png", "heic", "heif", "gif", "webp", "bmp", "tif", "tiff", "raw",
                "dng", "avif",
            ],
            "photos and images",
        ),
        "video" | "videos" | "movie" | "movies" | "clips" => (
            &["mp4", "mov", "mkv", "avi", "webm", "m4v", "wmv", "flv"],
            "videos",
        ),
        "audio" | "music" | "song" | "songs" | "sounds" => (
            &["mp3", "wav", "flac", "aac", "m4a", "ogg", "aiff", "wma"],
            "audio files",
        ),
        "document" | "documents" | "docs" | "doc" => (
            &[
                "pdf", "doc", "docx", "txt", "rtf", "odt", "pages", "md", "epub",
            ],
            "documents",
        ),
        "pdf" | "pdfs" => (&["pdf"], "PDF files"),
        "spreadsheet" | "spreadsheets" => {
            (&["xls", "xlsx", "csv", "numbers", "ods"], "spreadsheets")
        }
        "presentation" | "presentations" | "slides" | "keynotes" => {
            (&["ppt", "pptx", "key", "odp"], "presentations")
        }
        "archive" | "archives" | "zip" | "zips" | "zipped" => (
            &["zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz"],
            "archives",
        ),
        "installer" | "installers" | "dmg" | "dmgs" | "setup" | "setups" => {
            (&["dmg", "pkg", "exe", "msi", "iso"], "installers")
        }
        "text" | "txt" | "txts" => (&["txt", "text"], "text files"),
        "log" | "logs" => (&["log"], "log files"),
        "json" => (&["json"], "JSON files"),
        _ => return None,
    })
}
fn number_word(word: &str) -> Option<f64> {
    if let Ok(n) = word.parse::<f64>() {
        return Some(n);
    }
    Some(match word {
        "a" | "an" | "one" => 1.0,
        "two" => 2.0,
        "three" => 3.0,
        "four" => 4.0,
        "five" => 5.0,
        "six" => 6.0,
        "seven" => 7.0,
        "eight" => 8.0,
        "nine" => 9.0,
        "ten" => 10.0,
        "twelve" => 12.0,
        "couple" => 2.0,
        "few" => 3.0,
        _ => return None,
    })
}
fn unit_days(word: &str) -> Option<f64> {
    let w = word.trim_end_matches('s');
    Some(match w {
        "day" => 1.0,
        "week" => 7.0,
        "month" => 30.0,
        "year" => 365.0,
        _ => return None,
    })
}
fn parse_age(words: &[String]) -> Option<i64> {
    for (i, w) in words.iter().enumerate() {
        // "6 months old", "older than 2 years", "not modified in 30 days"
        if let (Some(n), Some(unit)) = (number_word(w), words.get(i + 1).and_then(|u| unit_days(u)))
        {
            let context = &words[i.saturating_sub(4)..(i + 4).min(words.len())];
            if has(
                context,
                &[
                    "older", "old", "ago", "before", "modified", "touched", "opened", "unused",
                    "over", "since", "than",
                ],
            ) {
                return Some((n * unit).round() as i64);
            }
        }
    }
    if has(
        words,
        &["old", "older", "ancient", "stale", "outdated", "unused"],
    ) {
        return Some(if has(words, &["installer", "installers", "dmg", "dmgs"]) {
            30
        } else {
            180
        });
    }
    None
}
fn parse_size(words: &[String]) -> Option<u64> {
    for (i, w) in words.iter().enumerate() {
        let (number, unit) = match w.find(|c: char| c.is_alphabetic()) {
            Some(at) if at > 0 => (w[..at].to_string(), w[at..].to_string()),
            _ => match words.get(i + 1) {
                Some(next) if w.parse::<f64>().is_ok() => (w.clone(), next.clone()),
                _ => continue,
            },
        };
        let Ok(n) = number.parse::<f64>() else {
            continue;
        };
        let factor = match unit.as_str() {
            "kb" | "kilobyte" | "kilobytes" | "k" => 1024.0,
            "mb" | "megabyte" | "megabytes" | "meg" | "megs" | "m" => 1024.0 * 1024.0,
            "gb" | "gigabyte" | "gigabytes" | "gig" | "gigs" | "g" => 1024.0 * 1024.0 * 1024.0,
            "tb" | "terabyte" | "terabytes" => 1024f64.powi(4),
            _ => continue,
        };
        return Some((n * factor) as u64);
    }
    None
}
fn phrase_after<'a>(words: &'a [String], markers: &[&str]) -> Option<&'a [String]> {
    let at = words.iter().position(|w| markers.contains(&w.as_str()))?;
    Some(&words[at + 1..])
}
fn parse_excludes(words: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for (i, w) in words.iter().enumerate() {
        let marker = matches!(w.as_str(), "except" | "excluding" | "besides")
            || (matches!(
                w.as_str(),
                "keep" | "leave" | "preserve" | "spare" | "skip" | "ignore"
            ) && i > 0)
            || (w == "touch" && i > 0 && matches!(words[i - 1].as_str(), "don't" | "not" | "dont"));
        if !marker {
            continue;
        }
        let mut phrase: Vec<String> = Vec::new();
        for next in &words[i + 1..] {
            if matches!(next.as_str(), "and" | "but" | "or" | "then" | ",") {
                if !phrase.is_empty() {
                    out.push(phrase.join(" "));
                    phrase.clear();
                }
                continue;
            }
            if !FILLER.contains(&next.as_str())
                && !matches!(next.as_str(), "files" | "file" | "alone" | "untouched")
            {
                phrase.push(next.clone());
            }
        }
        if !phrase.is_empty() {
            out.push(phrase.join(" "));
        }
        break;
    }
    out
}
fn parse_criteria(words: &[String], folders: &[Folder]) -> Criteria {
    let mut c = Criteria::default();
    let mut labels: Vec<String> = Vec::new();
    for w in words {
        let bare = w.trim_start_matches(['*', '.']);
        if w.starts_with('.') || w.starts_with("*.") {
            if !bare.is_empty()
                && bare.chars().all(|ch| ch.is_ascii_alphanumeric())
                && bare.len() <= 8
            {
                if !c.exts.iter().any(|e| e == bare) {
                    c.exts.push(bare.to_string());
                    labels.push(format!(".{bare} files"));
                }
                continue;
            }
        }
        if let Some((exts, label)) = kind_exts(w) {
            for e in exts {
                if !c.exts.iter().any(|x| x == e) {
                    c.exts.push((*e).to_string());
                }
            }
            labels.push(label.to_string());
        }
        match w.as_str() {
            "screenshot" | "screenshots" | "screengrab" | "screengrabs" => {
                c.screenshots = true;
                labels.push("screenshots".into());
            }
            "junk" | "temp" | "tmp" | "temporary" | "leftovers" | "garbage" | "trash"
                if w != "trash" =>
            {
                c.junk = true;
                labels.push("temporary and junk files".into());
            }
            "node_modules" | "pycache" | "__pycache__" | "derived" | "deriveddata" | "caches"
            | "cache" | "artifacts" | "artefacts" | "build" | "dependencies" => {
                c.dev = true;
            }
            _ => {}
        }
    }
    if contains_seq(words, &["dev", "artifacts"])
        || contains_seq(words, &["build", "artifacts"])
        || contains_seq(words, &["build", "files"])
        || contains_seq(words, &["derived", "data"])
        || has(words, &["node_modules", "pycache", "__pycache__"])
    {
        c.dev = true;
    }
    if c.dev {
        // Folders, not thousands of files, are the natural unit for artifacts.
        c.exts.clear();
        labels.clear();
    }
    if let Some(days) = parse_age(words) {
        c.older_days = Some(days);
    }
    let sized = has(
        words,
        &[
            "larger",
            "bigger",
            "over",
            "above",
            "exceeding",
            "heavier",
            "least",
            ">",
        ],
    );
    if let Some(b) = parse_size(words) {
        if sized || has(words, &["large", "big", "huge", "files"]) {
            c.min_bytes = Some(b);
        }
    } else if has(
        words,
        &[
            "large", "big", "huge", "heavy", "giant", "massive", "enormous",
        ],
    ) {
        c.min_bytes = Some(100 * 1024 * 1024);
    }
    if let Some(at) = words
        .iter()
        .position(|w| matches!(w.as_str(), "biggest" | "largest" | "heaviest" | "top"))
    {
        let n = words[at + 1..]
            .iter()
            .take(2)
            .chain(words[at.saturating_sub(2)..at].iter())
            .find_map(|w| {
                w.parse::<usize>()
                    .ok()
                    .or_else(|| number_word(w).map(|n| n as usize))
            });
        if words[at..].iter().any(|w| w == "files" || w == "file")
            || c.exts.len() + usize::from(c.screenshots) > 0
        {
            c.top_n = Some(n.unwrap_or(10).clamp(1, BATCH));
        }
    }
    // Quoted or filename-like terms
    for w in words {
        if w.contains('.')
            && !w.starts_with('.')
            && !w.starts_with("*.")
            && w.rsplit('.').next().is_some_and(|e| {
                !e.is_empty() && e.len() <= 8 && e.chars().all(|ch| ch.is_ascii_alphanumeric())
            })
            && w.len() > 3
        {
            c.name_terms.push(w.clone());
        }
    }
    for marker in [
        ["named"].as_slice(),
        ["containing", "contain", "contains"].as_slice(),
        ["starting", "starts", "beginning"].as_slice(),
    ] {
        if let Some(rest) = phrase_after(words, marker) {
            let phrase: Vec<_> = rest
                .iter()
                .take_while(|w| {
                    !matches!(
                        w.as_str(),
                        "and" | "older" | "in" | "from" | "larger" | "bigger" | "except" | "but"
                    )
                })
                .filter(|w| {
                    !FILLER.contains(&w.as_str())
                        && !matches!(w.as_str(), "with" | "word" | "the" | "text")
                })
                .cloned()
                .collect();
            if !phrase.is_empty() {
                c.name_terms.push(phrase.join(" "));
            }
        }
    }
    if let Some(rest) = phrase_after(words, &["in", "inside", "within", "under", "from"]) {
        let phrase: Vec<String> = rest
            .iter()
            .take_while(|w| {
                !matches!(
                    w.as_str(),
                    "older"
                        | "larger"
                        | "bigger"
                        | "that"
                        | "which"
                        | "except"
                        | "but"
                        | "and"
                        | "over"
                        | "with"
                        | "containing"
                        | "named"
                        | "not"
                        | "modified"
                )
            })
            .cloned()
            .collect();
        let generic = phrase.iter().all(|w| FILLER.contains(&w.as_str()));
        if !generic {
            if let Some(best) = resolve(&phrase, folders).first() {
                c.in_folder = Some(best.path.clone());
            }
        }
    }
    c.excludes = parse_excludes(words);
    if c.older_days.is_some() {
        let days = c.older_days.unwrap();
        labels.push(if days >= 365 && days % 365 == 0 {
            format!(
                "older than {}",
                plural((days / 365) as usize, "year", "years")
            )
        } else if days >= 30 && days % 30 == 0 {
            format!(
                "older than {}",
                plural((days / 30) as usize, "month", "months")
            )
        } else {
            format!("older than {}", plural(days as usize, "day", "days"))
        });
    }
    if let Some(min) = c.min_bytes {
        labels.push(format!("over {}", bytes(min)));
    }
    if c.name_terms.iter().any(|t| !t.is_empty()) {
        labels.push(format!("matching “{}”", c.name_terms.join("”, “")));
    }
    if let Some(folder) = &c.in_folder {
        labels.push(format!("in {}", folder.display()));
    }
    c.label = labels.join(" · ");
    c
}
fn is_screenshot(name: &str) -> bool {
    let n = name.to_lowercase();
    (n.starts_with("screenshot")
        || n.starts_with("screen shot")
        || n.starts_with("capture d")
        || n.starts_with("bildschirmfoto")
        || n.starts_with("captura de pantalla"))
        && [".png", ".jpg", ".jpeg", ".heic"]
            .iter()
            .any(|e| n.ends_with(e))
}
fn is_junk(name: &str, ext: &str) -> bool {
    name == ".DS_Store"
        || name.starts_with("._")
        || name.ends_with('~')
        || matches!(
            ext,
            "tmp" | "temp" | "bak" | "crdownload" | "part" | "swp" | "download" | "old"
        )
}
fn never_touch(path: &Path) -> bool {
    path.components().any(|c| {
        let p = c.as_os_str().to_string_lossy().to_lowercase();
        p == ".git"
            || p.ends_with(".app")
            || p.ends_with(".framework")
            || p.ends_with(".photoslibrary")
    })
}
fn matches_file(f: &FileCandidate, c: &Criteria, now: i64) -> bool {
    let name = f
        .relative_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = f
        .relative_path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if never_touch(&f.relative_path) {
        return false;
    }
    if let Some(folder) = &c.in_folder {
        if !f.relative_path.starts_with(folder) {
            return false;
        }
    }
    let kind_filter = !c.exts.is_empty() || c.screenshots || c.junk;
    if kind_filter {
        let ok = c.exts.iter().any(|e| *e == ext)
            || (c.screenshots && is_screenshot(&name))
            || (c.junk && is_junk(&name, &ext));
        if !ok {
            return false;
        }
    }
    if !c.name_terms.is_empty() {
        let hay = alnum(&f.relative_path.to_string_lossy());
        let base = alnum(&name);
        if !c.name_terms.iter().any(|t| {
            let t = alnum(t);
            !t.is_empty() && (base.contains(&t) || (t.len() > 4 && hay.contains(&t)))
        }) {
            return false;
        }
    }
    if let Some(days) = c.older_days {
        if f.modified <= 0 || now - f.modified < days * 86_400 {
            return false;
        }
    }
    if let Some(min) = c.min_bytes {
        if f.size < min {
            return false;
        }
    }
    if !c.excludes.is_empty() {
        let hay = alnum(&f.relative_path.to_string_lossy());
        if c.excludes.iter().any(|e| {
            let e = alnum(e);
            e.len() >= 2 && hay.contains(&e)
        }) {
            return false;
        }
    }
    true
}

fn artifact_dirs(files: &[FileCandidate]) -> Vec<FolderTarget> {
    const NAMES: &[&str] = &[
        "node_modules",
        "__pycache__",
        ".pytest_cache",
        ".mypy_cache",
        ".gradle",
        ".next",
        ".nuxt",
        ".parcel-cache",
        "DerivedData",
        "Pods",
        ".tox",
        ".turbo",
        ".angular",
        ".cache",
    ];
    let cargo_dirs: HashSet<&Path> = files
        .iter()
        .filter(|f| {
            f.relative_path
                .file_name()
                .is_some_and(|n| n == "Cargo.toml")
        })
        .filter_map(|f| f.relative_path.parent())
        .collect();
    let mut found: BTreeMap<PathBuf, (usize, u64)> = BTreeMap::new();
    for f in files {
        if never_touch(&f.relative_path) {
            continue;
        }
        let mut prefix = PathBuf::new();
        let mut dir = None;
        for component in f
            .relative_path
            .parent()
            .unwrap_or(Path::new(""))
            .components()
        {
            prefix.push(component);
            let name = component.as_os_str().to_string_lossy();
            if NAMES.contains(&name.as_ref())
                || (name == "target"
                    && cargo_dirs.contains(prefix.parent().unwrap_or(Path::new(""))))
            {
                dir = Some(prefix.clone());
                break;
            }
        }
        if let Some(dir) = dir {
            let entry = found.entry(dir).or_default();
            entry.0 += 1;
            entry.1 += f.size;
        }
    }
    let mut out: Vec<_> = found
        .into_iter()
        .map(|(path, (files, bytes))| FolderTarget {
            path: path.to_string_lossy().into_owned(),
            files,
            bytes,
            note: None,
        })
        .collect();
    out.sort_by_key(|f| std::cmp::Reverse(f.bytes));
    out
}

struct Ctx<'a> {
    files: &'a [FileCandidate],
    scope: &'a str,
    now: i64,
    /// The authorized folder on disk, so read-only metadata (launcher instance versions) can be read.
    root: Option<&'a Path>,
}
fn reply(ctx: &Ctx, workflow: &str, headline: &str, detail: String, text: String) -> Investigation {
    Investigation {
        engine: "instant".into(),
        workflow: Some(workflow.to_string()),
        proposal: Proposal {
            actions: vec![],
            rationale: text,
        },
        sources: vec![],
        folders: vec![],
        pick: false,
        sections: vec![],
        unresolved: false,
        trace: vec![Trace {
            label: headline.into(),
            detail,
        }],
        clarification: None,
        indexed: ctx.files.len(),
        examined: ctx.files.len(),
        remaining_matches: 0,
        complete: true,
    }
}
fn help(ctx: &Ctx) -> Investigation {
    reply(
        ctx,
        "clarify_request",
        "Ready",
        "No model needed for these requests".into(),
        format!(
            "I can look through {} ({}) and get things done — just tell me what you want:\n\n• “Delete the Lucky World Invasion folder”\n• “Remove all .log files older than 3 months”\n• “Rename the Old Stuff folder to Archive”\n• “Create a folder called Invoices”\n• “Rename setup.dmg to Installer and move it into Archive” (two steps, one approval)\n• “Move all PDFs into Documents/PDFs”\n• “Delete the 10 biggest files”\n• “List all my projects”\n• “Clean up build artifacts and node_modules”\n• “Organize this folder by type” or “by date”\n• “What’s taking the most space?”\n• “Find invoice”\n\nEverything goes to the Trash after you approve a preview, so it can always be restored from Finder.",
            plural(ctx.files.len(), "indexed file", "indexed files"),
            ctx.scope
        ),
    )
}
/// A friendly answer for requests the instant engine cannot map; never an error dialog.
pub fn not_understood(indexed: usize, why: &str) -> Investigation {
    let files: Vec<FileCandidate> = Vec::new();
    let ctx = Ctx {
        files: &files,
        scope: "this folder",
        now: 0,
        root: None,
    };
    let mut r = reply(
        &ctx,
        "clarify_request",
        "Could not map the request",
        why.into(),
        format!(
            "{why} Try being direct, for example:\n\n• “Delete the <folder name> folder”\n• “Remove all .log files older than 3 months”\n• “Delete the 10 biggest files”\n• “Organize by type” or “by date”\n• “What’s taking the most space?”"
        ),
    );
    r.indexed = indexed;
    r
}
fn overview(ctx: &Ctx, folders: &[Folder], words: &[String]) -> Investigation {
    let total: u64 = ctx.files.iter().map(|f| f.size).sum();
    let want_files = has(words, &["file", "files"])
        && has(words, &["biggest", "largest", "heaviest", "big", "large"]);
    let deep = has(
        words,
        &[
            "where",
            "deep",
            "deeper",
            "nested",
            "subfolders",
            "sub-folders",
        ],
    );
    let mut sections = Vec::new();
    if !want_files {
        let mut top: Vec<&Folder> = folders
            .iter()
            .filter(|f| f.path.components().count() == 1)
            .collect();
        top.sort_by_key(|f| std::cmp::Reverse(f.bytes));
        if !top.is_empty() {
            sections.push(Section {
                title: format!("Folders in {}", ctx.scope),
                items: top.iter().take(150).map(|f| folder_item(f)).collect(),
            });
        }
        let mut here: Vec<&FileCandidate> = ctx
            .files
            .iter()
            .filter(|f| f.relative_path.components().count() == 1)
            .collect();
        here.sort_by_key(|f| std::cmp::Reverse(f.size));
        if !here.is_empty() {
            sections.push(Section {
                title: format!("Files directly in {}", ctx.scope),
                items: here.iter().take(30).map(|f| file_item(f)).collect(),
            });
        }
        if deep {
            let mut nested: Vec<&Folder> = folders
                .iter()
                .filter(|f| f.path.components().count() > 1 && f.bytes > 0)
                .collect();
            nested.sort_by_key(|f| std::cmp::Reverse(f.bytes));
            let leaves: Vec<&Folder> = nested
                .iter()
                .copied()
                .filter(|f| {
                    !nested.iter().any(|o| {
                        o.path != f.path
                            && o.path.starts_with(&f.path)
                            && o.bytes * 10 >= f.bytes * 9
                    })
                })
                .take(8)
                .collect();
            if !leaves.is_empty() {
                sections.push(Section {
                    title: "Where the space actually sits".into(),
                    items: leaves.iter().map(|f| folder_item(f)).collect(),
                });
            }
        }
    }
    if want_files {
        let mut files: Vec<&FileCandidate> = ctx.files.iter().collect();
        files.sort_by_key(|f| std::cmp::Reverse(f.size));
        sections.push(Section {
            title: "Biggest files".into(),
            items: files.iter().take(15).map(|f| file_item(f)).collect(),
        });
    }
    let mut kinds: BTreeMap<String, (usize, u64)> = BTreeMap::new();
    for f in ctx.files {
        let e = kinds
            .entry(
                f.relative_path
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
                    .unwrap_or_else(|| "no extension".into()),
            )
            .or_default();
        e.0 += 1;
        e.1 += f.size;
    }
    let mut kinds: Vec<_> = kinds.into_iter().collect();
    kinds.sort_by_key(|(_, (_, b))| std::cmp::Reverse(*b));
    let text = format!(
        "{} holds {} ({}). {}\n\nMost space by type: {}.\nAsk “what’s inside <folder>” to look deeper, or tell me what to remove.",
        ctx.scope,
        plural(ctx.files.len(), "indexed file", "indexed files"),
        bytes(total),
        if want_files {
            "Here are its biggest files."
        } else {
            "Here are its folders, biggest first."
        },
        kinds
            .iter()
            .take(5)
            .map(|(k, (n, b))| format!(".{k} {} ({n})", bytes(*b)))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mut r = reply(
        ctx,
        "analyze_storage",
        "Read the index",
        format!("Summed all {} indexed files", ctx.files.len()),
        text,
    );
    r.sections = sections;
    r.examined = ctx.files.len();
    r
}
/// "what's inside X": the subfolders and files directly inside one folder, with sizes.
fn folder_contents(ctx: &Ctx, words: &[String], folders: &[Folder]) -> Option<Investigation> {
    let at = words
        .iter()
        .position(|w| matches!(w.as_str(), "inside" | "in" | "of" | "into" | "contents"))
        .or_else(|| {
            words
                .iter()
                .position(|w| matches!(w.as_str(), "show" | "open" | "list" | "see"))
        })?;
    let phrase: Vec<String> = words[at + 1..]
        .iter()
        .filter(|w| {
            !FILLER.contains(&w.as_str())
                && !matches!(
                    w.as_str(),
                    "contents"
                        | "content"
                        | "show"
                        | "me"
                        | "what"
                        | "what's"
                        | "is"
                        | "are"
                        | "there"
                        | "things"
                        | "stuff"
                        | "files"
                        | "folders"
                )
        })
        .cloned()
        .collect();
    if phrase.is_empty() {
        return None;
    }
    let folder = *resolve(&phrase, folders).first()?;
    let mut subs: Vec<&Folder> = folders
        .iter()
        .filter(|f| f.path.parent() == Some(folder.path.as_path()))
        .collect();
    subs.sort_by_key(|f| std::cmp::Reverse(f.bytes));
    let mut direct: Vec<&FileCandidate> = ctx
        .files
        .iter()
        .filter(|f| f.relative_path.parent() == Some(folder.path.as_path()))
        .collect();
    direct.sort_by_key(|f| std::cmp::Reverse(f.size));
    let name = folder
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut sections = Vec::new();
    if !subs.is_empty() {
        sections.push(Section {
            title: format!("Folders in {name}"),
            items: subs.iter().take(150).map(|f| folder_item(f)).collect(),
        });
    }
    if !direct.is_empty() {
        sections.push(Section {
            title: format!("Files directly in {name}"),
            items: direct.iter().take(40).map(|f| file_item(f)).collect(),
        });
    }
    let mut r = reply(
        ctx,
        "inspect_folder",
        "Opened a folder",
        format!("Listed {}", folder.path.display()),
        format!(
            "{} — {} in {} ({} and {} directly inside).",
            folder.path.display(),
            bytes(folder.bytes),
            plural(folder.files, "file", "files"),
            plural(subs.len(), "folder", "folders"),
            plural(direct.len(), "file", "files")
        ),
    );
    r.sections = sections;
    Some(r)
}
fn find(ctx: &Ctx, words: &[String]) -> Option<Investigation> {
    let terms: Vec<String> = words
        .iter()
        .filter(|w| {
            !FILLER.contains(&w.as_str())
                && !matches!(
                    w.as_str(),
                    "find"
                        | "show"
                        | "list"
                        | "search"
                        | "where"
                        | "which"
                        | "what"
                        | "files"
                        | "file"
                        | "look"
                        | "for"
                        | "with"
                        | "named"
                        | "called"
                        | "containing"
                        | "any"
                        | "is"
                        | "are"
                        | "in"
                        | "on"
                        | "my"
                        | "computer"
                        | "folder"
                        | "see"
                        | "check"
                        | "give"
                        | "display"
                )
        })
        .map(|w| alnum(w))
        .filter(|w| w.len() >= 2)
        .collect();
    if terms.is_empty() {
        return None;
    }
    let mut hits: Vec<&FileCandidate> = ctx
        .files
        .iter()
        .filter(|f| {
            let hay = alnum(&f.relative_path.to_string_lossy());
            terms.iter().all(|t| hay.contains(t))
        })
        .collect();
    hits.sort_by_key(|f| std::cmp::Reverse(f.size));
    let total = hits.len();
    let text = if total == 0 {
        format!(
            "I searched all {} indexed files and found nothing matching “{}”. It may be outside {} or not scanned yet.",
            ctx.files.len(),
            terms.join(" "),
            ctx.scope
        )
    } else {
        format!(
            "Found {} matching “{}”. Say “delete them” if you want these moved to the Trash.",
            plural(total, "file", "files"),
            terms.join(" ")
        )
    };
    let mut found = reply(
        ctx,
        "find_filename",
        "Searched filenames",
        format!("Checked all {} paths", ctx.files.len()),
        text,
    );
    if total > 0 {
        found.sections.push(Section {
            title: format!("Matching files ({total})"),
            items: hits.iter().take(40).map(|f| file_item(f)).collect(),
        });
    }
    found.unresolved = total == 0;
    Some(found)
}

fn trash_files(ctx: &Ctx, c: &Criteria) -> Investigation {
    let mut hits: Vec<&FileCandidate> = ctx
        .files
        .iter()
        .filter(|f| matches_file(f, c, ctx.now))
        .collect();
    hits.sort_by(|a, b| {
        b.size
            .cmp(&a.size)
            .then(a.relative_path.cmp(&b.relative_path))
    });
    let matched = hits.len();
    let total_bytes: u64 = hits.iter().map(|f| f.size).sum();
    if let Some(n) = c.top_n {
        hits.truncate(n);
    }
    let shown = hits.len().min(BATCH);
    let remaining = if c.top_n.is_some() {
        0
    } else {
        matched.saturating_sub(BATCH)
    };
    let workflow = if c.min_bytes.is_some() || c.top_n.is_some() {
        "large_files"
    } else if c.screenshots {
        "screenshots"
    } else {
        "trash_filtered_files"
    };
    let label = if c.label.is_empty() {
        "matching files".to_string()
    } else {
        c.label.clone()
    };
    let mut r = reply(
        ctx,
        workflow,
        "Understood your request",
        format!("Move files to Trash · {label}"),
        String::new(),
    );
    if shown == 0 {
        r.proposal.rationale = format!(
            "I checked all {} indexed files and none are {label}, so there is nothing to remove. Nothing changed.",
            ctx.files.len()
        );
        return r;
    }
    let kept_bytes: u64 = hits.iter().take(shown).map(|f| f.size).sum();
    r.sources = hits
        .iter()
        .take(shown)
        .map(|f| Source {
            id: f.id.0,
            path: f.relative_path.to_string_lossy().into(),
            size: f.size,
        })
        .collect();
    r.proposal.actions = hits
        .iter()
        .take(shown)
        .map(|f| ProposedAction::Trash { source: f.id })
        .collect();
    r.examined = ctx.files.len();
    r.remaining_matches = remaining;
    r.complete = remaining == 0;
    r.proposal.rationale = format!(
        "I found {} {label} ({} in total) and prepared {} ({}) for the Trash.{} Uncheck anything you want to keep, then approve — nothing changes until you do, and everything stays recoverable from Finder’s Trash.",
        matched,
        bytes(total_bytes),
        plural(shown, "file", "files"),
        bytes(kept_bytes),
        if remaining > 0 {
            format!(" {remaining} more match — approve this batch and ask again for the rest.")
        } else {
            String::new()
        },
    );
    if !c.excludes.is_empty() {
        r.proposal.rationale.push_str(&format!(
            " Kept out as you asked: {}.",
            c.excludes.join(", ")
        ));
    }
    r
}

/// The names after a delete verb: comma-separated chunks that may still contain “and” (“life and hell”),
/// plus an optional parent (“… from Downloads”).
fn phrases_for_folders(words: &[String]) -> (Vec<Vec<String>>, Option<Vec<String>>) {
    let mut chunk: Vec<String> = Vec::new();
    let mut parent: Option<Vec<String>> = None;
    let mut in_parent = false;
    let mut chunks = Vec::new();
    for w in words {
        if matches!(w.as_str(), "from" | "in" | "inside" | "within" | "under")
            && chunk.iter().any(|c| !FILLER.contains(&c.as_str()))
        {
            in_parent = true;
            parent = Some(Vec::new());
            continue;
        }
        if in_parent {
            if let Some(p) = parent.as_mut() {
                p.push(w.clone());
            }
            continue;
        }
        if matches!(w.as_str(), "," | "plus") {
            if !chunk.is_empty() {
                chunks.push(std::mem::take(&mut chunk));
            }
            continue;
        }
        if w == "&" {
            chunk.push("and".into());
            continue;
        }
        chunk.push(w.clone());
    }
    if !chunk.is_empty() {
        chunks.push(chunk);
    }
    (chunks, parent)
}
/// Files whose name (without the extension) is what the phrase says, with how good the match is.
fn match_files<'a>(ctx: &'a Ctx, phrase: &[String]) -> (Vec<&'a FileCandidate>, i32) {
    let whole: Option<String> = phrase
        .iter()
        .any(|w| matches!(w.as_str(), "and" | "&"))
        .then(|| phrase.iter().map(|w| alnum(w)).collect());
    let words: Vec<&String> = phrase
        .iter()
        .filter(|w| !FILLER.contains(&w.as_str()))
        .collect();
    let norm: String = words.iter().map(|w| alnum(w)).collect();
    if norm.len() < 4 {
        return (vec![], 0);
    }
    let mut hits: Vec<(i32, &FileCandidate)> = vec![];
    for f in ctx.files {
        if in_build_output(&f.relative_path) {
            continue;
        }
        let Some(stem) = f.relative_path.file_stem().map(|n| n.to_string_lossy()) else {
            continue;
        };
        let stem_norm = alnum(&stem);
        let tokens: Vec<String> = stem
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .map(|t| t.to_lowercase())
            .collect();
        let tier = if stem_norm == norm || whole.as_deref() == Some(stem_norm.as_str()) {
            10
        } else if norm.len() >= 5 && stem_norm.starts_with(&norm)
            || whole
                .as_deref()
                .is_some_and(|w| w.len() >= 6 && stem_norm.contains(w))
        {
            8
        } else if norm.len() >= 6
            && tokens
                .first()
                .is_some_and(|t| t.len() >= 5 && edit_distance(t, &norm) <= 1)
        {
            5
        } else {
            continue;
        };
        hits.push((tier, f));
    }
    let best = hits.iter().map(|(t, _)| *t).max().unwrap_or(0);
    hits.retain(|(t, _)| *t == best);
    hits.sort_by_key(|(_, f)| {
        (
            f.relative_path.components().count(),
            std::cmp::Reverse(f.size),
        )
    });
    (hits.into_iter().map(|(_, f)| f).collect(), best)
}
/// What one spoken name refers to: a folder (or several of the same name) or a few files.
enum Named<'a> {
    Folders(Vec<&'a Folder>, i32),
    Files(Vec<&'a FileCandidate>, i32),
    Nothing,
}
fn named<'a>(ctx: &'a Ctx, phrase: &[String], folders: &'a [Folder]) -> Named<'a> {
    let (found, ft) = resolve_q(phrase, folders);
    let (files, xt) = match_files(ctx, phrase);
    // A file is only trusted when the name is specific: a few files at most, a real name match.
    let file_ok = xt >= 5 && !files.is_empty() && files.len() <= 3;
    if file_ok && xt > ft {
        Named::Files(files, xt)
    } else if !found.is_empty() {
        Named::Folders(found, ft)
    } else if file_ok {
        Named::Files(files, xt)
    } else {
        Named::Nothing
    }
}
fn named_score(n: &Named) -> i32 {
    match n {
        Named::Folders(_, t) | Named::Files(_, t) => *t,
        Named::Nothing => -5,
    }
}
/// Splits a chunk like “beer and plunder and pokemon games” into names. “and” may separate two things or sit
/// inside one name (“life and hell”), so every split is tried and the one where the most names resolve wins.
fn best_split(ctx: &Ctx, chunk: &[String], folders: &[Folder]) -> Vec<Vec<String>> {
    let strip = |w: &[String]| -> Vec<String> {
        let mut v: Vec<String> = w.to_vec();
        while v.first().is_some_and(|x| FILLER.contains(&x.as_str())) {
            v.remove(0);
        }
        while v.last().is_some_and(|x| FILLER.contains(&x.as_str())) {
            v.pop();
        }
        v
    };
    let soft: Vec<usize> = chunk
        .iter()
        .enumerate()
        .filter(|(_, w)| w.as_str() == "and")
        .map(|(i, _)| i)
        .collect();
    let make = |mask: u32| -> Vec<Vec<String>> {
        let mut out = vec![];
        let mut cur: Vec<String> = vec![];
        let mut k = 0;
        for (i, w) in chunk.iter().enumerate() {
            if soft.get(k) == Some(&i) {
                let split = soft.len() > 12 || mask & (1 << k) != 0;
                k += 1;
                if split {
                    let p = strip(&cur);
                    if !p.is_empty() {
                        out.push(p);
                    }
                    cur.clear();
                    continue;
                }
            }
            cur.push(w.clone());
        }
        let p = strip(&cur);
        if !p.is_empty() {
            out.push(p);
        }
        out
    };
    if soft.is_empty() || soft.len() > 12 {
        return make(u32::MAX);
    }
    let mut best: Option<(i32, i32, Vec<Vec<String>>)> = None;
    for mask in 0..(1u32 << soft.len()) {
        let parts = make(mask);
        if parts.is_empty() {
            continue;
        }
        // Weight each name by the letters it explains, so “life and hell” beats “life” + “hell”.
        let score: i32 = parts
            .iter()
            .map(|p| {
                let letters: i32 = p.iter().map(|w| alnum(w).len() as i32).sum();
                named_score(&named(ctx, p, folders)) * letters
            })
            .sum();
        let key = (score, parts.len() as i32);
        if best.as_ref().is_none_or(|(s, n, _)| key > (*s, *n)) {
            best = Some((key.0, key.1, parts));
        }
    }
    best.map(|(_, _, p)| p).unwrap_or_default()
}
fn trash_folders(
    ctx: &Ctx,
    words: &[String],
    folders: &[Folder],
    verb_at: usize,
) -> Option<Investigation> {
    let (chunks, parent) = phrases_for_folders(&words[verb_at + 1..]);
    let phrases: Vec<Vec<String>> = chunks
        .iter()
        .flat_map(|c| best_split(ctx, c, folders))
        .collect();
    if phrases.is_empty() {
        return None;
    }
    let parent_filter: Option<Vec<&Folder>> = parent.as_ref().map(|p| resolve(p, folders));
    let mut chosen: Vec<&Folder> = Vec::new();
    let mut chosen_files: Vec<&FileCandidate> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    let label = |p: &[String]| {
        p.iter()
            .filter(|w| w.as_str() != "and" || p.len() > 2)
            .cloned()
            .collect::<Vec<_>>()
            .join(" ")
    };
    for phrase in &phrases {
        let wants_all = words
            .iter()
            .any(|w| matches!(w.as_str(), "all" | "every" | "each"));
        match named(ctx, phrase, folders) {
            Named::Nothing => missing.push(label(phrase)),
            Named::Files(files, _) => {
                for f in files {
                    if !chosen_files.iter().any(|c| c.id == f.id) {
                        chosen_files.push(f);
                    }
                }
            }
            Named::Folders(mut found, tier) => {
                if let Some(parents) = &parent_filter
                    && !parents.is_empty()
                {
                    // A vague parent hint only narrows the choice; it never removes every candidate.
                    let narrowed: Vec<&Folder> = found
                        .iter()
                        .copied()
                        .filter(|f| {
                            parents
                                .iter()
                                .any(|p| f.path.starts_with(&p.path) && f.path != p.path)
                        })
                        .collect();
                    if !narrowed.is_empty() {
                        found = narrowed;
                    }
                }
                let Some((best, rest)) = found.split_first() else {
                    missing.push(label(phrase));
                    continue;
                };
                if wants_all
                    && !rest.is_empty()
                    && rest.iter().all(|f| f.name_norm == best.name_norm)
                {
                    chosen.extend(found.iter().copied().take(FOLDER_BATCH));
                } else {
                    chosen.push(best);
                    if tier < 5 {
                        notes.push(format!(
                            "“{}” is only a loose match for {} — uncheck it if that isn’t what you meant.",
                            label(phrase),
                            best.path.display()
                        ));
                    }
                    if !rest.is_empty() {
                        notes.push(format!(
                            "“{}” also matches {}; I picked the largest ({}).",
                            label(phrase),
                            rest.iter()
                                .take(3)
                                .map(|f| f.path.display().to_string())
                                .collect::<Vec<_>>()
                                .join(", "),
                            best.path.display()
                        ));
                    }
                }
            }
        }
    }
    if chosen.is_empty() && chosen_files.is_empty() {
        return None;
    }
    let chosen = outermost(chosen);
    let mut seen = HashSet::new();
    let chosen: Vec<&Folder> = chosen
        .into_iter()
        .filter(|f| seen.insert(f.path.clone()))
        .collect();
    let targets: Vec<FolderTarget> = chosen.iter().map(|f| target(f)).collect();
    let total: u64 = targets.iter().map(|t| t.bytes).sum::<u64>()
        + chosen_files.iter().map(|f| f.size).sum::<u64>();
    let files: usize = targets.iter().map(|t| t.files).sum::<usize>() + chosen_files.len();
    let mut r = reply(
        ctx,
        "trash_named_files",
        "Understood your request",
        format!(
            "Move {} to Trash",
            targets
                .iter()
                .map(|t| t.path.clone())
                .chain(
                    chosen_files
                        .iter()
                        .map(|f| f.relative_path.display().to_string())
                )
                .collect::<Vec<_>>()
                .join(", ")
        ),
        String::new(),
    );
    let what = match (targets.len(), chosen_files.len()) {
        (1, 0) => format!("“{}”", targets[0].path),
        (n, 0) => plural(n, "folder", "folders"),
        (0, n) => plural(n, "file", "files"),
        (a, b) => format!(
            "{} and {}",
            plural(a, "folder", "folders"),
            plural(b, "file", "files")
        ),
    };
    r.proposal.rationale = format!(
        "{what} — {} in {} ready for the Trash. Each moves whole and stays recoverable from Finder’s Trash (Put Back). Nothing happens until you approve.{}{}",
        bytes(total),
        plural(files, "file", "files"),
        if notes.is_empty() {
            String::new()
        } else {
            format!(" {}", notes.join(" "))
        },
        if missing.is_empty() {
            String::new()
        } else {
            format!(" I couldn’t find anything called {}.", missing.join(", "))
        },
    );
    r.proposal.actions = chosen_files
        .iter()
        .map(|f| ProposedAction::Trash { source: f.id })
        .collect();
    r.sources = chosen_files
        .iter()
        .map(|f| Source {
            id: f.id.0,
            path: f.relative_path.to_string_lossy().into(),
            size: f.size,
        })
        .collect();
    r.folders = targets;
    Some(r)
}
fn organize(ctx: &Ctx, words: &[String]) -> Option<Investigation> {
    let known: HashSet<String> = name_tokens(ctx.scope).into_iter().collect();
    let generic = [
        "file",
        "files",
        "folder",
        "folders",
        "everything",
        "stuff",
        "up",
        "out",
        "by",
        "type",
        "types",
        "category",
        "categories",
        "kind",
        "kinds",
        "extension",
        "extensions",
        "format",
        "date",
        "dates",
        "month",
        "months",
        "year",
        "years",
        "time",
        "modified",
        "created",
        "into",
        "sub",
        "subfolders",
        "subfolder",
        "here",
        "messy",
        "mess",
        "downloads",
        "documents",
        "desktop",
        "automatically",
        "smart",
        "smartly",
        "properly",
        "nicely",
        "cleanly",
        "clean",
        "sort",
        "sorted",
        "organize",
        "organise",
        "tidy",
        "group",
        "arrange",
        "categorize",
        "categorise",
        "declutter",
        "structure",
        "classify",
        "on",
        "with",
        "using",
        "as",
        "based",
        "according",
        "per",
        "each",
        "different",
        "separate",
        "own",
        "so",
        "that",
        "it's",
        "its",
        "is",
        "are",
        "not",
    ];
    if words.iter().any(|w| {
        !FILLER.contains(&w.as_str())
            && !generic.contains(&w.as_str())
            && !known.contains(w)
            && w.parse::<i32>().is_err()
    }) {
        return None;
    }
    let by_date = has(
        words,
        &[
            "date", "dates", "month", "months", "year", "years", "time", "modified", "created",
        ],
    );
    let mode = if by_date {
        OrganizationMode::Date
    } else {
        OrganizationMode::Category
    };
    let mut proposal = tidy_organization::propose(mode, None, &ctx.files.to_vec());
    let matched = proposal.actions.len();
    let by_id: std::collections::HashMap<u64, &FileCandidate> =
        ctx.files.iter().map(|f| (f.id.0, f)).collect();
    proposal.actions.truncate(BATCH);
    let sources = proposal
        .actions
        .iter()
        .filter_map(|a| match a {
            ProposedAction::Move { source, .. } => by_id.get(&source.0).map(|f| Source {
                id: f.id.0,
                path: f.relative_path.to_string_lossy().into(),
                size: f.size,
            }),
            _ => None,
        })
        .collect();
    let mut r = reply(
        ctx,
        if by_date {
            "organize_by_date"
        } else {
            "organize_by_type"
        },
        "Understood your request",
        format!(
            "Organize top-level files by {}",
            if by_date { "date" } else { "type" }
        ),
        String::new(),
    );
    r.proposal.rationale = if proposal.actions.is_empty() {
        format!(
            "Nothing to organize: {} Nothing changed.",
            proposal.rationale
        )
    } else {
        format!(
            "{} Review the folders on the left, uncheck anything you want left alone, then approve. Moves can be undone from History.{}",
            proposal.rationale,
            if matched > BATCH {
                format!(
                    " This batch covers {BATCH} of {matched} files; approve it and ask again for the rest."
                )
            } else {
                String::new()
            }
        )
    };
    r.proposal.actions = proposal.actions;
    r.sources = sources;
    r.remaining_matches = matched.saturating_sub(BATCH);
    r.complete = r.remaining_matches == 0;
    Some(r)
}

pub fn respond(
    request: &str,
    files: &[FileCandidate],
    scope_name: &str,
    now: i64,
) -> Option<Investigation> {
    respond_in(request, files, scope_name, now, None)
}
pub fn respond_in(
    request: &str,
    files: &[FileCandidate],
    scope_name: &str,
    now: i64,
    root: Option<&Path>,
) -> Option<Investigation> {
    let ctx = Ctx {
        files,
        scope: scope_name,
        now,
        root,
    };
    let (first, follow) = match request.split_once("User follow-up:") {
        Some((a, b)) => (a.trim(), b.trim()),
        None => (request.trim(), ""),
    };
    let folders = folders_of(files);
    if !follow.is_empty() {
        // "delete them" / "all of them" after a list of folders: act on that list.
        let words = tokens(follow);
        let pronoun = !words.is_empty()
            && words.len() <= 6
            && has(&words, &["them", "those", "these", "all", "everything"])
            && has(&words, &["delete", "remove", "trash", "erase", "discard"]);
        if pronoun {
            let (w, r) = strip_purpose(tokens(first), raw_tokens(first));
            let _ = r;
            if let Some(list) = list_named(&ctx, &w, &folders, false, false) {
                let total: u64 = list.folders.iter().map(|f| f.bytes).sum();
                let mut out = reply(
                    &ctx,
                    "trash_named_files",
                    "Understood your request",
                    format!("Move {} folders to Trash", list.folders.len()),
                    format!(
                        "{} ({}) ready for the Trash. Each moves whole and stays recoverable from Finder’s Trash. Nothing happens until you approve.",
                        plural(list.folders.len(), "folder", "folders"),
                        bytes(total)
                    ),
                );
                out.folders = list.folders;
                return Some(out);
            }
        }
    }
    let last = if follow.is_empty() { first } else { follow };
    if let Some(r) = multi_task(&ctx, last, &folders) {
        return Some(r);
    }
    interpret(&ctx, last, &folders).or_else(|| {
        if last != request.trim() {
            interpret(&ctx, request, &folders)
        } else {
            None
        }
    })
}
fn interpret(ctx: &Ctx, text: &str, folders: &[Folder]) -> Option<Investigation> {
    let words = tokens(text);
    if words.is_empty() {
        return None;
    }
    let greeting = words.len() <= 4
        && has(
            &words,
            &[
                "hi", "hello", "hey", "help", "yo", "bonjour", "salut", "thanks", "thank",
            ],
        );
    if greeting
        || contains_seq(&words, &["what", "can", "you", "do"])
        || contains_seq(&words, &["how", "do", "you", "work"])
        || words == ["?"]
    {
        return Some(help(ctx));
    }
    let raw = raw_tokens(text);
    let (words, raw) = strip_purpose(words, raw);
    if let Some(r) = structural(ctx, &raw, &words, folders) {
        return Some(r);
    }
    let mut verb = detect_verb(&words)?;
    if verb == Verb::Organize && parse_criteria(&words, folders).dev {
        verb = Verb::Trash;
    }
    let verb_at = words
        .iter()
        .position(|w| match verb {
            Verb::Trash => [
                "delete",
                "remove",
                "trash",
                "erase",
                "discard",
                "wipe",
                "purge",
                "uninstall",
                "bin",
                "destroy",
                "dump",
                "nuke",
                "drop",
                "get",
                "throw",
                "clean",
                "clear",
            ]
            .contains(&w.as_str()),
            _ => false,
        })
        .map(|at| {
            if words[at] == "get"
                || words[at] == "throw"
                || words[at] == "clean"
                || words[at] == "clear"
            {
                at + 1
            } else {
                at
            }
        })
        .unwrap_or(0);
    match verb {
        Verb::Trash => {
            let c = parse_criteria(&words, folders);
            // "delete all my modrinth profiles": show the profiles to pick from instead of
            // guessing that the whole launcher folder is meant.
            if (has(&words, &["all", "every", "my", "the", "those", "these"])
                || split_filter(&words).1.is_some())
                && let Some(r) = list_named(ctx, &words, folders, true, true)
            {
                return Some(r);
            }
            let quoted_or_folderish = has(
                &words,
                &["folder", "folders", "directory", "directories", "dir"],
            );
            if c.dev {
                let mut artifacts = artifact_dirs(ctx.files);
                if let Some(folder) = &c.in_folder {
                    artifacts.retain(|a| Path::new(&a.path).starts_with(folder));
                }
                let total = artifacts.len();
                artifacts.truncate(FOLDER_BATCH);
                let mut r = reply(
                    ctx,
                    "dev_artifacts",
                    "Understood your request",
                    "Move build and dependency folders to Trash".into(),
                    String::new(),
                );
                if artifacts.is_empty() {
                    r.proposal.rationale = "I found no node_modules, caches or build folders in the index, so there is nothing to remove.".into();
                } else {
                    let bytes_total: u64 = artifacts.iter().map(|a| a.bytes).sum();
                    r.proposal.rationale = format!(
                        "I found {} of dependency, cache and build output ({} in total across {}). They can be rebuilt by your tools. Review and approve to send them to the Trash.{}",
                        bytes(bytes_total),
                        bytes_total_files(&artifacts),
                        plural(total, "folder", "folders"),
                        if total > FOLDER_BATCH {
                            format!(
                                " This batch has the {FOLDER_BATCH} largest; ask again for the rest."
                            )
                        } else {
                            String::new()
                        }
                    );
                    r.folders = artifacts;
                    r.remaining_matches = total.saturating_sub(FOLDER_BATCH);
                    r.complete = r.remaining_matches == 0;
                }
                return Some(r);
            }
            let folder_first = quoted_or_folderish || !c.targets_files();
            if folder_first {
                if let Some(r) = trash_folders(ctx, &words, folders, verb_at) {
                    return Some(r);
                }
                if quoted_or_folderish && !c.targets_files() {
                    let named = words[verb_at + 1..]
                        .iter()
                        .filter(|w| !FILLER.contains(&w.as_str()))
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(" ");
                    let mut close: Vec<&Folder> = folders.iter().collect();
                    close.sort_by_key(|f| std::cmp::Reverse(f.bytes));
                    let mut r = reply(
                        ctx,
                        "trash_named_files",
                        "Looked for the folder",
                        format!("No indexed folder matched “{named}”"),
                        format!(
                            "I couldn’t find a folder called “{named}” in {}. The largest folders I can see are: {}. Say one of those names and I’ll prepare it.",
                            ctx.scope,
                            close
                                .iter()
                                .take(5)
                                .map(|f| f.path.display().to_string())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    );
                    r.clarification = Some(r.proposal.rationale.clone());
                    return Some(r);
                }
            }
            if c.targets_files() {
                return Some(trash_files(ctx, &c));
            }
            None
        }
        Verb::Organize => organize(ctx, &words),
        Verb::Show => {
            // Questions about the last answer ("how come…", "why…") are not file searches.
            if contains_seq(&words, &["how", "come"]) || words.first().is_some_and(|w| w == "why") {
                return None;
            }
            // "what's inside X" is an explicit request to open one folder.
            if has(&words, &["inside", "contents"])
                && let Some(r) = folder_contents(ctx, &words, folders)
            {
                return Some(r);
            }
            // "find <words>" searches names; only "list all my <things>" maps a noun to folders.
            if !has(&words, &["find", "search", "where", "locate", "look"])
                && let Some(r) = list_named(ctx, &words, folders, false, false)
            {
                return Some(r);
            }
            let overview_words = has(
                &words,
                &[
                    "space", "storage", "biggest", "largest", "heaviest", "disk", "taking", "takes",
                ],
            );
            if !overview_words && let Some(r) = folder_contents(ctx, &words, folders) {
                return Some(r);
            }
            let wants_overview = has(
                &words,
                &[
                    "space",
                    "storage",
                    "biggest",
                    "largest",
                    "heaviest",
                    "big",
                    "large",
                    "summary",
                    "overview",
                    "everything",
                    "folder",
                    "folders",
                    "taking",
                    "takes",
                    "take",
                    "disk",
                    "many",
                    "much",
                    "describe",
                    "explain",
                    "analyze",
                    "analyse",
                    "inspect",
                    "summarize",
                    "summarise",
                    "count",
                ],
            ) || contains_seq(&words, &["what's", "in"])
                || contains_seq(&words, &["what", "is", "in"]);
            let c = parse_criteria(&words, folders);
            if wants_overview
                && !(c.targets_files() && !has(&words, &["biggest", "largest", "heaviest"]))
            {
                return Some(overview(ctx, folders, &words));
            }
            if c.targets_files() {
                let mut hits: Vec<&FileCandidate> = ctx
                    .files
                    .iter()
                    .filter(|f| matches_file(f, &c, ctx.now))
                    .collect();
                hits.sort_by_key(|f| std::cmp::Reverse(f.size));
                let total = hits.len();
                let total_bytes: u64 = hits.iter().map(|f| f.size).sum();
                let mut text = if total == 0 {
                    format!("No files match {}.", c.label)
                } else {
                    format!(
                        "{} match {} — {} in total. Say “delete them” and I’ll prepare these for the Trash.",
                        plural(total, "file", "files"),
                        c.label,
                        bytes(total_bytes)
                    )
                };
                if total > 40 {
                    text.push_str(" Showing the biggest 40.");
                }
                let mut r = reply(
                    ctx,
                    "find_filename",
                    "Filtered the index",
                    format!("Checked all {} indexed files", ctx.files.len()),
                    text,
                );
                if total > 0 {
                    r.sections.push(Section {
                        title: format!("Matching files ({total})"),
                        items: hits.iter().take(40).map(|f| file_item(f)).collect(),
                    });
                }
                return Some(r);
            }
            find(ctx, &words)
        }
    }
}
fn bytes_total_files(items: &[FolderTarget]) -> String {
    plural(items.iter().map(|i| i.files).sum(), "file", "files")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidy_organization::FileId;
    fn f(id: u64, path: &str, size: u64, modified: i64) -> FileCandidate {
        FileCandidate {
            id: FileId(id),
            relative_path: path.into(),
            size,
            modified,
            excerpt: None,
        }
    }
    fn fixture() -> Vec<FileCandidate> {
        vec![
            f(
                1,
                "curseforge/minecraft/Instances/Lucky World Invasion/mods/a.jar",
                800,
                0,
            ),
            f(
                2,
                "curseforge/minecraft/Instances/Lucky World Invasion/saves/w.dat",
                100,
                0,
            ),
            f(
                3,
                "curseforge/minecraft/Instances/FTB StoneBlock 4/x.jar",
                1000,
                0,
            ),
            f(
                4,
                "curseforge/minecraft/Instances/Cobblemon Academy 2.0/y.jar",
                500,
                0,
            ),
            f(5, "Downloads/old.log", 10, 100),
            f(6, "Downloads/new.log", 10, 9_000_000_000),
            f(7, "Downloads/setup.dmg", 4_000, 100),
            f(8, "proj/node_modules/x/index.js", 5, 0),
            f(9, "proj/node_modules/y/index.js", 5, 0),
            f(10, "proj/src/main.rs", 5, 0),
            f(11, "Downloads/photo.png", 50, 0),
            f(12, "Downloads/Screenshot 2026-01-01.png", 60, 0),
        ]
    }
    fn games_fixture() -> Vec<FileCandidate> {
        vec![
            f(1, "Games/Crossover/a.txt", 100, 0),
            f(2, "Games/liveandhell-template/x.txt", 100, 0),
            f(3, "Games/Beer and Plunder/y.txt", 100, 0),
            f(4, "Games/Pokemon Games/z.txt", 100, 0),
            f(5, "Games/clocktower-final/c.txt", 100, 0),
            f(6, "Videos/fin retour bateau.mp4", 5_000, 0),
        ]
    }
    #[test]
    fn several_tasks_in_one_message_become_one_plan() {
        let q = "delete crossover, life and hell, beer and plunder and pokemon games. Then rename fin retour bateau.mp4 to BSLFILMBateau.mp4";
        let r = respond(q, &games_fixture(), "Documents", 9_000_000_100).expect("handled");
        let trashed: Vec<&str> = r.folders.iter().map(|f| f.path.as_str()).collect();
        for want in [
            "Crossover",
            "liveandhell-template",
            "Beer and Plunder",
            "Pokemon Games",
        ] {
            assert!(
                trashed.iter().any(|p| p.ends_with(want)),
                "{want} missing: {trashed:?} / {}",
                r.proposal.rationale
            );
        }
        assert!(
            !trashed.iter().any(|p| p.contains("clocktower")),
            "{trashed:?}"
        );
        assert_eq!(r.proposal.actions.len(), 1, "{}", r.proposal.rationale);
        match &r.proposal.actions[0] {
            ProposedAction::Rename { source, new_name } => {
                assert_eq!(*source, FileId(6));
                assert_eq!(new_name, "BSLFILMBateau.mp4");
            }
            other => panic!("{other:?}"),
        }
        assert!(
            !r.proposal.rationale.contains("Not included"),
            "{}",
            r.proposal.rationale
        );
    }
    #[test]
    fn the_exact_sentence_with_a_typo_still_becomes_one_plan() {
        let q = "delete corssover, life and hell, beer and plunder and pokemon games. Then rename fin retour bateau.mp4 to BSLFILMBateau.mp4";
        let r = respond(q, &games_fixture(), "Documents", 9_000_000_100).expect("handled");
        let trashed: Vec<&str> = r.folders.iter().map(|f| f.path.as_str()).collect();
        println!("{trashed:?}\n{}", r.proposal.rationale);
        assert!(
            trashed.iter().any(|p| p.ends_with("Crossover")),
            "{trashed:?}\n{}",
            r.proposal.rationale
        );
        assert_eq!(r.proposal.actions.len(), 1, "{}", r.proposal.rationale);
        assert!(!trashed.iter().any(|p| p.contains("clocktower")));
    }
    fn real_world_fixture() -> Vec<FileCandidate> {
        vec![
            f(1, "Downloads/crossover-26.2.0.zip", 90_000, 0),
            f(2, "Downloads/beer-and-plunder-mac-universal.zip", 40_000, 0),
            f(
                3,
                "Coding Projects/liveandhell-template-1.21.11/src/a.java",
                500,
                0,
            ),
            f(
                4,
                "Coding Projects/lifeandhellcards-1.21.11/src/b.java",
                300,
                0,
            ),
            f(
                5,
                "Coding Projects/between-life-hell-website/index.html",
                200,
                0,
            ),
            f(6, "Coding Projects/Docker_code/Hello World/main.py", 100, 0),
            f(7, "Games/Project Pokemon/rom.nds", 9_000, 0),
            f(8, "Videos/fin retour bateau.mp4", 5_000, 0),
            f(
                9,
                "Coding Projects/GameLegacy/node_modules/lucide-react/dist/esm/icons/beer-off.mjs",
                5,
                0,
            ),
            f(10, "Downloads/beer.png", 50, 0),
        ]
    }
    #[test]
    fn a_list_with_and_inside_names_resolves_each_name_to_a_folder_or_a_file() {
        let q = "delete corssover, life and hell, beer and plunder and pokemon games. Then rename fin retour bateau.mp4 to BSLFILMBateau.mp4";
        let r = respond(q, &real_world_fixture(), "Home", 9_000_000_100).expect("handled");
        let folders: Vec<&str> = r.folders.iter().map(|f| f.path.as_str()).collect();
        println!("{folders:?}\n{}", r.proposal.rationale);
        // Files: the two zips are trashed, the .mp4 is renamed.
        let trashed: Vec<u64> = r
            .proposal
            .actions
            .iter()
            .filter_map(|a| match a {
                ProposedAction::Trash { source } => Some(source.0),
                _ => None,
            })
            .collect();
        assert!(
            trashed.contains(&1) && trashed.contains(&2),
            "{trashed:?}\n{}",
            r.proposal.rationale
        );
        assert!(
            !trashed.contains(&10) && !trashed.contains(&9),
            "{trashed:?}"
        );
        assert!(r.proposal.actions.iter().any(|a| matches!(a, ProposedAction::Rename { source, new_name } if source.0 == 8 && new_name == "BSLFILMBateau.mp4")));
        // Folders: “life and hell” is the folder that says so, never “Hello World”.
        assert!(
            folders
                .iter()
                .any(|p| p.ends_with("lifeandhellcards-1.21.11")),
            "{folders:?}"
        );
        assert!(
            !folders.iter().any(|p| p.contains("Hello World")),
            "{folders:?}"
        );
        assert!(
            !r.proposal.rationale.contains("couldn’t find"),
            "{}",
            r.proposal.rationale
        );
    }
    #[test]
    fn any_number_of_tasks_in_one_message_are_all_planned() {
        let q = "delete old.log. rename setup.dmg to Installer and move it into Archive. create a folder called Logs and move photo.png into it. Then delete new.log";
        let r = respond(q, &fixture(), "Documents", 9_000_000_100).expect("handled");
        println!("{}", r.proposal.rationale);
        assert_eq!(r.proposal.actions.len(), 4, "{}", r.proposal.rationale);
        assert!(
            r.proposal.rationale.starts_with("4 tasks"),
            "{}",
            r.proposal.rationale
        );
        assert!(!r.proposal.rationale.contains("Not included"));
        let q5 = "delete old.log and rename setup.dmg to Installer and delete new.log and move photo.png into Keep";
        let r = respond(q5, &fixture(), "Documents", 9_000_000_100).expect("handled");
        assert_eq!(r.proposal.actions.len(), 4, "{}", r.proposal.rationale);
    }
    #[test]
    fn a_file_name_with_an_extension_never_matches_a_folder() {
        // “fin” must not fuzzy-match the clocktower-final folder.
        let r = respond(
            "rename fin retour bateau.mp4 to BSLFILMBateau.mp4",
            &games_fixture(),
            "Documents",
            9_000_000_100,
        )
        .expect("handled");
        assert!(matches!(
            r.proposal.actions.as_slice(),
            [ProposedAction::Rename { source, .. }] if *source == FileId(6)
        ));
        let r = respond(
            "rename nothing here.mp4 to x.mp4",
            &games_fixture(),
            "Documents",
            9_000_000_100,
        )
        .expect("handled");
        assert!(r.proposal.actions.is_empty() && r.clarification.is_some());
    }
    #[test]
    fn tasks_are_split_on_sentences_and_then() {
        assert_eq!(
            split_tasks("delete a b. Then rename c.mp4 to d.mp4"),
            vec!["delete a b", "rename c.mp4 to d.mp4"]
        );
        assert_eq!(
            split_tasks(
                "delete old.log and then move setup.dmg into Archive, after that create a folder Z"
            ),
            vec![
                "delete old.log",
                "move setup.dmg into Archive",
                "create a folder Z"
            ]
        );
        assert_eq!(split_tasks("rename x.mp4 to y.mp4").len(), 1);
    }
    #[test]
    fn a_task_that_cannot_be_understood_is_named_not_dropped() {
        let r = respond(
            "delete old.log files older than 3 months. Then frobnicate the widgets",
            &fixture(),
            "Documents",
            9_000_000_100,
        );
        let r = r.expect("handled");
        // With a single understood task the normal path still answers; with two, the rest is named.
        let q = "delete old.log files older than 3 months. Then rename setup.dmg to Installer. Then frobnicate the widgets";
        let r2 = respond(q, &fixture(), "Documents", 9_000_000_100).expect("handled");
        assert!(
            r2.proposal.rationale.contains("Not included"),
            "{}",
            r2.proposal.rationale
        );
        assert!(
            r2.proposal.rationale.contains("frobnicate"),
            "{}",
            r2.proposal.rationale
        );
        assert!(!r2.proposal.actions.is_empty());
        let _ = r;
    }
    fn only_action(q: &str) -> ProposedAction {
        let r = run(q);
        assert_eq!(r.proposal.actions.len(), 1, "{q}: {}", r.proposal.rationale);
        r.proposal.actions[0].clone()
    }
    #[test]
    fn multi_step_rename_then_move_a_file_is_one_plan() {
        for q in [
            "rename setup.dmg to Installer and move it into Archive",
            "rename setup.dmg to Installer, then move it to Archive",
            "move setup.dmg into Archive and rename it to Installer",
            "please rename the file setup.dmg as Installer and put it in Archive",
        ] {
            match only_action(q) {
                ProposedAction::Move {
                    source,
                    destination_relative,
                } => {
                    assert_eq!(source, FileId(7), "{q}");
                    assert_eq!(
                        destination_relative,
                        PathBuf::from("Archive/Installer.dmg"),
                        "{q}"
                    );
                }
                other => panic!("{q}: {other:?}"),
            }
        }
    }
    #[test]
    fn multi_step_rename_and_move_a_folder() {
        match only_action("rename the FTB StoneBlock 4 folder to StoneBlock and move it into proj")
        {
            ProposedAction::MoveFolder {
                source,
                destination_relative,
            } => {
                assert!(source.ends_with("FTB StoneBlock 4"), "{source:?}");
                assert_eq!(destination_relative, PathBuf::from("proj/StoneBlock"));
            }
            other => panic!("{other:?}"),
        }
    }
    #[test]
    fn multi_step_refuses_to_overwrite_and_reports_missing_items() {
        let r = run("rename old.log to new.log and move it into Downloads");
        assert!(r.proposal.actions.is_empty());
        assert!(
            r.proposal.rationale.contains("already exists"),
            "{}",
            r.proposal.rationale
        );
        let r = run("rename nosuchfile.txt to x and move it into Archive");
        assert!(r.proposal.actions.is_empty());
        assert!(r.clarification.is_some());
    }
    #[test]
    fn multi_step_create_a_folder_and_move_into_it() {
        match only_action("create a folder called Installers and move setup.dmg into it") {
            ProposedAction::Move {
                destination_relative,
                ..
            } => assert_eq!(destination_relative, PathBuf::from("Installers/setup.dmg")),
            other => panic!("{other:?}"),
        }
        let r = run("make a new folder named Logs and then move all log files into it");
        assert_eq!(r.proposal.actions.len(), 2, "{}", r.proposal.rationale);
        assert!(r.proposal.actions.iter().all(|a| matches!(
            a,
            ProposedAction::Move { destination_relative, .. } if destination_relative.starts_with("Logs")
        )));
    }
    #[test]
    fn single_step_rename_and_move_still_work() {
        assert!(matches!(
            only_action("rename setup.dmg to Installer"),
            ProposedAction::Rename { .. }
        ));
        assert!(matches!(
            only_action("move setup.dmg into Archive"),
            ProposedAction::Move { .. }
        ));
    }
    fn run(q: &str) -> Investigation {
        respond(q, &fixture(), "Documents", 9_000_000_100)
            .unwrap_or_else(|| panic!("unhandled: {q}"))
    }
    #[test]
    fn folder_delete_resolves_fuzzy_names_and_never_asks() {
        for q in [
            "delete the Lucky World Invasion folder",
            "remove lucky world",
            "Please trash the folder lucky world invasion",
            "can you delete Lucky World Invasion for me",
        ] {
            let r = run(q);
            assert!(r.clarification.is_none());
            assert_eq!(r.folders.len(), 1, "{q}");
            assert_eq!(
                r.folders[0].path,
                "curseforge/minecraft/Instances/Lucky World Invasion"
            );
            assert_eq!(r.folders[0].files, 2);
        }
    }
    #[test]
    fn folder_inside_parent_and_multiple() {
        let r = run("delete FTB StoneBlock 4 and Cobblemon Academy 2.0");
        assert_eq!(r.folders.len(), 2);
        let r = run("remove Lucky World Invasion from Instances");
        assert_eq!(r.folders.len(), 1);
    }
    #[test]
    fn criteria_and_age() {
        let r = run("delete all .log files older than 3 months");
        assert_eq!(r.proposal.actions.len(), 1);
        assert_eq!(r.sources[0].path, "Downloads/old.log");
        let r = run("remove installers");
        assert_eq!(r.sources[0].path, "Downloads/setup.dmg");
        let r = run("delete my screenshots");
        assert_eq!(r.proposal.actions.len(), 1);
        let r = run("delete the 2 biggest files");
        assert_eq!(r.proposal.actions.len(), 2);
        assert_eq!(r.sources[0].path, "Downloads/setup.dmg");
    }
    #[test]
    fn exclusions_and_negation() {
        let r = run("delete all log files except new");
        assert_eq!(r.proposal.actions.len(), 1);
        assert!(respond("don't delete anything", &fixture(), "Documents", 0).is_none());
    }
    #[test]
    fn dev_artifacts_become_folders() {
        let r = run("clean up node_modules and build artifacts");
        assert_eq!(r.folders.len(), 1);
        assert_eq!(r.folders[0].path.replace('\\', "/"), "proj/node_modules");
        assert_eq!(r.folders[0].files, 2);
    }
    #[test]
    fn info_and_help() {
        let r = run("what's taking the most space?");
        // Only the top-level folders are listed, as folder rows with sizes, never nested paths.
        let top = &r.sections[0];
        assert!(top.title.starts_with("Folders in"));
        assert!(
            top.items
                .iter()
                .any(|i| i.path == "curseforge" && i.kind == "folder")
        );
        assert!(
            top.items.iter().all(|i| !i.path.contains('/')),
            "{:?}",
            top.items.iter().map(|i| &i.path).collect::<Vec<_>>()
        );
        assert!(r.proposal.actions.is_empty());
        // Opening a folder shows its own subfolders and files.
        let r = run("what's inside curseforge/minecraft/Instances");
        assert!(
            r.sections[0]
                .items
                .iter()
                .any(|i| i.path.ends_with("Lucky World Invasion"))
        );
        assert!(run("hi").proposal.rationale.contains("Delete the Lucky"));
        let r = run("find setup");
        assert!(
            r.sections[0]
                .items
                .iter()
                .any(|i| i.path.ends_with("setup.dmg") && i.kind == "file")
        );
    }
    #[test]
    fn missing_folder_explains_instead_of_asking() {
        let r = run("delete the Nonexistent Things folder");
        assert!(r.folders.is_empty());
        assert!(r.proposal.rationale.contains("couldn’t find"));
    }
    #[test]
    fn organize_by_type_defers_specific_requests() {
        assert!(respond("organize this folder by type", &fixture(), "Documents", 0).is_some());
        assert!(
            respond(
                "organize my invoices into client folders",
                &fixture(),
                "Documents",
                0
            )
            .is_none()
        );
    }
    #[test]
    fn structural_requests_create_move_rename() {
        let r = run("create a new folder called Client Work");
        assert!(
            matches!(&r.proposal.actions[0], ProposedAction::CreateFolder { path } if path == Path::new("Client Work"))
        );
        let r = run("create folders Invoices and Receipts in Downloads");
        assert_eq!(r.proposal.actions.len(), 2);
        assert!(
            matches!(&r.proposal.actions[0], ProposedAction::CreateFolder { path } if path == Path::new("Downloads/Invoices"))
        );
        let r = run("rename the Lucky World Invasion folder to Lucky Archive");
        assert!(
            matches!(&r.proposal.actions[0], ProposedAction::MoveFolder { destination_relative, .. } if destination_relative == Path::new("curseforge/minecraft/Instances/Lucky Archive"))
        );
        let r = run("rename setup.dmg to Installer");
        assert!(
            matches!(&r.proposal.actions[0], ProposedAction::Rename { new_name, .. } if new_name == "Installer.dmg")
        );
        let r = run("move all log files into Old Logs");
        assert_eq!(r.proposal.actions.len(), 2);
        assert!(r.proposal.rationale.contains("will be created"));
        let r = run("move FTB StoneBlock 4 into Downloads");
        assert!(
            matches!(&r.proposal.actions[0], ProposedAction::MoveFolder { destination_relative, .. } if destination_relative == Path::new("Downloads/FTB StoneBlock 4"))
        );
        assert!(
            run("edit my notes")
                .proposal
                .rationale
                .contains("don’t edit")
        );
    }
    #[test]
    fn nested_folder_with_fuzzy_parent_and_bare_path_followups() {
        let files = vec![
            f(1, "liveandhell-template-1.21.11/run/saves/w/a.mca", 100, 0),
            f(2, "liveandhell-template-1.21.11/src/main.java", 10, 0),
            f(3, "GameLegacy/run/x.txt", 5, 0),
        ];
        let go = |q: &str| respond(q, &files, "Coding Projects", 0);
        let r = go("remove the run folder in life and hell").expect("handled");
        assert_eq!(r.folders.len(), 1, "{}", r.proposal.rationale);
        assert_eq!(r.folders[0].path, "liveandhell-template-1.21.11/run");
        let r = go("delete the run folder in liveandhell-template-1.21.11").expect("handled");
        assert_eq!(r.folders[0].path, "liveandhell-template-1.21.11/run");
        // The parent hint matches an unrelated folder called "hell": still finds the run folder.
        let mut more = files.clone();
        more.push(f(9, "Hell Docs/readme.txt", 1, 0));
        let r = respond(
            "remove the run folder in life and hell",
            &more,
            "Coding Projects",
            0,
        )
        .unwrap();
        assert_eq!(r.folders.len(), 1, "{}", r.proposal.rationale);
        // Bare path as a follow-up to a failed removal.
        let r = go("remove the run folder in life and hell\nUser follow-up: liveandhell-template-1.21.11/run").expect("handled");
        assert_eq!(r.folders[0].path, "liveandhell-template-1.21.11/run");
    }
    #[test]
    fn comma_separated_folder_lists_are_all_prepared() {
        let files = vec![
            f(1, "Cluedo-BSL2026/a.txt", 5, 0),
            f(2, "Background-animated/index.html", 10, 0),
            f(3, "Tank Game/main.js", 20, 0),
            f(4, "AI Empire/x.py", 30, 0),
            f(5, "Lone Blossom/y.ts", 40, 0),
            f(6, "clocktower/src/game/G.java", 1, 0),
            f(7, "other/game/H.java", 1, 0),
            f(8, "keep/z.txt", 1, 0),
        ];
        let r = respond(
            "delete the project like cluedo bsl, background animated web, tank game, ai empire, lone blossom",
            &files,
            "Coding Projects",
            0,
        )
        .unwrap();
        let paths: Vec<_> = r.folders.iter().map(|f| f.path.as_str()).collect();
        for expected in [
            "Cluedo-BSL2026",
            "Background-animated",
            "Tank Game",
            "AI Empire",
            "Lone Blossom",
        ] {
            assert!(
                paths.contains(&expected),
                "{expected} missing from {paths:?}: {}",
                r.proposal.rationale
            );
        }
        assert_eq!(paths.len(), 5, "{paths:?}");
        assert!(!paths.iter().any(|p| p.ends_with("/game")));
    }
    #[test]
    fn launcher_profiles_are_found_as_instances_and_can_be_picked() {
        let files = vec![
            f(
                1,
                "curseforge/minecraft/Instances/NightfallCraft/mods/a.jar",
                900,
                0,
            ),
            f(
                2,
                "curseforge/minecraft/Instances/FTB StoneBlock 4/x.jar",
                800,
                0,
            ),
            f(
                3,
                "curseforge/minecraft/Instances/Lucky World/y.jar",
                700,
                0,
            ),
            f(4, "notes/todo.txt", 1, 0),
        ];
        let q = "List all my modrinth profiles on my computer so I will be able to remove those";
        let r = respond(q, &files, "Documents", 0).expect("handled");
        assert!(r.pick, "{}", r.proposal.rationale);
        assert_eq!(r.folders.len(), 3);
        assert!(r.folders.iter().any(|f| f.path.ends_with("NightfallCraft")));
        // Follow-up prepares all of them.
        let r = respond(
            &format!("{q}\nUser follow-up: delete all of them"),
            &files,
            "Documents",
            0,
        )
        .expect("handled");
        assert!(!r.pick);
        assert_eq!(r.folders.len(), 3);
        // Nothing matching leaves the door open for the AI fallback.
        let r = respond("find zzzunknownthing", &files, "Documents", 0).unwrap();
        assert!(r.unresolved);
    }
    #[test]
    fn instances_not_matching_a_version_are_removed_using_metadata() {
        let dir = std::env::temp_dir().join(format!("tidy_versions_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (name, version) in [
            ("Old Pack", "1.20.1"),
            ("New Pack", "26.2"),
            ("Mid Pack", "1.21.1"),
        ] {
            let inst = dir.join("curseforge/minecraft/Instances").join(name);
            std::fs::create_dir_all(inst.join("mods")).unwrap();
            std::fs::write(
                inst.join("minecraftinstance.json"),
                format!("{{\"name\":\"{name}\",\"gameVersion\": \"{version}\"}}"),
            )
            .unwrap();
        }
        let files = vec![
            f(
                1,
                "curseforge/minecraft/Instances/Old Pack/mods/a.jar",
                900,
                0,
            ),
            f(
                2,
                "curseforge/minecraft/Instances/New Pack/mods/b.jar",
                800,
                0,
            ),
            f(
                3,
                "curseforge/minecraft/Instances/Mid Pack/mods/c.jar",
                700,
                0,
            ),
            f(
                4,
                "curseforge/minecraft/Instances/Old Pack/mods/x-26.2.jar",
                5,
                0,
            ),
        ];
        let r = respond_in(
            "remove the curseforge instances that are not the 26.2 version",
            &files,
            "Documents",
            0,
            Some(&dir),
        )
        .expect("handled");
        let _ = std::fs::remove_dir_all(&dir);
        let mut paths: Vec<_> = r
            .folders
            .iter()
            .map(|f| f.path.rsplit('/').next().unwrap().to_string())
            .collect();
        paths.sort();
        assert_eq!(paths, ["Mid Pack", "Old Pack"], "{}", r.proposal.rationale);
        assert!(r.proposal.actions.is_empty());
        assert!(!r.pick);
        assert!(r.proposal.rationale.contains("New Pack (Minecraft 26.2)"));
    }
    #[test]
    fn nested_profiles_folders_are_not_offered_next_to_instances() {
        let files = vec![
            f(
                1,
                "curseforge/minecraft/Instances/Pack A/config/jade/profiles/1/x.json",
                3,
                0,
            ),
            f(
                2,
                "curseforge/minecraft/Instances/Pack A/mods/a.jar",
                900,
                0,
            ),
            f(
                3,
                "curseforge/minecraft/Instances/Pack B/mods/b.jar",
                800,
                0,
            ),
        ];
        let r = respond("remove the curseforge instances that are not the 26.2 version, I found 189 matching 26.2", &files, "Docs", 0).unwrap();
        let mut paths: Vec<_> = r.folders.iter().map(|f| f.path.as_str()).collect();
        paths.sort();
        assert_eq!(
            paths,
            [
                "curseforge/minecraft/Instances/Pack A",
                "curseforge/minecraft/Instances/Pack B"
            ],
            "{}",
            r.proposal.rationale
        );
        assert!(
            r.proposal.rationale.contains("matching 26.2)"),
            "{}",
            r.proposal.rationale
        );
        assert!(!r.proposal.rationale.contains("found /"));
    }
    #[test]
    fn a_name_prefers_the_project_over_folders_inside_its_build_output() {
        let files = vec![
            f(
                1,
                "chefmod-template-1.21.11/src/main/resources/assets/chefmod/a.png",
                5,
                0,
            ),
            f(
                2,
                "chefmod-template-1.21.11/build/resources/main/assets/chefmod/a.png",
                9,
                0,
            ),
            f(3, "chefmod-template-1.21.11/build.gradle", 1, 0),
        ];
        let r = respond("delete chef mod", &files, "Downloads", 0).unwrap();
        assert_eq!(r.folders.len(), 1, "{}", r.proposal.rationale);
        assert_eq!(r.folders[0].path, "chefmod-template-1.21.11");
    }
    #[test]
    fn project_listing_uses_markers_including_git() {
        let dir = std::env::temp_dir().join(format!("tidy_projects_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Alpha/.git")).unwrap();
        std::fs::create_dir_all(dir.join("Beta")).unwrap();
        std::fs::write(dir.join("Beta/Cargo.toml"), b"").unwrap();
        std::fs::create_dir_all(dir.join("Notes")).unwrap();
        assert!(wants_projects("List me all the projects I have"));
        assert!(!wants_projects("delete the projects folder"));
        let r = list_projects(&dir, "Coding", &[]);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            r.proposal.rationale.contains("2 projects"),
            "{}",
            r.proposal.rationale
        );
        let items = &r.sections[0].items;
        assert!(
            items
                .iter()
                .any(|i| i.path == "Alpha" && i.note.as_deref().unwrap_or("").contains("Git"))
        );
        assert!(
            items
                .iter()
                .any(|i| i.path == "Beta" && i.note.as_deref().unwrap_or("").contains("Rust"))
        );
        assert!(items.iter().all(|i| i.path != "Notes"));
    }
}

/// True for "list/show my projects" style requests.
pub fn wants_projects(request: &str) -> bool {
    let words = tokens(request.rsplit("User follow-up:").next().unwrap_or(request));
    has(
        &words,
        &[
            "project",
            "projects",
            "repo",
            "repos",
            "repositories",
            "codebases",
            "apps",
        ],
    ) && has(
        &words,
        &[
            "list", "show", "find", "what", "which", "all", "every", "give", "display", "where",
            "my", "have", "got", "see",
        ],
    ) && !has(
        &words,
        &[
            "delete", "remove", "trash", "erase", "organize", "sort", "group",
        ],
    )
}
const PROJECT_MARKERS: &[(&str, &str)] = &[
    (".git", "Git"),
    ("package.json", "Node"),
    ("Cargo.toml", "Rust"),
    ("pyproject.toml", "Python"),
    ("setup.py", "Python"),
    ("requirements.txt", "Python"),
    ("go.mod", "Go"),
    ("pom.xml", "Java"),
    ("build.gradle", "Gradle"),
    ("build.gradle.kts", "Gradle"),
    ("Package.swift", "Swift"),
    ("CMakeLists.txt", "C/C++"),
    ("Gemfile", "Ruby"),
    ("composer.json", "PHP"),
    ("pubspec.yaml", "Flutter"),
    ("index.html", "Web"),
];
/// Finds projects by their marker files on disk (Git repositories are not in the file index, so the
/// index alone would miss them). Read-only, no-follow, bounded.
pub fn list_projects(root: &Path, scope: &str, files: &[FileCandidate]) -> Investigation {
    list_projects_with(root, scope, files, &ProjectOpts::default())
}
/// How a project list is presented; used for follow-ups like “only the Rust ones” or “biggest first”.
#[derive(Default, Clone)]
pub struct ProjectOpts {
    /// Measure sizes from disk for every project missing from the index (longer time budget).
    pub measure_all: bool,
    pub only_kind: Option<String>,
    pub sort: Option<String>,
}
/// Allocated bytes below `path`, stopping at `max_entries` or `deadline` (returns `None` then).
fn measure_dir(
    path: &Path,
    max_entries: usize,
    deadline: std::time::Instant,
) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let (mut bytes, mut files, mut seen) = (0u64, 0u64, 0usize);
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).ok()?.flatten() {
            seen += 1;
            if seen > max_entries || (seen % 512 == 0 && std::time::Instant::now() > deadline) {
                return None;
            }
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                bytes += meta.blocks().saturating_mul(512);
                files += 1;
            }
        }
    }
    Some((bytes, files))
}
pub fn list_projects_with(
    root: &Path,
    scope: &str,
    files: &[FileCandidate],
    opts: &ProjectOpts,
) -> Investigation {
    let ctx = Ctx {
        files,
        scope,
        now: 0,
        root: Some(root),
    };
    let folders = folders_of(files);
    let skip = [
        "node_modules",
        "target",
        ".build",
        "Pods",
        "venv",
        ".venv",
        "__pycache__",
        "Library",
        "DerivedData",
        "dist",
        "build",
    ];
    let mut found: Vec<(PathBuf, Vec<&str>, std::time::SystemTime)> = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut visited = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        visited += 1;
        if visited > 30_000 {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut kinds: Vec<&str> = Vec::new();
        let mut subdirs = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if let Some((_, label)) = PROJECT_MARKERS.iter().find(|(m, _)| *m == name) {
                if !(name == "index.html" && !kind.is_file()) && !kinds.contains(label) {
                    kinds.push(label);
                }
            }
            if name.ends_with(".xcodeproj") || name.ends_with(".xcworkspace") {
                if !kinds.contains(&"Xcode") {
                    kinds.push("Xcode");
                }
            }
            if kind.is_dir()
                && !kind.is_symlink()
                && !name.starts_with('.')
                && !skip.contains(&name.as_str())
                && !name.ends_with(".app")
                && !name.ends_with(".xcodeproj")
                && !name.ends_with(".xcworkspace")
            {
                subdirs.push(entry.path());
            }
        }
        // A bare index.html or requirements.txt alone is weak evidence; require a stronger marker at depth 0 of the scope.
        let strong = kinds.iter().any(|k| !matches!(*k, "Web"));
        if !kinds.is_empty() && (strong || dir != root) && dir != root {
            let modified = std::fs::metadata(&dir)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            found.push((dir, kinds, modified));
            continue;
        }
        if depth < 5 {
            for sub in subdirs {
                stack.push((sub, depth + 1));
            }
        }
    }
    found.sort_by(|a, b| b.2.cmp(&a.2));
    let mut text = if found.is_empty() {
        format!(
            "I looked through {scope} and found no folders that look like projects (no package.json, Cargo.toml, .git, and so on)."
        )
    } else {
        format!(
            "I found {} in {scope}, most recently changed first:",
            plural(found.len(), "project", "projects")
        )
    };
    let mut items = Vec::new();
    let budget = if opts.measure_all {
        std::time::Duration::from_secs(20)
    } else {
        std::time::Duration::from_secs(3)
    };
    let deadline = std::time::Instant::now() + budget;
    let (mut unmeasured, mut measured_now) = (0usize, 0usize);
    if let Some(kind) = &opts.only_kind {
        found.retain(|(_, kinds, _)| kinds.iter().any(|k| k.eq_ignore_ascii_case(kind)));
    }
    for (path, kinds, modified) in found.iter().take(80) {
        let rel = path.strip_prefix(root).unwrap_or(path);
        let indexed = folders.iter().find(|f| f.path == rel);
        let (mut bytes_here, mut files_here) = indexed.map_or((0, 0), |f| (f.bytes, f.files));
        let mut from_disk = false;
        if indexed.is_none_or(|f| f.files == 0) {
            match measure_dir(
                path,
                if opts.measure_all { 3_000_000 } else { 250_000 },
                deadline,
            ) {
                Some((b, n)) => {
                    (bytes_here, files_here) = (b, n as usize);
                    from_disk = true;
                    measured_now += 1;
                }
                None => unmeasured += 1,
            }
        }
        let days = std::time::SystemTime::now()
            .duration_since(*modified)
            .map(|d| d.as_secs() / 86_400)
            .unwrap_or(0);
        let age = match days {
            0 => "today".to_string(),
            1 => "yesterday".into(),
            d if d < 60 => format!("{d} days ago"),
            d => format!("{} months ago", d / 30),
        };
        let mut note = format!("{} · changed {age}", kinds.join(", "));
        if bytes_here == 0 && files_here == 0 {
            note.push_str(if from_disk {
                " · empty"
            } else {
                " · size not measured"
            });
        } else if from_disk {
            note.push_str(" · size read from disk");
        }
        items.push(ListItem {
            kind: "folder",
            path: rel.to_string_lossy().into_owned(),
            bytes: bytes_here,
            files: files_here,
            note: Some(note),
        });
    }
    match opts.sort.as_deref() {
        Some("size") => items.sort_by_key(|i| std::cmp::Reverse(i.bytes)),
        Some("name") => items.sort_by(|a, b| a.path.to_lowercase().cmp(&b.path.to_lowercase())),
        Some("oldest") => items.reverse(),
        _ => {}
    }
    let _ = measured_now;
    if unmeasured > 0 {
        text.push_str(&format!(
            " {unmeasured} are too large to measure quickly; say “measure them” to take longer."
        ));
    }
    if found.len() > 80 {
        text.push_str(&format!(" Showing the first 80 of {}.", found.len()));
    }
    if !found.is_empty() {
        text.push_str(" Say “delete <project name>” to move one to the Trash.");
    }
    let mut result = reply(
        &ctx,
        "find_project",
        "Looked for project markers",
        format!("Scanned folder structure below {scope}"),
        text,
    );
    if !items.is_empty() {
        result.sections.push(Section {
            title: format!("Projects in {scope}"),
            items,
        });
    }
    result
}

fn plan_reply(
    ctx: &Ctx,
    workflow: &str,
    headline: &str,
    detail: String,
    text: String,
    actions: Vec<ProposedAction>,
    sources: Vec<Source>,
) -> Investigation {
    let mut r = reply(ctx, workflow, headline, detail, text);
    r.proposal.actions = actions;
    r.sources = sources;
    r
}
fn clean_name(raw: &[String]) -> String {
    let stop = [
        "the",
        "a",
        "an",
        "new",
        "folder",
        "folders",
        "directory",
        "called",
        "named",
        "file",
        "it",
        "as",
        "to",
        "into",
        "in",
        "my",
        "this",
        "that",
    ];
    let kept: Vec<&str> = raw
        .iter()
        .map(|w| w.as_str())
        .skip_while(|w| stop.contains(&w.to_lowercase().as_str()))
        .collect();
    let mut name = kept.join(" ");
    name = name
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '.' | ' '))
        .to_string();
    name
}
fn safe_relative(name: &str) -> Option<PathBuf> {
    let path = PathBuf::from(name.trim_matches('/'));
    if name.is_empty()
        || name.len() > 200
        || path.is_absolute()
        || path.components().count() == 0
        || path.components().any(|c| {
            let text = c.as_os_str().to_string_lossy();
            !matches!(c, std::path::Component::Normal(_)) || text.starts_with('.')
        })
    {
        None
    } else {
        Some(path)
    }
}
fn folder_exists(path: &Path, files: &[FileCandidate], folders: &[Folder]) -> bool {
    folders.iter().any(|f| f.path == path) || files.iter().any(|f| f.relative_path == path)
}
/// Move / rename / create-folder requests. Names keep the capitalization the user typed.
/// A name like `movie.mp4` is a file: never fuzzy-match it to a folder.
fn looks_like_file(word: &str) -> bool {
    match word.rsplit_once('.') {
        Some((stem, ext)) => {
            !stem.is_empty()
                && (1..=5).contains(&ext.len())
                && ext.chars().all(|c| c.is_ascii_alphanumeric())
                && ext.chars().any(|c| c.is_ascii_alphabetic())
        }
        None => false,
    }
}
/// The indexed file a spoken name refers to (extension optional), shallowest path first.
fn find_file_named<'a>(ctx: &'a Ctx, subject: &[String]) -> Option<&'a FileCandidate> {
    let needle = alnum(
        &subject
            .iter()
            .filter(|w| !matches!(w.as_str(), "file" | "files"))
            .cloned()
            .collect::<Vec<_>>()
            .join(""),
    );
    if needle.is_empty() {
        return None;
    }
    let mut hits: Vec<&FileCandidate> = ctx
        .files
        .iter()
        .filter(|f| {
            f.relative_path.file_name().is_some_and(|n| {
                let n = alnum(&n.to_string_lossy());
                n == needle || n.starts_with(&needle) && needle.len() > 3
            })
        })
        .collect();
    hits.sort_by_key(|f| f.relative_path.components().count());
    hits.first().copied()
}
/// Splits one message into the separate tasks it asks for: sentences, and “… then …” / “after that …”.
fn split_tasks(text: &str) -> Vec<String> {
    let text = text.replace('\n', " . ");
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut tasks: Vec<Vec<&str>> = vec![vec![]];
    let mut i = 0;
    while i < words.len() {
        let w = words[i];
        let bare = w
            .to_lowercase()
            .trim_matches(|c| matches!(c, ',' | '.' | ';' | '!'))
            .to_string();
        let after_that = bare == "after"
            && words
                .get(i + 1)
                .is_some_and(|n| n.to_lowercase().starts_with("that"));
        if matches!(bare.as_str(), "then" | "afterwards" | "afterward") || after_that {
            // A sequencing word starts the next task; at the start of a task it is just dropped.
            if !tasks.last().unwrap().is_empty() {
                if tasks
                    .last()
                    .unwrap()
                    .last()
                    .is_some_and(|p| p.eq_ignore_ascii_case("and"))
                {
                    tasks.last_mut().unwrap().pop();
                }
                tasks.push(vec![]);
            }
            i += if after_that { 2 } else { 1 };
            continue;
        }
        if w == "." || w == ";" {
            tasks.push(vec![]);
            i += 1;
            continue;
        }
        tasks.last_mut().unwrap().push(w);
        if w.len() > 1 && w.ends_with(['.', '!', '?', ';']) {
            tasks.push(vec![]);
        }
        i += 1;
    }
    // “delete A and rename B to C”: “and” followed by a new verb starts another task.
    const VERBS: &[&str] = &[
        "delete",
        "remove",
        "trash",
        "erase",
        "discard",
        "wipe",
        "rename",
        "move",
        "put",
        "send",
        "create",
        "make",
        "organize",
        "organise",
        "sort",
        "copy",
        "duplicate",
        "find",
        "list",
        "show",
        "clean",
        "clear",
    ];
    let mut pieces: Vec<Vec<&str>> = vec![];
    for t in tasks {
        let mut cur: Vec<&str> = vec![];
        for (k, w) in t.iter().enumerate() {
            let next_is_verb = t.get(k + 1).is_some_and(|n| {
                VERBS.contains(
                    &n.to_lowercase()
                        .trim_matches(|c: char| !c.is_alphanumeric()),
                )
            });
            if w.eq_ignore_ascii_case("and") && next_is_verb && !cur.is_empty() {
                pieces.push(std::mem::take(&mut cur));
                continue;
            }
            cur.push(w);
        }
        pieces.push(cur);
    }
    let clean = |t: &[&str]| {
        t.join(" ")
            .trim_matches(|c: char| matches!(c, ',' | ';' | '.' | '!' | ' '))
            .to_string()
    };
    let mut out: Vec<String> = vec![];
    for piece in pieces {
        let text = clean(&piece);
        if text.split_whitespace().count() < 2 {
            continue;
        }
        let lower: Vec<String> = text
            .split_whitespace()
            .map(|w| {
                w.to_lowercase()
                    .trim_matches(|c: char| !c.is_alphanumeric())
                    .to_string()
            })
            .collect();
        // “… move it into Z” / “… into it” refer back to the previous task: they are one job.
        let refers_back = lower
            .get(1)
            .is_some_and(|w| matches!(w.as_str(), "it" | "them" | "that" | "those" | "these"))
            || lower
                .last()
                .is_some_and(|w| matches!(w.as_str(), "it" | "there" | "them"));
        match out.last_mut() {
            Some(prev) if refers_back => {
                prev.push_str(" and ");
                prev.push_str(&text);
            }
            _ => out.push(text),
        }
    }
    out
}
/// Several tasks in one message (“delete A and B. Then rename C to D”): each task is understood on its
/// own and they are merged into ONE reviewable plan. A task that can’t be understood is named, never dropped.
fn multi_task(ctx: &Ctx, text: &str, folders: &[Folder]) -> Option<Investigation> {
    let tasks = split_tasks(text);
    if tasks.len() < 2 {
        return None;
    }
    let mut merged: Option<Investigation> = None;
    let mut steps: Vec<String> = vec![];
    let mut skipped: Vec<String> = vec![];
    for task in &tasks {
        let Some(r) = interpret(ctx, task, folders) else {
            skipped.push(format!("“{task}” — I didn’t understand this part"));
            continue;
        };
        let has_plan = !r.proposal.actions.is_empty() || !r.folders.is_empty();
        if !has_plan || r.pick || !r.sections.is_empty() {
            let why = if r.proposal.rationale.is_empty() {
                "nothing matched".to_string()
            } else {
                r.proposal.rationale.clone()
            };
            skipped.push(format!("“{task}” — {why}"));
            continue;
        }
        steps.push(r.proposal.rationale.clone());
        match merged.as_mut() {
            None => merged = Some(r),
            Some(m) => {
                m.proposal.actions.extend(r.proposal.actions);
                m.sources.extend(r.sources);
                m.folders.extend(r.folders);
                m.trace.extend(r.trace);
                m.examined += r.examined;
                m.remaining_matches += r.remaining_matches;
                m.complete &= r.complete;
            }
        }
    }
    let mut m = merged?;
    m.workflow = Some("multi_task".into());
    let mixed = !m.folders.is_empty() && !m.proposal.actions.is_empty();
    let mut text = format!(
        "{} tasks, one plan — {}\n",
        steps.len(),
        if mixed {
            "everything is listed below. The Trash step is approved first, then the rest follows:"
        } else {
            "review everything below and approve once:"
        }
    );
    for (i, step) in steps.iter().enumerate() {
        text.push_str(&format!("\n{}. {}", i + 1, step.trim()));
    }
    if !skipped.is_empty() {
        text.push_str("\n\nNot included:");
        for s in &skipped {
            text.push_str(&format!("\n• {s}"));
        }
        text.push_str("\nSay those again on their own and I’ll prepare them.");
    }
    m.proposal.rationale = text;
    m.clarification = None;
    Some(m)
}
const MOVE_VERBS: &[&str] = &["move", "put", "send", "relocate", "transfer", "shift"];
const DEST_MARKERS: &[&str] = &["to", "into", "in", "inside", "under", "within", "onto"];
fn is_connector(word: &str) -> bool {
    matches!(word, "and" | "then" | "also" | "," | "afterwards" | "after")
}
fn position_from(words: &[String], from: usize, list: &[&str]) -> Option<usize> {
    words
        .iter()
        .enumerate()
        .skip(from)
        .find(|(_, w)| list.contains(&w.as_str()))
        .map(|(i, _)| i)
}
/// Two-step requests folded into ONE reviewable plan, so nothing depends on the order things run in:
///   “rename X to Y and move it into Z”  (or “move X into Z and rename it to Y”) → one move that also renames
///   “create a folder Z and move X into it”                                     → the move creates Z
fn multi_step(
    ctx: &Ctx,
    raw: &[String],
    words: &[String],
    folders: &[Folder],
) -> Option<Investigation> {
    let rename_at = position_from(words, 0, &["rename"]);
    let move_at = position_from(words, 0, MOVE_VERBS);
    if let (Some(r), Some(m)) = (rename_at, move_at) {
        if negated(words, r) || negated(words, m) {
            return None;
        }
        let pronouns = ["it", "them", "that", "this", "the", "then", "also", "and"];
        let skip = |at: usize| {
            let mut i = at;
            while i < words.len() && pronouns.contains(&words[i].as_str()) {
                i += 1;
            }
            i
        };
        // (subject, new name, destination words, destination raw words)
        let parsed = if r < m {
            // rename <X> to <Y> and move it into <Z>
            let to_at = position_from(words, r + 1, &["to", "as", "into"]).filter(|t| *t < m)?;
            let c = (to_at + 1..m).find(|i| is_connector(&words[*i]))?;
            let subject = words[r + 1..to_at].to_vec();
            let new_name = clean_name(&raw[to_at + 1..c]);
            let dm = position_from(words, skip(m + 1), DEST_MARKERS)?;
            (
                subject,
                new_name,
                words[dm + 1..].to_vec(),
                raw[dm + 1..].to_vec(),
            )
        } else {
            // move <X> into <Z> and rename it to <Y>
            let dm = position_from(words, m + 1, DEST_MARKERS).filter(|d| *d < r)?;
            let c = (dm + 1..r).rev().find(|i| is_connector(&words[*i]))?;
            let subject = words[m + 1..dm].to_vec();
            let to_at = position_from(words, skip(r + 1), &["to", "as", "into"])?;
            let new_name = clean_name(&raw[to_at + 1..]);
            (
                subject,
                new_name,
                words[dm + 1..c].to_vec(),
                raw[dm + 1..c].to_vec(),
            )
        };
        let (subject, new_name, dest_words, dest_raw) = parsed;
        // Bulk renames (“all pdfs”, “every screenshot”) are handled elsewhere.
        if has(&subject, &["all", "every", "each", "any"]) {
            return None;
        }
        let subject: Vec<String> = subject
            .into_iter()
            .filter(|w| !FILLER.contains(&w.as_str()))
            .collect();
        if subject.is_empty() || new_name.is_empty() || new_name.contains('/') {
            return None;
        }
        let dest_phrase: Vec<String> = dest_words
            .iter()
            .filter(|w| !FILLER.contains(&w.as_str()))
            .cloned()
            .collect();
        if dest_phrase.is_empty() {
            return None;
        }
        return rename_and_move(ctx, &subject, &new_name, &dest_phrase, &dest_raw, folders);
    }
    // create a folder <Z> [in <P>] and move <X> into it
    let create_at = position_from(words, 0, &["create", "make", "add"])?;
    let m = move_at.filter(|m| *m > create_at)?;
    if negated(words, create_at) || negated(words, m) {
        return None;
    }
    let folder_at = position_from(words, create_at + 1, &["folder", "directory"])?;
    if folder_at >= m {
        return None;
    }
    let start = position_from(words, create_at + 1, &["called", "named"])
        .map(|i| i + 1)
        .unwrap_or(folder_at + 1);
    let end = (start..m).find(|i| {
        is_connector(&words[*i]) || DEST_MARKERS.contains(&words[*i].as_str()) && words[*i] != "to"
    })?;
    let name = clean_name(&raw[start..end]);
    if name.is_empty() || name.contains('/') {
        return None;
    }
    // Optional parent: “… in Downloads and …”
    let mut full = name.clone();
    if DEST_MARKERS.contains(&words[end].as_str()) {
        let c = (end + 1..m).rev().find(|i| is_connector(&words[*i]))?;
        let parent: Vec<String> = words[end + 1..c]
            .iter()
            .filter(|w| !FILLER.contains(&w.as_str()))
            .cloned()
            .collect();
        let dir = resolve(&parent, folders).first().map(|f| f.path.clone())?;
        full = format!("{}/{}", dir.display(), name);
    }
    // The destination of the move must be “it” / “there” (the folder just named).
    let dm = position_from(words, m + 1, DEST_MARKERS)?;
    let tail: Vec<&String> = words[dm + 1..]
        .iter()
        .filter(|w| !matches!(w.as_str(), "the" | "new" | "that" | "folder" | "directory"))
        .collect();
    if !matches!(tail.as_slice(), [w] if matches!(w.as_str(), "it" | "there" | "inside")) {
        return None;
    }
    let mut words2: Vec<String> = words[m..dm].to_vec();
    words2.push("into".into());
    words2.push(full.to_lowercase());
    let mut raw2: Vec<String> = raw[m..dm].to_vec();
    raw2.push("into".into());
    raw2.push(full);
    structural(ctx, &raw2, &words2, folders)
}
fn rename_and_move(
    ctx: &Ctx,
    subject: &[String],
    new_name: &str,
    dest_phrase: &[String],
    dest_raw: &[String],
    folders: &[Folder],
) -> Option<Investigation> {
    let workflow = "rename_and_move";
    let (dest_dir, dest_note) = match resolve(dest_phrase, folders).first() {
        Some(f) => (f.path.clone(), String::new()),
        None => {
            let rel = safe_relative(&clean_name(dest_raw))?;
            let note = format!(
                " “{}” doesn’t exist yet, so it will be created.",
                rel.display()
            );
            (rel, note)
        }
    };
    let nothing = |detail: &str, text: String| {
        reply(
            ctx,
            workflow,
            "Understood your request",
            detail.into(),
            text,
        )
    };
    let names_a_file = subject.last().is_some_and(|w| looks_like_file(w));
    if !names_a_file
        && let Some(folder) = resolve(subject, folders)
            .into_iter()
            .find(|f| f.path != dest_dir)
    {
        let dest = dest_dir.join(new_name);
        if dest.starts_with(&folder.path) {
            return Some(nothing(
                "Rename and move folder",
                "A folder can’t be moved inside itself. Nothing changed.".into(),
            ));
        }
        if folder_exists(&dest, ctx.files, folders) {
            return Some(nothing(
                "Rename and move folder",
                format!(
                    "“{}” already exists, so I won’t overwrite it. Nothing changed.",
                    dest.display()
                ),
            ));
        }
        return Some(plan_reply(
            ctx,
            workflow,
            "Understood your request",
            format!(
                "Rename {} → {} and move it",
                folder.path.display(),
                dest.display()
            ),
            format!(
                "Two steps, one approval: rename the folder “{}” to “{new_name}” and move it into “{}”.{dest_note} Everything inside stays together. Undo it from History.",
                folder.path.display(),
                dest_dir.display()
            ),
            vec![ProposedAction::MoveFolder {
                source: folder.path.clone(),
                destination_relative: dest,
            }],
            vec![],
        ));
    }
    let Some(file) = find_file_named(ctx, subject) else {
        let mut r = nothing(
            "Looked for what to rename and move",
            format!(
                "I couldn’t find “{}” to rename and move. Try its exact name, for example “rename setup.dmg to Installer and move it into Archive”.",
                subject.join(" ")
            ),
        );
        r.clarification = Some(r.proposal.rationale.clone());
        return Some(r);
    };
    let ext = file
        .relative_path
        .extension()
        .map(|e| e.to_string_lossy().into_owned());
    let final_name = match (&ext, Path::new(new_name).extension()) {
        (Some(e), None) => format!("{new_name}.{e}"),
        _ => new_name.to_string(),
    };
    let dest = dest_dir.join(&final_name);
    if folder_exists(&dest, ctx.files, folders) {
        return Some(nothing(
            "Rename and move file",
            format!(
                "“{}” already exists, so I won’t overwrite it. Nothing changed.",
                dest.display()
            ),
        ));
    }
    let mut r = plan_reply(
        ctx,
        workflow,
        "Understood your request",
        format!(
            "Rename {} → {} and move it",
            file.relative_path.display(),
            dest.display()
        ),
        format!(
            "Two steps, one approval: rename “{}” to “{final_name}”{} and move it into “{}”.{dest_note} Undo it from History.",
            file.relative_path.display(),
            if ext.is_some() && Path::new(new_name).extension().is_none() {
                " (keeping its extension)"
            } else {
                ""
            },
            dest_dir.display()
        ),
        vec![ProposedAction::Move {
            source: file.id,
            destination_relative: dest,
        }],
        vec![Source {
            id: file.id.0,
            path: file.relative_path.to_string_lossy().into(),
            size: file.size,
        }],
    );
    r.examined = 1;
    Some(r)
}
fn structural(
    ctx: &Ctx,
    raw: &[String],
    words: &[String],
    folders: &[Folder],
) -> Option<Investigation> {
    if let Some(r) = multi_step(ctx, raw, words, folders) {
        return Some(r);
    }
    if let Some(r) = bulk_ops(ctx, raw, words, folders) {
        return Some(r);
    }
    let first = |list: &[&str]| words.iter().position(|w| list.contains(&w.as_str()));
    let neg = |at: usize| negated(words, at);
    let dest_markers = ["to", "into", "in", "inside", "under", "within", "onto"];
    if let Some(at) = first(&["rename"]) {
        if neg(at) {
            return None;
        }
        let split = words[at + 1..]
            .iter()
            .position(|w| matches!(w.as_str(), "to" | "as" | "into"))?
            + at
            + 1;
        let subject: Vec<String> = words[at + 1..split]
            .iter()
            .filter(|w| !FILLER.contains(&w.as_str()))
            .cloned()
            .collect();
        let new_name = clean_name(&raw[split + 1..]);
        if subject.is_empty() || new_name.is_empty() || new_name.contains('/') {
            return None;
        }
        // Folder first, then a file with that name. A name with an extension (movie.mp4) is a file.
        let names_a_file = subject.last().is_some_and(|w| looks_like_file(w));
        if !names_a_file && let Some(folder) = resolve(&subject, folders).first() {
            let dest = folder.path.with_file_name(&new_name);
            let text = if folder_exists(&dest, ctx.files, folders) {
                format!(
                    "A folder or file named “{new_name}” already exists there, so I can’t rename “{}” to it. Nothing changed.",
                    folder.path.display()
                )
            } else {
                String::new()
            };
            if !text.is_empty() {
                return Some(reply(
                    ctx,
                    "rename_descriptive",
                    "Understood your request",
                    "Rename folder".into(),
                    text,
                ));
            }
            return Some(plan_reply(
                ctx,
                "rename_descriptive",
                "Understood your request",
                format!("Rename folder {} → {new_name}", folder.path.display()),
                format!(
                    "Rename the folder “{}” to “{new_name}”. Everything inside stays where it is. Approve to apply; you can undo it from History.",
                    folder.path.display()
                ),
                vec![ProposedAction::MoveFolder {
                    source: folder.path.clone(),
                    destination_relative: dest,
                }],
                vec![],
            ));
        }
        let Some(file) = find_file_named(ctx, &subject) else {
            if names_a_file {
                let mut r = reply(
                    ctx,
                    "rename_descriptive",
                    "Looked for the file",
                    "No indexed file matched".into(),
                    format!(
                        "I couldn’t find a file called “{}” in {}. Nothing changed.",
                        raw[at + 1..split].join(" "),
                        ctx.scope
                    ),
                );
                r.clarification = Some(r.proposal.rationale.clone());
                return Some(r);
            }
            return None;
        };
        let ext = file
            .relative_path
            .extension()
            .map(|e| e.to_string_lossy().into_owned());
        let final_name = match (&ext, Path::new(&new_name).extension()) {
            (Some(e), None) => format!("{new_name}.{e}"),
            _ => new_name.clone(),
        };
        if folder_exists(
            &file.relative_path.with_file_name(&final_name),
            ctx.files,
            folders,
        ) {
            return Some(reply(
                ctx,
                "rename_descriptive",
                "Understood your request",
                "Rename file".into(),
                format!(
                    "“{final_name}” already exists in that folder, so I can’t rename to it. Nothing changed."
                ),
            ));
        }
        let mut r = plan_reply(
            ctx,
            "rename_descriptive",
            "Understood your request",
            format!("Rename {} → {final_name}", file.relative_path.display()),
            format!(
                "Rename “{}” to “{final_name}”{}.",
                file.relative_path.display(),
                if ext.is_some() && Path::new(&new_name).extension().is_none() {
                    " (keeping its extension)"
                } else {
                    ""
                }
            ),
            vec![ProposedAction::Rename {
                source: file.id,
                new_name: final_name,
            }],
            vec![Source {
                id: file.id.0,
                path: file.relative_path.to_string_lossy().into(),
                size: file.size,
            }],
        );
        r.examined = 1;
        return Some(r);
    }
    if let Some(at) = first(&["create", "make", "add", "new"]) {
        if !neg(at)
            && has(words, &["folder", "folders", "directory", "directories"])
            && !has(
                words,
                &["move", "put", "delete", "remove", "trash", "rename"],
            )
        {
            let start = words
                .iter()
                .position(|w| matches!(w.as_str(), "called" | "named"))
                .map(|i| i + 1)
                .or_else(|| {
                    words
                        .iter()
                        .position(|w| {
                            matches!(
                                w.as_str(),
                                "folder" | "folders" | "directory" | "directories"
                            )
                        })
                        .map(|i| i + 1)
                })?;
            let end = words[start..]
                .iter()
                .position(|w| dest_markers.contains(&w.as_str()) && *w != "to")
                .map(|i| i + start)
                .unwrap_or(words.len());
            let names_raw = &raw[start..end];
            let parent = if end < words.len() {
                let phrase: Vec<String> = words[end + 1..]
                    .iter()
                    .filter(|w| !FILLER.contains(&w.as_str()))
                    .cloned()
                    .collect();
                if phrase.is_empty() {
                    None
                } else {
                    resolve(&phrase, folders).first().map(|f| f.path.clone())
                }
            } else {
                None
            };
            let mut names = Vec::new();
            let mut current: Vec<String> = Vec::new();
            for w in names_raw {
                if matches!(w.to_lowercase().as_str(), "and" | "&") {
                    names.push(std::mem::take(&mut current));
                } else {
                    current.push(w.clone());
                }
            }
            names.push(current);
            let mut actions = Vec::new();
            let mut lines = Vec::new();
            for n in names.iter().take(10) {
                let name = clean_name(n);
                let Some(rel) = safe_relative(&name) else {
                    continue;
                };
                let path = parent.clone().unwrap_or_default().join(rel);
                if folder_exists(&path, ctx.files, folders) {
                    lines.push(format!("“{}” already exists", path.display()));
                } else {
                    lines.push(format!("“{}”", path.display()));
                    actions.push(ProposedAction::CreateFolder { path });
                }
            }
            if lines.is_empty() {
                return None;
            }
            let text = if actions.is_empty() {
                format!("Nothing to do: {}.", lines.join(", "))
            } else {
                format!(
                    "I’ll create {} in {}. It’s empty until you move things in — try “move all PDFs into it”.",
                    lines.join(", "),
                    ctx.scope
                )
            };
            return Some(plan_reply(
                ctx,
                "custom_hierarchy",
                "Understood your request",
                "Create folders".into(),
                text,
                actions,
                vec![],
            ));
        }
    }
    if let Some(at) = first(&["move", "put", "transfer", "relocate", "shift", "send"]) {
        if neg(at) {
            return None;
        }
        let split = words[at + 1..]
            .iter()
            .position(|w| dest_markers.contains(&w.as_str()))?
            + at
            + 1;
        let thing: Vec<String> = words[at + 1..split].to_vec();
        let dest_raw = &raw[split + 1..];
        let dest_words: Vec<String> = words[split + 1..].to_vec();
        if thing.is_empty() || dest_words.is_empty() {
            return None;
        }
        let dest_phrase: Vec<String> = dest_words
            .iter()
            .filter(|w| !FILLER.contains(&w.as_str()))
            .cloned()
            .collect();
        let (dest_dir, dest_note) = match resolve(&dest_phrase, folders).first() {
            Some(f) => (f.path.clone(), String::new()),
            None => {
                let name = clean_name(dest_raw);
                let rel = safe_relative(&name)?;
                let note = format!(
                    " “{}” doesn’t exist yet, so it will be created.",
                    rel.display()
                );
                (rel, note)
            }
        };
        let criteria = parse_criteria(&thing, folders);
        // “the Old Stuff folder” names a folder even though “old” also reads as an age filter.
        let names_a_folder = has(&thing, &["folder", "directory"])
            && !resolve(
                &thing
                    .iter()
                    .filter(|w| !FILLER.contains(&w.as_str()))
                    .cloned()
                    .collect::<Vec<_>>(),
                folders,
            )
            .is_empty();
        if criteria.targets_files() && !names_a_folder {
            let mut hits: Vec<&FileCandidate> = ctx
                .files
                .iter()
                .filter(|f| {
                    matches_file(f, &criteria, ctx.now)
                        && f.relative_path.parent() != Some(dest_dir.as_path())
                })
                .collect();
            hits.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
            let total = hits.len();
            let mut taken: HashSet<String> = ctx
                .files
                .iter()
                .map(|f| f.relative_path.to_string_lossy().to_lowercase())
                .collect();
            let mut actions = Vec::new();
            let mut sources = Vec::new();
            let mut skipped = 0;
            for f in hits.into_iter().take(BATCH) {
                let Some(name) = f.relative_path.file_name() else {
                    continue;
                };
                let dest = dest_dir.join(name);
                if !taken.insert(dest.to_string_lossy().to_lowercase()) {
                    skipped += 1;
                    continue;
                }
                actions.push(ProposedAction::Move {
                    source: f.id,
                    destination_relative: dest,
                });
                sources.push(Source {
                    id: f.id.0,
                    path: f.relative_path.to_string_lossy().into(),
                    size: f.size,
                });
            }
            let text = if actions.is_empty() {
                "I found nothing to move that isn’t already there. Nothing changed.".to_string()
            } else {
                format!(
                    "Moving {} ({}) into “{}”.{dest_note}{}{} Uncheck anything to leave it; approve when ready. Moves can be undone from History.",
                    plural(actions.len(), "file", "files"),
                    criteria.label,
                    dest_dir.display(),
                    if skipped > 0 {
                        format!(
                            " {skipped} skipped because a file with that name is already there."
                        )
                    } else {
                        String::new()
                    },
                    if total > BATCH {
                        format!(" This batch covers {BATCH} of {total}; ask again for the rest.")
                    } else {
                        String::new()
                    }
                )
            };
            let mut r = plan_reply(
                ctx,
                "consolidate_photos",
                "Understood your request",
                format!("Move {} → {}", criteria.label, dest_dir.display()),
                text,
                actions,
                sources,
            );
            r.remaining_matches = total.saturating_sub(BATCH);
            r.complete = r.remaining_matches == 0;
            return Some(r);
        }
        let subject: Vec<String> = thing
            .iter()
            .filter(|w| !FILLER.contains(&w.as_str()))
            .cloned()
            .collect();
        if let Some(folder) = resolve(&subject, folders)
            .into_iter()
            .find(|f| f.path != dest_dir)
        {
            let name = folder.path.file_name()?.to_owned();
            let dest = dest_dir.join(&name);
            if dest.starts_with(&folder.path) {
                return Some(reply(
                    ctx,
                    "consolidate_photos",
                    "Understood your request",
                    "Move folder".into(),
                    "A folder can’t be moved inside itself. Nothing changed.".into(),
                ));
            }
            if folder_exists(&dest, ctx.files, folders) {
                return Some(reply(
                    ctx,
                    "consolidate_photos",
                    "Understood your request",
                    "Move folder".into(),
                    format!(
                        "“{}” already exists inside “{}”, so I won’t overwrite it. Nothing changed.",
                        name.to_string_lossy(),
                        dest_dir.display()
                    ),
                ));
            }
            return Some(plan_reply(
                ctx,
                "consolidate_photos",
                "Understood your request",
                format!(
                    "Move folder {} → {}",
                    folder.path.display(),
                    dest_dir.display()
                ),
                format!(
                    "Move the folder “{}” ({}, {}) into “{}”.{dest_note} Undo is available in History.",
                    folder.path.display(),
                    bytes(folder.bytes),
                    plural(folder.files, "file", "files"),
                    dest_dir.display()
                ),
                vec![ProposedAction::MoveFolder {
                    source: folder.path.clone(),
                    destination_relative: dest,
                }],
                vec![],
            ));
        }
        let needle = alnum(&subject.join(""));
        if needle.len() > 2 {
            if let Some(file) = ctx.files.iter().find(|f| {
                f.relative_path
                    .file_name()
                    .is_some_and(|n| alnum(&n.to_string_lossy()) == needle)
            }) {
                let dest = dest_dir.join(file.relative_path.file_name()?);
                if folder_exists(&dest, ctx.files, folders) {
                    return Some(reply(
                        ctx,
                        "consolidate_photos",
                        "Understood your request",
                        "Move file".into(),
                        format!(
                            "“{}” already exists there. Nothing changed.",
                            dest.display()
                        ),
                    ));
                }
                return Some(plan_reply(
                    ctx,
                    "consolidate_photos",
                    "Understood your request",
                    format!(
                        "Move {} → {}",
                        file.relative_path.display(),
                        dest_dir.display()
                    ),
                    format!(
                        "Move “{}” into “{}”.{dest_note}",
                        file.relative_path.display(),
                        dest_dir.display()
                    ),
                    vec![ProposedAction::Move {
                        source: file.id,
                        destination_relative: dest,
                    }],
                    vec![Source {
                        id: file.id.0,
                        path: file.relative_path.to_string_lossy().into(),
                        size: file.size,
                    }],
                ));
            }
        }
        return Some(reply(
            ctx,
            "consolidate_photos",
            "Looked for what to move",
            "Nothing matched".into(),
            format!(
                "I couldn’t find “{}” to move. Try its exact name, or describe it (for example “all PDFs” or “photos older than a year”).",
                thing.join(" ")
            ),
        ));
    }
    if has(words, &["edit", "modify", "rewrite", "append", "prepend"])
        && !has(
            words,
            &[
                "extension",
                "extensions",
                "name",
                "names",
                "permission",
                "permissions",
            ],
        )
    {
        return Some(reply(ctx, "clarify_request", "Explained a limit", "Content editing is not supported".into(), "I can move, rename, copy, create folders, change permissions and send things to the Trash — but I don’t edit what’s inside files. Tell me which of those you’d like.".into()));
    }
    None
}

/// Prompt asking the local model to restate a request as one plain command the engine parses.
pub fn rewrite_prompt(request: &str, files: &[FileCandidate]) -> String {
    let folders = folders_of(files);
    let mut names: Vec<&Folder> = folders
        .iter()
        .filter(|f| f.path.components().count() <= 3)
        .collect();
    names.sort_by_key(|f| std::cmp::Reverse(f.bytes));
    let known: Vec<String> = names
        .iter()
        .take(60)
        .map(|f| f.path.to_string_lossy().into_owned())
        .collect();
    let request: String = request
        .rsplit("User follow-up:")
        .next()
        .unwrap_or(request)
        .chars()
        .take(400)
        .collect();
    format!(
        "Rewrite the USER REQUEST as ONE short English command for a file assistant. Output only the command on one line, nothing else.\n\
Allowed command forms (use exactly these shapes):\n\
delete the <folder> folder\n\
delete all <type> files older than <N> months\n\
delete the <N> biggest files\n\
move <thing> into <folder>\n\
rename <folder or file> to <new name>\n\
create a folder called <name> in <folder>\n\
organize by type\n\
organize by date\n\
what is taking the most space\n\
find <words>\n\
list all my projects\n\
Rules: when the user names an existing folder, copy its exact path from KNOWN_FOLDERS (fix typos and spacing). If the request cannot be expressed with these forms, output UNCLEAR.\n\
KNOWN_FOLDERS: {}\n\
USER REQUEST: {}",
        serde_json::to_string(&known).unwrap_or_default(),
        serde_json::to_string(&request).unwrap_or_default(),
    )
}
/// First usable line of a model's rewrite, or `None` for UNCLEAR/empty output.
pub fn clean_rewrite(text: &str) -> Option<String> {
    let mut text = text.to_string();
    while let Some(start) = text.find("<think>") {
        let end = text[start..]
            .find("</think>")
            .map_or(text.len(), |e| start + e + 8);
        text.replace_range(start..end, "");
    }
    let line = text
        .lines()
        .map(|l| {
            l.trim()
                .trim_matches(|c| matches!(c, '"' | '`' | '\'' | '“' | '”'))
                .trim()
        })
        .find(|l| !l.is_empty())?
        .to_string();
    if line.to_uppercase().starts_with("UNCLEAR") || line.len() > 300 {
        None
    } else {
        Some(line)
    }
}

/// Drops a trailing purpose clause ("…, so I can remove those") that would pollute a search.
fn strip_purpose(words: Vec<String>, raw: Vec<String>) -> (Vec<String>, Vec<String>) {
    let cut = words.iter().enumerate().position(|(i, w)| {
        i > 1
            && (w == "so"
                || w == "because"
                || w == "since"
                || (w == "in" && words.get(i + 1).is_some_and(|n| n == "order")))
            || (i > 1
                && w == "to"
                && words.get(i + 1).is_some_and(|n| n == "be")
                && words.get(i + 2).is_some_and(|n| n == "able"))
    });
    match cut {
        Some(at) => {
            let mut words = words;
            let mut raw = raw;
            words.truncate(at);
            raw.truncate(at.min(raw.len()));
            (words, raw)
        }
        None => (words, raw),
    }
}
/// Folder names that mean the same thing to people: a launcher "profile" is an "instance".
fn concept_names(noun: &str) -> Option<&'static [&'static str]> {
    Some(match noun {
        "profile" | "profiles" | "instance" | "instances" | "modpack" | "modpacks" | "pack"
        | "packs" => &["instances", "profiles", "modpacks"],
        "world" | "worlds" | "save" | "saves" => &["saves", "worlds"],
        "backup" | "backups" => &["backups", "backup"],
        "mod" | "mods" => &["mods"],
        "resourcepack" | "resourcepacks" | "texturepack" | "texturepacks" => &["resourcepacks"],
        "screenshot" | "screenshots" => &["screenshots"],
        "download" | "downloads" => &["downloads"],
        "cache" | "caches" => &["cache", "caches"],
        "log" | "logs" => &["logs"],
        "template" | "templates" => &["templates", "template"],
        "plugin" | "plugins" => &["plugins"],
        _ => return None,
    })
}
const APP_WORDS: &[&str] = &[
    "modrinth",
    "curseforge",
    "minecraft",
    "prism",
    "prismlauncher",
    "atlauncher",
    "launcher",
    "ftb",
    "technic",
    "forge",
    "fabric",
    "steam",
    "epic",
    "game",
    "games",
    "gaming",
];
const LIST_NOISE: &[&str] = &[
    "list",
    "show",
    "find",
    "display",
    "give",
    "see",
    "check",
    "look",
    "search",
    "all",
    "every",
    "each",
    "my",
    "me",
    "the",
    "of",
    "on",
    "in",
    "computer",
    "mac",
    "device",
    "disk",
    "that",
    "i",
    "have",
    "got",
    "own",
    "which",
    "what",
    "where",
    "are",
    "is",
    "there",
    "possible",
    "files",
    "file",
    "folder",
    "folders",
    "directory",
    "directories",
    "named",
    "called",
    "delete",
    "remove",
    "trash",
    "erase",
    "get",
    "rid",
    "those",
    "them",
    "these",
    "and",
    "or",
    ",",
    "to",
    "for",
    "please",
    "can",
    "you",
    "could",
    "see",
];
/// "list all my modrinth profiles": resolve the noun to real folders (launcher profiles live in an
/// `Instances` folder) and list what is inside, ready to pick from.
/// Splits "instances that are not the 26.2 version" into the noun part and a keep/drop filter.
fn split_filter(words: &[String]) -> (Vec<String>, Option<(Vec<String>, bool)>) {
    let negative = [
        "not",
        "except",
        "excluding",
        "besides",
        "without",
        "aren't",
        "isn't",
        "non",
        "other",
    ];
    let marker = words.iter().position(|w| negative.contains(&w.as_str()));
    let stop = [
        "than",
        "the",
        "a",
        "an",
        "version",
        "versions",
        "one",
        "ones",
        "of",
        "any",
        "those",
        "that",
        "are",
        "is",
        "have",
        "with",
        "on",
        "in",
        "running",
        "using",
        "mc",
        "minecraft",
        "for",
        "or",
        "and",
        ",",
        "to",
        "it",
        "they",
        "them",
    ];
    if let Some(at) = marker {
        let mut terms: Vec<String> = words[at + 1..]
            .iter()
            .filter(|w| {
                !stop.contains(&w.as_str())
                    && (w.chars().any(|c| c.is_ascii_digit()) || w.len() >= 3)
            })
            .cloned()
            .collect();
        // Versions are what people filter on; when numbers are present ignore stray words.
        if terms
            .iter()
            .any(|t| t.contains('.') && t.chars().any(|c| c.is_ascii_digit()))
        {
            terms.retain(|t| t.contains('.') && t.chars().any(|c| c.is_ascii_digit()));
        } else if terms.iter().any(|t| t.chars().any(|c| c.is_ascii_digit())) {
            terms.retain(|t| t.chars().any(|c| c.is_ascii_digit()));
        }
        terms.dedup();
        terms.truncate(4);
        if !terms.is_empty() {
            // The noun part also drops the "that are" lead-in.
            let mut head = words[..at].to_vec();
            while head.last().is_some_and(|w| {
                matches!(
                    w.as_str(),
                    "that" | "which" | "are" | "is" | "who" | "whose"
                )
            }) {
                head.pop();
            }
            return (head, Some((terms, true)));
        }
    }
    let version_terms: Vec<String> = words
        .iter()
        .filter(|w| w.contains('.') && w.chars().any(|c| c.is_ascii_digit()) && !w.starts_with('.'))
        .cloned()
        .collect();
    if !version_terms.is_empty()
        && has(
            words,
            &["version", "versions", "only", "that", "which", "with"],
        )
    {
        let at = words
            .iter()
            .position(|w| matches!(w.as_str(), "that" | "which" | "with" | "only" | "version"))
            .unwrap_or(words.len());
        return (
            words[..at.min(words.len())].to_vec(),
            Some((version_terms, false)),
        );
    }
    (words.to_vec(), None)
}
/// The Minecraft version a launcher instance/profile folder declares, read from its metadata
/// (CurseForge, Prism/MultiMC, Modrinth or a modpack manifest). Read-only and bounded.
fn instance_version(dir: &Path) -> Option<String> {
    let quoted_after = |text: &str, key: &str, from: usize| -> Option<String> {
        let at = text[from..].find(key)? + from + key.len();
        let rest = &text[at..];
        let open = rest.find('"')?;
        let close = rest[open + 1..].find('"')?;
        // Skip the colon/space between key and value: the first quote after the key opens the value
        // unless it belongs to the closing quote of the key itself.
        let value = &rest[open + 1..open + 1 + close];
        if value.chars().all(|c| c == ':' || c == ' ') {
            let rest2 = &rest[open + 1 + close + 1..];
            let o2 = rest2.find('"')?;
            let c2 = rest2[o2 + 1..].find('"')?;
            Some(rest2[o2 + 1..o2 + 1 + c2].to_string())
        } else {
            Some(value.to_string())
        }
    };
    let read = |name: &str| -> Option<String> {
        let path = dir.join(name);
        let meta = std::fs::symlink_metadata(&path).ok()?;
        if !meta.is_file() || meta.len() > 4 * 1024 * 1024 {
            return None;
        }
        std::fs::read_to_string(path).ok()
    };
    if let Some(t) = read("minecraftinstance.json")
        && let Some(v) = quoted_after(&t, "\"gameVersion\"", 0)
    {
        return Some(v);
    }
    if let Some(t) = read("profile.json")
        && let Some(v) = quoted_after(&t, "\"game_version\"", 0)
    {
        return Some(v);
    }
    if let Some(t) = read("mmc-pack.json")
        && let Some(at) = t.find("\"net.minecraft\"")
        && let Some(v) = quoted_after(&t, "\"version\"", at)
    {
        return Some(v);
    }
    if let Some(t) = read("manifest.json")
        && let Some(at) = t.find("\"minecraft\"")
        && let Some(v) = quoted_after(&t, "\"version\"", at)
    {
        return Some(v);
    }
    None
}
fn list_named(
    ctx: &Ctx,
    words: &[String],
    folders: &[Folder],
    concept_only: bool,
    remove: bool,
) -> Option<Investigation> {
    let (words, filter) = split_filter(words);
    let words = &words[..];
    let app: Vec<&str> = words
        .iter()
        .map(String::as_str)
        .filter(|w| APP_WORDS.contains(w))
        .collect();
    let nouns: Vec<&String> = words
        .iter()
        .filter(|w| {
            !LIST_NOISE.contains(&w.as_str())
                && !APP_WORDS.contains(&w.as_str())
                && !FILLER.contains(&w.as_str())
        })
        .collect();
    if nouns.is_empty() || nouns.len() > 3 {
        return None;
    }
    let mut containers: Vec<&Folder> = Vec::new();
    for noun in nouns {
        let names: Vec<String> = match concept_names(noun) {
            Some(n) => n.iter().map(|s| s.to_string()).collect(),
            None if concept_only => continue,
            None => vec![noun.trim_end_matches('s').to_string(), noun.clone()],
        };
        // Names are in priority order: launcher "instances" beat generic "profiles" folders
        // that mods keep deep inside each instance.
        let mut found: Vec<&Folder> = Vec::new();
        for name in &names {
            found = folders
                .iter()
                .filter(|f| f.name_norm == alnum(name))
                .collect();
            if !found.is_empty() {
                break;
            }
        }
        if !app.is_empty() {
            let narrowed: Vec<&Folder> = found
                .iter()
                .copied()
                .filter(|f| {
                    let path = alnum(&f.path.to_string_lossy());
                    app.iter().any(|a| path.contains(a))
                })
                .collect();
            if !narrowed.is_empty() {
                found = narrowed;
            }
        }
        containers.extend(found);
    }
    containers.sort_by(|a, b| a.path.cmp(&b.path));
    containers.dedup_by(|a, b| a.path == b.path);
    // A container living inside another container is part of its contents, not a separate list.
    let all_paths: Vec<PathBuf> = containers.iter().map(|c| c.path.clone()).collect();
    containers.retain(|c| {
        !all_paths
            .iter()
            .any(|o| *o != c.path && c.path.starts_with(o))
    });
    if containers.is_empty() {
        return None;
    }
    let mut picks: Vec<FolderTarget> = Vec::new();
    let mut kept: Vec<String> = Vec::new();
    let mut kept_items: Vec<ListItem> = Vec::new();
    let mut unknown = 0usize;
    let text = String::new();
    for container in containers.iter().take(8) {
        let mut children: Vec<&Folder> = folders
            .iter()
            .filter(|f| f.path.parent() == Some(container.path.as_path()))
            .collect();
        children.sort_by_key(|f| std::cmp::Reverse(f.bytes));
        if children.is_empty() {
            continue;
        }

        for child in children.iter().take(60) {
            let name = child
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let version = ctx
                .root
                .and_then(|root| instance_version(&root.join(&child.path)));
            if version.is_none() {
                unknown += 1;
            }
            let haystack = format!(
                "{} {}",
                name.to_lowercase(),
                version.clone().unwrap_or_default().to_lowercase()
            );
            let matches_filter = filter.as_ref().is_none_or(|(terms, negate)| {
                let hit = terms.iter().any(|t| haystack.contains(t.as_str()));
                hit != *negate
            });
            let label = match &version {
                Some(v) => format!("{name} (Minecraft {v})"),
                None => name.clone(),
            };
            if matches_filter {
                if picks.len() < FOLDER_BATCH {
                    let mut t = target(child);
                    t.note = version.as_ref().map(|v| format!("Minecraft {v}"));
                    picks.push(t);
                }
            } else {
                let mut item = folder_item(child);
                item.note = version.as_ref().map(|v| format!("Minecraft {v} · kept"));
                kept_items.push(item);
                kept.push(label);
            }
        }
    }
    {
        let paths: Vec<String> = picks.iter().map(|p| p.path.clone()).collect();
        picks.retain(|p| {
            !paths
                .iter()
                .any(|o| *o != p.path && p.path.starts_with(&format!("{o}/")))
        });
    }
    if picks.is_empty() {
        if filter.is_some() && !kept.is_empty() {
            return Some(reply(
                ctx,
                "find_filename",
                "Filtered the folders",
                format!("Every match was kept: {}", kept.join(", ")),
                format!(
                    "Nothing to remove: every one of them matches what you wanted to keep ({}). Nothing changed.",
                    kept.join(", ")
                ),
            ));
        }
        return None;
    }
    let container_name = containers[0]
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let filter_note = match &filter {
        Some((terms, true)) => format!(
            " Keeping {} (matching {}).",
            if kept.is_empty() {
                "none".to_string()
            } else {
                kept.join(", ")
            },
            terms.join(" / ")
        ),
        Some((terms, false)) => format!(" Only those matching {}.", terms.join(" / ")),
        None => String::new(),
    };
    let coverage = if filter.is_some() && unknown > 0 {
        format!(
            " I could read the game version for {} of {}; for the rest I only had the folder name, so double-check those.",
            picks.len() + kept.len() - unknown.min(picks.len() + kept.len()),
            picks.len() + kept.len()
        )
    } else {
        String::new()
    };
    let total: u64 = picks.iter().map(|p| p.bytes).sum();
    let mut r = reply(
        ctx,
        if remove {
            "trash_named_files"
        } else {
            "find_filename"
        },
        "Looked for matching folders",
        format!(
            "Matched “{}” to {}",
            words.join(" "),
            containers
                .iter()
                .take(3)
                .map(|c| c.path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        String::new(),
    );
    if !kept_items.is_empty() {
        r.sections.push(Section {
            title: "Kept (matches what you wanted to keep)".into(),
            items: std::mem::take(&mut kept_items),
        });
    }
    r.folders = picks;
    if remove && filter.is_some() {
        r.proposal.rationale = format!(
            "{} of the folders in “{container_name}” ({}) are ready for the Trash — each moves whole and stays recoverable.{filter_note}{coverage}{text}\n\nUncheck anything you want to keep. Nothing happens until you approve.",
            r.folders.len(),
            bytes(total),
        );
    } else {
        r.pick = true;
        r.proposal.rationale = format!(
            "Found {} in {} — in this folder they are called “{container_name}”:{filter_note}{coverage}{text}\n\nTick the ones to move to the Trash below, or tell me which.",
            plural(r.folders.len(), "match", "matches"),
            ctx.scope,
        );
    }
    Some(r)
}

const MAX_HASH_BYTES: u64 = 1 << 30;
const MAX_HASHED_FILES: usize = 20_000;
/// Rename many files by rule: change extensions, tidy name style, or find and remove duplicates.
fn bulk_ops(
    ctx: &Ctx,
    raw: &[String],
    words: &[String],
    folders: &[Folder],
) -> Option<Investigation> {
    let denies = words.iter().enumerate().any(|(i, w)| {
        matches!(
            w.as_str(),
            "change"
                | "convert"
                | "rename"
                | "switch"
                | "turn"
                | "make"
                | "replace"
                | "remove"
                | "delete"
                | "lowercase"
                | "find"
                | "show"
                | "list"
                | "clean"
                | "get"
        ) && negated(words, i)
    });
    if denies {
        return None;
    }
    // Duplicates: identical content, decided by hashing files of equal size.
    if has(
        words,
        &["duplicate", "duplicates", "duplicated", "dupes", "dupe"],
    ) && has(
        words,
        &[
            "delete", "remove", "trash", "clean", "find", "show", "list", "get", "erase", "detect",
        ],
    ) {
        let remove = has(
            words,
            &["delete", "remove", "trash", "clean", "erase", "get"],
        );
        return Some(duplicates(ctx, words, folders, remove));
    }
    let names_talk = has(words, &["names", "filenames", "filename"]);
    // Extension change: "change all .txt files to .md".
    let action = has(
        words,
        &["change", "convert", "rename", "switch", "turn", "make"],
    );
    if action && has(words, &["extension", "extensions", "files"]) {
        let to_at = words
            .iter()
            .rposition(|w| matches!(w.as_str(), "to" | "into" | "as"));
        if let Some(at) = to_at {
            let target = words
                .get(at + 1)
                .map(|w| w.trim_start_matches('.').to_lowercase())
                .unwrap_or_default();
            let head: Vec<String> = words[..at].to_vec();
            let c = parse_criteria(&head, folders);
            let target_is_ext = !target.is_empty()
                && target.len() <= 10
                && target.chars().all(|ch| ch.is_ascii_alphanumeric())
                && (words[at + 1].starts_with('.')
                    || has(words, &["extension", "extensions"])
                    || KNOWN_EXTS.contains(&target.as_str()));
            if !c.exts.is_empty()
                && target_is_ext
                && kind_exts(&target).is_none_or(|_| words[at + 1].starts_with('.'))
                && !names_talk
            {
                return Some(change_extensions(ctx, &c, &target));
            }
        }
    }
    // Name styles and find/replace inside file names.
    if names_talk
        && has(
            words,
            &[
                "rename",
                "change",
                "make",
                "convert",
                "replace",
                "remove",
                "delete",
                "lowercase",
                "fix",
                "clean",
                "normalize",
                "normalise",
                "turn",
            ],
        )
    {
        return bulk_rename(ctx, raw, words, folders);
    }
    None
}
const KNOWN_EXTS: &[&str] = &[
    "txt", "md", "markdown", "json", "csv", "html", "htm", "xml", "yaml", "yml", "log", "rtf",
    "doc", "docx", "pdf", "jpg", "jpeg", "png", "gif", "webp", "heic", "mp4", "mov", "mp3", "wav",
    "zip", "js", "ts", "py", "rs", "swift", "java", "c", "cpp", "h", "sh", "bak", "old", "tmp",
    "dat", "bin",
];
fn change_extensions(ctx: &Ctx, c: &Criteria, target: &str) -> Investigation {
    let occupied: HashSet<String> = ctx
        .files
        .iter()
        .map(|f| f.relative_path.to_string_lossy().to_lowercase())
        .collect();
    let mut taken: HashSet<String> = HashSet::new();
    let mut actions = Vec::new();
    let mut sources = Vec::new();
    let (mut collisions, mut eligible) = (0usize, 0usize);
    let mut sorted: Vec<&FileCandidate> = ctx
        .files
        .iter()
        .filter(|f| matches_file(f, c, ctx.now))
        .collect();
    sorted.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    for f in sorted {
        let current = f
            .relative_path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if current == target {
            continue;
        }
        eligible += 1;
        let dest = f.relative_path.with_extension(target);
        let key = dest.to_string_lossy().to_lowercase();
        if occupied.contains(&key) || !taken.insert(key) {
            collisions += 1;
            continue;
        }
        if actions.len() < BATCH {
            let Some(name) = dest.file_name() else {
                continue;
            };
            actions.push(ProposedAction::Rename {
                source: f.id,
                new_name: name.to_string_lossy().into_owned(),
            });
            sources.push(Source {
                id: f.id.0,
                path: f.relative_path.to_string_lossy().into(),
                size: f.size,
            });
        }
    }
    let text = if actions.is_empty() {
        format!(
            "Nothing to change: no {} would get a new .{target} extension{}.",
            if c.label.is_empty() {
                "matching files"
            } else {
                c.label.as_str()
            },
            if collisions > 0 {
                format!(" without colliding with an existing name ({collisions} would)")
            } else {
                String::new()
            }
        )
    } else {
        format!(
            "Renaming {} ({}) to .{target}. Only the name changes — the contents stay exactly as they are, so this does not convert the file format.{}{} Undo is available in History.",
            plural(actions.len(), "file", "files"),
            if c.label.is_empty() {
                "matching files".to_string()
            } else {
                c.label.clone()
            },
            if collisions > 0 {
                format!(" {collisions} skipped because that name already exists.")
            } else {
                String::new()
            },
            if eligible > BATCH {
                format!(" This batch covers {BATCH} of {eligible}; ask again for the rest.")
            } else {
                String::new()
            },
        )
    };
    let mut r = plan_reply(
        ctx,
        "change_extensions",
        "Understood your request",
        format!("Change extensions to .{target}"),
        text,
        actions,
        sources,
    );
    r.remaining_matches = eligible.saturating_sub(BATCH);
    r.complete = r.remaining_matches == 0;
    r
}
fn bulk_rename(
    ctx: &Ctx,
    raw: &[String],
    words: &[String],
    folders: &[Folder],
) -> Option<Investigation> {
    // Which transformation?
    #[derive(Clone)]
    enum Op {
        Lower,
        Spaces(&'static str),
        Replace(String, String),
        Remove(String),
    }
    let op = if has(words, &["lowercase"]) || contains_seq(words, &["lower", "case"]) {
        Op::Lower
    } else if has(words, &["spaces", "space"]) && has(words, &["underscore", "underscores"]) {
        Op::Spaces("_")
    } else if has(words, &["spaces", "space"])
        && has(words, &["dash", "dashes", "hyphen", "hyphens"])
    {
        Op::Spaces("-")
    } else if has(words, &["spaces", "space"]) && has(words, &["remove", "delete", "without"]) {
        Op::Spaces("")
    } else if let Some(at) = words.iter().position(|w| w == "replace") {
        let with_at = words[at..].iter().position(|w| w == "with")? + at;
        let end = words[with_at..]
            .iter()
            .position(|w| matches!(w.as_str(), "in" | "from" | "within" | "across"))?
            .checked_add(with_at)
            .unwrap_or(words.len());
        let from = clean_name(&raw[at + 1..with_at]);
        let to = clean_name(&raw[with_at + 1..end.min(raw.len())]);
        if from.is_empty() {
            return None;
        }
        Op::Replace(from, to)
    } else if let Some(at) = words
        .iter()
        .position(|w| matches!(w.as_str(), "remove" | "delete"))
    {
        let end = words[at..]
            .iter()
            .position(|w| matches!(w.as_str(), "from" | "in"))?
            + at;
        let text = clean_name(&raw[at + 1..end.min(raw.len())]);
        let text = text
            .trim_start_matches("the word ")
            .trim_start_matches("word ")
            .trim()
            .to_string();
        if text.is_empty() {
            return None;
        }
        Op::Remove(text)
    } else {
        return None;
    };
    let c = parse_criteria(words, folders);
    let scoped = c.targets_files() || c.in_folder.is_some();
    let occupied: HashSet<String> = ctx
        .files
        .iter()
        .map(|f| f.relative_path.to_string_lossy().to_lowercase())
        .collect();
    let mut taken: HashSet<String> = HashSet::new();
    let (mut actions, mut sources) = (Vec::new(), Vec::new());
    let (mut collisions, mut changed) = (0usize, 0usize);
    let mut sorted: Vec<&FileCandidate> = ctx
        .files
        .iter()
        .filter(|f| !never_touch(&f.relative_path) && (!scoped || matches_file(f, &c, ctx.now)))
        .collect();
    sorted.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    for f in sorted {
        let Some(name) = f
            .relative_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
        else {
            continue;
        };
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
            _ => (name.clone(), String::new()),
        };
        let new_stem = match &op {
            Op::Lower => stem.to_lowercase(),
            Op::Spaces(with) => stem.replace(' ', with),
            Op::Replace(from, to) => replace_ci(&stem, from, to),
            Op::Remove(text) => {
                let cleaned = replace_ci(&stem, text, "")
                    .replace("()", "")
                    .replace("( )", "");
                cleaned
                    .split([' ', '-', '_'])
                    .filter(|p| !p.is_empty())
                    .collect::<Vec<_>>()
                    .join(if stem.contains(' ') {
                        " "
                    } else if stem.contains('_') {
                        "_"
                    } else {
                        "-"
                    })
            }
        };
        let new_ext = if matches!(op, Op::Lower) {
            ext.to_lowercase()
        } else {
            ext.clone()
        };
        let new_name = format!("{new_stem}{new_ext}");
        if new_stem.is_empty() || new_name == name {
            continue;
        }
        changed += 1;
        let dest = f.relative_path.with_file_name(&new_name);
        let key = dest.to_string_lossy().to_lowercase();
        // A case-only rename maps to itself on case-insensitive volumes; treat the file's own key as free.
        let own = f.relative_path.to_string_lossy().to_lowercase();
        if (key != own && occupied.contains(&key)) || !taken.insert(key) {
            collisions += 1;
            continue;
        }
        if actions.len() < BATCH {
            actions.push(ProposedAction::Rename {
                source: f.id,
                new_name,
            });
            sources.push(Source {
                id: f.id.0,
                path: f.relative_path.to_string_lossy().into(),
                size: f.size,
            });
        }
    }
    let what = match &op {
        Op::Lower => "lowercase names".to_string(),
        Op::Spaces(w) => format!("spaces replaced by “{w}”"),
        Op::Replace(a, b) => format!("“{a}” replaced by “{b}”"),
        Op::Remove(t) => format!("“{t}” removed from names"),
    };
    let text = if actions.is_empty() {
        format!("No file names would change ({what}). Nothing changed.")
    } else {
        format!(
            "Renaming {} — {what}. Extensions and contents stay as they are.{}{} Undo is available in History.",
            plural(actions.len(), "file", "files"),
            if collisions > 0 {
                format!(" {collisions} skipped to avoid a name clash.")
            } else {
                String::new()
            },
            if changed > BATCH {
                format!(" This batch covers {BATCH} of {changed}; ask again for the rest.")
            } else {
                String::new()
            }
        )
    };
    let mut r = plan_reply(
        ctx,
        "normalize_names",
        "Understood your request",
        format!("Rename files: {what}"),
        text,
        actions,
        sources,
    );
    r.remaining_matches = changed.saturating_sub(BATCH);
    r.complete = r.remaining_matches == 0;
    Some(r)
}
fn replace_ci(text: &str, from: &str, to: &str) -> String {
    if from.is_empty() {
        return text.to_string();
    }
    let (lower, needle) = (text.to_lowercase(), from.to_lowercase());
    if lower.len() != text.len() {
        return text.replace(from, to);
    }
    let mut out = String::new();
    let mut at = 0;
    while let Some(found) = lower[at..].find(&needle) {
        out.push_str(&text[at..at + found]);
        out.push_str(to);
        at += found + needle.len();
    }
    out.push_str(&text[at..]);
    out
}
fn hash_file(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Some(format!("{:x}", hasher.finalize()))
}
/// Identical files (same size, then same SHA-256). The oldest copy, then the shortest path, is kept.
fn duplicates(ctx: &Ctx, words: &[String], folders: &[Folder], remove: bool) -> Investigation {
    let Some(root) = ctx.root else {
        return reply(
            ctx,
            "exact_duplicates",
            "Looked for duplicates",
            "No folder access".into(),
            "I need access to the folder on disk to compare file contents.".into(),
        );
    };
    let c = parse_criteria(words, folders);
    let mut by_size: BTreeMap<u64, Vec<&FileCandidate>> = BTreeMap::new();
    for f in ctx.files.iter().filter(|f| {
        f.size > 0
            && !never_touch(&f.relative_path)
            && matches_file(
                f,
                &Criteria {
                    older_days: None,
                    min_bytes: None,
                    top_n: None,
                    name_terms: vec![],
                    excludes: c.excludes.clone(),
                    ..Criteria {
                        exts: c.exts.clone(),
                        screenshots: c.screenshots,
                        junk: c.junk,
                        in_folder: c.in_folder.clone(),
                        ..Default::default()
                    }
                },
                ctx.now,
            )
    }) {
        by_size.entry(f.size).or_default().push(f);
    }
    let (mut read, mut hashed, mut skipped_budget) = (0u64, 0usize, false);
    let mut groups: Vec<Vec<&FileCandidate>> = Vec::new();
    for (size, same) in by_size.into_iter().rev().filter(|(_, v)| v.len() > 1) {
        let mut by_hash: BTreeMap<String, Vec<&FileCandidate>> = BTreeMap::new();
        for f in same {
            if read + size > MAX_HASH_BYTES || hashed >= MAX_HASHED_FILES {
                skipped_budget = true;
                continue;
            }
            if let Some(h) = hash_file(&root.join(&f.relative_path)) {
                read += size;
                hashed += 1;
                by_hash.entry(h).or_default().push(f);
            }
        }
        groups.extend(by_hash.into_values().filter(|g| g.len() > 1));
    }
    for g in &mut groups {
        g.sort_by(|a, b| {
            a.modified
                .cmp(&b.modified)
                .then(
                    a.relative_path
                        .components()
                        .count()
                        .cmp(&b.relative_path.components().count()),
                )
                .then(a.relative_path.cmp(&b.relative_path))
        });
    }
    groups.sort_by_key(|g| std::cmp::Reverse(g[0].size * (g.len() as u64 - 1)));
    let redundant: Vec<&FileCandidate> =
        groups.iter().flat_map(|g| g[1..].iter().copied()).collect();
    let reclaim: u64 = redundant.iter().map(|f| f.size).sum();
    let head = if groups.is_empty() {
        "I compared the contents of every file that shares a size with another and found no exact duplicates.".to_string()
    } else {
        format!(
            "Found {} in {} — {} would be freed by keeping one copy of each (the oldest, at the shortest path).",
            plural(redundant.len(), "duplicate file", "duplicate files"),
            plural(groups.len(), "group", "groups"),
            bytes(reclaim)
        )
    };
    let mut r = reply(
        ctx,
        "exact_duplicates",
        "Compared file contents",
        format!("Hashed {hashed} files ({}) of equal size", bytes(read)),
        head,
    );
    if skipped_budget {
        r.proposal
            .rationale
            .push_str(" The comparison stopped at its read budget, so there may be more.");
    }
    for g in groups.iter().take(20) {
        r.sections.push(Section {
            title: format!("{} identical copies · {} each", g.len(), bytes(g[0].size)),
            items: g
                .iter()
                .enumerate()
                .map(|(i, f)| ListItem {
                    note: Some(if i == 0 {
                        "keep".into()
                    } else {
                        "duplicate".into()
                    }),
                    ..file_item(f)
                })
                .collect(),
        });
    }
    if remove && !redundant.is_empty() {
        for f in redundant.iter().take(BATCH) {
            r.proposal
                .actions
                .push(ProposedAction::Trash { source: f.id });
            r.sources.push(Source {
                id: f.id.0,
                path: f.relative_path.to_string_lossy().into(),
                size: f.size,
            });
        }
        r.proposal.rationale.push_str(" The extra copies are ready for the Trash; the originals stay. Uncheck any you want to keep.");
        r.remaining_matches = redundant.len().saturating_sub(BATCH);
    }
    r.examined = hashed;
    r
}

const PROJECT_KIND_WORDS: &[(&str, &str)] = &[
    ("git", "Git"),
    ("node", "Node"),
    ("npm", "Node"),
    ("javascript", "Node"),
    ("rust", "Rust"),
    ("cargo", "Rust"),
    ("python", "Python"),
    ("gradle", "Gradle"),
    ("java", "Gradle"),
    ("web", "Web"),
    ("html", "Web"),
    ("flutter", "Flutter"),
    ("xcode", "Xcode"),
    ("swift", "Swift"),
    ("php", "PHP"),
    ("ruby", "Ruby"),
    ("go", "Go"),
];
/// Follow-ups that only make sense with the previous request in mind (“why no size?”, “measure
/// them”, “only the Rust ones”, “biggest first”, “delete them”). `previous` is the last request that
/// produced a listing.
pub fn follow_up(
    request: &str,
    previous: &str,
    files: &[FileCandidate],
    scope: &str,
    now: i64,
    root: Option<&Path>,
) -> Option<Investigation> {
    let words = tokens(request.rsplit("User follow-up:").next().unwrap_or(request));
    if words.is_empty() || words.len() > 14 {
        return None;
    }
    let ctx = Ctx {
        files,
        scope,
        now,
        root,
    };
    let projects = wants_projects(previous);
    let size_talk = has(
        &words,
        &[
            "size", "sizes", "big", "large", "heavy", "weigh", "weighs", "heavy",
        ],
    );
    let questioning = has(
        &words,
        &[
            "why", "come", "missing", "no", "without", "blank", "unknown", "empty", "don't",
            "dont", "not", "some",
        ],
    );
    let pronoun = has(
        &words,
        &[
            "them", "those", "these", "it", "ones", "all", "each", "they",
        ],
    );
    if size_talk
        && questioning
        && !has(
            &words,
            &[
                "biggest", "largest", "heaviest", "sort", "delete", "remove", "first",
            ],
        )
    {
        return Some(reply(
            &ctx,
            "analyze_storage",
            "Explained missing sizes",
            "Sizes come from Tidy's index".into(),
            "Sizes come from Tidy's saved index. A folder shows none when it was skipped by an older scan (Git repositories weren't indexed before) or hasn't been scanned yet. Say “measure them” and I'll read their sizes straight from disk now, or press Rescan in Storage to index them properly.".into(),
        ));
    }
    let measure_verbs = has(
        &words,
        &[
            "measure",
            "calculate",
            "compute",
            "get",
            "show",
            "add",
            "fill",
            "find",
            "give",
            "check",
            "read",
            "take",
            "figure",
        ],
    );
    if projects {
        let root = root?;
        let opts_for = |mutate: &dyn Fn(&mut ProjectOpts)| {
            let mut o = ProjectOpts::default();
            mutate(&mut o);
            list_projects_with(root, scope, files, &o)
        };
        if (size_talk
            && (measure_verbs || pronoun)
            && !has(
                &words,
                &[
                    "biggest", "largest", "heaviest", "first", "sort", "delete", "remove",
                ],
            ))
            || (has(&words, &["measure", "calculate", "compute"]) && pronoun)
        {
            return Some(opts_for(&|o| o.measure_all = true));
        }
        if has(&words, &["delete", "remove", "trash", "erase"]) && pronoun {
            let list = opts_for(&|o| o.measure_all = true);
            let targets: Vec<FolderTarget> = list
                .sections
                .first()?
                .items
                .iter()
                .take(FOLDER_BATCH)
                .map(|i| FolderTarget {
                    path: i.path.clone(),
                    files: i.files,
                    bytes: i.bytes,
                    note: i.note.clone(),
                })
                .collect();
            let total: u64 = targets.iter().map(|t| t.bytes).sum();
            let mut r = reply(
                &ctx,
                "trash_named_files",
                "Understood your request",
                format!("Move {} project folders to Trash", targets.len()),
                format!(
                    "{} ({}) ready for the Trash — each project moves whole and stays recoverable. Uncheck any you want to keep. Nothing happens until you approve.",
                    plural(targets.len(), "project folder", "project folders"),
                    bytes(total)
                ),
            );
            r.folders = targets;
            return Some(r);
        }
        if let Some((_, label)) = PROJECT_KIND_WORDS
            .iter()
            .find(|(w, _)| words.iter().any(|x| x == w))
            && has(
                &words,
                &[
                    "only", "just", "which", "show", "list", "filter", "those", "ones", "are",
                ],
            )
        {
            let label = label.to_string();
            return Some(opts_for(&|o| o.only_kind = Some(label.clone())));
        }
        if has(&words, &["biggest", "largest", "heaviest", "heaviest"])
            || (size_talk && has(&words, &["sort", "by", "order", "first"]))
        {
            return Some(opts_for(&|o| {
                o.sort = Some("size".into());
                o.measure_all = true;
            }));
        }
        if has(&words, &["oldest", "stale", "untouched"]) {
            return Some(opts_for(&|o| o.sort = Some("oldest".into())));
        }
        if has(
            &words,
            &["alphabetical", "alphabetically", "name", "names", "a-z"],
        ) && has(
            &words,
            &[
                "sort",
                "by",
                "order",
                "alphabetical",
                "alphabetically",
                "a-z",
            ],
        ) {
            return Some(opts_for(&|o| o.sort = Some("name".into())));
        }
    }
    None
}

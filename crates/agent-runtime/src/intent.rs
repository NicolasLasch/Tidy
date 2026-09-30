//! Instant understanding of everyday requests. No model is loaded and nothing is ever executed:
//! the result is a reviewable preview like every other plan. Ambiguity is resolved with the most
//! sensible reading and disclosed in the reply instead of turning into a question.

#![allow(clippy::all)]

use crate::investigation::{FolderTarget, Investigation, Source, Trace};
use std::{
    collections::{BTreeMap, HashSet},
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
    text.to_lowercase()
        .replace(['’', '‘', '“', '”'], "'")
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '!' | '?' | '(' | ')' | '"'))
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
    let phrase: Vec<&String> = phrase
        .iter()
        .filter(|w| !FILLER.contains(&w.as_str()))
        .collect();
    if phrase.is_empty() {
        return vec![];
    }
    let phrase_norm: String = phrase.iter().map(|w| alnum(w)).collect();
    if phrase_norm.len() < 2 {
        return vec![];
    }
    let mut scored: Vec<(u8, &Folder)> = folders
        .iter()
        .filter_map(|f| {
            if f.name_norm.is_empty() {
                return None;
            }
            let score = if f.name_norm == phrase_norm {
                4
            } else if phrase.iter().all(|p| {
                f.name_tokens
                    .iter()
                    .any(|t| t == *p || (p.len() >= 3 && t.starts_with(p.as_str())))
            }) {
                3
            } else if phrase_norm.len() >= 4 && f.name_norm.contains(&phrase_norm) {
                2
            } else if f.name_norm.len() >= 4 && phrase_norm.contains(&f.name_norm) {
                1
            } else {
                return None;
            };
            Some((score, f))
        })
        .collect();
    let best = scored.iter().map(|(s, _)| *s).max().unwrap_or(0);
    scored.retain(|(s, _)| *s == best);
    scored.sort_by(|a, b| b.1.bytes.cmp(&a.1.bytes).then(a.1.path.cmp(&b.1.path)));
    scored.into_iter().map(|(_, f)| f).collect()
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
            if matches!(next.as_str(), "and" | "but" | "or" | "then") {
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
        })
        .collect();
    out.sort_by_key(|f| std::cmp::Reverse(f.bytes));
    out
}

struct Ctx<'a> {
    files: &'a [FileCandidate],
    scope: &'a str,
    now: i64,
}
fn reply(ctx: &Ctx, workflow: &str, headline: &str, detail: String, text: String) -> Investigation {
    Investigation {
        engine: "instant".into(),
        workflow: crate::workflows::get(workflow).cloned(),
        proposal: Proposal {
            actions: vec![],
            rationale: text,
        },
        sources: vec![],
        folders: vec![],
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
            "I can look through {} ({}) and get things done — just tell me what you want:\n\n• “Delete the Lucky World Invasion folder”\n• “Remove all .log files older than 3 months”\n• “Delete the 10 biggest files”\n• “Clean up build artifacts and node_modules”\n• “Organize this folder by type” or “by date”\n• “What’s taking the most space?”\n• “Find invoice”\n\nEverything goes to the Trash after you approve a preview, so it can always be restored from Finder.",
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
    let mut text = format!(
        "{} holds {} ({}).",
        ctx.scope,
        plural(ctx.files.len(), "indexed file", "indexed files"),
        bytes(total)
    );
    let want_files = has(words, &["file", "files"])
        && has(words, &["biggest", "largest", "heaviest", "big", "large"]);
    let want_folders = has(words, &["folder", "folders", "directory", "directories"]);
    if !want_files || want_folders {
        let mut top: Vec<&Folder> = folders
            .iter()
            .filter(|f| f.path.components().count() == 1)
            .collect();
        top.sort_by_key(|f| std::cmp::Reverse(f.bytes));
        if !top.is_empty() {
            text.push_str("\n\nLargest folders here:");
            for f in top.iter().take(8) {
                text.push_str(&format!(
                    "\n• {} — {} ({} files)",
                    f.path.display(),
                    bytes(f.bytes),
                    f.files
                ));
            }
        }
        let mut deep: Vec<&Folder> = folders
            .iter()
            .filter(|f| f.path.components().count() > 1 && f.bytes > 0)
            .collect();
        deep.sort_by_key(|f| std::cmp::Reverse(f.bytes));
        let leaves: Vec<&&Folder> = deep
            .iter()
            .filter(|f| !top.iter().any(|t| t.path == f.path))
            .filter(|f| {
                !deep.iter().any(|o| {
                    o.path != f.path && o.path.starts_with(&f.path) && o.bytes * 10 >= f.bytes * 9
                })
            })
            .take(5)
            .collect();
        if !leaves.is_empty() {
            text.push_str("\n\nWhere the space actually sits:");
            for f in leaves {
                text.push_str(&format!("\n• {} — {}", f.path.display(), bytes(f.bytes)));
            }
        }
    }
    let mut files: Vec<&FileCandidate> = ctx.files.iter().collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.size));
    text.push_str("\n\nBiggest files:");
    for f in files.iter().take(if want_files { 12 } else { 5 }) {
        text.push_str(&format!(
            "\n• {} — {}",
            f.relative_path.display(),
            bytes(f.size)
        ));
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
    text.push_str("\n\nMost space by type: ");
    text.push_str(
        &kinds
            .iter()
            .take(5)
            .map(|(k, (n, b))| format!(".{k} {} ({n})", bytes(*b)))
            .collect::<Vec<_>>()
            .join(", "),
    );
    text.push_str("\n\nTell me what to remove — for example “delete the biggest folder” — and I’ll prepare it for your approval.");
    let mut r = reply(
        ctx,
        "analyze_storage",
        "Read the index",
        format!("Summed all {} indexed files", ctx.files.len()),
        text,
    );
    r.examined = ctx.files.len();
    r
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
        let mut t = format!(
            "Found {} matching “{}”:",
            plural(total, "file", "files"),
            terms.join(" ")
        );
        for f in hits.iter().take(15) {
            t.push_str(&format!(
                "\n• {} — {}",
                f.relative_path.display(),
                bytes(f.size)
            ));
        }
        if total > 15 {
            t.push_str(&format!("\n…and {} more.", total - 15));
        }
        t.push_str("\n\nSay “delete them” if you want these moved to the Trash.");
        t
    };
    Some(reply(
        ctx,
        "find_filename",
        "Searched filenames",
        format!("Checked all {} paths", ctx.files.len()),
        text,
    ))
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

fn phrases_for_folders(words: &[String]) -> (Vec<Vec<String>>, Option<Vec<String>>) {
    // "<target> from|in|inside <parent>"
    let mut target: Vec<String> = Vec::new();
    let mut parent: Option<Vec<String>> = None;
    let mut in_parent = false;
    let mut phrases = Vec::new();
    for w in words {
        if matches!(w.as_str(), "from" | "in" | "inside" | "within" | "under") && !target.is_empty()
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
        if matches!(w.as_str(), "and" | "&" | "plus") {
            if !target.is_empty() {
                phrases.push(std::mem::take(&mut target));
            }
            continue;
        }
        if !FILLER.contains(&w.as_str()) {
            target.push(w.clone());
        }
    }
    if !target.is_empty() {
        phrases.push(target);
    }
    (phrases, parent)
}
fn trash_folders(
    ctx: &Ctx,
    words: &[String],
    folders: &[Folder],
    verb_at: usize,
) -> Option<Investigation> {
    let (phrases, parent) = phrases_for_folders(&words[verb_at + 1..]);
    if phrases.is_empty() {
        return None;
    }
    let parent_filter: Option<Vec<&Folder>> = parent.as_ref().map(|p| resolve(p, folders));
    let mut chosen: Vec<&Folder> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for phrase in &phrases {
        let mut found = resolve(phrase, folders);
        if let Some(parents) = &parent_filter {
            if !parents.is_empty() {
                found.retain(|f| {
                    parents
                        .iter()
                        .any(|p| f.path.starts_with(&p.path) && f.path != p.path)
                });
            }
        }
        let wants_all = words
            .iter()
            .any(|w| matches!(w.as_str(), "all" | "every" | "each"));
        match found.split_first() {
            None => missing.push(phrase.join(" ")),
            Some((best, rest)) => {
                if wants_all
                    && !rest.is_empty()
                    && rest.iter().all(|f| f.name_norm == best.name_norm)
                {
                    chosen.extend(found.iter().copied().take(FOLDER_BATCH));
                } else {
                    chosen.push(best);
                    if !rest.is_empty() {
                        notes.push(format!(
                            "“{}” also matches {}; I picked the largest ({}).",
                            phrase.join(" "),
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
    if chosen.is_empty() {
        return None;
    }
    let chosen = outermost(chosen);
    let mut seen = HashSet::new();
    let chosen: Vec<&Folder> = chosen
        .into_iter()
        .filter(|f| seen.insert(f.path.clone()))
        .collect();
    let targets: Vec<FolderTarget> = chosen.iter().map(|f| target(f)).collect();
    let total: u64 = targets.iter().map(|t| t.bytes).sum();
    let files: usize = targets.iter().map(|t| t.files).sum();
    let mut r = reply(
        ctx,
        "trash_named_files",
        "Understood your request",
        format!(
            "Move {} to Trash",
            targets
                .iter()
                .map(|t| t.path.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        String::new(),
    );
    r.proposal.rationale = format!(
        "{} — {} in {} ready for the Trash. The whole folder moves as one action and stays recoverable from Finder’s Trash (Put Back). Nothing happens until you approve.{}{}",
        if targets.len() == 1 {
            format!("“{}”", targets[0].path)
        } else {
            plural(targets.len(), "folder", "folders")
        },
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
            format!(" I couldn’t find a folder called {}.", missing.join(", "))
        },
    );
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
    let ctx = Ctx {
        files,
        scope: scope_name,
        now,
    };
    let last = request
        .rsplit("User follow-up:")
        .next()
        .unwrap_or(request)
        .trim();
    let folders = folders_of(files);
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
                    return Some(reply(
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
                    ));
                }
            }
            if c.targets_files() {
                return Some(trash_files(ctx, &c));
            }
            None
        }
        Verb::Organize => organize(ctx, &words),
        Verb::Show => {
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
                        "{} match {} — {} in total:",
                        plural(total, "file", "files"),
                        c.label,
                        bytes(total_bytes)
                    )
                };
                for f in hits.iter().take(15) {
                    text.push_str(&format!(
                        "\n• {} — {}",
                        f.relative_path.display(),
                        bytes(f.size)
                    ));
                }
                if total > 0 {
                    text.push_str("\n\nSay “delete them” and I’ll prepare these for the Trash.");
                }
                return Some(reply(
                    ctx,
                    "find_filename",
                    "Filtered the index",
                    format!("Checked all {} indexed files", ctx.files.len()),
                    text,
                ));
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
        assert!(r.proposal.rationale.contains("Instances"));
        assert!(r.proposal.actions.is_empty());
        assert!(run("hi").proposal.rationale.contains("Delete the Lucky"));
        let r = run("find setup");
        assert!(r.proposal.rationale.contains("setup.dmg"));
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
        assert!(r.proposal.rationale.contains("Alpha") && r.proposal.rationale.contains("Rust"));
        assert!(!r.proposal.rationale.contains("Notes"));
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
    let ctx = Ctx {
        files,
        scope,
        now: 0,
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
    for (path, kinds, modified) in found.iter().take(60) {
        let rel = path.strip_prefix(root).unwrap_or(path);
        let size = folders
            .iter()
            .find(|f| f.path == rel)
            .map(|f| format!(" · {}", bytes(f.bytes)))
            .unwrap_or_default();
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
        text.push_str(&format!(
            "\n• {} ({}) — {}{size} · {age}",
            rel.display(),
            kinds.join(", "),
            rel.display()
        ));
    }
    if found.len() > 60 {
        text.push_str(&format!("\n…and {} more.", found.len() - 60));
    }
    if !found.is_empty() {
        text.push_str("\n\nSay “delete <project name>” to move one to the Trash, or “what’s taking space?” to see the heaviest.");
    }
    reply(
        &ctx,
        "find_project",
        "Looked for project markers",
        format!("Scanned folder structure below {scope}"),
        text,
    )
}

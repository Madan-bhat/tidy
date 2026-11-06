//! Accuracy report: treat an already-sorted folder as ground truth.
//!
//! Layout assumed: `ROOT/<Subject>/…/<file>`. Subject folders are matched to the
//! user's configured subjects by (cleaned) name; nothing about the subjects is
//! built in. Used by `cargo run --release --example accuracy`.

use crate::classify_subject::{clean_subject_name, classify_detailed, enrich_all, SubjectIndex};
use crate::config;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

struct Row {
    file: String,
    expected: String,
    predicted: String,
    confidence: String,
    reason: String,
}

fn is_junk(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with('.') || n == "thumbs.db" || n == "desktop.ini"
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if is_junk(name) {
            continue;
        }
        if p.is_dir() {
            walk(&p, out);
        } else if p.is_file() {
            out.push(p);
        }
    }
}

fn pct(n: usize, d: usize) -> String {
    if d == 0 {
        "n/a".into()
    } else {
        format!("{:.1}%", 100.0 * n as f64 / d as f64)
    }
}

pub fn run(args: &[String]) -> i32 {
    let mut folder: Option<PathBuf> = None;
    let mut with_learned = false;
    let mut show_misses = 40usize;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--with-learned" => with_learned = true,
            "--misses" => {
                i += 1;
                show_misses = args.get(i).and_then(|v| v.parse().ok()).unwrap_or(40);
            }
            "-h" | "--help" => {
                println!("usage: accuracy [FOLDER] [--with-learned] [--misses N]");
                return 0;
            }
            other => folder = Some(PathBuf::from(other)),
        }
        i += 1;
    }

    // Learned terms come from the very files being graded, so leave them out by default.
    let cfg = if with_learned {
        config::load_effective()
    } else {
        config::load()
    };
    let root = match folder.or_else(|| cfg.destination_folder.as_ref().map(PathBuf::from)) {
        Some(r) if r.is_dir() => r,
        _ => {
            eprintln!("No folder to evaluate (pass one, or set a destination in Tidy).");
            return 2;
        }
    };

    let mut subjects = cfg.subjects.clone();
    enrich_all(&mut subjects);
    if subjects.iter().all(|s| !s.enabled) {
        eprintln!("No subjects configured — add some in Tidy first.");
        return 2;
    }
    let known: HashSet<String> = subjects
        .iter()
        .map(|s| s.name.to_lowercase())
        .chain(std::iter::once("general".to_string()))
        .collect();
    let index = SubjectIndex::build(&subjects);

    // Ground truth from the folder layout.
    let mut graded: Vec<(PathBuf, String)> = Vec::new();
    let mut skipped_folders: BTreeMap<String, usize> = BTreeMap::new();
    for entry in fs::read_dir(&root).into_iter().flatten().flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let raw = dir.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        if is_junk(&raw) {
            continue;
        }
        let expected = clean_subject_name(&raw);
        let mut files = Vec::new();
        walk(&dir, &mut files);
        if !known.contains(&expected.to_lowercase()) {
            *skipped_folders.entry(raw).or_insert(0) += files.len();
            continue;
        }
        // Use the configured spelling of the subject name.
        let canonical = subjects
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(&expected))
            .map(|s| s.name.clone())
            .unwrap_or_else(|| "General".to_string());
        graded.extend(files.into_iter().map(|f| (f, canonical.clone())));
    }

    if graded.is_empty() {
        eprintln!(
            "No graded files under {} (subject folders must match your subject names).",
            root.display()
        );
        return 2;
    }

    println!("Folder:  {}", root.display());
    println!(
        "Subjects: {}   Files graded: {}   Learned terms: {}",
        subjects.iter().filter(|s| s.enabled).count(),
        graded.len(),
        if with_learned { "included" } else { "excluded" }
    );
    for (f, n) in &skipped_folders {
        println!("  (skipped folder {f:?}: not one of your subjects — {n} files)");
    }

    let started = Instant::now();
    let mut rows: Vec<Row> = Vec::new();
    for (path, expected) in &graded {
        let v = classify_detailed(path, &index, &subjects, true, false);
        rows.push(Row {
            file: path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string(),
            expected: expected.clone(),
            predicted: v.subject.clone(),
            confidence: v.confidence_str().to_string(),
            reason: v.reason,
        });
    }

    // Only files whose true home is a real subject count toward recall.
    let real: Vec<&Row> = rows
        .iter()
        .filter(|r| !r.expected.eq_ignore_ascii_case("general"))
        .collect();
    let correct = real.iter().filter(|r| r.predicted == r.expected).count();
    let missed = real
        .iter()
        .filter(|r| r.predicted.eq_ignore_ascii_case("general"))
        .count();
    let wrong = real.len() - correct - missed;
    let predicted_real = real.len() - missed;

    println!("\n== Overall (files that belong to a subject: {}) ==", real.len());
    println!("  correct        {:>5}  {}", correct, pct(correct, real.len()));
    println!("  sent to General{:>5}  {}", missed, pct(missed, real.len()));
    println!("  wrong subject  {:>5}  {}", wrong, pct(wrong, real.len()));
    println!(
        "  precision when Tidy picks a subject: {}",
        pct(correct, predicted_real)
    );
    println!("  ({:.1}s)", started.elapsed().as_secs_f64());

    println!("\n== By confidence (is the label trustworthy?) ==");
    for level in ["high", "medium", "low", "none"] {
        let group: Vec<&&Row> = real.iter().filter(|r| r.confidence == level).collect();
        let ok = group.iter().filter(|r| r.predicted == r.expected).count();
        println!("  {level:<7} {:>5} files   correct {}", group.len(), pct(ok, group.len()));
    }

    println!("\n== By subject ==");
    let mut by_subject: BTreeMap<&str, (usize, usize, usize)> = BTreeMap::new();
    for r in &real {
        let e = by_subject.entry(r.expected.as_str()).or_insert((0, 0, 0));
        e.0 += 1;
        if r.predicted == r.expected {
            e.1 += 1;
        } else if r.predicted.eq_ignore_ascii_case("general") {
            e.2 += 1;
        }
    }
    for (name, (total, ok, to_general)) in by_subject {
        println!(
            "  {name:<40} {total:>4} files  correct {:>6}  to General {:>4}",
            pct(ok, total),
            to_general
        );
    }

    let mut bad: Vec<&&Row> = real.iter().filter(|r| r.predicted != r.expected).collect();
    bad.sort_by(|a, b| a.expected.cmp(&b.expected).then(a.file.cmp(&b.file)));
    if !bad.is_empty() && show_misses > 0 {
        println!("\n== Misses (first {}) ==", show_misses.min(bad.len()));
        for r in bad.iter().take(show_misses) {
            println!(
                "  {}\n      in {:?}  →  {:?} [{}]  {}",
                r.file, r.expected, r.predicted, r.confidence, r.reason
            );
        }
    }
    0
}

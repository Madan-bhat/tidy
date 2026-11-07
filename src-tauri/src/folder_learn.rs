//! Learn keywords from files the user has already sorted into subject folders,
//! and suggest keywords for first-run / empty subjects.
//!
//! Layout assumed: `DEST/<Subject>/…/<file>`. Subject folder names are matched
//! to configured subjects by cleaned name — nothing about the subjects is built in.

use crate::classify_subject::{clean_subject_name, learnable_terms};
use crate::extract;
use crate::learned;
use crate::subjects::SubjectDef;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestResult {
    pub subject: String,
    pub keywords: Vec<String>,
    pub from_files: usize,
}

fn is_junk(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with('.') || n == "thumbs.db" || n == "desktop.ini"
}

fn walk_files(dir: &Path, out: &mut Vec<PathBuf>, limit: usize) {
    if out.len() >= limit {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        if out.len() >= limit {
            break;
        }
        let p = e.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if is_junk(name) {
            continue;
        }
        if p.is_dir() {
            walk_files(&p, out, limit);
        } else if p.is_file() {
            out.push(p);
        }
    }
}

fn files_by_subject(dest: &Path, subjects: &[SubjectDef]) -> HashMap<String, Vec<PathBuf>> {
    let known: HashMap<String, String> = subjects
        .iter()
        .filter(|s| s.enabled && !s.name.eq_ignore_ascii_case("General"))
        .map(|s| (s.name.to_ascii_lowercase(), s.name.clone()))
        .collect();

    let mut map: HashMap<String, Vec<PathBuf>> = HashMap::new();
    let Ok(entries) = fs::read_dir(dest) else {
        return map;
    };
    for e in entries.flatten() {
        let dir = e.path();
        if !dir.is_dir() {
            continue;
        }
        let raw = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if is_junk(raw) {
            continue;
        }
        let cleaned = clean_subject_name(raw);
        let Some(canonical) = known.get(&cleaned.to_ascii_lowercase()) else {
            continue;
        };
        let mut files = Vec::new();
        walk_files(&dir, &mut files, 80);
        if !files.is_empty() {
            map.insert(canonical.clone(), files);
        }
    }
    map
}

/// Harvest distinctive terms from already-filed files into the learned store.
/// Returns how many new terms were added across all subjects.
pub fn learn_from_destination(dest: &Path, subjects: &[SubjectDef]) -> usize {
    let by_subject = files_by_subject(dest, subjects);
    let mut added = 0;
    for (subject, files) in by_subject {
        for path in files.into_iter().take(40) {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            // Prefer filename; peek at document text only for a few files to stay fast.
            let text = if extract::can_extract(&path) {
                extract::extract_text(&path)
            } else {
                None
            };
            added += learned::learn(&subject, &stem, text.as_deref());
        }
    }
    added
}

/// Propose keywords for each subject from its existing folder, without writing them.
/// Useful at first run / when a subject has few keywords.
pub fn suggest_keywords(dest: &Path, subjects: &[SubjectDef]) -> Vec<SuggestResult> {
    let by_subject = files_by_subject(dest, subjects);
    let mut out = Vec::new();

    for subject in subjects {
        if !subject.enabled || subject.name.eq_ignore_ascii_case("General") {
            continue;
        }
        let Some(files) = by_subject.get(&subject.name) else {
            continue;
        };
        let existing: HashSet<String> = subject
            .keywords
            .iter()
            .map(|k| k.to_ascii_lowercase())
            .collect();

        let mut freq: HashMap<String, u32> = HashMap::new();
        for path in files.iter().take(50) {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            // Filename only for suggestions — fast and domain-free.
            for term in learnable_terms(stem, None) {
                if term.len() < 4 {
                    continue;
                }
                if existing.contains(&term.to_ascii_lowercase()) {
                    continue;
                }
                *freq.entry(term).or_insert(0) += 1;
            }
        }
        let mut ranked: Vec<_> = freq.into_iter().collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let keywords: Vec<String> = ranked
            .into_iter()
            .map(|(t, _)| t)
            .take(24)
            .collect();
        if !keywords.is_empty() {
            out.push(SuggestResult {
                subject: subject.name.clone(),
                keywords,
                from_files: files.len(),
            });
        }
    }
    out
}

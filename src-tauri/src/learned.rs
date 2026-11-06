//! Learn from the user's corrections.
//!
//! When someone reassigns a file to a different subject in the preview, we remember
//! distinctive terms from that file under the subject. They are merged into the
//! subject's keywords *in memory only* (see `config::load_effective`) so
//! `config.json` stays exactly what the user typed.

use crate::classify_subject::{learnable_terms, tokens_of};
use crate::subjects::SubjectDef;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

const MAX_TERMS_PER_SUBJECT: usize = 120;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct Learned {
    terms: Vec<String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    subjects: HashMap<String, Learned>,
}

fn store_path() -> Option<PathBuf> {
    let dir = dirs::config_dir()?.join("Tidy");
    fs::create_dir_all(&dir).ok()?;
    Some(dir.join("learned.json"))
}

fn load() -> Store {
    store_path()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save(store: &Store) {
    if let Some(path) = store_path() {
        if let Ok(json) = serde_json::to_string_pretty(store) {
            let _ = fs::write(path, json);
        }
    }
}

/// Remember that `stem` (+ optional document text) belongs to `subject`.
/// Returns how many new terms were stored.
pub fn learn(subject: &str, stem: &str, doc_text: Option<&str>) -> usize {
    if subject.trim().is_empty() || subject.eq_ignore_ascii_case("general") {
        return 0;
    }
    let terms = learnable_terms(stem, doc_text);
    if terms.is_empty() {
        return 0;
    }
    let mut store = load();
    let entry = store.subjects.entry(subject.to_string()).or_default();
    let mut added = 0;
    for t in terms {
        if !entry.terms.iter().any(|e| e == &t) {
            entry.terms.push(t);
            added += 1;
        }
    }
    // Keep the most recent lessons.
    if entry.terms.len() > MAX_TERMS_PER_SUBJECT {
        let drop = entry.terms.len() - MAX_TERMS_PER_SUBJECT;
        entry.terms.drain(..drop);
    }
    save(&store);
    added
}

/// Append learned terms to each subject's keywords (in memory).
/// A term is skipped if it is already part of a *different* subject's name or
/// keywords — the user's own words always win.
pub fn apply(subjects: &mut [SubjectDef]) {
    let store = load();
    if store.subjects.is_empty() {
        return;
    }

    let owned: Vec<(String, HashSet<String>)> = subjects
        .iter()
        .map(|s| {
            let mut set: HashSet<String> = tokens_of(&s.name).into_iter().collect();
            for k in &s.keywords {
                set.extend(tokens_of(k));
            }
            (s.name.clone(), set)
        })
        .collect();

    for subject in subjects.iter_mut() {
        let Some(learned) = store.subjects.get(&subject.name) else {
            continue;
        };
        for term in &learned.terms {
            let toks = tokens_of(term);
            let taken_elsewhere = owned
                .iter()
                .any(|(n, set)| n != &subject.name && toks.iter().all(|t| set.contains(t)));
            if taken_elsewhere {
                continue;
            }
            if !subject.keywords.iter().any(|k| k.eq_ignore_ascii_case(term)) {
                subject.keywords.push(term.clone());
            }
        }
    }
}

pub fn counts() -> HashMap<String, usize> {
    load()
        .subjects
        .into_iter()
        .map(|(k, v)| (k, v.terms.len()))
        .filter(|(_, n)| *n > 0)
        .collect()
}

pub fn forget(subject: &str) {
    let mut store = load();
    if store.subjects.remove(subject).is_some() {
        save(&store);
    }
}

pub fn forget_all() {
    save(&Store::default());
}

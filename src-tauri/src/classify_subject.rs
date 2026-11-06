//! Subject recognition via Okapi BM25 (+ phrase / acronym / fuzzy n-gram boosts).
//!
//! Ranking uses only each subject's name and user-supplied keywords (no domain
//! dictionaries). Filenames are tokenized with camelCase / digit splits and
//! longest-match dictionary splitting against the user's own terms.

use crate::extract;
use crate::subjects::SubjectDef;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Tuned for short keyword "documents" and short filename queries.
const BM25_K1: f64 = 0.55;
const BM25_B: f64 = 0.85;
/// Minimum BM25(+boosts) score to accept a subject.
const MIN_SCORE: f64 = 1.8;
/// Require a clear winner over the runner-up.
const MIN_MARGIN: f64 = 0.55;
/// Char-trigram Jaccard threshold for soft token matches.
const FUZZY_JACCARD: f64 = 0.72;
/// Terms appearing in this fraction of a subject's keyword phrases are "generic"
/// to that list and alone cannot decide the match.
const GENERIC_CF: f64 = 0.18;

const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "or", "the", "of", "to", "in", "on", "for", "with", "using",
    "from", "into", "by", "as", "is", "are", "be", "this", "that", "these", "those",
    "including", "include", "other", "others", "such", "their", "its", "at", "via",
    "also", "more", "most", "than", "then", "over", "under", "about",
    // Generic file/course wording — not subject-discriminating on its own.
    "notes", "note", "lecture", "lectures", "lab", "labs", "laboratory", "manual",
    "assignment", "assignments", "tutorial", "tutorials", "question", "questions",
    "paper", "papers", "exam", "exams", "quiz", "test", "tests", "practical",
    "experiment", "experiments", "viva", "record", "report", "reports", "pdf",
    "docx", "pptx", "xlsx", "ppt", "doc", "sheet", "unit", "module", "chapter",
    "semester", "midterm", "internal", "assessment", "homework", "worksheet",
    "handout", "slides", "slide", "presentation", "syllabus", "reference",
    "textbook", "book", "solved", "problems", "problem", "numerical", "derivation",
    "formula", "revision", "important", "previous", "year", "pyq", "ia", "end",
    "sem", "mid", "file", "study", "material", "materials", "topic", "topics",
];

/// Precomputed BM25 index over the user's subjects — build once per scan/batch.
#[derive(Debug, Clone)]
pub struct SubjectIndex {
    docs: Vec<SubjectDoc>,
    idf: HashMap<String, f64>,
    avgdl: f64,
    /// User terms longest-first for splitting concatenated filename pieces.
    dict: Vec<String>,
}

#[derive(Debug, Clone)]
struct SubjectDoc {
    name: String,
    /// Binary term presence (capped) — large keyword lists must not inflate TF.
    tf: HashMap<String, u32>,
    /// Per-term rarity inside this subject's keyword list (1 = distinctive).
    rarity: HashMap<String, f64>,
    dl: f64,
    phrases: Vec<String>,
    acronyms: HashSet<String>,
    /// Distinctive terms only (for fuzzy matching).
    terms: Vec<String>,
    keyword_count: usize,
}

impl SubjectIndex {
    pub fn build(subjects: &[SubjectDef]) -> Self {
        let mut docs = Vec::new();
        let mut df: HashMap<String, u32> = HashMap::new();
        let mut dict_set: HashSet<String> = HashSet::new();

        for subject in subjects {
            if !subject.enabled || subject.name.eq_ignore_ascii_case("General") {
                continue;
            }

            let mut tf: HashMap<String, u32> = HashMap::new();
            let mut rarity: HashMap<String, f64> = HashMap::new();
            let mut phrases = Vec::new();
            let mut acronyms = HashSet::new();
            let mut term_phrase_df: HashMap<String, u32> = HashMap::new();

            let mut present = |term: &str, weight: u32| {
                if term.is_empty() || is_stop(term) {
                    return;
                }
                let e = tf.entry(term.to_string()).or_insert(0);
                // Cap TF so a 600-keyword dump cannot dominate via repeats.
                *e = (*e + weight).min(3);
                dict_set.insert(term.to_string());
            };

            // Name tokens are always distinctive for this subject.
            for piece in needles_from_text(&subject.name) {
                if piece.contains(' ') {
                    phrases.push(piece.clone());
                    for t in piece.split_whitespace() {
                        present(t, 2);
                        rarity.insert(t.to_string(), 1.0);
                    }
                } else if looks_like_code(&piece) || piece.len() <= 5 {
                    acronyms.insert(piece.clone());
                    present(&piece, 3);
                    rarity.insert(piece.clone(), 1.0);
                } else {
                    present(&piece, 2);
                    rarity.insert(piece.clone(), 1.0);
                }
            }

            let keyword_count = subject.keywords.len();
            for kw in &subject.keywords {
                let pieces = needles_from_text(kw);
                let mut seen_in_phrase: HashSet<String> = HashSet::new();
                for piece in &pieces {
                    if piece.contains(' ') {
                        let words = piece.split_whitespace().count();
                        // Keep multi-word phrases; skip ultra-long dumps as phrases.
                        if (2..=8).contains(&words) {
                            phrases.push(piece.clone());
                        }
                        for t in piece.split_whitespace() {
                            if !is_stop(t) {
                                seen_in_phrase.insert(t.to_string());
                            }
                            present(t, 1);
                        }
                    } else if looks_like_code(piece)
                        || (piece.len() <= 5 && piece.chars().all(|c| c.is_ascii_alphanumeric()))
                    {
                        acronyms.insert(piece.clone());
                        present(piece, 1);
                        seen_in_phrase.insert(piece.clone());
                    } else {
                        present(piece, 1);
                        seen_in_phrase.insert(piece.clone());
                    }
                }
                for t in seen_in_phrase {
                    *term_phrase_df.entry(t).or_insert(0) += 1;
                }
            }

            phrases.sort();
            phrases.dedup();
            // Prefer longer phrases when matching; cap for scan speed.
            phrases.sort_by_key(|p| std::cmp::Reverse(p.len()));
            if phrases.len() > 400 {
                phrases.truncate(400);
            }

            let phrase_n = keyword_count.max(1) as f64;
            for (term, count) in &term_phrase_df {
                let cf = *count as f64 / phrase_n;
                // Only punish "list-generic" terms when the keyword bank is large.
                // Small lists keep full weight so normal subjects still match.
                let r = if phrase_n >= 40.0 && cf >= GENERIC_CF {
                    (1.0 - cf).max(0.05).powi(2)
                } else if phrase_n >= 40.0 {
                    (1.0 - cf * 0.35).max(0.45)
                } else {
                    1.0
                };
                rarity
                    .entry(term.clone())
                    .and_modify(|e| *e = (*e).max(r))
                    .or_insert(r);
            }
            for term in tf.keys() {
                rarity.entry(term.clone()).or_insert(1.0);
            }

            for term in tf.keys() {
                *df.entry(term.clone()).or_insert(0) += 1;
            }

            let dl = tf.values().map(|c| *c as f64).sum::<f64>().max(1.0);
            let terms: Vec<String> = rarity
                .iter()
                .filter(|(t, r)| **r >= 0.45 && t.len() >= 4)
                .map(|(t, _)| t.clone())
                .collect();

            docs.push(SubjectDoc {
                name: subject.name.clone(),
                tf,
                rarity,
                dl,
                phrases,
                acronyms,
                terms,
                keyword_count,
            });
        }

        let n = docs.len() as f64;
        let mut idf = HashMap::new();
        for (term, count) in df {
            let nn = count as f64;
            let value = ((n - nn + 0.5) / (nn + 0.5) + 1.0).ln();
            idf.insert(term, value);
        }

        let avgdl = if docs.is_empty() {
            1.0
        } else {
            docs.iter().map(|d| d.dl).sum::<f64>() / docs.len() as f64
        };

        let mut dict: Vec<String> = dict_set
            .into_iter()
            .filter(|t| t.len() >= 4 || looks_like_code(t))
            .collect();
        dict.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
        if dict.len() > 2000 {
            dict.truncate(2000);
        }

        Self {
            docs,
            idf,
            avgdl,
            dict,
        }
    }

    /// Rank subjects for a normalized text blob (filename stem and/or extract).
    pub fn score_text(&self, text: &str) -> HashMap<String, f64> {
        let haystack = normalize(text);
        let query_tokens = tokenize_query(&haystack, &self.dict);
        let query_set: HashSet<&str> = query_tokens.iter().map(|s| s.as_str()).collect();

        let mut scores = HashMap::new();
        if self.docs.is_empty() || haystack.is_empty() {
            return scores;
        }

        for doc in &self.docs {
            let mut score = 0.0;
            let mut distinctive_hits = 0u32;
            let mut phrase_hits = 0u32;
            let mut acronym_hits = 0u32;

            // --- Okapi BM25 over query tokens (rarity-weighted) ---
            for qt in &query_tokens {
                if is_stop(qt) {
                    continue;
                }
                let tf = *doc.tf.get(qt).unwrap_or(&0) as f64;
                let (term_key, term_tf, mut idf, sim) = if tf > 0.0 {
                    (
                        qt.as_str(),
                        tf,
                        *self.idf.get(qt).unwrap_or(&0.0),
                        1.0,
                    )
                } else if let Some((matched, sim)) = best_fuzzy(qt, &doc.terms) {
                    let base_tf = *doc.tf.get(matched).unwrap_or(&1) as f64;
                    (
                        matched,
                        base_tf * sim,
                        *self.idf.get(matched).unwrap_or(&0.0) * sim,
                        sim,
                    )
                } else {
                    continue;
                };

                let rarity = *doc.rarity.get(term_key).unwrap_or(&0.35);
                idf *= rarity;
                if idf <= 0.0 && term_tf <= 0.0 {
                    continue;
                }
                let denom = term_tf + BM25_K1 * (1.0 - BM25_B + BM25_B * (doc.dl / self.avgdl));
                let contrib = idf * (term_tf * (BM25_K1 + 1.0)) / denom.max(1e-9);
                score += contrib;

                if rarity >= 0.45 && sim >= 0.9 && contrib >= 0.35 {
                    distinctive_hits += 1;
                }
            }

            // --- Phrase boost (only multi-word; gated by shared query token) ---
            for phrase in &doc.phrases {
                let words = phrase.split_whitespace().count();
                if words < 2 {
                    continue;
                }
                if !phrase.split_whitespace().any(|w| query_set.contains(w)) {
                    continue;
                }
                if haystack.contains(phrase.as_str()) {
                    let boost = 2.8 + words as f64 * 0.7;
                    score += boost;
                    phrase_hits += 1;
                    // A few strong phrases are enough.
                    if phrase_hits >= 3 {
                        break;
                    }
                }
            }

            // --- Acronym / course-code boost ---
            for ac in &doc.acronyms {
                if query_set.contains(ac.as_str()) {
                    let rarity = *doc.rarity.get(ac).unwrap_or(&1.0);
                    // Generic short codes that appear across huge keyword dumps stay weak.
                    if ac.len() <= 2 && rarity < 0.5 {
                        score += 0.4;
                    } else {
                        score += if ac.len() <= 2 { 1.2 } else { 2.6 };
                        acronym_hits += 1;
                    }
                }
            }

            // Evidence gate: a fat keyword list must not win on one generic token.
            let strong = phrase_hits >= 1 || acronym_hits >= 1 || distinctive_hits >= 2;
            if !strong {
                if distinctive_hits == 1 {
                    score *= 0.35;
                } else {
                    score *= 0.12;
                }
            }

            // Larger keyword banks need slightly more score to win.
            let size_penalty = 1.0 + (doc.keyword_count as f64).max(1.0).ln() * 0.08;
            score /= size_penalty;

            if score > 0.0 {
                scores.insert(doc.name.clone(), score);
            }
        }

        scores
    }

    pub fn best(&self, scores: &HashMap<String, f64>) -> Option<String> {
        best_subject(scores)
    }
}

pub fn classify_subject_opts(
    path: &Path,
    subjects: &[SubjectDef],
    allow_extract: bool,
) -> String {
    let index = SubjectIndex::build(subjects);
    classify_with_index(path, &index, subjects, allow_extract)
}

pub fn classify_with_index(
    path: &Path,
    index: &SubjectIndex,
    subjects: &[SubjectDef],
    allow_extract: bool,
) -> String {
    let raw_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    let stem = Path::new(&raw_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(&raw_name);

    let mut scores = index.score_text(stem);

    if let Some(subject) = decide(index, subjects, stem, &scores) {
        return subject;
    }

    // Filename ambiguous — only then pay for document text extraction.
    if allow_extract {
        if let Some(text) = extract::extract_text(path) {
            let sample: String = text.chars().take(4000).collect();
            let doc_scores = index.score_text(&sample);
            for (name, pts) in doc_scores {
                *scores.entry(name).or_insert(0.0) += pts * 0.85;
            }
            let query = format!("{stem} {}", sample.chars().take(400).collect::<String>());
            if let Some(subject) = decide(index, subjects, &query, &scores) {
                return subject;
            }
        }
    }

    "General".to_string()
}

/// Clear BM25 win → take it. Thin margin / miss → try MiniLM hybrid.
fn decide(
    index: &SubjectIndex,
    subjects: &[SubjectDef],
    query: &str,
    scores: &HashMap<String, f64>,
) -> Option<String> {
    let mut ranked: Vec<_> = scores.iter().collect();
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(b.0))
    });

    let clear_margin = MIN_MARGIN * 2.2;
    if let Some((top_name, top_score)) = ranked.first() {
        let second = ranked.get(1).map(|(_, s)| **s).unwrap_or(0.0);
        if **top_score >= MIN_SCORE && **top_score - second >= clear_margin {
            return Some((*top_name).clone());
        }
        // Lexical winner exists but is close / weak — fuse with embeddings.
        if let Some(fused) = crate::classify_embed::fuse_with_bm25(query, scores, subjects) {
            return Some(fused);
        }
        // Fall back to plain BM25 best if it still clears the normal bar.
        if let Some(subject) = index.best(scores) {
            return Some(subject);
        }
    } else if let Some(fused) = crate::classify_embed::fuse_with_bm25(query, scores, subjects) {
        return Some(fused);
    }
    None
}

/// Enrich keywords for matching while keeping every user-entered phrase.
///
/// Order:
/// 1. User phrases exactly as typed (trim + dedupe)
/// 2. Extra needles derived from the subject name + those phrases
///    (tokens, bigrams, parenthetical codes) — never replace (1)
pub fn enrich_subject(subject: &mut SubjectDef) {
    let mut seen: HashSet<String> = HashSet::new();
    let mut user_phrases = Vec::new();

    for kw in subject.keywords.drain(..) {
        let k = kw.trim().to_string();
        if k.is_empty() {
            continue;
        }
        let key = k.to_ascii_lowercase();
        if seen.insert(key) {
            user_phrases.push(k);
        }
    }

    let mut derived: HashSet<String> = HashSet::new();
    for piece in needles_from_text(&subject.name) {
        derived.insert(piece);
    }
    for phrase in &user_phrases {
        for piece in needles_from_text(phrase) {
            derived.insert(piece);
        }
    }

    // User-supplied short codes (e.g. "EM", "OC") are kept via user_phrases.
    // Auto-derived 1–2 letter fragments are noise and dropped.
    let mut extras: Vec<String> = derived
        .into_iter()
        .filter(|k| {
            if is_stop(k) {
                return false;
            }
            let key = k.to_ascii_lowercase();
            if seen.contains(&key) {
                return false;
            }
            // Keep multi-word / longer tokens; allow 3+ char codes from expansion.
            if k.contains(' ') {
                return true;
            }
            k.len() >= 3 || looks_like_code(k)
        })
        .collect();
    extras.sort_by_key(|k| (k.len() > 48, k.len()));

    let mut out = user_phrases;
    for extra in extras {
        let key = extra.to_ascii_lowercase();
        if seen.insert(key) {
            out.push(extra);
        }
    }

    // Generous cap so large pasted lists aren't truncated away.
    if out.len() > 800 {
        out.truncate(800);
    }
    subject.keywords = out;
}

pub fn enrich_all(subjects: &mut [SubjectDef]) {
    for subject in subjects.iter_mut() {
        if subject.name.eq_ignore_ascii_case("General") {
            continue;
        }
        enrich_subject(subject);
    }
}

fn best_subject(scores: &HashMap<String, f64>) -> Option<String> {
    let mut ranked: Vec<_> = scores.iter().collect();
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(b.0))
    });

    let (top_name, top_score) = ranked.first()?;
    if **top_score < MIN_SCORE {
        return None;
    }
    let second = ranked.get(1).map(|(_, s)| **s).unwrap_or(0.0);
    if **top_score - second < MIN_MARGIN {
        if **top_score >= MIN_SCORE * 2.2 && **top_score > second {
            return Some((*top_name).clone());
        }
        return None;
    }
    Some((*top_name).clone())
}

fn best_fuzzy<'a>(query: &str, terms: &'a [String]) -> Option<(&'a str, f64)> {
    if query.len() < 5 {
        return None;
    }
    let mut best: Option<(&str, f64)> = None;
    for term in terms {
        if term.len() < 5 {
            continue;
        }
        // Cheap prefix gate before Jaccard.
        let shared = query
            .chars()
            .zip(term.chars())
            .take_while(|(a, b)| a == b)
            .count();
        if shared < 3 && (query.len() as i32 - term.len() as i32).abs() > 4 {
            continue;
        }
        let sim = trigram_jaccard(query, term);
        if sim >= FUZZY_JACCARD {
            if best.map(|(_, s)| sim > s).unwrap_or(true) {
                best = Some((term.as_str(), sim));
            }
        }
    }
    best
}

fn trigram_jaccard(a: &str, b: &str) -> f64 {
    let ta = char_ngrams(a, 3);
    let tb = char_ngrams(b, 3);
    if ta.is_empty() || tb.is_empty() {
        return 0.0;
    }
    let inter = ta.intersection(&tb).count() as f64;
    let union = ta.union(&tb).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

fn char_ngrams(s: &str, n: usize) -> HashSet<String> {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() < n {
        return HashSet::from([s.to_string()]);
    }
    let mut set = HashSet::new();
    for i in 0..=chars.len() - n {
        set.insert(chars[i..i + n].iter().collect());
    }
    set
}

/// Build match needles only from text the user (or subject name) already contains.
fn needles_from_text(raw: &str) -> Vec<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let norm = normalize(raw);
    if !norm.is_empty() {
        let words = norm.split_whitespace().count();
        if words <= 6 && norm.len() <= 60 {
            out.push(norm.clone());
        }
    }

    for part in extract_parens(raw) {
        let p = normalize(&part);
        if !p.is_empty() {
            out.push(p);
        }
    }

    let tokens: Vec<String> = tokenize(&norm)
        .into_iter()
        .filter(|t| !is_stop(t))
        .collect();

    for token in &tokens {
        if token.len() >= 2 {
            out.push(token.clone());
        }
    }

    for window in tokens.windows(2) {
        out.push(format!("{} {}", window[0], window[1]));
    }
    for window in tokens.windows(3) {
        out.push(format!("{} {} {}", window[0], window[1], window[2]));
    }

    if tokens.len() >= 2 {
        let ac: String = tokens.iter().filter_map(|t| t.chars().next()).collect();
        if (2..=8).contains(&ac.len()) {
            out.push(ac);
        }
    }

    out
}

fn tokenize_query(haystack: &str, dict: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for rough in split_filename_pieces(haystack) {
        if rough.is_empty() {
            continue;
        }
        // Try to split concatenated words using the user's keyword dictionary.
        let parts = longest_match_split(&rough, dict);
        if parts.len() > 1 {
            out.extend(parts);
        } else {
            out.push(rough);
        }
    }
    out.into_iter().filter(|t| !t.is_empty() && !is_stop(t)).collect()
}

/// Split on separators, camelCase, and letter↔digit boundaries.
fn split_filename_pieces(s: &str) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = s.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_ascii_alphanumeric() {
            if !cur.is_empty() {
                pieces.push(std::mem::take(&mut cur).to_ascii_lowercase());
            }
            continue;
        }
        if !cur.is_empty() {
            let prev = cur.chars().last().unwrap();
            let next = chars.get(i + 1).copied();
            let camel = prev.is_ascii_lowercase() && c.is_ascii_uppercase();
            let acronym_end = prev.is_ascii_uppercase()
                && c.is_ascii_uppercase()
                && next.is_some_and(|n| n.is_ascii_lowercase());
            let alnum = (prev.is_ascii_alphabetic() && c.is_ascii_digit())
                || (prev.is_ascii_digit() && c.is_ascii_alphabetic());
            if camel || acronym_end || alnum {
                pieces.push(std::mem::take(&mut cur).to_ascii_lowercase());
            }
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        pieces.push(cur.to_ascii_lowercase());
    }
    pieces
}

fn longest_match_split(token: &str, dict: &[String]) -> Vec<String> {
    if token.len() < 6 || dict.is_empty() {
        return vec![token.to_string()];
    }
    let mut i = 0;
    let bytes = token.as_bytes();
    let mut parts = Vec::new();
    while i < bytes.len() {
        let mut matched = None;
        for term in dict {
            let tb = term.as_bytes();
            if i + tb.len() <= bytes.len() && &bytes[i..i + tb.len()] == tb {
                // Prefer leaving a remainder that is empty or also dict-friendly.
                matched = Some(tb.len());
                break; // dict is longest-first
            }
        }
        if let Some(len) = matched {
            parts.push(token[i..i + len].to_string());
            i += len;
        } else {
            // Consume one char and continue; if nothing matched overall, bail.
            if parts.is_empty() {
                return vec![token.to_string()];
            }
            // Stick remaining as one piece.
            parts.push(token[i..].to_string());
            break;
        }
    }
    if parts.len() == 1 {
        vec![token.to_string()]
    } else {
        parts
    }
}

fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(' ');
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn tokenize(s: &str) -> Vec<String> {
    s.split_whitespace()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

fn is_stop(token: &str) -> bool {
    STOP_WORDS.iter().any(|s| *s == token)
}

fn looks_like_code(token: &str) -> bool {
    if token.len() < 2 || token.len() > 12 {
        return false;
    }
    let has_alpha = token.chars().any(|c| c.is_ascii_alphabetic());
    let has_digit = token.chars().any(|c| c.is_ascii_digit());
    has_alpha && has_digit
}

fn extract_parens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find('(') {
        let after = &rest[start + 1..];
        if let Some(end) = after.find(')') {
            let inner = after[..end].trim();
            if !inner.is_empty() && inner.len() <= 16 {
                out.push(inner.to_string());
            }
            rest = &after[end + 1..];
        } else {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subjects::SubjectDef;
    use std::path::PathBuf;

    fn user_subjects() -> Vec<SubjectDef> {
        let mut subjects = vec![
            SubjectDef {
                name: "Electrical Circuit Analysis (ECA)".into(),
                keywords: vec!["network theorems".into(), "kirchhoff".into()],
                enabled: true,
            },
            SubjectDef {
                name: "Digital System Design (DSD)".into(),
                keywords: vec!["digital circuits".into(), "mealy".into()],
                enabled: true,
            },
            SubjectDef {
                name: "Organic Chemistry".into(),
                keywords: vec!["alkanes".into(), "NMR".into(), "spectroscopy".into()],
                enabled: true,
            },
            SubjectDef {
                name: "General".into(),
                keywords: vec![],
                enabled: true,
            },
        ];
        enrich_all(&mut subjects);
        subjects
    }

    #[test]
    fn bm25_matches_user_name_and_keywords() {
        let subjects = user_subjects();
        assert_eq!(
            classify_subject_opts(&PathBuf::from("ECA_network_theorems.pdf"), &subjects, false),
            "Electrical Circuit Analysis (ECA)"
        );
        assert_eq!(
            classify_subject_opts(&PathBuf::from("dsd labmanual.pdf"), &subjects, false),
            "Digital System Design (DSD)"
        );
        assert_eq!(
            classify_subject_opts(&PathBuf::from("NMR_lab_report.pdf"), &subjects, false),
            "Organic Chemistry"
        );
        assert_eq!(
            classify_subject_opts(&PathBuf::from("vacation.zip"), &subjects, false),
            "General"
        );
    }

    #[test]
    fn fuzzy_and_concatenated_tokens() {
        let subjects = user_subjects();
        // Soft match spectroscopy ≈ spectroscop (typo / stem)
        assert_eq!(
            classify_subject_opts(
                &PathBuf::from("organic_spectroscopy_notes.pdf"),
                &subjects,
                false
            ),
            "Organic Chemistry"
        );
        // Dictionary split: "networktheorems" using user keywords
        assert_eq!(
            classify_subject_opts(&PathBuf::from("networktheorems.pdf"), &subjects, false),
            "Electrical Circuit Analysis (ECA)"
        );
    }

    #[test]
    fn enrich_preserves_user_phrases_and_adds_needles() {
        let long = "electrical machinery lecture notes, three phase transformer, induction motor";
        let mut subjects = vec![SubjectDef {
            name: "Electrical Machinery".into(),
            keywords: long
                .split(',')
                .map(|s| s.trim().to_string())
                .collect(),
            enabled: true,
        }];
        enrich_all(&mut subjects);
        let kws = &subjects[0].keywords;
        // User phrases kept at the front
        assert_eq!(&kws[0], "electrical machinery lecture notes");
        assert_eq!(&kws[1], "three phase transformer");
        assert_eq!(&kws[2], "induction motor");
        // Derived needles added (e.g. tokens from those phrases)
        assert!(kws.iter().any(|k| k == "transformer" || k == "induction"));
        assert!(kws.len() > 3);
        // Auto-derived 2-letter noise must not appear
        assert!(!kws.iter().any(|k| k.len() <= 2));
    }

    #[test]
    fn index_reused_matches_oneshot() {
        let subjects = user_subjects();
        let index = SubjectIndex::build(&subjects);
        let path = PathBuf::from("Correcting-a-Mealy-Machine.pdf");
        assert_eq!(
            classify_with_index(&path, &index, &subjects, false),
            classify_subject_opts(&path, &subjects, false)
        );
    }

    #[test]
    fn huge_keyword_list_does_not_steal_everything() {
        let mut flooded: Vec<String> = (0..250)
            .map(|i| format!("electrical machinery topic number {i} notes lab tutorial"))
            .collect();
        flooded.push("three phase induction motor torque slip".into());
        flooded.push("transformer open circuit test".into());

        let mut subjects = vec![
            SubjectDef {
                name: "Electrical Machinery".into(),
                keywords: flooded,
                enabled: true,
            },
            SubjectDef {
                name: "Digital System Design (DSD)".into(),
                keywords: vec!["digital circuits".into(), "mealy".into(), "dsd".into()],
                enabled: true,
            },
            SubjectDef {
                name: "General".into(),
                keywords: vec![],
                enabled: true,
            },
        ];
        enrich_all(&mut subjects);

        // Clear DSD signal still wins
        assert_eq!(
            classify_subject_opts(&PathBuf::from("dsd_mealy_labmanual.pdf"), &subjects, false),
            "Digital System Design (DSD)"
        );
        // Generic lab notes alone → General (not vacuumed into the fat subject)
        assert_eq!(
            classify_subject_opts(&PathBuf::from("random_lab_notes.pdf"), &subjects, false),
            "General"
        );
        // Real machinery signal still matches
        assert_eq!(
            classify_subject_opts(
                &PathBuf::from("Three_Phase_Induction_Motor_Torque_Slip.pdf"),
                &subjects,
                false
            ),
            "Electrical Machinery"
        );
    }
}

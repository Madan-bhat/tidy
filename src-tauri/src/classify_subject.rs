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

/// Only language function words and file-format names. Nothing domain specific:
/// whether "report", "lab" or "invoice" matters is decided by each user's own
/// subjects/keywords (BM25 IDF + per-subject rarity), never by a built-in list.
const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "or", "the", "of", "to", "in", "on", "for", "with", "using",
    "from", "into", "by", "as", "is", "are", "be", "this", "that", "these", "those",
    "including", "include", "other", "others", "such", "their", "its", "at", "via",
    "also", "more", "most", "than", "then", "over", "under", "about",
    // File-format words say nothing about the subject.
    "pdf", "docx", "pptx", "xlsx", "ppt", "doc", "txt", "csv", "jpg", "jpeg", "png",
];

/// Precomputed BM25 index over the user's subjects — build once per scan/batch.
#[derive(Debug, Clone)]
pub struct SubjectIndex {
    docs: Vec<SubjectDoc>,
    idf: HashMap<String, f64>,
    /// Number of subjects whose keyword bank contains the term.
    df: HashMap<String, u32>,
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
                if term.is_empty() || is_stop(term) || is_numeric(term) {
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
        for (term, count) in &df {
            let nn = *count as f64;
            // Normalised so scores don't depend on how many subjects the user has:
            // a term owned by one subject is always strong (1.5), terms shared by
            // every subject fade toward zero.
            let value = 1.5 * ((n - nn + 1.0) / n.max(1.0)).max(0.0).powf(1.5);
            idf.insert(term.clone(), value);
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
            df,
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
            let mut unique_hits = 0u32;

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
                    // A real word that only ONE subject owns is decisive evidence.
                    if rarity >= 0.6
                        && term_key.len() >= 5
                        && self.df.get(term_key).copied().unwrap_or(0) == 1
                        && self.docs.len() > 1
                    {
                        unique_hits += 1;
                    }
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
                // Phrases built only from list-generic words must not carry a match.
                if !phrase
                    .split_whitespace()
                    .any(|w| doc.rarity.get(w).copied().unwrap_or(0.35) >= 0.45)
                {
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
                    if rarity < 0.45 || (ac.len() <= 2 && rarity < 0.5) {
                        score += 0.4;
                    } else {
                        score += if ac.len() <= 2 { 1.2 } else { 2.6 };
                        acronym_hits += 1;
                    }
                }
            }

            // Evidence gate: a fat keyword list must not win on one generic token.
            let strong =
                phrase_hits >= 1 || acronym_hits >= 1 || distinctive_hits >= 2 || unique_hits >= 1;
            score += 0.9 * unique_hits.min(2) as f64;
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
    classify_with_index(path, &index, subjects, allow_extract, true)
}

pub fn classify_with_index(
    path: &Path,
    index: &SubjectIndex,
    subjects: &[SubjectDef],
    allow_extract: bool,
    allow_embed: bool,
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

    if let Some(subject) = decide(index, subjects, stem, &scores, allow_embed) {
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
            if let Some(subject) = decide(index, subjects, &query, &scores, allow_embed) {
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
    allow_embed: bool,
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
        if allow_embed {
            if let Some(fused) = crate::classify_embed::fuse_with_bm25(query, scores, subjects) {
                return Some(fused);
            }
        }
        // Fall back to plain BM25 best if it still clears the normal bar.
        if let Some(subject) = index.best(scores) {
            return Some(subject);
        }
    } else if allow_embed {
        if let Some(fused) = crate::classify_embed::fuse_with_bm25(query, scores, subjects) {
            return Some(fused);
        }
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
    let cleaned_name = clean_subject_name(&subject.name);
    if !cleaned_name.is_empty() {
        subject.name = cleaned_name;
    }
    let old = std::mem::take(&mut subject.keywords);
    subject.keywords = clean_keywords(&subject.name, old);

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
        .filter(|t| !is_stop(t) && !is_numeric(t))
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
    out.into_iter()
        .filter(|t| !t.is_empty() && !is_stop(t) && !is_numeric(t))
        .collect()
}

fn is_numeric(t: &str) -> bool {
    !t.is_empty() && t.chars().all(|c| c.is_ascii_digit())
}

/// Strip list numbering / markdown noise from a subject name:
/// `1\. Circuits (ECA)`, `**2)** Machines`, `3 - Digital`, `## 4. Micro` → clean title.
pub fn clean_subject_name(raw: &str) -> String {
    let mut s = raw.trim();
    loop {
        let before = s;
        s = s.trim_start_matches(|c: char| matches!(c, '#' | '*' | '_' | '>' | '-' | '\\' | '•' | '`') || c.is_whitespace());
        // leading "12" + one of . ) ] : - (optionally backslash-escaped)
        let digits = s.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 && digits <= 3 {
            let rest = &s[digits..];
            let rest_trim = rest.trim_start_matches('\\');
            if let Some(c) = rest_trim.chars().next() {
                let after = &rest_trim[c.len_utf8()..];
                if matches!(c, '.' | ')' | ']' | ':' | '-')
                    && after.chars().next().map_or(true, |n| n.is_whitespace() || n == '*' || n == '_')
                {
                    s = after;
                }
            }
        }
        if s == before {
            break;
        }
    }
    let s = s.trim().trim_matches(|c: char| matches!(c, '*' | '_' | '`' | '\\')).trim();
    s.replace("\\", "")
}

/// Remove junk that earlier versions auto-generated (digit-led fragments such as
/// `1ecae`, numbering tokens, escaped list prefixes) without touching real user phrases.
fn clean_keywords(name: &str, keywords: Vec<String>) -> Vec<String> {
    let name_norm = normalize(name);
    let name_tokens: HashSet<&str> = name_norm.split_whitespace().collect();

    // Initials strings of long phrases: auto-generated fragments are slices of these.
    let mut initials: Vec<String> = Vec::new();
    let all_norm: Vec<String> = keywords.iter().map(|k| normalize(k)).collect();
    for n in std::iter::once(&name_norm).chain(all_norm.iter()) {
        let words: Vec<&str> = n.split_whitespace().collect();
        if words.len() >= 3 {
            initials.push(words.iter().filter_map(|w| w.chars().next()).collect());
            initials.push(
                words
                    .iter()
                    .filter(|w| !is_stop(w))
                    .filter_map(|w| w.chars().next())
                    .collect(),
            );
        }
    }
    let phrase_tokens: HashSet<String> = all_norm
        .iter()
        .filter(|n| n.contains(' '))
        .flat_map(|n| n.split_whitespace().map(|t| t.to_string()).collect::<Vec<_>>())
        .collect();

    let mut out = Vec::new();
    for kw in keywords {
        let k = clean_subject_name(&kw);
        let k = k.trim().to_string();
        if k.is_empty() || is_numeric(&k) {
            continue;
        }
        let n = normalize(&k);
        if n.is_empty() || is_numeric(&n) {
            continue;
        }
        // "1 electrical circuit" — numbering glued onto a phrase that exists without it.
        if let Some((first, rest)) = n.split_once(' ') {
            if is_numeric(first) && first.len() <= 3 {
                let in_name = name_norm.contains(rest);
                let elsewhere = all_norm.iter().any(|o| o == rest);
                if in_name || elsewhere {
                    continue;
                }
            }
        }
        if !k.contains(' ') && n.len() <= 9 {
            let digit_led = n.chars().next().is_some_and(|c| c.is_ascii_digit())
                && n.chars().any(|c| c.is_ascii_alphabetic());
            if digit_led {
                continue;
            }
            // Short alpha fragment that is only a slice of a phrase's initials.
            if n.len() <= 4
                && n.chars().all(|c| c.is_ascii_alphabetic())
                && !name_tokens.contains(n.as_str())
                && !phrase_tokens.contains(&n)
                && initials.iter().any(|ini| ini.len() > n.len() && ini.contains(&n))
            {
                continue;
            }
        }
        out.push(k);
    }
    out
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

    // Fixtures are invented and deliberately unrelated to any real user's data:
    // the engine has no built-in subjects, so tests supply their own.
    fn sample_subjects() -> Vec<SubjectDef> {
        let mut subjects = vec![
            SubjectDef {
                name: "Home Cooking (HC)".into(),
                keywords: vec!["pasta sauce".into(), "sourdough".into()],
                enabled: true,
            },
            SubjectDef {
                name: "Personal Finance (PF)".into(),
                keywords: vec!["tax return".into(), "deductions".into(), "invoice".into()],
                enabled: true,
            },
            SubjectDef {
                name: "Travel Planning".into(),
                keywords: vec!["itinerary".into(), "boarding pass".into(), "hotel booking".into()],
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

    fn classify(name: &str, subjects: &[SubjectDef]) -> String {
        classify_subject_opts(&PathBuf::from(name), subjects, false)
    }

    #[test]
    fn bm25_matches_name_and_keywords() {
        let s = sample_subjects();
        assert_eq!(classify("HC_pasta_sauce.pdf", &s), "Home Cooking (HC)");
        assert_eq!(classify("pf tax return 2025.pdf", &s), "Personal Finance (PF)");
        assert_eq!(classify("Tokyo itinerary.docx", &s), "Travel Planning");
        assert_eq!(classify("holiday.zip", &s), "General");
    }

    #[test]
    fn fuzzy_and_concatenated_tokens() {
        let s = sample_subjects();
        // Dictionary split of a concatenated name using the user's own keywords
        assert_eq!(classify("hotelbooking.pdf", &s), "Travel Planning");
        assert_eq!(classify("sourdough_starter_notes.pdf", &s), "Home Cooking (HC)");
    }

    #[test]
    fn generic_words_are_not_hardcoded_as_noise() {
        // "report" is meaningful if a user says so — no built-in list may ignore it.
        let mut s = vec![
            SubjectDef {
                name: "Quarterly Reports".into(),
                keywords: vec!["report".into(), "quarterly".into()],
                enabled: true,
            },
            SubjectDef {
                name: "Photos".into(),
                keywords: vec!["camera".into(), "raw".into()],
                enabled: true,
            },
        ];
        enrich_all(&mut s);
        assert_eq!(classify("q3 report final.pdf", &s), "Quarterly Reports");
    }

    #[test]
    fn enrich_preserves_user_phrases_and_adds_needles() {
        let long = "weekly meal prep plan, slow cooker stew, bread baking";
        let mut subjects = vec![SubjectDef {
            name: "Kitchen".into(),
            keywords: long.split(',').map(|s| s.trim().to_string()).collect(),
            enabled: true,
        }];
        enrich_all(&mut subjects);
        let kws = &subjects[0].keywords;
        assert_eq!(&kws[0], "weekly meal prep plan");
        assert_eq!(&kws[1], "slow cooker stew");
        assert_eq!(&kws[2], "bread baking");
        assert!(kws.iter().any(|k| k == "stew" || k == "bread"));
        assert!(kws.len() > 3);
        assert!(!kws.iter().any(|k| k.len() <= 2));
    }

    #[test]
    fn index_reused_matches_oneshot() {
        let s = sample_subjects();
        let index = SubjectIndex::build(&s);
        let path = PathBuf::from("Hotel-Booking-Confirmation.pdf");
        assert_eq!(
            classify_with_index(&path, &index, &s, false, false),
            classify_subject_opts(&path, &s, false)
        );
    }

    #[test]
    fn cleans_list_numbering_and_junk_fragments() {
        assert_eq!(clean_subject_name("1\\. Home Cooking (HC)"), "Home Cooking (HC)");
        assert_eq!(clean_subject_name("**2)** Travel"), "Travel");
        assert_eq!(clean_subject_name("3D Printing"), "3D Printing");
        assert_eq!(clean_subject_name("2-pole motors"), "2-pole motors");
        let mut s = SubjectDef {
            name: "1\\. Home Cooking (HC)".into(),
            keywords: vec![
                "1\\. home cooking (hc)".into(),
                "Quick weeknight pasta dinners".into(),
                "1 home".into(),
                "1hcc".into(),
                "qwpd".into(),
                "wpd".into(),
                "hc".into(),
                "BBQ".into(),
            ],
            enabled: true,
        };
        enrich_subject(&mut s);
        assert_eq!(s.name, "Home Cooking (HC)");
        for bad in ["1 home", "1hcc", "wpd", "1 home cooking hc"] {
            assert!(!s.keywords.iter().any(|k| k == bad), "{bad} survived: {:?}", s.keywords);
        }
        assert!(s.keywords.iter().any(|k| k == "BBQ"));
        assert!(s.keywords.iter().any(|k| k == "hc"));
    }

    #[test]
    fn huge_keyword_list_does_not_steal_everything() {
        let mut flooded: Vec<String> = (0..250)
            .map(|i| format!("kitchen recipe number {i} notes tips guide"))
            .collect();
        flooded.push("slow braised short ribs".into());
        flooded.push("sourdough starter feeding".into());

        let mut subjects = vec![
            SubjectDef {
                name: "Kitchen".into(),
                keywords: flooded,
                enabled: true,
            },
            SubjectDef {
                name: "Personal Finance (PF)".into(),
                keywords: vec!["tax return".into(), "deductions".into(), "pf".into()],
                enabled: true,
            },
            SubjectDef {
                name: "General".into(),
                keywords: vec![],
                enabled: true,
            },
        ];
        enrich_all(&mut subjects);

        assert_eq!(classify("pf_tax_return_summary.pdf", &subjects), "Personal Finance (PF)");
        assert_eq!(classify("random_tips_guide.pdf", &subjects), "General");
        assert_eq!(classify("Slow_Braised_Short_Ribs.pdf", &subjects), "Kitchen");
    }
}

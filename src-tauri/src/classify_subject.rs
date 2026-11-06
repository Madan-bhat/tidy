use crate::extract;
use crate::subjects::SubjectDef;
use std::collections::{HashMap, HashSet};
use std::path::Path;

const MIN_SCORE: i32 = 2;
const MIN_MARGIN: i32 = 1;

/// Plain English filler only — never course / domain terms.
const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "or", "the", "of", "to", "in", "on", "for", "with", "using",
    "from", "into", "by", "as", "is", "are", "be", "this", "that", "these", "those",
    "including", "include", "other", "others", "such", "their", "its", "at", "via",
    "also", "more", "most", "than", "then", "over", "under", "about",
];

pub fn classify_subject_opts(
    path: &Path,
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
    let haystack = normalize(stem);

    let mut scores = score_haystack(&haystack, subjects);

    if let Some(subject) = best_subject(&scores) {
        return subject;
    }

    if allow_extract {
        if let Some(text) = extract::extract_text(path) {
            let sample: String = text.chars().take(5000).collect();
            let extracted = normalize(&sample);
            let doc_scores = score_haystack(&extracted, subjects);
            for (name, pts) in doc_scores {
                *scores.entry(name).or_insert(0) += pts;
            }
            if let Some(subject) = best_subject(&scores) {
                return subject;
            }
        }
    }

    "General".to_string()
}

/// Split the subject's own name + user keywords into matchable needles.
/// Does not invent domain vocabulary.
pub fn enrich_subject(subject: &mut SubjectDef) {
    let mut set: HashSet<String> = HashSet::new();
    for piece in needles_from_text(&subject.name) {
        set.insert(piece);
    }
    for kw in &subject.keywords {
        for piece in needles_from_text(kw) {
            set.insert(piece);
        }
    }

    let mut keywords: Vec<String> = set.into_iter().filter(|k| !is_stop(k)).collect();
    keywords.sort();
    keywords.sort_by_key(|k| (k.len() > 40, k.len()));
    if keywords.len() > 100 {
        keywords.truncate(100);
    }
    subject.keywords = keywords;
}

pub fn enrich_all(subjects: &mut [SubjectDef]) {
    for subject in subjects.iter_mut() {
        if subject.name.eq_ignore_ascii_case("General") {
            continue;
        }
        enrich_subject(subject);
    }
}

fn score_haystack(haystack: &str, subjects: &[SubjectDef]) -> HashMap<String, i32> {
    let file_tokens: HashSet<String> = tokenize(haystack).into_iter().collect();
    let mut scores: HashMap<String, i32> = HashMap::new();

    for subject in subjects {
        if !subject.enabled || subject.name.eq_ignore_ascii_case("General") {
            continue;
        }

        let mut needles: HashSet<String> = HashSet::new();
        for piece in needles_from_text(&subject.name) {
            needles.insert(piece);
        }
        for kw in &subject.keywords {
            for piece in needles_from_text(kw) {
                needles.insert(piece);
            }
        }

        let mut score = 0i32;

        for needle in &needles {
            if needle.is_empty() || is_stop(needle) {
                continue;
            }

            if needle.contains(' ') {
                if haystack.contains(needle.as_str()) {
                    score += 4 + needle.split_whitespace().count() as i32;
                }
                continue;
            }

            let short = needle.len() <= 5 && needle.chars().all(|c| c.is_ascii_alphanumeric());
            if short {
                if file_tokens.contains(needle) {
                    // Prefer codes/acronyms that appear as whole tokens in the filename.
                    score += if needle.len() <= 2 { 3 } else { 6 };
                }
                continue;
            }

            if needle.len() >= 4
                && (file_tokens.contains(needle) || haystack.contains(needle.as_str()))
            {
                score += if needle.len() >= 8 { 3 } else { 2 };
            }
        }

        if score > 0 {
            scores.insert(subject.name.clone(), score);
        }
    }

    scores
}

fn best_subject(scores: &HashMap<String, i32>) -> Option<String> {
    let mut ranked: Vec<_> = scores.iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));

    let (top_name, top_score) = ranked.first()?;
    if **top_score < MIN_SCORE {
        return None;
    }
    let second = ranked.get(1).map(|(_, s)| **s).unwrap_or(0);
    if **top_score - second < MIN_MARGIN {
        if **top_score >= MIN_SCORE * 3 && **top_score > second {
            return Some((*top_name).clone());
        }
        return None;
    }
    Some((*top_name).clone())
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

    // Initials from multi-word titles: "Digital System Design" → dsd
    if tokens.len() >= 2 {
        let ac: String = tokens.iter().filter_map(|t| t.chars().next()).collect();
        if (2..=8).contains(&ac.len()) {
            out.push(ac);
        }
    }

    out
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
                keywords: vec!["digital circuits".into()],
                enabled: true,
            },
            SubjectDef {
                name: "Organic Chemistry".into(),
                keywords: vec!["alkanes".into(), "NMR".into()],
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
    fn matches_using_only_user_name_and_keywords() {
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
    fn enrich_does_not_invent_vocabulary() {
        let mut subjects = vec![SubjectDef {
            name: "Basket Weaving".into(),
            keywords: vec!["reed coil patterns".into()],
            enabled: true,
        }];
        enrich_all(&mut subjects);
        let joined = subjects[0].keywords.join(" ");
        assert!(joined.contains("basket") || joined.contains("weaving"));
        assert!(joined.contains("reed"));
        assert!(!joined.contains("kirchhoff"));
        assert!(!joined.contains("microcontroller"));
        assert!(!joined.contains("fourier"));
    }
}

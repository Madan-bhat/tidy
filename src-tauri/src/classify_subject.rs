use crate::extract;
use crate::subjects::SubjectDef;
use std::collections::HashMap;
use std::path::Path;

/// Minimum score and margin required to avoid falling back to General.
const MIN_SCORE: i32 = 1;
const MIN_MARGIN: i32 = 1;

pub fn classify_subject_opts(
    path: &Path,
    subjects: &[SubjectDef],
    allow_extract: bool,
) -> String {
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let mut scores = score_text(&filename, subjects);

    if let Some(subject) = best_subject(&scores) {
        return subject;
    }

    if allow_extract {
        if let Some(text) = extract::extract_text(path) {
            let sample = text.chars().take(4000).collect::<String>().to_ascii_lowercase();
            scores = score_text(&sample, subjects);
            if let Some(subject) = best_subject(&scores) {
                return subject;
            }
        }
    }

    "General".to_string()
}

fn score_text(text: &str, subjects: &[SubjectDef]) -> HashMap<String, i32> {
    let mut scores: HashMap<String, i32> = HashMap::new();
    for subject in subjects {
        if !subject.enabled || subject.name.eq_ignore_ascii_case("General") {
            continue;
        }
        let mut score = 0;
        for kw in &subject.keywords {
            let needle = kw.trim().to_ascii_lowercase();
            if needle.is_empty() {
                continue;
            }
            if text.contains(&needle) {
                score += if needle.contains(' ') { 2 } else { 1 };
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
        return None;
    }
    Some((*top_name).clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subjects;
    use std::path::PathBuf;

    #[test]
    fn classifies_electrical_preset_from_filename() {
        let subjects = subjects::btech_electrical_preset();
        assert_eq!(
            classify_subject_opts(
                &PathBuf::from("Power Systems Load Flow.pdf"),
                &subjects,
                false
            ),
            "Power Systems"
        );
        assert_eq!(
            classify_subject_opts(
                &PathBuf::from("PID Control Systems Notes.pdf"),
                &subjects,
                false
            ),
            "Control Systems"
        );
        assert_eq!(
            classify_subject_opts(&PathBuf::from("random-photo.jpg"), &subjects, false),
            "General"
        );
    }
}

//! Group files that share a name family so a strong match can help weak siblings.
//!
//! Example: `L4.pdf` and `L5.pdf`, or `Notes 8.pdf` / `Notes 9.pdf`, or
//! `report-final.pdf` / `report-final(1).pdf`. Families are derived only from
//! the filename — never from domain knowledge.

use crate::mover::PlannedMove;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Strip copy suffixes, trailing numbers, and date-ish tails to get a family key.
pub fn family_key(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let mut s = stem;
    // Drop common "copy" / duplicate markers: " (1)", "-1", "_copy", " copy"
    for _ in 0..3 {
        let before = s.clone();
        if let Some(rest) = s.strip_suffix(" copy") {
            s = rest.trim().to_string();
        }
        if let Some(rest) = s.strip_suffix("_copy") {
            s = rest.to_string();
        }
        // trailing " (12)" or "(12)"
        if let Some(open) = s.rfind('(') {
            if s.ends_with(')') {
                let inside = &s[open + 1..s.len() - 1];
                if !inside.is_empty() && inside.chars().all(|c| c.is_ascii_digit()) {
                    s = s[..open].trim().to_string();
                }
            }
        }
        // trailing -1 / _2 /  3  (short numeric suffix after separator)
        if let Some(idx) = s.rfind(|c: char| c == '-' || c == '_' || c == ' ') {
            let tail = &s[idx + 1..];
            if (1..=3).contains(&tail.len()) && tail.chars().all(|c| c.is_ascii_digit()) {
                s = s[..idx].trim().to_string();
            }
        }
        // trailing digits glued on ("notes8", "l4") — keep the letter stem
        let trimmed = s.trim_end_matches(|c: char| c.is_ascii_digit());
        if trimmed.len() < s.len()
            && !trimmed.is_empty()
            && trimmed.chars().any(|c| c.is_ascii_alphabetic())
        {
            s = trimmed
                .trim_end_matches(|c: char| c == '-' || c == '_' || c == ' ')
                .to_string();
        }
        if s == before {
            break;
        }
    }

    // Collapse separators
    s.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// For each family, if one member has a clear (high/medium) subject and another
/// is General/low, copy the strong subject across.
pub fn propagate(
    items: &mut [(PathBuf, Option<PlannedMove>)],
    replan: &dyn Fn(&Path, &str) -> Option<PlannedMove>,
) -> usize {
    let mut families: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, (path, _)) in items.iter().enumerate() {
        let key = family_key(path);
        if key.is_empty() {
            continue;
        }
        // Pure digit families aren't useful.
        if key.chars().all(|c| c.is_ascii_digit() || c == ' ') {
            continue;
        }
        families.entry(key).or_default().push(i);
    }

    let mut changed = 0usize;
    for (_key, idxs) in families {
        if idxs.len() < 2 {
            continue;
        }
        // Pick the strongest non-General member as the donor.
        let donor = idxs.iter().find_map(|&i| {
            let plan = items[i].1.as_ref()?;
            if plan.subject.eq_ignore_ascii_case("General") {
                return None;
            }
            match plan.confidence.as_str() {
                "high" | "medium" => Some((i, plan.subject.clone(), plan.confidence.clone())),
                _ => None,
            }
        });
        let Some((_di, subject, conf)) = donor else {
            continue;
        };

        for &i in &idxs {
            let needs = match &items[i].1 {
                None => true,
                Some(p) if p.subject.eq_ignore_ascii_case("General") => true,
                Some(p) if p.confidence == "low" && p.subject != subject => true,
                _ => false,
            };
            if !needs {
                continue;
            }
            let path = items[i].0.clone();
            if let Some(mut plan) = replan(&path, &subject) {
                plan.reason = format!(
                    "Related to other files in this group that match “{subject}”"
                );
                // Related inheritance is helpful but not as strong as a direct hit.
                plan.confidence = if conf == "high" {
                    "medium".into()
                } else {
                    "low".into()
                };
                items[i].1 = Some(plan);
                changed += 1;
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_group_numbered_siblings() {
        assert_eq!(
            family_key(Path::new("Notes 8.pdf")),
            family_key(Path::new("Notes 9.pdf"))
        );
        assert_eq!(
            family_key(Path::new("L4.pdf")),
            family_key(Path::new("L5.pdf"))
        );
        assert_eq!(
            family_key(Path::new("report-final.pdf")),
            family_key(Path::new("report-final(1).pdf"))
        );
        assert_ne!(
            family_key(Path::new("dsd labmanual.pdf")),
            family_key(Path::new("emft labmanual.pdf"))
        );
    }
}

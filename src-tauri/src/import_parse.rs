use crate::subjects::SubjectDef;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposedSubject {
    pub name: String,
    pub keywords: Vec<String>,
    pub enabled: bool,
}

/// Turn free-text (syllabus paste, course list, folder tree) into subject proposals.
pub fn parse_prompt(text: &str) -> Vec<ProposedSubject> {
    let mut proposals: Vec<ProposedSubject> = Vec::new();

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        // Skip obvious noise
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("http://")
            || lower.starts_with("https://")
            || lower.starts_with("#")
            || lower == "subjects"
            || lower == "courses"
            || lower == "folder structure"
        {
            continue;
        }

        if let Some(parsed) = parse_line(line) {
            merge_proposal(&mut proposals, parsed);
        }
    }

    // If nothing structured matched, treat non-empty words-as-lines as subject names
    if proposals.is_empty() {
        for raw_line in text.lines() {
            let line = strip_list_prefix(raw_line.trim());
            if line.len() < 2 || line.len() > 60 {
                continue;
            }
            if looks_like_keyword_dump(line) {
                continue;
            }
            merge_proposal(
                &mut proposals,
                ProposedSubject {
                    name: title_case(line),
                    keywords: auto_keywords(line),
                    enabled: true,
                },
            );
        }
    }

    proposals
}

fn parse_line(line: &str) -> Option<ProposedSubject> {
    let line = strip_list_prefix(line);
    if line.is_empty() {
        return None;
    }

    // "Garden Design: raised beds, irrigation"
    // "Garden Design - raised beds, irrigation"
    // "Garden Design | raised beds; irrigation"
    for sep in [": ", " - ", " – ", " — ", " | "] {
        if let Some((name, rest)) = line.split_once(sep) {
            let name = clean_name(name)?;
            let keywords = split_keywords(rest);
            let mut kws = auto_keywords(&name);
            for k in keywords {
                if !kws.iter().any(|e| e.eq_ignore_ascii_case(&k)) {
                    kws.push(k);
                }
            }
            return Some(ProposedSubject {
                name,
                keywords: kws,
                enabled: true,
            });
        }
    }

    // Tree-style: "├── Power Systems" or "/Power Systems/" or "Power Systems/"
    let trimmed = line
        .trim_matches(|c| c == '/' || c == '\\' || c == '|' || c == '+' || c == '-' || c == '`')
        .trim();
    let trimmed = trimmed
        .trim_start_matches(['├', '└', '│', '─', '►', '•', '*'])
        .trim();

    if let Some(name) = clean_name(trimmed) {
        // Skip lines that look like filenames with extensions
        if name.contains('.') && name.split('.').last().map(|e| e.len() <= 5).unwrap_or(false) {
            let ext = name.split('.').last().unwrap_or("");
            if ["pdf", "docx", "png", "jpg", "txt", "zip", "mp4"]
                .iter()
                .any(|e| ext.eq_ignore_ascii_case(e))
            {
                return None;
            }
        }
        return Some(ProposedSubject {
            name: name.clone(),
            keywords: auto_keywords(&name),
            enabled: true,
        });
    }

    None
}

fn strip_list_prefix(line: &str) -> &str {
    let mut s = line.trim();
    for prefix in ["- ", "* ", "• ", "> ", "→ "] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.trim();
        }
    }
    // "1. Control Systems" / "12) Power Systems"
    if let Some(rest) = s.split_once(['.', ')', ']']) {
        let idx = rest.0.trim();
        if !idx.is_empty() && idx.chars().all(|c| c.is_ascii_digit()) {
            return rest.1.trim();
        }
    }
    s
}

fn clean_name(name: &str) -> Option<String> {
    let name = name
        .trim()
        .trim_matches(|c: char| c == '"' || c == '\'' || c == '`')
        .trim();
    if name.len() < 2 || name.len() > 64 {
        return None;
    }
    if name.eq_ignore_ascii_case("general") {
        return None;
    }
    Some(title_case(name))
}

fn split_keywords(rest: &str) -> Vec<String> {
    rest.split([',', ';', '/', '|'])
        .map(|k| k.trim().to_string())
        .filter(|k| k.len() >= 2)
        .collect()
}

fn auto_keywords(name: &str) -> Vec<String> {
    let lower = name.to_ascii_lowercase();
    let mut kws = vec![lower.clone()];
    // Also add without filler words for matching
    let compact = lower
        .replace(" and ", " ")
        .replace(" & ", " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if compact != lower {
        kws.push(compact);
    }
    kws
}

fn merge_proposal(list: &mut Vec<ProposedSubject>, next: ProposedSubject) {
    if let Some(existing) = list
        .iter_mut()
        .find(|s| s.name.eq_ignore_ascii_case(&next.name))
    {
        for k in next.keywords {
            if !existing.keywords.iter().any(|e| e.eq_ignore_ascii_case(&k)) {
                existing.keywords.push(k);
            }
        }
    } else {
        list.push(next);
    }
}

fn looks_like_keyword_dump(line: &str) -> bool {
    line.matches(',').count() >= 3 && !line.contains(':')
}

fn title_case(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn to_subject_defs(proposals: &[ProposedSubject]) -> Vec<SubjectDef> {
    proposals
        .iter()
        .map(|p| SubjectDef {
            name: p.name.clone(),
            keywords: p.keywords.clone(),
            enabled: p.enabled,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_syllabus_style() {
        let text = r#"
My projects:
1. Garden Design: raised beds, irrigation, soil
2. Home Budget - rent, utilities, savings
3. Travel Plans
"#;
        let out = parse_prompt(text);
        assert!(out.iter().any(|s| s.name == "Garden Design"));
        assert!(out.iter().any(|s| s.name == "Home Budget"));
        assert!(out.iter().any(|s| s.name == "Travel Plans"));
        let ps = out.iter().find(|s| s.name == "Garden Design").unwrap();
        assert!(ps.keywords.iter().any(|k| k.contains("raised beds")));
    }
}

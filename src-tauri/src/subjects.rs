use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubjectDef {
    pub name: String,
    pub keywords: Vec<String>,
    pub enabled: bool,
}

/// Fresh installs start with no subjects — the user imports or adds their own.
/// Classification falls back to the `General` folder when nothing matches.
pub fn empty_subjects() -> Vec<SubjectDef> {
    Vec::new()
}

/// No canned course packs. Subjects come from the user (import / add).
pub fn preset_list() -> Vec<(&'static str, &'static str)> {
    Vec::new()
}

pub fn preset_by_id(_id: &str) -> Option<Vec<SubjectDef>> {
    None
}

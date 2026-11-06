use crate::subjects::{self, SubjectDef};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub watch_folder: Option<String>,
    pub destination_folder: Option<String>,
    pub monitor_on_launch: bool,
    pub monitoring_enabled: bool,
    #[serde(default = "default_subjects")]
    pub subjects: Vec<SubjectDef>,
    #[serde(default)]
    pub onboarding_complete: bool,
}

fn default_subjects() -> Vec<SubjectDef> {
    subjects::empty_subjects()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            watch_folder: dirs::download_dir().map(|p| p.to_string_lossy().to_string()),
            destination_folder: None,
            monitor_on_launch: true,
            monitoring_enabled: false,
            subjects: subjects::empty_subjects(),
            onboarding_complete: false,
        }
    }
}

fn config_path() -> Result<PathBuf, String> {
    let base = dirs::config_dir().ok_or("Could not resolve config directory")?;
    let dir = base.join("Tidy");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("config.json"))
}

pub fn load() -> AppConfig {
    match config_path() {
        Ok(path) if path.exists() => {
            let raw = fs::read_to_string(&path).ok();
            let mut cfg = raw
                .as_deref()
                .and_then(|s| serde_json::from_str::<AppConfig>(s).ok())
                .unwrap_or_else(|| migrate_legacy(raw.as_deref()).unwrap_or_default());
            // Expand match needles; user phrases stay first (see enrich_subject).
            crate::classify_subject::enrich_all(&mut cfg.subjects);
            cfg
        }
        _ => AppConfig::default(),
    }
}

/// Older configs only had `enabledSubjects` maps — rebuild a subject list if needed.
fn migrate_legacy(raw: Option<&str>) -> Option<AppConfig> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Legacy {
        watch_folder: Option<String>,
        destination_folder: Option<String>,
        monitor_on_launch: Option<bool>,
        monitoring_enabled: Option<bool>,
        enabled_subjects: Option<std::collections::HashMap<String, bool>>,
    }

    let legacy: Legacy = serde_json::from_str(raw?).ok()?;
    // Legacy only stored enable flags for a previous hardcoded list — start blank.
    let _ = legacy.enabled_subjects;
    Some(AppConfig {
        watch_folder: legacy.watch_folder,
        destination_folder: legacy.destination_folder,
        monitor_on_launch: legacy.monitor_on_launch.unwrap_or(true),
        monitoring_enabled: legacy.monitoring_enabled.unwrap_or(false),
        subjects: subjects::empty_subjects(),
        onboarding_complete: false,
    })
}

pub fn save(config: &AppConfig) -> Result<(), String> {
    let path = config_path()?;
    let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

pub fn paths_ready(config: &AppConfig) -> bool {
    matches!(
        (
            config.watch_folder.as_deref().map(Path::new),
            config.destination_folder.as_deref().map(Path::new)
        ),
        (Some(w), Some(d)) if w.is_dir() && d.is_dir()
    )
}

use crate::classify_subject;
use crate::classify_type;
use crate::config::AppConfig;
use crate::history::MoveRecord;
use chrono::Utc;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct PlannedMove {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub subject: String,
    pub file_type: String,
    pub filename: String,
    /// Why this subject was chosen (shown in the preview).
    pub reason: String,
    /// "high" | "medium" | "low" | "none"
    pub confidence: String,
}

#[allow(dead_code)]
pub fn plan_move(path: &Path, config: &AppConfig) -> Result<PlannedMove, String> {
    plan_move_opts(path, config, true, true)
}

pub fn plan_move_opts(
    path: &Path,
    config: &AppConfig,
    allow_extract: bool,
    allow_embed: bool,
) -> Result<PlannedMove, String> {
    let index = classify_subject::SubjectIndex::build(&config.subjects);
    plan_move_with_index(path, config, &index, allow_extract, allow_embed)
}

pub fn plan_move_with_index(
    path: &Path,
    config: &AppConfig,
    index: &classify_subject::SubjectIndex,
    allow_extract: bool,
    allow_embed: bool,
) -> Result<PlannedMove, String> {
    let dest_root = config
        .destination_folder
        .as_ref()
        .ok_or("Destination folder is not set")?;

    if !Path::new(dest_root).is_dir() {
        return Err("Destination folder does not exist".into());
    }

    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Invalid filename")?
        .to_string();

    let verdict = classify_subject::classify_detailed(
        path,
        index,
        &config.subjects,
        allow_extract,
        allow_embed,
    );
    let confidence = verdict.confidence_str().to_string();
    build_plan(path, dest_root, filename, verdict.subject, verdict.reason, confidence)
}

/// Plan a move into an already-decided subject (preview result, semantic pass).
pub fn plan_move_to_subject(
    path: &Path,
    config: &AppConfig,
    subject: &str,
) -> Result<PlannedMove, String> {
    let dest_root = config
        .destination_folder
        .as_ref()
        .ok_or("Destination folder is not set")?;
    if !Path::new(dest_root).is_dir() {
        return Err("Destination folder does not exist".into());
    }
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Invalid filename")?
        .to_string();
    build_plan(
        path,
        dest_root,
        filename,
        subject.to_string(),
        "Chosen by you".to_string(),
        "high".to_string(),
    )
}

fn build_plan(
    path: &Path,
    dest_root: &str,
    filename: String,
    subject: String,
    reason: String,
    confidence: String,
) -> Result<PlannedMove, String> {
    let file_type = classify_type::classify_type(path).to_string();
    let target_dir = PathBuf::from(dest_root).join(&subject).join(&file_type);
    let destination = unique_path_for(path, &target_dir, &filename);

    Ok(PlannedMove {
        source: path.to_path_buf(),
        destination,
        subject,
        file_type,
        filename,
        reason,
        confidence,
    })
}

pub fn paths_equal(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => false,
    }
}

pub fn execute_move(planned: &PlannedMove) -> Result<MoveRecord, String> {
    // Already in the right place (e.g. re-organize no-op).
    if paths_equal(&planned.source, &planned.destination) {
        return Ok(MoveRecord {
            id: Uuid::new_v4().to_string(),
            source: planned.source.to_string_lossy().to_string(),
            destination: planned.destination.to_string_lossy().to_string(),
            subject: planned.subject.clone(),
            file_type: planned.file_type.clone(),
            filename: planned.filename.clone(),
            timestamp: Utc::now().to_rfc3339(),
            undone: false,
            batch_id: None,
        });
    }

    if let Some(parent) = planned.destination.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Could not create folders: {e}"))?;
    }

    if planned.destination.exists() {
        return Err("Destination already exists after unique naming".into());
    }

    fs::rename(&planned.source, &planned.destination).or_else(|_| {
        // Cross-device fallback: copy then remove source.
        fs::copy(&planned.source, &planned.destination)
            .map_err(|e| format!("Could not copy file: {e}"))?;
        fs::remove_file(&planned.source).map_err(|e| format!("Copied but could not remove source: {e}"))
    })?;

    Ok(MoveRecord {
        id: Uuid::new_v4().to_string(),
        source: planned.source.to_string_lossy().to_string(),
        destination: planned.destination.to_string_lossy().to_string(),
        subject: planned.subject.clone(),
        file_type: planned.file_type.clone(),
        filename: planned
            .destination
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&planned.filename)
            .to_string(),
        timestamp: Utc::now().to_rfc3339(),
        undone: false,
        batch_id: None,
    })
}

#[allow(dead_code)]
pub fn organize_file(path: &Path, config: &AppConfig) -> Result<MoveRecord, String> {
    organize_file_opts(path, config, true, None)
}

pub fn organize_file_opts(
    path: &Path,
    config: &AppConfig,
    allow_extract: bool,
    batch_id: Option<String>,
) -> Result<MoveRecord, String> {
    let planned = plan_move_opts(path, config, allow_extract, true)?;
    let mut record = execute_move(&planned)?;
    record.batch_id = batch_id;
    Ok(record)
}

/// Never overwrite — append " (2)", " (3)", … before the extension.
/// If `source` is already at the candidate path, keep that path (re-organize).
fn unique_path_for(source: &Path, dir: &Path, filename: &str) -> PathBuf {
    let candidate = dir.join(filename);
    if !candidate.exists() || paths_equal(source, &candidate) {
        return candidate;
    }

    let path = Path::new(filename);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(filename);
    let ext = path.extension().and_then(|e| e.to_str());

    for i in 2..10_000 {
        let name = match ext {
            Some(e) => format!("{stem} ({i}).{e}"),
            None => format!("{stem} ({i})"),
        };
        let candidate = dir.join(&name);
        if !candidate.exists() || paths_equal(source, &candidate) {
            return candidate;
        }
    }

    dir.join(format!(
        "{stem}-{}",
        Uuid::new_v4().to_string().split('-').next().unwrap_or("dup")
    ))
}

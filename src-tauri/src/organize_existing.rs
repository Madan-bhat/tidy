use crate::config::AppConfig;
use crate::history::{self, MoveRecord};
use crate::mover::{self, PlannedMove};
use crate::stability;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewItem {
    pub source: String,
    pub destination: String,
    pub subject: String,
    pub file_type: String,
    pub filename: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizeProgress {
    pub phase: String,
    pub current: usize,
    pub total: usize,
    pub filename: String,
}

fn emit_progress(app: &AppHandle, phase: &str, current: usize, total: usize, filename: &str) {
    let _ = app.emit(
        "organize-progress",
        OrganizeProgress {
            phase: phase.to_string(),
            current,
            total,
            filename: filename.to_string(),
        },
    );
}

pub fn preview(app: &AppHandle, config: &AppConfig) -> Result<Vec<PreviewItem>, String> {
    let watch = config
        .watch_folder
        .as_ref()
        .ok_or("Watch folder is not set")?;
    let watch_path = Path::new(watch);
    if !watch_path.is_dir() {
        return Err("Watch folder does not exist".into());
    }
    if config.destination_folder.is_none() {
        return Err("Destination folder is not set".into());
    }

    let entries: Vec<_> = fs::read_dir(watch_path)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && !stability::looks_incomplete(p))
        .collect();

    let total = entries.len();
    emit_progress(app, "matching", 0, total, "");

    let mut items = Vec::new();
    for (i, path) in entries.into_iter().enumerate() {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        emit_progress(app, "matching", i + 1, total, &name);

        match mover::plan_move_opts(&path, config, false) {
            Ok(planned) => items.push(to_preview(planned)),
            Err(_) => continue,
        }
    }

    items.sort_by(|a, b| a.filename.to_lowercase().cmp(&b.filename.to_lowercase()));
    emit_progress(app, "done", total, total, "");
    Ok(items)
}

pub fn apply(
    app: &AppHandle,
    config: &AppConfig,
    sources: &[String],
) -> Result<Vec<MoveRecord>, String> {
    let total = sources.len();
    emit_progress(app, "moving", 0, total, "");

    let batch_id = history::new_batch_id();
    let mut results = Vec::new();
    let mut errors = Vec::new();

    for (i, source) in sources.iter().enumerate() {
        let path = Path::new(source);
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(source)
            .to_string();
        emit_progress(app, "moving", i + 1, total, &name);

        if !path.is_file() {
            errors.push(format!("Missing: {source}"));
            continue;
        }
        if stability::looks_incomplete(path) {
            errors.push(format!("Skipped incomplete: {source}"));
            continue;
        }

        match mover::organize_file_opts(path, config, false, Some(batch_id.clone())) {
            Ok(record) => {
                results.push(record);
            }
            Err(e) => errors.push(format!("{source}: {e}")),
        }
    }

    if !results.is_empty() {
        let _ = history::append_many(results.clone());
    }

    crate::pending::remove_sources(sources);
    emit_progress(app, "done", total, total, "");

    if results.is_empty() && !errors.is_empty() {
        return Err(errors.join("\n"));
    }

    Ok(results)
}

fn to_preview(planned: PlannedMove) -> PreviewItem {
    PreviewItem {
        source: planned.source.to_string_lossy().to_string(),
        destination: planned.destination.to_string_lossy().to_string(),
        subject: planned.subject,
        file_type: planned.file_type,
        filename: planned.filename,
    }
}

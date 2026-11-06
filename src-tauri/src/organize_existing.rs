use crate::config::AppConfig;
use crate::history::{self, MoveRecord};
use crate::mover::{self, PlannedMove};
use crate::stability;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewItem {
    pub source: String,
    pub destination: String,
    pub subject: String,
    pub file_type: String,
    pub filename: String,
    /// Current subject folder when re-organizing already-sorted files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_file_type: Option<String>,
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
        .filter(|p| p.is_file() && !stability::looks_incomplete(p) && !is_junk_name(p))
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

        // Extract document text when the filename alone is too weak.
        match mover::plan_move_opts(&path, config, true) {
            Ok(planned) => items.push(to_preview(planned, None, None)),
            Err(_) => continue,
        }
    }

    items.sort_by(|a, b| a.filename.to_lowercase().cmp(&b.filename.to_lowercase()));
    emit_progress(app, "done", total, total, "");
    Ok(items)
}

/// Re-classify files already under Destination/{Subject}/{Type}/.
/// Only returns files whose subject or type folder would change.
pub fn preview_reorganize(app: &AppHandle, config: &AppConfig) -> Result<Vec<PreviewItem>, String> {
    let dest = config
        .destination_folder
        .as_ref()
        .ok_or("Destination folder is not set")?;
    let dest_path = Path::new(dest);
    if !dest_path.is_dir() {
        return Err("Destination folder does not exist".into());
    }

    let entries = collect_organized_files(dest_path);
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

        let (from_subject, from_type) = current_placement(&path, dest_path);

        match mover::plan_move_opts(&path, config, true) {
            Ok(planned) => {
                if mover::paths_equal(&planned.source, &planned.destination) {
                    continue;
                }
                // Also skip if relative placement is unchanged (same subject + type + name)
                if from_subject.as_deref() == Some(planned.subject.as_str())
                    && from_type.as_deref() == Some(planned.file_type.as_str())
                    && planned.destination.file_name() == path.file_name()
                {
                    continue;
                }
                items.push(to_preview(planned, from_subject, from_type));
            }
            Err(_) => continue,
        }
    }

    items.sort_by(|a, b| {
        a.from_subject
            .cmp(&b.from_subject)
            .then(a.filename.to_lowercase().cmp(&b.filename.to_lowercase()))
    });
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

        match mover::plan_move_opts(path, config, true) {
            Ok(planned) => {
                if mover::paths_equal(&planned.source, &planned.destination) {
                    continue;
                }
                match mover::execute_move(&planned) {
                    Ok(mut record) => {
                        record.batch_id = Some(batch_id.clone());
                        results.push(record);
                    }
                    Err(e) => errors.push(format!("{source}: {e}")),
                }
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

fn collect_organized_files(dest_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(subjects) = fs::read_dir(dest_root) else {
        return out;
    };
    for subject_entry in subjects.flatten() {
        let subject_path = subject_entry.path();
        if !subject_path.is_dir() || is_junk_name(&subject_path) {
            continue;
        }
        let Ok(types) = fs::read_dir(&subject_path) else {
            continue;
        };
        for type_entry in types.flatten() {
            let type_path = type_entry.path();
            if type_path.is_file() {
                // Loose file directly under subject — still re-home it.
                if !stability::looks_incomplete(&type_path) && !is_junk_name(&type_path) {
                    out.push(type_path);
                }
                continue;
            }
            if !type_path.is_dir() || is_junk_name(&type_path) {
                continue;
            }
            let Ok(files) = fs::read_dir(&type_path) else {
                continue;
            };
            for file_entry in files.flatten() {
                let path = file_entry.path();
                if path.is_file() && !stability::looks_incomplete(&path) && !is_junk_name(&path) {
                    out.push(path);
                }
            }
        }
    }
    out
}

fn current_placement(path: &Path, dest_root: &Path) -> (Option<String>, Option<String>) {
    let Ok(rel) = path.strip_prefix(dest_root) else {
        return (None, None);
    };
    let mut comps = rel.components();
    let subject = comps
        .next()
        .and_then(|c| c.as_os_str().to_str())
        .map(|s| s.to_string());
    let file_type = comps
        .next()
        .and_then(|c| c.as_os_str().to_str())
        .map(|s| s.to_string());
    // If the next component is the filename, file_type may actually be the file —
    // detect by checking whether path's parent name equals file_type and grandparent is subject.
    if let (Some(subj), Some(ft)) = (&subject, &file_type) {
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|name| name == ft.as_str())
        {
            // Layout was Destination/Subject/file (no type folder)
            return (Some(subj.clone()), None);
        }
    }
    (subject, file_type)
}

fn is_junk_name(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    name.starts_with('.')
        || name == "thumbs.db"
        || name == "desktop.ini"
        || name == ".ds_store"
}

fn to_preview(
    planned: PlannedMove,
    from_subject: Option<String>,
    from_file_type: Option<String>,
) -> PreviewItem {
    PreviewItem {
        source: planned.source.to_string_lossy().to_string(),
        destination: planned.destination.to_string_lossy().to_string(),
        subject: planned.subject,
        file_type: planned.file_type,
        filename: planned.filename,
        from_subject,
        from_file_type,
    }
}

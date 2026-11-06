use crate::classify_subject::SubjectIndex;
use crate::config::AppConfig;
use crate::history::{self, MoveRecord};
use crate::mover::{self, PlannedMove};
use crate::stability;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
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
    /// Why Tidy picked this subject.
    #[serde(default)]
    pub reason: String,
    /// "high" | "medium" | "low" | "none"
    #[serde(default)]
    pub confidence: String,
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

    let index = SubjectIndex::build(&config.subjects);
    let items = classify_paths_fast(app, config, &index, &entries, |planned, _path| {
        Some(to_preview(planned, None, None))
    });

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

    let index = SubjectIndex::build(&config.subjects);
    let items = classify_paths_fast(app, config, &index, &entries, |planned, path| {
        if mover::paths_equal(&planned.source, &planned.destination) {
            return None;
        }
        let (from_subject, from_type) = current_placement(path, dest_path);
        // No evidence either way → don't evict a file from the folder it's already in.
        if planned.subject.eq_ignore_ascii_case("General")
            && from_subject
                .as_deref()
                .is_some_and(|s| !s.eq_ignore_ascii_case("General"))
        {
            return None;
        }
        if from_subject.as_deref() == Some(planned.subject.as_str())
            && from_type.as_deref() == Some(planned.file_type.as_str())
            && planned.destination.file_name() == path.file_name()
        {
            return None;
        }
        Some(to_preview(planned, from_subject, from_type))
    });

    emit_progress(app, "done", total, total, "");
    Ok(items)
}

/// Two-phase scan: fast filename BM25, then document text for weak / General
/// matches — with its own progress bar, low concurrency, and per-file timeouts
/// so a slow PDF cannot freeze the UI at 100%.
fn classify_paths_fast<F>(
    app: &AppHandle,
    config: &AppConfig,
    index: &SubjectIndex,
    entries: &[PathBuf],
    keep: F,
) -> Vec<PreviewItem>
where
    F: Fn(PlannedMove, &Path) -> Option<PreviewItem> + Sync,
{
    let total = entries.len();
    let done = AtomicUsize::new(0);

    // Phase 1 — filename only (cheap, parallel).
    let phase1: Vec<(PathBuf, Option<PlannedMove>)> = entries
        .par_iter()
        .map(|path| {
            let planned = catch_unwind(AssertUnwindSafe(|| {
                mover::plan_move_with_index(path, config, index, false, false).ok()
            }))
            .ok()
            .flatten();
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            if n == 1 || n == total || n % 4 == 0 {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                emit_progress(app, "matching", n, total, name);
            }
            (path.clone(), planned)
        })
        .collect();

    // Phase 2 — extract text only when filename ranking was weak / General.
    let needs_extract: Vec<PathBuf> = phase1
        .iter()
        .filter_map(|(path, planned)| {
            let weak = planned
                .as_ref()
                .map(|p| p.subject.eq_ignore_ascii_case("General"))
                .unwrap_or(true);
            if weak && crate::extract::can_extract(path) {
                Some(path.clone())
            } else {
                None
            }
        })
        .collect();

    let extracted: HashMapPath = if needs_extract.is_empty() {
        HashMapPath::default()
    } else {
        let extract_total = needs_extract.len();
        let extract_done = AtomicUsize::new(0);
        let deadline = Instant::now() + Duration::from_secs(90);
        emit_progress(app, "reading", 0, extract_total, "reading documents…");

        // Cap parallelism — pdf-extract is heavy; flooding cores feels frozen.
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(2)
            .build()
            .unwrap_or_else(|_| rayon::ThreadPoolBuilder::new().build().expect("rayon"));

        pool.install(|| {
            needs_extract
                .par_iter()
                .filter_map(|path| {
                    // Global budget: never let a pile of slow documents hold the scan hostage.
                    let planned = if Instant::now() > deadline {
                        None
                    } else {
                        catch_unwind(AssertUnwindSafe(|| {
                            mover::plan_move_with_index(path, config, index, true, false).ok()
                        }))
                        .ok()
                        .flatten()
                    };
                    let n = extract_done.fetch_add(1, Ordering::Relaxed) + 1;
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("");
                    if n == 1 || n == extract_total || n % 2 == 0 {
                        emit_progress(app, "reading", n, extract_total, name);
                    }
                    planned.map(|p| (path.clone(), p))
                })
                .collect()
        })
    };

    // Merge phase 1 + 2.
    let mut merged: Vec<(PathBuf, Option<PlannedMove>)> = phase1
        .into_iter()
        .map(|(path, planned)| {
            let planned = extracted.get(&path).cloned().or(planned);
            (path, planned)
        })
        .collect();

    // Phase 3 — hybrid semantic pass (MiniLM) for documents still unmatched.
    // One batched call, never per file, so it cannot stall the scan.
    if crate::classify_embed::is_enabled() {
        let idxs: Vec<usize> = merged
            .iter()
            .enumerate()
            .filter(|(_, (path, planned))| {
                is_document(path)
                    && planned
                        .as_ref()
                        .map(|p| p.subject.eq_ignore_ascii_case("General"))
                        .unwrap_or(false)
            })
            .map(|(i, _)| i)
            .collect();
        if !idxs.is_empty() {
            emit_progress(app, "reading", idxs.len(), idxs.len(), "semantic matching…");
            let queries: Vec<String> = idxs
                .iter()
                .map(|i| semantic_query(&merged[*i].0))
                .collect();
            let subjects = config.subjects.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(crate::classify_embed::classify_batch(&queries, &subjects));
            });
            if let Ok(results) = rx.recv_timeout(Duration::from_secs(30)) {
                for (slot, found) in idxs.iter().zip(results) {
                    if let Some((subject, cos)) = found {
                        if let Ok(mut plan) =
                            mover::plan_move_to_subject(&merged[*slot].0, config, &subject)
                        {
                            plan.reason =
                                "Name and contents are close in meaning to this subject".into();
                            plan.confidence =
                                if cos >= 0.45 { "medium" } else { "low" }.to_string();
                            merged[*slot].1 = Some(plan);
                        }
                    }
                }
            }
        }
    }

    let mut items = Vec::with_capacity(entries.len());
    for (path, planned) in merged {
        if let Some(planned) = planned {
            if let Some(item) = keep(planned, &path) {
                items.push(item);
            }
        }
    }

    items.sort_by(|a, b| {
        a.from_subject
            .cmp(&b.from_subject)
            .then(a.filename.to_lowercase().cmp(&b.filename.to_lowercase()))
    });
    items
}

fn is_document(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "pdf" | "doc" | "docx" | "ppt" | "pptx" | "txt" | "md" | "odt" | "odp" | "rtf" | "key"
            | "pages" | "tex"
    )
}

/// Filename as natural-language text for the embedder.
fn semantic_query(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .replace(['_', '-', '.'], " ");
    let mut q = stem;
    if let Some(text) = crate::extract::peek_cached(path) {
        q.push_str(". ");
        q.push_str(&text);
    }
    q
}

type HashMapPath = std::collections::HashMap<PathBuf, PlannedMove>;


pub fn apply(
    app: &AppHandle,
    config: &AppConfig,
    sources: &[String],
    subjects: &std::collections::HashMap<String, String>,
) -> Result<Vec<MoveRecord>, String> {
    let total = sources.len();
    emit_progress(app, "moving", 0, total, "");

    let batch_id = history::new_batch_id();
    let index = SubjectIndex::build(&config.subjects);
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

        // Use exactly what the preview showed; only re-classify if we weren't told.
        let plan = match subjects.get(source) {
            Some(subject) => mover::plan_move_to_subject(path, config, subject),
            None => mover::plan_move_with_index(path, config, &index, true, false),
        };
        match plan {
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
        reason: planned.reason,
        confidence: planned.confidence,
    }
}

use crate::cancel;
use crate::classify_subject::SubjectIndex;
use crate::config::AppConfig;
use crate::history::{self, MoveRecord};
use crate::mover::{self, PlannedMove};
use crate::related;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_file_type: Option<String>,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub confidence: String,
    /// ready | review | unsorted — drives the preview grouping.
    #[serde(default)]
    pub bucket: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizeProgress {
    pub phase: String,
    pub current: usize,
    pub total: usize,
    pub filename: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eta_secs: Option<u32>,
}

fn emit_progress(app: &AppHandle, phase: &str, current: usize, total: usize, filename: &str) {
    let _ = app.emit(
        "organize-progress",
        OrganizeProgress {
            phase: phase.to_string(),
            current,
            total,
            filename: filename.to_string(),
            eta_secs: cancel::eta_secs(current, total),
        },
    );
}

fn check_cancel() -> Result<(), String> {
    if cancel::is_cancelled() {
        Err("Cancelled".into())
    } else {
        Ok(())
    }
}

pub fn preview(app: &AppHandle, config: &AppConfig) -> Result<Vec<PreviewItem>, String> {
    let _gen = cancel::begin();
    let result = (|| {
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
        })?;

        emit_progress(app, "done", total, total, "");
        Ok(items)
    })();
    cancel::end();
    result
}

pub fn preview_reorganize(app: &AppHandle, config: &AppConfig) -> Result<Vec<PreviewItem>, String> {
    let _gen = cancel::begin();
    let result = (|| {
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
        })?;

        emit_progress(app, "done", total, total, "");
        Ok(items)
    })();
    cancel::end();
    result
}

fn classify_paths_fast<F>(
    app: &AppHandle,
    config: &AppConfig,
    index: &SubjectIndex,
    entries: &[PathBuf],
    keep: F,
) -> Result<Vec<PreviewItem>, String>
where
    F: Fn(PlannedMove, &Path) -> Option<PreviewItem> + Sync,
{
    let total = entries.len();
    let done = AtomicUsize::new(0);

    // Phase 1 — filename only.
    let phase1: Vec<(PathBuf, Option<PlannedMove>)> = entries
        .par_iter()
        .map(|path| {
            if cancel::is_cancelled() {
                return (path.clone(), None);
            }
            let planned = catch_unwind(AssertUnwindSafe(|| {
                mover::plan_move_with_index(path, config, index, false, false).ok()
            }))
            .ok()
            .flatten();
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            if n == 1 || n == total || n % 4 == 0 {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                emit_progress(app, "matching", n, total, name);
            }
            (path.clone(), planned)
        })
        .collect();
    check_cancel()?;

    // Phase 2 — extract / OCR for weak matches.
    let needs_extract: Vec<PathBuf> = phase1
        .iter()
        .filter_map(|(path, planned)| {
            let weak = planned
                .as_ref()
                .map(|p| {
                    p.subject.eq_ignore_ascii_case("General") || p.confidence == "low"
                })
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
        let deadline = Instant::now() + Duration::from_secs(120);
        emit_progress(app, "reading", 0, extract_total, "reading documents…");

        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(2)
            .build()
            .unwrap_or_else(|_| rayon::ThreadPoolBuilder::new().build().expect("rayon"));

        pool.install(|| {
            needs_extract
                .par_iter()
                .filter_map(|path| {
                    if cancel::is_cancelled() || Instant::now() > deadline {
                        let n = extract_done.fetch_add(1, Ordering::Relaxed) + 1;
                        emit_progress(
                            app,
                            "reading",
                            n,
                            extract_total,
                            path.file_name().and_then(|n| n.to_str()).unwrap_or(""),
                        );
                        return None;
                    }
                    let planned = catch_unwind(AssertUnwindSafe(|| {
                        mover::plan_move_with_index(path, config, index, true, false).ok()
                    }))
                    .ok()
                    .flatten();
                    let n = extract_done.fetch_add(1, Ordering::Relaxed) + 1;
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if n == 1 || n == extract_total || n % 2 == 0 {
                        emit_progress(app, "reading", n, extract_total, name);
                    }
                    planned.map(|p| (path.clone(), p))
                })
                .collect()
        })
    };
    check_cancel()?;

    let mut merged: Vec<(PathBuf, Option<PlannedMove>)> = phase1
        .into_iter()
        .map(|(path, planned)| {
            let planned = extracted.get(&path).cloned().or(planned);
            (path, planned)
        })
        .collect();

    // Phase 3 — semantic for General + low-confidence borderline matches.
    if crate::classify_embed::is_enabled() {
        let idxs: Vec<usize> = merged
            .iter()
            .enumerate()
            .filter(|(_, (path, planned))| {
                is_matchable(path)
                    && planned
                        .as_ref()
                        .map(|p| {
                            p.subject.eq_ignore_ascii_case("General") || p.confidence == "low"
                        })
                        .unwrap_or(false)
            })
            .map(|(i, _)| i)
            .collect();
        if !idxs.is_empty() {
            emit_progress(app, "reading", 0, idxs.len(), "semantic matching…");
            let queries: Vec<String> = idxs
                .iter()
                .map(|i| semantic_query(&merged[*i].0))
                .collect();
            let subjects = config.subjects.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(crate::classify_embed::classify_batch(&queries, &subjects));
            });
            if let Ok(results) = rx.recv_timeout(Duration::from_secs(45)) {
                for (i, (slot, found)) in idxs.iter().zip(results).enumerate() {
                    if cancel::is_cancelled() {
                        break;
                    }
                    if let Some((subject, cos)) = found {
                        // Don't override a low-confidence lexical hit with a weaker semantic one
                        // unless the file was General.
                        let was_general = merged[*slot]
                            .1
                            .as_ref()
                            .map(|p| p.subject.eq_ignore_ascii_case("General"))
                            .unwrap_or(true);
                        if !was_general && cos < 0.38 {
                            continue;
                        }
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
                    if i % 4 == 0 || i + 1 == idxs.len() {
                        emit_progress(app, "reading", i + 1, idxs.len(), "semantic matching…");
                    }
                }
            }
        }
    }
    check_cancel()?;

    // Phase 4 — related-file grouping.
    emit_progress(app, "matching", total, total, "grouping related files…");
    let _ = related::propagate(&mut merged, &|path, subject| {
        mover::plan_move_to_subject(path, config, subject).ok()
    });

    let mut items = Vec::with_capacity(entries.len());
    for (path, planned) in merged {
        if let Some(planned) = planned {
            if let Some(item) = keep(planned, &path) {
                items.push(item);
            }
        }
    }

    items.sort_by(|a, b| {
        bucket_rank(&a.bucket)
            .cmp(&bucket_rank(&b.bucket))
            .then(a.from_subject.cmp(&b.from_subject))
            .then(a.filename.to_lowercase().cmp(&b.filename.to_lowercase()))
    });
    Ok(items)
}

fn bucket_rank(b: &str) -> u8 {
    match b {
        "ready" => 0,
        "review" => 1,
        _ => 2,
    }
}

fn is_matchable(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "pdf"
            | "doc"
            | "docx"
            | "ppt"
            | "pptx"
            | "txt"
            | "md"
            | "odt"
            | "odp"
            | "rtf"
            | "key"
            | "pages"
            | "tex"
            | "png"
            | "jpg"
            | "jpeg"
            | "webp"
            | "tif"
            | "tiff"
            | "bmp"
            | "gif"
    )
}

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
    let _gen = cancel::begin();
    let result = (|| {
        let total = sources.len();
        emit_progress(app, "moving", 0, total, "");

        let batch_id = history::new_batch_id();
        let index = SubjectIndex::build(&config.subjects);
        let mut results = Vec::new();
        let mut errors = Vec::new();

        for (i, source) in sources.iter().enumerate() {
            check_cancel()?;
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
    })();
    cancel::end();
    result
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
    if let (Some(subj), Some(ft)) = (&subject, &file_type) {
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|name| name == ft.as_str())
        {
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

fn bucket_for(confidence: &str, subject: &str) -> String {
    if subject.eq_ignore_ascii_case("General") || confidence == "none" {
        "unsorted".into()
    } else if confidence == "low" {
        "review".into()
    } else {
        "ready".into()
    }
}

fn to_preview(
    planned: PlannedMove,
    from_subject: Option<String>,
    from_file_type: Option<String>,
) -> PreviewItem {
    let bucket = bucket_for(&planned.confidence, &planned.subject);
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
        bucket,
    }
}

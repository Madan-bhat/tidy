use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

const MAX_HISTORY: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveRecord {
    pub id: String,
    pub source: String,
    pub destination: String,
    pub subject: String,
    pub file_type: String,
    pub filename: String,
    pub timestamp: String,
    pub undone: bool,
    #[serde(default)]
    pub batch_id: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct HistoryFile {
    records: Vec<MoveRecord>,
}

fn history_path() -> Result<PathBuf, String> {
    let base = dirs::data_dir().ok_or("Could not resolve data directory")?;
    let dir = base.join("Tidy");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("history.json"))
}

pub fn load() -> Vec<MoveRecord> {
    match history_path() {
        Ok(path) if path.exists() => fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<HistoryFile>(&s).ok())
            .map(|h| h.records)
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn save(records: &[MoveRecord]) -> Result<(), String> {
    let path = history_path()?;
    let file = HistoryFile {
        records: records.to_vec(),
    };
    let json = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

pub fn new_batch_id() -> String {
    Uuid::new_v4().to_string()
}

#[allow(dead_code)]
pub fn append(record: MoveRecord) -> Result<MoveRecord, String> {
    let mut records = load();
    records.insert(0, record.clone());
    if records.len() > MAX_HISTORY {
        records.truncate(MAX_HISTORY);
    }
    save(&records)?;
    Ok(record)
}

pub fn append_many(new_records: Vec<MoveRecord>) -> Result<Vec<MoveRecord>, String> {
    let mut records = load();
    for record in new_records.iter().rev() {
        records.insert(0, record.clone());
    }
    if records.len() > MAX_HISTORY {
        records.truncate(MAX_HISTORY);
    }
    save(&records)?;
    Ok(new_records)
}

pub fn list(limit: Option<usize>) -> Vec<MoveRecord> {
    let records = load();
    match limit {
        Some(n) => records.into_iter().take(n).collect(),
        None => records,
    }
}

/// Most recent batch that still has at least one non-undone move.
pub fn latest_revertable_batch() -> Option<String> {
    let records = load();
    for r in &records {
        if !r.undone {
            if let Some(batch) = &r.batch_id {
                return Some(batch.clone());
            }
            // Legacy single moves without batch — treat id as a one-item batch.
            return Some(format!("single:{}", r.id));
        }
    }
    None
}

pub fn undo_last() -> Result<MoveRecord, String> {
    let records = load();
    let id = records
        .iter()
        .find(|r| !r.undone)
        .map(|r| r.id.clone())
        .ok_or("Nothing to undo")?;
    undo(&id)
}

pub fn revert_previous_changes() -> Result<Vec<MoveRecord>, String> {
    let batch = latest_revertable_batch().ok_or("Nothing to revert")?;
    if let Some(id) = batch.strip_prefix("single:") {
        return Ok(vec![undo(id)?]);
    }
    undo_batch(&batch)
}

pub fn undo_batch(batch_id: &str) -> Result<Vec<MoveRecord>, String> {
    let records = load();
    let ids: Vec<String> = records
        .iter()
        .filter(|r| !r.undone && r.batch_id.as_deref() == Some(batch_id))
        .map(|r| r.id.clone())
        .collect();
    if ids.is_empty() {
        return Err("Nothing to revert in the previous change set".into());
    }
    let mut undone = Vec::new();
    for id in ids {
        match undo(&id) {
            Ok(r) => undone.push(r),
            Err(e) => {
                if undone.is_empty() {
                    return Err(e);
                }
                // Partial success — return what we could revert.
                break;
            }
        }
    }
    Ok(undone)
}

pub fn undo(id: &str) -> Result<MoveRecord, String> {
    let mut records = load();
    let idx = records
        .iter()
        .position(|r| r.id == id)
        .ok_or("Move not found")?;

    if records[idx].undone {
        return Err("This move was already undone".into());
    }

    let dest = PathBuf::from(&records[idx].destination);
    let source = PathBuf::from(&records[idx].source);

    if !dest.exists() {
        return Err("File is no longer at the organized location".into());
    }

    if let Some(parent) = source.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let restore_to = if source.exists() {
        unique_restore_path(&source)
    } else {
        source
    };

    fs::rename(&dest, &restore_to).or_else(|_| {
        fs::copy(&dest, &restore_to).map_err(|e| e.to_string())?;
        fs::remove_file(&dest).map_err(|e| e.to_string())
    })?;

    records[idx].undone = true;
    records[idx].source = restore_to.to_string_lossy().to_string();
    save(&records)?;
    Ok(records[idx].clone())
}

fn unique_restore_path(original: &PathBuf) -> PathBuf {
    let parent = original.parent().unwrap_or_else(|| std::path::Path::new("."));
    let filename = original
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("restored");
    let stem = original
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(filename);
    let ext = original.extension().and_then(|e| e.to_str());

    for i in 2..10_000 {
        let name = match ext {
            Some(e) => format!("{stem} restored ({i}).{e}"),
            None => format!("{stem} restored ({i})"),
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{stem}-restored"))
}

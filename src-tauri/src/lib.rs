mod cancel;
mod classify_embed;
mod classify_subject;
mod classify_type;
mod config;
pub mod eval;
mod extract;
mod folder_learn;
mod history;
mod import_parse;
mod learned;
mod mover;
mod ocr;
mod organize_existing;
mod pending;
mod related;
mod stability;
mod subjects;
mod tray;
mod watcher;

use config::AppConfig;
use history::MoveRecord;
use organize_existing::PreviewItem;
use serde::Serialize;
use subjects::SubjectDef;
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WindowEvent};
use watcher::{Monitor, MonitorStatus};

struct AppState {
    monitor: Monitor,
}

#[tauri::command]
fn get_config() -> AppConfig {
    config::load()
}

#[tauri::command]
fn save_config(
    mut new_config: AppConfig,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<AppConfig, String> {
    let current = config::load();
    new_config.monitoring_enabled = current.monitoring_enabled;

    classify_subject::enrich_all(&mut new_config.subjects); // trim/dedupe only

    config::save(&new_config)?;

    if state.monitor.is_active() && current.watch_folder != new_config.watch_folder {
        state.monitor.stop(&app);
        if config::paths_ready(&new_config) {
            let _ = state.monitor.start(app.clone());
        }
    }

    Ok(config::load())
}

#[tauri::command]
fn get_monitor_status(state: State<'_, AppState>) -> MonitorStatus {
    state.monitor.status()
}

#[tauri::command]
fn start_monitoring(state: State<'_, AppState>, app: AppHandle) -> Result<MonitorStatus, String> {
    state.monitor.start(app)
}

#[tauri::command]
fn stop_monitoring(state: State<'_, AppState>, app: AppHandle) -> MonitorStatus {
    state.monitor.stop(&app)
}

#[tauri::command]
fn toggle_monitoring(state: State<'_, AppState>, app: AppHandle) -> Result<MonitorStatus, String> {
    state.monitor.toggle(app)
}

#[tauri::command]
fn get_history(limit: Option<usize>) -> Vec<MoveRecord> {
    history::list(limit)
}

#[tauri::command]
fn undo_move(id: String) -> Result<MoveRecord, String> {
    history::undo(&id)
}

#[tauri::command]
fn undo_last_move() -> Result<MoveRecord, String> {
    history::undo_last()
}

#[tauri::command]
fn revert_previous_changes() -> Result<Vec<MoveRecord>, String> {
    let undone = history::revert_previous_changes()?;
    Ok(undone)
}

#[tauri::command]
fn can_revert_previous() -> bool {
    history::latest_revertable_batch().is_some()
}

#[tauri::command]
fn get_pending() -> Vec<PreviewItem> {
    pending::list()
}

#[tauri::command]
fn dismiss_pending(sources: Option<Vec<String>>) -> Vec<PreviewItem> {
    match sources {
        Some(list) if !list.is_empty() => pending::remove_sources(&list),
        _ => pending::clear(),
    }
    pending::list()
}

#[tauri::command]
fn propose_subjects_from_text(text: String) -> Result<Vec<import_parse::ProposedSubject>, String> {
    let proposals = import_parse::parse_prompt(&text);
    if proposals.is_empty() {
        return Err(
            "Could not find any subjects. Try lines like:\nCourse Name: topic one, topic two"
                .into(),
        );
    }
    Ok(proposals)
}

#[tauri::command]
fn confirm_proposed_subjects(
    subjects: Vec<import_parse::ProposedSubject>,
    mode: String,
) -> Result<AppConfig, String> {
    if subjects.is_empty() {
        return Err("No subjects to apply".into());
    }
    let mut incoming = import_parse::to_subject_defs(&subjects);
    classify_subject::enrich_all(&mut incoming);
    let mut cfg = config::load();
    match mode.as_str() {
        "merge" => {
            for next in incoming {
                if let Some(existing) = cfg
                    .subjects
                    .iter_mut()
                    .find(|s| s.name.eq_ignore_ascii_case(&next.name))
                {
                    for k in next.keywords {
                        if !existing.keywords.iter().any(|e| e.eq_ignore_ascii_case(&k)) {
                            existing.keywords.push(k);
                        }
                    }
                    existing.enabled = true;
                    classify_subject::enrich_subject(existing);
                } else {
                    cfg.subjects.push(next);
                }
            }
        }
        _ => {
            cfg.subjects = incoming;
        }
    }
    classify_subject::enrich_all(&mut cfg.subjects);
    config::save(&cfg)?;
    Ok(cfg)
}

// Heavy scans must be async + off the main thread: sync Tauri commands run on the
// UI thread, which freezes the webview (progress overlay "sticks" on the last file).
#[tauri::command]
async fn preview_existing(app: AppHandle) -> Result<Vec<PreviewItem>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let cfg = config::load_effective();
        organize_existing::preview(&app, &cfg)
    })
    .await
    .map_err(|e| format!("Scan failed: {e}"))?
}

#[tauri::command]
async fn preview_reorganize(app: AppHandle) -> Result<Vec<PreviewItem>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let cfg = config::load_effective();
        organize_existing::preview_reorganize(&app, &cfg)
    })
    .await
    .map_err(|e| format!("Scan failed: {e}"))?
}

#[tauri::command]
async fn apply_existing(
    app: AppHandle,
    sources: Vec<String>,
    subjects: Option<std::collections::HashMap<String, String>>,
) -> Result<Vec<MoveRecord>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let cfg = config::load_effective();
        organize_existing::apply(&app, &cfg, &sources, &subjects.unwrap_or_default())
    })
    .await
    .map_err(|e| format!("Apply failed: {e}"))?
}

#[derive(Debug, Clone, serde::Deserialize)]
struct Correction {
    source: String,
    subject: String,
}

/// The user moved files to a different subject than Tidy suggested — remember why.
#[tauri::command]
async fn record_corrections(items: Vec<Correction>) -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut learned_terms = 0;
        for item in items {
            let path = std::path::Path::new(&item.source);
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let text = if extract::can_extract(path) {
                extract::extract_text(path)
            } else {
                None
            };
            learned_terms += learned::learn(&item.subject, &stem, text.as_deref());
        }
        learned_terms
    })
    .await
    .map_err(|e| format!("Could not record corrections: {e}"))
}

#[tauri::command]
fn learned_counts() -> std::collections::HashMap<String, usize> {
    learned::counts()
}

#[tauri::command]
fn forget_learned(subject: Option<String>) {
    match subject {
        Some(s) => learned::forget(&s),
        None => learned::forget_all(),
    }
}

#[tauri::command]
fn cancel_organize() {
    cancel::request();
}

#[tauri::command]
fn ocr_status() -> serde_json::Value {
    serde_json::json!({
        "available": ocr::is_available(),
        "bundled": ocr::bundled_available(),
    })
}

#[tauri::command]
async fn learn_from_existing_files() -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let cfg = config::load();
        let dest = cfg
            .destination_folder
            .as_ref()
            .ok_or_else(|| "Destination folder is not set".to_string())?;
        let path = std::path::Path::new(dest);
        if !path.is_dir() {
            return Err("Destination folder does not exist".into());
        }
        Ok(folder_learn::learn_from_destination(path, &cfg.subjects))
    })
    .await
    .map_err(|e| format!("Learn failed: {e}"))?
}

#[tauri::command]
async fn suggest_keywords_from_folder() -> Result<Vec<folder_learn::SuggestResult>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let cfg = config::load();
        let dest = cfg
            .destination_folder
            .as_ref()
            .ok_or_else(|| "Destination folder is not set".to_string())?;
        let path = std::path::Path::new(dest);
        if !path.is_dir() {
            return Err("Destination folder does not exist".into());
        }
        Ok(folder_learn::suggest_keywords(path, &cfg.subjects))
    })
    .await
    .map_err(|e| format!("Suggest failed: {e}"))?
}

#[tauri::command]
fn apply_suggested_keywords(
    suggestions: Vec<folder_learn::SuggestResult>,
) -> Result<AppConfig, String> {
    let mut cfg = config::load();
    for sug in suggestions {
        if let Some(subject) = cfg.subjects.iter_mut().find(|s| s.name == sug.subject) {
            for kw in sug.keywords {
                if !subject
                    .keywords
                    .iter()
                    .any(|e| e.eq_ignore_ascii_case(&kw))
                {
                    subject.keywords.push(kw);
                }
            }
            classify_subject::enrich_subject(subject);
        }
    }
    config::save(&cfg)?;
    Ok(config::load())
}


#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TypeRuleDto {
    extensions: Vec<String>,
    folder: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PresetDto {
    id: String,
    label: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RulesDto {
    type_rules: Vec<TypeRuleDto>,
    subjects: Vec<SubjectDef>,
    presets: Vec<PresetDto>,
}

#[tauri::command]
fn get_rules() -> RulesDto {
    let cfg = config::load();
    let type_rules = classify_type::type_rules()
        .into_iter()
        .map(|(exts, folder)| TypeRuleDto {
            extensions: exts.into_iter().map(|e| e.to_string()).collect(),
            folder: folder.to_string(),
        })
        .collect();

    RulesDto {
        type_rules,
        subjects: cfg.subjects,
        presets: subjects::preset_list()
            .into_iter()
            .map(|(id, label)| PresetDto {
                id: id.to_string(),
                label: label.to_string(),
            })
            .collect(),
    }
}

#[tauri::command]
fn set_subject_enabled(subject: String, enabled: bool) -> Result<AppConfig, String> {
    if subject.eq_ignore_ascii_case("General") {
        return Err("General cannot be disabled".into());
    }
    let mut cfg = config::load();
    if let Some(item) = cfg.subjects.iter_mut().find(|s| s.name == subject) {
        item.enabled = enabled;
    } else {
        return Err("Subject not found".into());
    }
    config::save(&cfg)?;
    Ok(cfg)
}

#[tauri::command]
fn upsert_subject(name: String, keywords: Vec<String>) -> Result<AppConfig, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Subject name is required".into());
    }
    if name.eq_ignore_ascii_case("General") {
        return Err("General is reserved".into());
    }

    let keywords: Vec<String> = keywords
        .into_iter()
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .collect();

    let mut cfg = config::load();
    if let Some(existing) = cfg.subjects.iter_mut().find(|s| s.name == name) {
        existing.keywords = keywords;
        classify_subject::enrich_subject(existing);
    } else {
        let mut subject = SubjectDef {
            name,
            keywords,
            enabled: true,
        };
        classify_subject::enrich_subject(&mut subject);
        cfg.subjects.push(subject);
    }
    config::save(&cfg)?;
    Ok(cfg)
}

#[tauri::command]
fn delete_subject(name: String) -> Result<AppConfig, String> {
    if name.eq_ignore_ascii_case("General") {
        return Err("General cannot be deleted".into());
    }
    let mut cfg = config::load();
    let before = cfg.subjects.len();
    cfg.subjects.retain(|s| s.name != name);
    if cfg.subjects.len() == before {
        return Err("Subject not found".into());
    }
    learned::forget(&name);
    config::save(&cfg)?;
    Ok(cfg)
}

#[tauri::command]
fn apply_subject_preset(preset_id: String) -> Result<AppConfig, String> {
    let mut subjects = subjects::preset_by_id(&preset_id)
        .ok_or_else(|| format!("Unknown preset: {preset_id}"))?;
    classify_subject::enrich_all(&mut subjects);
    let mut cfg = config::load();
    cfg.subjects = subjects;
    config::save(&cfg)?;
    Ok(cfg)
}

/// Re-expand keywords for current subjects (fixes syllabus-style imports).
#[tauri::command]
fn refresh_subject_keywords() -> Result<AppConfig, String> {
    let mut cfg = config::load();
    classify_subject::enrich_all(&mut cfg.subjects);
    config::save(&cfg)?;
    Ok(cfg)
}

#[tauri::command]
fn embedding_status() -> serde_json::Value {
    serde_json::json!({
        "ready": classify_embed::is_ready(),
        "enabled": classify_embed::is_enabled(),
    })
}

#[tauri::command]
fn default_downloads_path() -> Option<String> {
    dirs::download_dir().map(|p| p.to_string_lossy().to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            monitor: Monitor::new(),
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            get_monitor_status,
            start_monitoring,
            stop_monitoring,
            toggle_monitoring,
            get_history,
            undo_move,
            undo_last_move,
            revert_previous_changes,
            can_revert_previous,
            get_pending,
            dismiss_pending,
            propose_subjects_from_text,
            confirm_proposed_subjects,
            preview_existing,
            preview_reorganize,
            apply_existing,
            record_corrections,
            learned_counts,
            forget_learned,
            cancel_organize,
            ocr_status,
            learn_from_existing_files,
            suggest_keywords_from_folder,
            apply_suggested_keywords,
            get_rules,
            set_subject_enabled,
            upsert_subject,
            delete_subject,
            apply_subject_preset,
            refresh_subject_keywords,
            embedding_status,
            default_downloads_path,
        ])
        .setup(|app| {
            tray::setup(app.handle())?;

            // Prefetch MiniLM in a child process (first run may download ~23MB).
            // If ort aborts, only the worker dies — UI keeps BM25 ranking.
            std::thread::spawn(|| {
                // Quietly harvest terms from already-filed folders (no-op if empty).
                let cfg = config::load();
                if let Some(dest) = cfg.destination_folder.as_ref() {
                    let path = std::path::Path::new(dest);
                    if path.is_dir() && learned::counts().is_empty() {
                        let n = folder_learn::learn_from_destination(path, &cfg.subjects);
                        if n > 0 {
                            eprintln!("[tidy] learned {n} terms from existing folders");
                        }
                    }
                }
                ocr::warm_up();
                match classify_embed::warm_up() {
                    Ok(()) => eprintln!("[tidy] hybrid embeddings ready"),
                    Err(e) => eprintln!("[tidy] hybrid embeddings unavailable: {e}"),
                }
            });

            let handle = app.handle().clone();
            let state = app.state::<AppState>();
            watcher::try_auto_start(handle.clone(), &state.monitor);
            let status = state.monitor.status();
            let _ = handle.emit("monitor-status", status);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Keep monitoring alive — hide to tray instead of quitting.
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            if let RunEvent::ExitRequested { api, code, .. } = &event {
                // Closing the last window should not exit while tray is running.
                // Explicit Quit (tray) passes a code and is allowed through.
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}

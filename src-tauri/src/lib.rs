mod classify_subject;
mod classify_type;
mod config;
mod extract;
mod history;
mod import_parse;
mod mover;
mod organize_existing;
mod pending;
mod stability;
mod subjects;
mod watcher;

use config::AppConfig;
use history::MoveRecord;
use organize_existing::PreviewItem;
use serde::Serialize;
use subjects::SubjectDef;
use tauri::{AppHandle, Emitter, Manager, State};
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

    if new_config.subjects.is_empty() {
        new_config.subjects = subjects::general_defaults();
    }

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
            "Could not find any subjects. Try lines like:\nPower Systems: load flow, transmission"
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
    let incoming = import_parse::to_subject_defs(&subjects);
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
                } else {
                    cfg.subjects.push(next);
                }
            }
        }
        _ => {
            // replace
            cfg.subjects = incoming;
        }
    }
    config::save(&cfg)?;
    Ok(cfg)
}

#[tauri::command]
fn preview_existing(app: AppHandle) -> Result<Vec<PreviewItem>, String> {
    let cfg = config::load();
    organize_existing::preview(&app, &cfg)
}

#[tauri::command]
fn apply_existing(app: AppHandle, sources: Vec<String>) -> Result<Vec<MoveRecord>, String> {
    let cfg = config::load();
    organize_existing::apply(&app, &cfg, &sources)
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
    } else {
        cfg.subjects.push(SubjectDef {
            name,
            keywords,
            enabled: true,
        });
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
    config::save(&cfg)?;
    Ok(cfg)
}

#[tauri::command]
fn apply_subject_preset(preset_id: String) -> Result<AppConfig, String> {
    let subjects = subjects::preset_by_id(&preset_id)
        .ok_or_else(|| format!("Unknown preset: {preset_id}"))?;
    let mut cfg = config::load();
    cfg.subjects = subjects;
    config::save(&cfg)?;
    Ok(cfg)
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
            apply_existing,
            get_rules,
            set_subject_enabled,
            upsert_subject,
            delete_subject,
            apply_subject_preset,
            default_downloads_path,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let state = app.state::<AppState>();
            watcher::try_auto_start(handle.clone(), &state.monitor);
            let status = state.monitor.status();
            let _ = handle.emit("monitor-status", status);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

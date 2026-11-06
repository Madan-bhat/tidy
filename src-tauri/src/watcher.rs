use crate::config;
use crate::mover;
use crate::stability;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorStatus {
    pub active: bool,
    pub watch_folder: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizeErrorEvent {
    pub path: String,
    pub message: String,
}

struct WatcherState {
    stop_tx: Option<mpsc::Sender<()>>,
    active: bool,
}

pub struct Monitor {
    inner: Mutex<WatcherState>,
}

impl Monitor {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(WatcherState {
                stop_tx: None,
                active: false,
            }),
        }
    }

    pub fn is_active(&self) -> bool {
        self.inner.lock().active
    }

    pub fn status(&self) -> MonitorStatus {
        let cfg = config::load();
        let active = self.is_active();
        MonitorStatus {
            active,
            watch_folder: cfg.watch_folder.clone(),
            message: if active {
                "Monitoring for new files".into()
            } else if !config::paths_ready(&cfg) {
                "Set watch and destination folders in Settings".into()
            } else {
                "Paused".into()
            },
        }
    }

    pub fn start(&self, app: AppHandle) -> Result<MonitorStatus, String> {
        let cfg = config::load();
        if !config::paths_ready(&cfg) {
            return Err("Watch and destination folders must be set and exist".into());
        }

        {
            let mut state = self.inner.lock();
            if state.active {
                return Ok(self.status());
            }

            let watch = cfg.watch_folder.clone().unwrap();
            let (stop_tx, stop_rx) = mpsc::channel::<()>();
            let watch_path = PathBuf::from(&watch);
            let app_clone = app.clone();

            thread::spawn(move || {
                run_watch_loop(app_clone, watch_path, stop_rx);
            });

            state.stop_tx = Some(stop_tx);
            state.active = true;
        }

        let mut cfg = config::load();
        cfg.monitoring_enabled = true;
        let _ = config::save(&cfg);

        let status = self.status();
        let _ = app.emit("monitor-status", &status);
        Ok(status)
    }

    pub fn stop(&self, app: &AppHandle) -> MonitorStatus {
        {
            let mut state = self.inner.lock();
            if let Some(tx) = state.stop_tx.take() {
                let _ = tx.send(());
            }
            state.active = false;
        }

        let mut cfg = config::load();
        cfg.monitoring_enabled = false;
        let _ = config::save(&cfg);

        let status = self.status();
        let _ = app.emit("monitor-status", &status);
        status
    }

    pub fn toggle(&self, app: AppHandle) -> Result<MonitorStatus, String> {
        if self.is_active() {
            Ok(self.stop(&app))
        } else {
            self.start(app)
        }
    }
}

fn run_watch_loop(app: AppHandle, watch_path: PathBuf, stop_rx: mpsc::Receiver<()>) {
    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();

    let mut watcher = match RecommendedWatcher::new(
        move |res| {
            let _ = tx.send(res);
        },
        notify::Config::default(),
    ) {
        Ok(w) => w,
        Err(e) => {
            let _ = app.emit(
                "organize-error",
                OrganizeErrorEvent {
                    path: watch_path.to_string_lossy().to_string(),
                    message: format!("Could not start watcher: {e}"),
                },
            );
            return;
        }
    };

    if let Err(e) = watcher.watch(&watch_path, RecursiveMode::NonRecursive) {
        let _ = app.emit(
            "organize-error",
            OrganizeErrorEvent {
                path: watch_path.to_string_lossy().to_string(),
                message: format!("Could not watch folder: {e}"),
            },
        );
        return;
    }

    let mut pending: HashSet<PathBuf> = HashSet::new();
    let processing: Arc<Mutex<HashSet<PathBuf>>> = Arc::new(Mutex::new(HashSet::new()));

    loop {
        match stop_rx.try_recv() {
            Ok(()) | Err(mpsc::TryRecvError::Disconnected) => break,
            Err(mpsc::TryRecvError::Empty) => {}
        }

        match rx.recv_timeout(Duration::from_millis(400)) {
            Ok(Ok(event)) => {
                if matches!(
                    event.kind,
                    EventKind::Create(_) | EventKind::Modify(_) | EventKind::Any
                ) {
                    for path in event.paths {
                        if path.parent() == Some(watch_path.as_path()) {
                            pending.insert(path);
                        }
                    }
                }
            }
            Ok(Err(e)) => {
                let _ = app.emit(
                    "organize-error",
                    OrganizeErrorEvent {
                        path: watch_path.to_string_lossy().to_string(),
                        message: format!("Watcher error: {e}"),
                    },
                );
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        let candidates: Vec<PathBuf> = pending.iter().cloned().collect();
        for path in candidates {
            if processing.lock().contains(&path) {
                continue;
            }
            if !path.exists() || !path.is_file() {
                pending.remove(&path);
                continue;
            }
            if stability::looks_incomplete(&path) {
                continue;
            }

            pending.remove(&path);
            processing.lock().insert(path.clone());

            let app2 = app.clone();
            let path2 = path.clone();
            let processing2 = processing.clone();
            thread::spawn(move || {
                process_new_file(&app2, &path2);
                processing2.lock().remove(&path2);
            });
        }
    }
}

fn process_new_file(app: &AppHandle, path: &Path) {
    if !stability::wait_until_stable(path) {
        return;
    }
    if !path.is_file() {
        return;
    }

    let cfg = config::load();
    if !cfg.monitoring_enabled || !config::paths_ready(&cfg) {
        return;
    }

    let watch = match &cfg.watch_folder {
        Some(w) => PathBuf::from(w),
        None => return,
    };
    if path.parent() != Some(watch.as_path()) {
        return;
    }

    // Never auto-move: propose and wait for user confirmation.
    match mover::plan_move_opts(path, &cfg, true) {
        Ok(planned) => {
            crate::pending::upsert(crate::organize_existing::PreviewItem {
                source: planned.source.to_string_lossy().to_string(),
                destination: planned.destination.to_string_lossy().to_string(),
                subject: planned.subject,
                file_type: planned.file_type,
                filename: planned.filename,
                from_subject: None,
                from_file_type: None,
            });
            let _ = app.emit("organize-pending", crate::pending::list());
        }
        Err(e) => {
            let _ = app.emit(
                "organize-error",
                OrganizeErrorEvent {
                    path: path.to_string_lossy().to_string(),
                    message: e,
                },
            );
        }
    }
}

pub fn try_auto_start(app: AppHandle, monitor: &Monitor) {
    let cfg = config::load();
    if cfg.monitor_on_launch && config::paths_ready(&cfg) {
        let _ = monitor.start(app);
    }
}

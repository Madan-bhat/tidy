//! Cooperative cancellation for long organize / reorganize scans.

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

static CANCELLED: AtomicBool = AtomicBool::new(false);
static GENERATION: AtomicU64 = AtomicU64::new(0);
static STARTED: Lazy<Mutex<Option<Instant>>> = Lazy::new(|| Mutex::new(None));

/// Begin a new cancellable job. Returns the generation token for this run.
pub fn begin() -> u64 {
    CANCELLED.store(false, Ordering::SeqCst);
    let g = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    *STARTED.lock() = Some(Instant::now());
    g
}

pub fn request() {
    CANCELLED.store(true, Ordering::SeqCst);
}

pub fn is_cancelled() -> bool {
    CANCELLED.load(Ordering::SeqCst)
}

pub fn end() {
    CANCELLED.store(false, Ordering::SeqCst);
    *STARTED.lock() = None;
}

/// Rough remaining-seconds estimate from progress so far. `None` until we have
/// enough samples to be useful.
pub fn eta_secs(current: usize, total: usize) -> Option<u32> {
    if current == 0 || total == 0 || current >= total {
        return None;
    }
    let started = STARTED.lock().clone()?;
    let elapsed = started.elapsed().as_secs_f64();
    if elapsed < 0.4 {
        return None;
    }
    let rate = current as f64 / elapsed;
    if rate <= 0.0 {
        return None;
    }
    let remaining = (total - current) as f64 / rate;
    Some(remaining.ceil().clamp(1.0, 3600.0) as u32)
}

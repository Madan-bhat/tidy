use std::path::Path;
use std::thread;
use std::time::Duration;

const INCOMPLETE_EXTENSIONS: &[&str] = &[
    "crdownload",
    "part",
    "tmp",
    "temp",
    "download",
    "partial",
    "opdownload",
];

const STABLE_CHECKS: u32 = 3;
const STABLE_INTERVAL_MS: u64 = 500;

pub fn looks_incomplete(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if name.starts_with('.') || name.starts_with('~') {
        return true;
    }

    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let ext = ext.to_ascii_lowercase();
        if INCOMPLETE_EXTENSIONS.contains(&ext.as_str()) {
            return true;
        }
    }

    // Chrome/Edge sometimes use "filename.crdownload" already covered;
    // also skip "Unconfirmed ..." Firefox temps.
    name.starts_with("unconfirmed ") || name.ends_with(".crdownload")
}

/// Wait until the file size is stable across several polls.
/// Returns false if the file disappears or never stabilizes within attempts.
pub fn wait_until_stable(path: &Path) -> bool {
    if looks_incomplete(path) {
        return false;
    }

    let mut last_size: Option<u64> = None;
    let mut stable_count = 0u32;
    let max_attempts = 60; // ~30s

    for _ in 0..max_attempts {
        if !path.exists() {
            return false;
        }
        if looks_incomplete(path) {
            return false;
        }

        let meta = match std::fs::metadata(path) {
            Ok(m) if m.is_file() => m,
            _ => return false,
        };
        let size = meta.len();

        if Some(size) == last_size {
            stable_count += 1;
            if stable_count >= STABLE_CHECKS {
                return true;
            }
        } else {
            last_size = Some(size);
            stable_count = 0;
        }

        thread::sleep(Duration::from_millis(STABLE_INTERVAL_MS));
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn detects_incomplete_downloads() {
        assert!(looks_incomplete(&PathBuf::from("file.crdownload")));
        assert!(looks_incomplete(&PathBuf::from("file.part")));
        assert!(looks_incomplete(&PathBuf::from(".hidden")));
        assert!(!looks_incomplete(&PathBuf::from("report.pdf")));
    }
}

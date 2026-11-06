use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Formats we can pull subject-matching text from.
pub fn can_extract(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "txt" | "md" | "tex" | "csv" | "pdf" | "docx" | "pptx"
    )
}

/// Extract a limited amount of text from supported document formats.
/// Never panics — corrupt / hung PDF parsers are timed out or swallowed.
pub fn extract_text(path: &Path) -> Option<String> {
    let text = extract_text_uncached(path)?;
    {
        let mut cache = SNIPPETS.lock();
        if cache.len() > 1500 {
            cache.clear();
        }
        let snippet: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
        cache.insert(path.to_path_buf(), snippet.chars().take(500).collect());
    }
    Some(text)
}

/// Short snippet from a previous successful extraction (used by the semantic pass).
pub fn peek_cached(path: &Path) -> Option<String> {
    SNIPPETS.lock().get(path).cloned()
}

static SNIPPETS: Lazy<Mutex<HashMap<PathBuf, String>>> = Lazy::new(|| Mutex::new(HashMap::new()));

fn extract_text_uncached(path: &Path) -> Option<String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match ext.as_str() {
        "txt" | "md" | "tex" | "csv" => extract_txt(path),
        "pdf" => extract_pdf_timed(path, Duration::from_millis(4000)),
        "docx" => extract_docx(path),
        "pptx" => extract_pptx(path),
        _ => None,
    }
}

fn extract_txt(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    if bytes.len() > 2_000_000 {
        return Some(String::from_utf8_lossy(&bytes[..8000]).to_string());
    }
    let text = String::from_utf8_lossy(&bytes);
    Some(text.chars().take(8000).collect())
}

/// Run PDF parse on a helper thread and abandon it after `limit`.
/// A hung parser must not block bulk matching forever.
fn extract_pdf_timed(path: &Path, limit: Duration) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    // Smaller cap for responsiveness during bulk scans.
    if meta.len() > 12_000_000 {
        return None;
    }

    let path = path.to_path_buf();
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("tidy-pdf-extract".into())
        .spawn(move || {
            let result = extract_pdf_safe(&path);
            let _ = tx.send(result);
        })
        .ok()?;

    match rx.recv_timeout(limit) {
        Ok(text) => text,
        Err(_) => {
            // Thread may still be running; we move on so matching can finish.
            None
        }
    }
}

fn extract_pdf_safe(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        pdf_extract::extract_text_from_mem(&bytes).ok()
    }));

    match result {
        Ok(Some(text)) => Some(text.chars().take(8000).collect()),
        _ => None,
    }
}

fn strip_xml(xml: &str, out: &mut String, limit: usize) {
    let mut in_tag = false;
    for ch in xml.chars() {
        match ch {
            '<' => {
                in_tag = true;
                // Keep word boundaries between runs/paragraphs.
                if !out.ends_with(' ') {
                    out.push(' ');
                }
            }
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
        if out.len() > limit {
            break;
        }
    }
}

fn extract_docx(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut document = archive.by_name("word/document.xml").ok()?;
    let mut xml = String::new();
    document.read_to_string(&mut xml).ok()?;

    let mut text = String::new();
    strip_xml(&xml, &mut text, 8000);
    Some(text)
}

/// PowerPoint (.pptx): text of the first slides (titles / outline carry the subject).
fn extract_pptx(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut slides: Vec<(u32, String)> = archive
        .file_names()
        .filter_map(|n| {
            let n = n.ok()?;
            let num = n
                .strip_prefix("ppt/slides/slide")?
                .strip_suffix(".xml")?
                .parse::<u32>()
                .ok()?;
            Some((num, n.to_string()))
        })
        .collect();
    slides.sort();

    let mut text = String::new();
    for (_, name) in slides.into_iter().take(10) {
        let Ok(mut entry) = archive.by_name(&name) else {
            continue;
        };
        let mut xml = String::new();
        if entry.read_to_string(&mut xml).is_err() {
            continue;
        }
        strip_xml(&xml, &mut text, 8000);
        text.push(' ');
        if text.len() > 8000 {
            break;
        }
    }
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

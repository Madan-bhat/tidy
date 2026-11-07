//! Optional OCR for images and image-heavy PDFs.
//!
//! Uses tools already on the machine when present:
//! - `tesseract` for recognition (Homebrew / apt / chocolatey)
//! - `pdftoppm` (poppler) or macOS `qlmanage` to rasterise the first page of a PDF
//!
//! If none of those are available, OCR is a no-op and matching falls back to
//! filename / embedded text. Nothing is hardcoded about languages beyond asking
//! tesseract for English when no `TESSDATA_PREFIX` language is set — users can
//! export `TIDY_OCR_LANG` (e.g. `eng+deu`) to override.

use once_cell::sync::Lazy;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

static TESSERACT_OK: Lazy<AtomicBool> = Lazy::new(|| AtomicBool::new(probe_tesseract()));
static WARNED: AtomicBool = AtomicBool::new(false);

fn probe_tesseract() -> bool {
    Command::new("tesseract")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn is_available() -> bool {
    TESSERACT_OK.load(Ordering::Relaxed)
}

fn ocr_lang() -> String {
    std::env::var("TIDY_OCR_LANG").unwrap_or_else(|_| "eng".into())
}

/// Recognise text in an image file. Returns `None` when OCR isn't installed or fails.
pub fn ocr_image(path: &Path) -> Option<String> {
    if !is_available() {
        maybe_warn();
        return None;
    }
    let meta = fs::metadata(path).ok()?;
    // Keep OCR snappy during bulk scans.
    if meta.len() > 15_000_000 {
        return None;
    }
    run_tesseract(path, Duration::from_secs(8))
}

/// When a PDF has almost no extractable text, rasterise page 1 and OCR it.
pub fn ocr_pdf_first_page(path: &Path) -> Option<String> {
    if !is_available() {
        maybe_warn();
        return None;
    }
    let meta = fs::metadata(path).ok()?;
    if meta.len() > 25_000_000 {
        return None;
    }
    let tmp = tempfile_dir()?;
    let raster = rasterise_pdf_page(path, &tmp)?;
    let text = run_tesseract(&raster, Duration::from_secs(12));
    let _ = fs::remove_dir_all(&tmp);
    text
}

fn maybe_warn() {
    if WARNED.swap(true, Ordering::Relaxed) {
        return;
    }
    eprintln!(
        "[tidy] OCR unavailable — install tesseract to read screenshots and scanned PDFs \
         (macOS: brew install tesseract; Linux: sudo apt install tesseract-ocr; \
         Windows: choco install tesseract)"
    );
}

fn tempfile_dir() -> Option<PathBuf> {
    let base = std::env::temp_dir().join(format!(
        "tidy-ocr-{}-{}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    fs::create_dir_all(&base).ok()?;
    Some(base)
}

fn rasterise_pdf_page(pdf: &Path, out_dir: &Path) -> Option<PathBuf> {
    // Prefer poppler's pdftoppm when present (Linux / Homebrew).
    let prefix = out_dir.join("page");
    if Command::new("pdftoppm")
        .args(["-f", "1", "-l", "1", "-png", "-r", "150"])
        .arg(pdf)
        .arg(&prefix)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        let candidate = out_dir.join("page-1.png");
        if candidate.is_file() {
            return Some(candidate);
        }
        // Some builds write page1.png
        let alt = out_dir.join("page1.png");
        if alt.is_file() {
            return Some(alt);
        }
    }

    // macOS fallback: Quick Look thumbnail of the PDF.
    #[cfg(target_os = "macos")]
    {
        let status = Command::new("qlmanage")
            .args(["-t", "-s", "1400", "-o"])
            .arg(out_dir)
            .arg(pdf)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if status.map(|s| s.success()).unwrap_or(false) {
            if let Ok(entries) = fs::read_dir(out_dir) {
                for e in entries.flatten() {
                    let p = e.path();
                    if p.extension().and_then(|x| x.to_str()) == Some("png") {
                        return Some(p);
                    }
                }
            }
        }
    }

    None
}

fn run_tesseract(image: &Path, limit: Duration) -> Option<String> {
    let image = image.to_path_buf();
    let lang = ocr_lang();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("tidy-ocr".into())
        .spawn(move || {
            let output = Command::new("tesseract")
                .arg(&image)
                .arg("stdout")
                .args(["-l", &lang])
                .args(["--psm", "6"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .output();
            let _ = tx.send(output.ok().and_then(|o| {
                if !o.status.success() {
                    return None;
                }
                let text = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if text.chars().filter(|c| c.is_alphanumeric()).count() < 4 {
                    None
                } else {
                    Some(text.chars().take(4000).collect())
                }
            }));
        })
        .ok()?;

    match rx.recv_timeout(limit) {
        Ok(text) => text,
        Err(_) => None,
    }
}


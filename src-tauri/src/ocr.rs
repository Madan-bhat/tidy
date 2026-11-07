//! Bundled OCR — ships with the app. Users do not install tesseract.
//!
//! Uses the pure-Rust [`ocrs`] engine with ONNX models packaged under
//! `resources/ocr/`. Optional fallbacks: system `tesseract` if present, and on
//! macOS Quick Look for rasterising PDF pages (built into macOS).

use once_cell::sync::Lazy;
use ocrs::{ImageSource, OcrEngine, OcrEngineParams};
use parking_lot::Mutex;
use rten::Model;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

static ENGINE: Lazy<Mutex<Option<OcrEngine>>> = Lazy::new(|| Mutex::new(None));
static INIT_FAILED: AtomicBool = AtomicBool::new(false);
static READY: AtomicBool = AtomicBool::new(false);

/// True when bundled models are on disk (or system tesseract is available).
pub fn is_available() -> bool {
    READY.load(Ordering::Relaxed) || models_dir().is_some() || tesseract_on_path()
}

/// Whether the app-shipped models are present (not the optional tesseract fallback).
pub fn bundled_available() -> bool {
    models_dir().is_some()
}

fn tesseract_on_path() -> bool {
    Command::new("tesseract")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Locate the directory that contains `text-detection.onnx` + `text-recognition.onnx`.
pub fn models_dir() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("TIDY_OCR_MODELS") {
        let p = PathBuf::from(p);
        if has_models(&p) {
            return Some(p);
        }
    }

    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // Dev / sidecar layouts
            candidates.push(dir.join("resources").join("ocr"));
            candidates.push(dir.join("ocr"));
            // macOS .app: Contents/MacOS/../../Resources/resources/ocr
            //             Contents/MacOS/../Resources/ocr
            candidates.push(dir.join("../Resources/resources/ocr"));
            candidates.push(dir.join("../Resources/ocr"));
            candidates.push(dir.join("../../Resources/resources/ocr"));
            // Linux AppImage / deb style
            candidates.push(dir.join("../lib/tidy/resources/ocr"));
            candidates.push(dir.join("../resources/ocr"));
        }
    }
    if let Some(cfg) = dirs::config_dir() {
        candidates.push(cfg.join("Tidy").join("ocr-models"));
    }
    // Workspace path when running `cargo test` / `cargo run` from src-tauri
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/ocr"));

    for c in candidates {
        if let Ok(canon) = c.canonicalize() {
            if has_models(&canon) {
                return Some(canon);
            }
        } else if has_models(&c) {
            return Some(c);
        }
    }
    None
}

fn has_models(dir: &Path) -> bool {
    dir.join("text-detection.onnx").is_file() && dir.join("text-recognition.onnx").is_file()
}

fn ensure_engine() -> Result<(), String> {
    if INIT_FAILED.load(Ordering::Relaxed) {
        return Err("OCR init previously failed".into());
    }
    {
        let guard = ENGINE.lock();
        if guard.is_some() {
            READY.store(true, Ordering::Relaxed);
            return Ok(());
        }
    }

    let dir = models_dir().ok_or_else(|| {
        INIT_FAILED.store(true, Ordering::Relaxed);
        "Bundled OCR models not found".to_string()
    })?;

    let detection = Model::load_file(dir.join("text-detection.onnx"))
        .map_err(|e| format!("OCR detection model: {e}"))?;
    let recognition = Model::load_file(dir.join("text-recognition.onnx"))
        .map_err(|e| format!("OCR recognition model: {e}"))?;

    let engine = OcrEngine::new(OcrEngineParams {
        detection_model: Some(detection),
        recognition_model: Some(recognition),
        ..Default::default()
    })
    .map_err(|e| format!("OCR engine: {e}"))?;

    *ENGINE.lock() = Some(engine);
    READY.store(true, Ordering::Relaxed);
    eprintln!("[tidy] bundled OCR ready ({})", dir.display());
    Ok(())
}

/// Prefetch models on a background thread so the first scan isn't cold.
pub fn warm_up() {
    let _ = ensure_engine();
}

fn run_ocrs_on_image_bytes(rgb: &[u8], width: u32, height: u32) -> Option<String> {
    ensure_engine().ok()?;
    let guard = ENGINE.lock();
    let engine = guard.as_ref()?;
    let source = ImageSource::from_bytes(rgb, (width, height)).ok()?;
    let input = engine.prepare_input(source).ok()?;
    let text = engine.get_text(&input).ok()?;
    let trimmed = text.trim().to_string();
    if trimmed.chars().filter(|c| c.is_alphanumeric()).count() < 4 {
        None
    } else {
        Some(trimmed.chars().take(4000).collect())
    }
}

fn ocr_image_path_ocrs(path: &Path) -> Option<String> {
    let img = image::open(path).ok()?.into_rgb8();
    let (w, h) = img.dimensions();
    // Skip enormous images — downscale first.
    let img = if w > 2400 || h > 2400 {
        let scale = 2400f32 / w.max(h) as f32;
        let nw = ((w as f32) * scale) as u32;
        let nh = ((h as f32) * scale) as u32;
        image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let (w, h) = img.dimensions();
    run_ocrs_on_image_bytes(img.as_raw(), w, h)
}

fn ocr_image_path_tesseract(path: &Path) -> Option<String> {
    if !tesseract_on_path() {
        return None;
    }
    let lang = std::env::var("TIDY_OCR_LANG").unwrap_or_else(|_| "eng".into());
    let path = path.to_path_buf();
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("tidy-ocr-tess".into())
        .spawn(move || {
            let output = Command::new("tesseract")
                .arg(&path)
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
    rx.recv_timeout(Duration::from_secs(8)).ok().flatten()
}

/// Recognise text in an image file. Prefer bundled ocrs; fall back to tesseract.
pub fn ocr_image(path: &Path) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    if meta.len() > 20_000_000 {
        return None;
    }

    let path = path.to_path_buf();
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("tidy-ocr".into())
        .spawn(move || {
            let result = ocr_image_path_ocrs(&path).or_else(|| ocr_image_path_tesseract(&path));
            let _ = tx.send(result);
        })
        .ok()?;

    rx.recv_timeout(Duration::from_secs(20)).ok().flatten()
}

/// When a PDF has almost no extractable text, OCR the first page / embedded images.
pub fn ocr_pdf_first_page(path: &Path) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    if meta.len() > 30_000_000 {
        return None;
    }

    // 1) Images embedded in the PDF (common for phone scans) — no external tools.
    if let Some(text) = ocr_pdf_embedded_images(path) {
        return Some(text);
    }

    // 2) Rasterise page 1 with a built-in / common tool, then OCR.
    let tmp = tempfile_dir()?;
    let raster = rasterise_pdf_page(path, &tmp)?;
    let text = ocr_image(&raster);
    let _ = fs::remove_dir_all(&tmp);
    text
}

fn ocr_pdf_embedded_images(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    // Minimal PDF image hunt: pull the largest JPEG/PNG streams and OCR them.
    let mut images: Vec<Vec<u8>> = Vec::new();
    extract_pdf_image_bytes(&bytes, &mut images);
    images.sort_by_key(|b| std::cmp::Reverse(b.len()));
    let mut combined = String::new();
    for img_bytes in images.into_iter().take(3) {
        if img_bytes.len() < 8_000 {
            continue;
        }
        let Ok(img) = image::load_from_memory(&img_bytes) else {
            continue;
        };
        let rgb = img.into_rgb8();
        let (w, h) = rgb.dimensions();
        if let Some(t) = run_ocrs_on_image_bytes(rgb.as_raw(), w, h) {
            if !combined.is_empty() {
                combined.push('\n');
            }
            combined.push_str(&t);
            if combined.len() > 3500 {
                break;
            }
        }
    }
    if combined.chars().filter(|c| c.is_alphanumeric()).count() >= 4 {
        Some(combined.chars().take(4000).collect())
    } else {
        None
    }
}

/// Find JPEG / PNG payloads inside a PDF by magic bytes (no PDF library needed).
fn extract_pdf_image_bytes(pdf: &[u8], out: &mut Vec<Vec<u8>>) {
    let mut i = 0;
    while i + 3 < pdf.len() {
        // JPEG
        if pdf[i] == 0xFF && pdf[i + 1] == 0xD8 && pdf[i + 2] == 0xFF {
            if let Some(end) = find_jpeg_end(pdf, i) {
                out.push(pdf[i..=end].to_vec());
                i = end + 1;
                continue;
            }
        }
        // PNG
        if pdf[i..].starts_with(b"\x89PNG\r\n\x1a\n") {
            if let Some(end) = find_png_end(pdf, i) {
                out.push(pdf[i..=end].to_vec());
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }
}

fn find_jpeg_end(data: &[u8], start: usize) -> Option<usize> {
    let mut i = start + 2;
    while i + 1 < data.len() {
        if data[i] == 0xFF && data[i + 1] == 0xD9 {
            return Some(i + 1);
        }
        i += 1;
        // Cap search so a corrupt stream can't scan the whole file for every SOI.
        if i - start > 12_000_000 {
            break;
        }
    }
    None
}

fn find_png_end(data: &[u8], start: usize) -> Option<usize> {
    // Walk PNG chunks until IEND.
    let mut i = start + 8;
    while i + 8 <= data.len() {
        let len = u32::from_be_bytes(data[i..i + 4].try_into().ok()?) as usize;
        let ty = &data[i + 4..i + 8];
        let chunk_end = i + 8 + len + 4; // data + CRC
        if chunk_end > data.len() {
            return None;
        }
        if ty == b"IEND" {
            return Some(chunk_end - 1);
        }
        i = chunk_end;
        if i - start > 12_000_000 {
            break;
        }
    }
    None
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
        for name in ["page-1.png", "page1.png"] {
            let p = out_dir.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        // Built into macOS — no Homebrew needed.
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

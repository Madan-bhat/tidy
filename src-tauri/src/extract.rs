use std::fs;
use std::io::Read;
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;

/// Extract a limited amount of text from supported document formats.
/// Never panics — corrupt PDFs from `pdf-extract` are treated as no text.
pub fn extract_text(path: &Path) -> Option<String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match ext.as_str() {
        "txt" => extract_txt(path),
        "pdf" => extract_pdf_safe(path),
        "docx" => extract_docx(path),
        _ => None,
    }
}

fn extract_txt(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    // Skip huge text dumps for responsiveness.
    if bytes.len() > 2_000_000 {
        return Some(String::from_utf8_lossy(&bytes[..8000]).to_string());
    }
    let text = String::from_utf8_lossy(&bytes);
    Some(text.chars().take(8000).collect())
}

fn extract_pdf_safe(path: &Path) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    // Large PDFs are slow and more likely to trip buggy parsers — skip.
    if meta.len() > 8_000_000 {
        return None;
    }

    let bytes = fs::read(path).ok()?;
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        pdf_extract::extract_text_from_mem(&bytes).ok()
    }));

    match result {
        Ok(Some(text)) => Some(text.chars().take(8000).collect()),
        _ => None,
    }
}

fn extract_docx(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut document = archive.by_name("word/document.xml").ok()?;
    let mut xml = String::new();
    document.read_to_string(&mut xml).ok()?;

    let mut text = String::new();
    let mut in_tag = false;
    for ch in xml.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => text.push(ch),
            _ => {}
        }
        if text.len() > 8000 {
            break;
        }
    }
    Some(text)
}

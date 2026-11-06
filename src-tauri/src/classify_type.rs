use std::path::Path;

/// Maps file extensions to destination type folder names.
pub fn classify_type(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match ext.as_str() {
        "pdf" | "docx" | "txt" => "Documents",
        "jpg" | "jpeg" | "png" | "svg" | "webp" => "Images",
        "mp4" | "mkv" | "mov" => "Videos",
        "mp3" | "wav" | "flac" => "Audio",
        "zip" | "rar" | "7z" | "tar" => "Archives",
        "xlsx" | "csv" => "Spreadsheets",
        "pptx" | "ppt" => "Presentations",
        "py" | "js" | "ts" | "c" | "cpp" | "java" | "v" => "Code",
        "exe" | "dmg" | "pkg" | "msi" => "Installers",
        _ => "Other",
    }
}

pub fn type_rules() -> Vec<(Vec<&'static str>, &'static str)> {
    vec![
        (vec!["pdf", "docx", "txt"], "Documents"),
        (vec!["jpg", "jpeg", "png", "svg", "webp"], "Images"),
        (vec!["mp4", "mkv", "mov"], "Videos"),
        (vec!["mp3", "wav", "flac"], "Audio"),
        (vec!["zip", "rar", "7z", "tar"], "Archives"),
        (vec!["xlsx", "csv"], "Spreadsheets"),
        (vec!["pptx", "ppt"], "Presentations"),
        (vec!["py", "js", "ts", "c", "cpp", "java", "v"], "Code"),
        (vec!["exe", "dmg", "pkg", "msi"], "Installers"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn maps_known_and_unknown_extensions() {
        assert_eq!(classify_type(Path::new("notes.PDF")), "Documents");
        assert_eq!(classify_type(Path::new("shot.webp")), "Images");
        assert_eq!(classify_type(Path::new("app.dmg")), "Installers");
        assert_eq!(classify_type(Path::new("mystery.xyz")), "Other");
    }
}

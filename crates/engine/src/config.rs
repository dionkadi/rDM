//! Settings persistence + sensible default categories.

use crate::model::{Category, Settings};
use crate::storage::Storage;
use std::path::PathBuf;

/// Default category routing by file extension.
pub fn default_categories() -> Vec<Category> {
    let mk = |id: &str, name: &str, exts: &[&str], sub: &str| Category {
        id: id.into(),
        name: name.into(),
        extensions: exts.iter().map(|s| s.to_string()).collect(),
        directory: default_subdir(sub),
    };
    vec![
        mk("video", "Videos", &["mp4", "mkv", "avi", "mov", "webm", "m4v", "flv"], "Videos"),
        mk("audio", "Music", &["mp3", "flac", "wav", "ogg", "m4a", "aac"], "Music"),
        mk("compressed", "Compressed", &["zip", "rar", "7z", "tar", "gz", "bz2", "xz"], "Compressed"),
        mk("doc", "Documents", &["pdf", "epub", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt"], "Documents"),
        mk("program", "Programs", &["exe", "msi", "dmg", "apk", "deb", "rpm", "appimage"], "Programs"),
        mk("image", "Images", &["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp"], "Images"),
    ]
}

fn default_subdir(sub: &str) -> PathBuf {
    directories::UserDirs::new()
        .map(|u| u.download_dir().unwrap_or_else(|| u.home_dir()).join(sub))
        .unwrap_or_else(|| PathBuf::from(format!("downloads/{sub}")))
}

/// Pick the category whose extension list contains `ext` (case-insensitive).
pub fn categorize(categories: &[Category], filename: &str) -> Option<String> {
    let lower = filename.to_ascii_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("").to_string();
    if ext.is_empty() {
        return None;
    }
    for c in categories {
        if c.extensions.iter().any(|e| e.eq_ignore_ascii_case(&ext)) {
            return Some(c.id.clone());
        }
    }
    None
}

/// Load settings from storage, seeding defaults + categories on first run.
pub fn load_or_default(storage: &Storage) -> Settings {
    if let Ok(Some(mut s)) = storage.get_settings() {
        if s.categories.is_empty() {
            s.categories = default_categories();
            let _ = storage.save_categories(&s.categories);
        }
        return s;
    }
    let mut s = Settings::default();
    s.categories = default_categories();
    let _ = storage.save_settings(&s);
    let _ = storage.save_categories(&s.categories);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categorizes_by_extension() {
        let cats = default_categories();
        assert_eq!(categorize(&cats, "movie.MP4"), Some("video".into()));
        assert_eq!(categorize(&cats, "song.mp3"), Some("audio".into()));
        assert_eq!(categorize(&cats, "archive.zip"), Some("compressed".into()));
        assert_eq!(categorize(&cats, "readme"), None);
    }
}

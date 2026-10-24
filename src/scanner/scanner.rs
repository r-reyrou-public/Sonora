use std::path::Path;
use walkdir::WalkDir;

/// フォルダ内の音声ファイルのパスだけを列挙する（タグは読まない。ごく高速）
pub fn list_audio_files(folder: &Path, include_subfolders: bool) -> Vec<std::path::PathBuf> {
    let mut file_paths = Vec::new();

    if include_subfolders {
        for entry in WalkDir::new(folder).into_iter().filter_map(|e| e.ok()) {
            if entry.file_type().is_file() {
                let p = entry.path();
                if is_audio_file(p) {
                    file_paths.push(p.to_path_buf());
                }
            }
        }
    } else if let Ok(entries) = std::fs::read_dir(folder) {
        for entry in entries.flatten() {
            if let Ok(ft) = entry.file_type() {
                if ft.is_file() {
                    let p = entry.path();
                    if is_audio_file(&p) {
                        file_paths.push(p);
                    }
                }
            }
        }
    }

    file_paths
}

fn is_audio_file(path: &Path) -> bool {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        matches!(
            ext.to_ascii_lowercase().as_str(),
            "mp3" | "flac" | "m4a" | "aac" | "ogg" | "opus" | "wav" | "wma" | "aiff" | "alac"
        )
    } else {
        false
    }
}

use crate::scanner::metadata::TrackItem;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenamePattern {
    ArtistTitle,
    TitleOnly,
    AlbumArtistTitle,
    NumberedTitle,
}

#[derive(Clone, Debug)]
pub struct RenamePlanItem {
    pub original_path: PathBuf,
    pub current_name: String,
    pub new_name: String,
    pub new_path: PathBuf,
}

pub fn generate_rename_plan(tracks: &[TrackItem], pattern: RenamePattern) -> Vec<RenamePlanItem> {
    let mut plan = Vec::new();

    for (idx, track) in tracks.iter().enumerate() {
        let parent = track.file_path.parent().unwrap_or_else(|| std::path::Path::new("."));
        let ext = track.file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let title = if track.title.trim().is_empty() {
            &track.filename
        } else {
            &track.title
        };
        let artist = track.artist.trim();
        let album = track.album.trim();

        let base_name = match pattern {
            RenamePattern::ArtistTitle => {
                if !artist.is_empty() {
                    format!("{} - {}", artist, title)
                } else {
                    title.to_string()
                }
            }
            RenamePattern::TitleOnly => title.to_string(),
            RenamePattern::AlbumArtistTitle => {
                if !album.is_empty() && !artist.is_empty() {
                    format!("{} - {} - {}", album, artist, title)
                } else if !artist.is_empty() {
                    format!("{} - {}", artist, title)
                } else {
                    title.to_string()
                }
            }
            RenamePattern::NumberedTitle => format!("{:02}_{}", idx + 1, title),
        };

        let sanitized = sanitize_filename(&base_name);
        let new_name = if ext.is_empty() {
            sanitized
        } else {
            format!("{}.{}", sanitized, ext)
        };
        let new_path = parent.join(&new_name);

        plan.push(RenamePlanItem {
            original_path: track.file_path.clone(),
            current_name: track.filename.clone(),
            new_name,
            new_path,
        });
    }

    plan
}

pub fn execute_rename_plan(plan: &[RenamePlanItem]) -> (usize, Vec<String>) {
    let mut success_count = 0;
    let mut errors = Vec::new();

    for item in plan {
        if item.original_path == item.new_path {
            continue;
        }
        if item.new_path.exists() {
            errors.push(format!("既に同名ファイルが存在します: {}", item.new_name));
            continue;
        }
        match std::fs::rename(&item.original_path, &item.new_path) {
            Ok(_) => success_count += 1,
            Err(e) => errors.push(format!("{}: {}", item.current_name, e)),
        }
    }

    (success_count, errors)
}

/// Windows で使えるファイル名に整える（禁止文字・制御文字・末尾の . や空白・予約名・長さ）
fn sanitize_filename(name: &str) -> String {
    let invalid = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    let cleaned: String = name.chars().map(|c| if invalid.contains(&c) || c.is_control() { '_' } else { c }).collect();
    let mut s = cleaned.trim().trim_end_matches(['.', ' ']).to_string();
    if s.chars().count() > 150 {
        s = s.chars().take(150).collect::<String>().trim_end().to_string();
    }
    let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit());
    if s.is_empty() {
        "_".to_string()
    } else if reserved {
        format!("_{}", s)
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_filename;

    #[test]
    fn sanitize_dangerous_names() {
        assert_eq!(sanitize_filename("a/b\\c:d"), "a_b_c_d");
        assert_eq!(sanitize_filename(".."), "_");
        assert_eq!(sanitize_filename("name. . "), "name");
        assert_eq!(sanitize_filename("CON"), "_CON");
        assert_eq!(sanitize_filename("nul.txt"), "_nul.txt");
        assert_eq!(sanitize_filename("COM1"), "_COM1");
        assert_eq!(sanitize_filename("a\u{0007}b"), "a_b");
        assert_eq!(sanitize_filename(&"あ".repeat(400)).chars().count(), 150);
    }
}

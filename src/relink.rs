//! アプリ外でファイルを移動・名前変更して外れたラベル／プレイリストの紐付けを復旧する（C# 版 RelinkService 相当）。
//! 照合ルール: 見失ったファイルと同じファイル名の音楽ファイルが検索フォルダ内に「1件だけ」ある場合のみ付け替える
//! （同名の候補が複数ある場合は誤った紐付けを避けるため変更しない）。
use crate::db::labels::CustomTagService;
use crate::playlist::PlaylistService;
use crate::scanner::metadata::is_audio_ext;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_SCAN_FILES: usize = 200_000;

#[derive(Default)]
pub struct RelinkResult {
    pub missing_count: usize,
    /// 見失ったパス -> 新しいパス
    pub relinked: Vec<(String, String)>,
    pub ambiguous: usize,
    pub not_found: usize,
    pub limit_reached: bool,
}

/// ラベル・プレイリストが参照しているが存在しないファイルを列挙する（I/O を伴うのでバックグラウンドで呼ぶ）
pub fn find_missing_files(tags: &CustomTagService, playlists: &PlaylistService, cancel: &AtomicBool) -> Vec<String> {
    let mut all: Vec<String> = tags.all_tagged_paths();
    all.extend(playlists.all_referenced_paths());
    // 表記違い（大文字小文字・区切り文字）の同じファイルは1件にまとめる
    let key = |p: &String| crate::store::path_key(Path::new(p));
    all.sort_by_key(key);
    all.dedup_by_key(|p| key(p));

    let mut missing = Vec::new();
    for p in all {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if !Path::new(&p).exists() {
            missing.push(p);
        }
    }
    missing
}

pub fn find_new_locations(missing: &[String], search_root: &Path, cancel: &AtomicBool) -> RelinkResult {
    let mut result = RelinkResult { missing_count: missing.len(), ..Default::default() };
    if missing.is_empty() {
        return result;
    }

    // 検索フォルダ内の音楽ファイルを、ファイル名（小文字）で索引化
    let mut index: HashMap<String, Vec<PathBuf>> = HashMap::new();
    let mut scanned = 0usize;
    for entry in walkdir::WalkDir::new(search_root).into_iter().filter_map(|e| e.ok()) {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if !entry.file_type().is_file() || !is_audio_ext(entry.path()) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_lowercase();
        index.entry(name).or_default().push(entry.path().to_path_buf());
        scanned += 1;
        if scanned >= MAX_SCAN_FILES {
            result.limit_reached = true;
            break;
        }
    }

    for m in missing {
        let name = Path::new(m)
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        match index.get(&name).map(|v| v.as_slice()) {
            None | Some([]) => result.not_found += 1,
            Some([one]) => result.relinked.push((m.clone(), one.to_string_lossy().to_string())),
            Some(_) => result.ambiguous += 1,
        }
    }
    result
}

pub fn apply(result: &RelinkResult, tags: &CustomTagService, playlists: &PlaylistService) {
    for (old, new) in &result.relinked {
        tags.relink_path(old, new);
    }
    playlists.relink_paths(&result.relinked);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn relink_moved_files() {
        let base = std::env::temp_dir().join(format!("relink_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("new/sub")).unwrap();
        std::fs::create_dir_all(base.join("new/dup1")).unwrap();
        std::fs::create_dir_all(base.join("new/dup2")).unwrap();
        std::fs::write(base.join("new/sub/moved.mp3"), b"x").unwrap();
        std::fs::write(base.join("new/dup1/twin.mp3"), b"x").unwrap();
        std::fs::write(base.join("new/dup2/twin.mp3"), b"x").unwrap();
        std::fs::write(base.join("exists.mp3"), b"x").unwrap();

        let store = Store::open(&base.join("lib.bin"));
        let tags = CustomTagService::new(store.clone());
        let lists = PlaylistService::new(store.clone());

        let old_moved = base.join("old/moved.mp3");
        let old_twin = base.join("old/twin.mp3");
        let old_gone = base.join("old/gone.mp3");
        let still = base.join("exists.mp3");
        tags.set_labels_for_file(&old_moved, &["fav".to_string()]);
        tags.set_labels_for_file(&old_twin, &["x".to_string()]);
        tags.set_labels_for_file(&old_gone, &["y".to_string()]);
        tags.set_labels_for_file(&still, &["ok".to_string()]);
        let pid = lists.create_playlist("pl").unwrap();
        lists.add_track(pid, &old_moved).unwrap();

        let cancel = AtomicBool::new(false);
        let missing = find_missing_files(&tags, &lists, &cancel);
        assert_eq!(missing.len(), 3, "存在しない3件だけ（exists.mp3 は含まない）: {:?}", missing);

        let result = find_new_locations(&missing, &base.join("new"), &cancel);
        assert_eq!(result.missing_count, 3);
        assert_eq!(result.relinked.len(), 1); // moved.mp3 だけ（同名が1件）
        assert_eq!(result.ambiguous, 1); // twin.mp3 は候補が2件 → 変更しない
        assert_eq!(result.not_found, 1); // gone.mp3

        apply(&result, &tags, &lists);
        assert_eq!(tags.get_labels_for_file(&base.join("new/sub/moved.mp3")), ["fav"]);
        assert!(tags.get_labels_for_file(&old_moved).is_empty());
        assert_eq!(lists.get_playlist_tracks(pid), [base.join("new/sub/moved.mp3")]);
        assert_eq!(tags.get_labels_for_file(&old_twin), ["x"]); // 触らない
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn folder_labels_apply_to_all_files() {
        let base = std::env::temp_dir().join(format!("labels_test_dir_{}.bin", std::process::id()));
        let store = Store::open(&base);
        let tags = CustomTagService::new(store);
        let p = std::path::Path::new;
        let files = vec![std::path::PathBuf::from("C:/m/a/1.mp3"), std::path::PathBuf::from("C:/m/a/2.mp3")];
        tags.set_labels_for_file(&files[1], &["own".to_string()]); // 元から付いているラベルは残す

        tags.apply_folder_labels(p("C:/m/a"), &files, &["bgm".to_string()], &[]);
        assert_eq!(tags.get_labels_for_file(p("C:/m/a")), ["bgm"]);
        assert_eq!(tags.get_labels_for_file(&files[0]), ["bgm"]);
        assert_eq!(tags.get_labels_for_file(&files[1]), ["own", "bgm"]);

        tags.apply_folder_labels(p("C:/m/a"), &files, &[], &["bgm".to_string()]); // 一括で外す
        assert!(tags.get_labels_for_file(p("C:/m/a")).is_empty());
        assert!(tags.get_labels_for_file(&files[0]).is_empty());
        assert_eq!(tags.get_labels_for_file(&files[1]), ["own"]);
        let _ = std::fs::remove_file(&base);
    }

    #[test]
    fn rename_and_usage_counts() {
        let base = std::env::temp_dir().join(format!("labels_test_{}.bin", std::process::id()));
        let store = Store::open(&base);
        let tags = CustomTagService::new(store);
        tags.add_preset_label("unused");
        tags.set_labels_for_file(std::path::Path::new("C:/a.mp3"), &["rock".to_string(), "live".to_string()]);
        tags.set_labels_for_file(std::path::Path::new("C:/b.mp3"), &["rock".to_string()]);
        let usage: std::collections::HashMap<_, _> = tags.get_labels_with_usage().into_iter().collect();
        assert_eq!(usage["rock"], 2);
        assert_eq!(usage["live"], 1);
        assert_eq!(usage["unused"], 0);

        tags.rename_label("rock", "live"); // 既存ラベルへの統合
        assert_eq!(tags.get_labels_for_file(std::path::Path::new("C:/a.mp3")), ["live"]);
        assert_eq!(tags.get_labels_for_file(std::path::Path::new("C:/b.mp3")), ["live"]);
        tags.delete_preset_label("live");
        assert!(tags.get_labels_for_file(std::path::Path::new("C:/a.mp3")).is_empty());
        let _ = std::fs::remove_file(&base);
    }
}

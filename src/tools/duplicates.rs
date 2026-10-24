use crate::scanner::metadata::TrackItem;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct DuplicateGroup {
    pub label: String,
    pub reason: String,
    pub tracks: Vec<TrackItem>,
}

pub fn find_duplicates(tracks: &[TrackItem]) -> Vec<DuplicateGroup> {
    let mut groups = Vec::new();
    let mut group_num = 1;

    // 1. 同一ファイルサイズによるグループ化
    let mut size_map: HashMap<u64, Vec<TrackItem>> = HashMap::new();
    for t in tracks {
        if t.file_size_bytes > 0 {
            size_map.entry(t.file_size_bytes).or_default().push(t.clone());
        }
    }

    let mut size_duplicates = Vec::new();
    for (size, list) in size_map {
        if list.len() > 1 {
            size_duplicates.push((size, list));
        }
    }
    size_duplicates.sort_by_key(|d| std::cmp::Reverse(d.0)); // サイズの大きい順

    for (size, list) in size_duplicates {
        groups.push(DuplicateGroup {
            label: format!("G-{:02}", group_num),
            reason: format!("同一サイズ ({} バイト)", size),
            tracks: list,
        });
        group_num += 1;
    }

    // 2. 同一タイトル & アーティストによるグループ化（未処理のもの）
    let mut title_map: HashMap<String, Vec<TrackItem>> = HashMap::new();
    for t in tracks {
        let key = format!("{}____{}", t.title.to_lowercase().trim(), t.artist.to_lowercase().trim());
        if !t.title.trim().is_empty() {
            title_map.entry(key).or_default().push(t.clone());
        }
    }

    for (_, list) in title_map {
        if list.len() > 1 {
            // 既に同じグループに入っていないか確認
            let first_path = &list[0].file_path;
            let already_grouped = groups.iter().any(|g| g.tracks.iter().any(|t| &t.file_path == first_path));
            if !already_grouped {
                groups.push(DuplicateGroup {
                    label: format!("G-{:02}", group_num),
                    reason: "同名タイトル & アーティスト".to_string(),
                    tracks: list,
                });
                group_num += 1;
            }
        }
    }

    groups
}

use crate::store::{PlaylistData, Store};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct PlaylistInfo {
    pub id: i64,
    pub name: String,
    pub count: usize,
}

/// プレイリスト。データは `Store`（メモリ上）に保持され、変更時だけ保存される。
#[derive(Clone)]
pub struct PlaylistService {
    store: Arc<Store>,
}

impl PlaylistService {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }

    /// 表示順のプレイリスト一覧
    pub fn get_all_playlists(&self) -> Vec<PlaylistInfo> {
        self.store.read(|lib| {
            lib.playlists
                .iter()
                .map(|p| PlaylistInfo { id: p.id, name: p.name.clone(), count: p.tracks.len() })
                .collect()
        })
    }

    pub fn create_playlist(&self, name: &str) -> Result<i64, String> {
        let clean = name.trim();
        if clean.is_empty() {
            return Err("プレイリスト名を入力してください".to_string());
        }
        self.store.mutate(|lib| {
            if lib.playlists.iter().any(|p| p.name.eq_ignore_ascii_case(clean)) {
                return Err(format!("同名のプレイリストが既に存在します: {}", clean));
            }
            lib.next_playlist_id += 1;
            let id = lib.next_playlist_id;
            lib.playlists.push(PlaylistData { id, name: clean.to_string(), tracks: Vec::new() });
            Ok(id)
        })
    }

    pub fn rename_playlist(&self, id: i64, name: &str) -> Result<(), String> {
        let clean = name.trim();
        if clean.is_empty() {
            return Err("プレイリスト名を入力してください".to_string());
        }
        self.store.mutate(|lib| {
            if lib.playlists.iter().any(|p| p.id != id && p.name.eq_ignore_ascii_case(clean)) {
                return Err(format!("同名のプレイリストが既に存在します: {}", clean));
            }
            match lib.playlists.iter_mut().find(|p| p.id == id) {
                Some(p) => {
                    p.name = clean.to_string();
                    Ok(())
                }
                None => Err("プレイリストが見つかりません".to_string()),
            }
        })
    }

    /// プレイリストを複製（名前は「〜 のコピー」、重複時は連番）
    pub fn duplicate_playlist(&self, id: i64) -> Result<i64, String> {
        self.store.mutate(|lib| {
            let src = lib.playlists.iter().find(|p| p.id == id).ok_or("プレイリストが見つかりません")?.clone();
            let mut name = format!("{} のコピー", src.name);
            let mut n = 2;
            while lib.playlists.iter().any(|p| p.name.eq_ignore_ascii_case(&name)) {
                name = format!("{} のコピー ({})", src.name, n);
                n += 1;
            }
            lib.next_playlist_id += 1;
            let new_id = lib.next_playlist_id;
            lib.playlists.push(PlaylistData { id: new_id, name, tracks: src.tracks });
            Ok(new_id)
        })
    }

    /// 指定した ID 順にプレイリスト自体を並び替える
    pub fn set_playlist_order(&self, ids: &[i64]) -> Result<(), String> {
        self.store.mutate(|lib| {
            let mut rest = std::mem::take(&mut lib.playlists);
            let mut ordered = Vec::with_capacity(rest.len());
            for id in ids {
                if let Some(pos) = rest.iter().position(|p| p.id == *id) {
                    ordered.push(rest.remove(pos));
                }
            }
            ordered.extend(rest); // 指定に無かったものは末尾へ
            lib.playlists = ordered;
        });
        Ok(())
    }

    /// プレイリストの曲を丸ごと置き換える（並び替え・テキスト編集の保存用）
    pub fn set_playlist_tracks(&self, id: i64, tracks: &[PathBuf]) -> Result<(), String> {
        self.store.mutate(|lib| match lib.playlists.iter_mut().find(|p| p.id == id) {
            Some(p) => {
                p.tracks = tracks.iter().map(|t| t.to_string_lossy().to_string()).collect();
                Ok(())
            }
            None => Err("プレイリストが見つかりません".to_string()),
        })
    }

    pub fn delete_playlist(&self, id: i64) -> Result<(), String> {
        self.store.mutate(|lib| lib.playlists.retain(|p| p.id != id));
        Ok(())
    }

    pub fn get_playlist_tracks(&self, id: i64) -> Vec<PathBuf> {
        self.store.read(|lib| {
            lib.playlists
                .iter()
                .find(|p| p.id == id)
                .map(|p| p.tracks.iter().map(PathBuf::from).collect())
                .unwrap_or_default()
        })
    }

    pub fn add_track(&self, playlist_id: i64, path: &Path) -> Result<(), String> {
        self.store.mutate(|lib| match lib.playlists.iter_mut().find(|p| p.id == playlist_id) {
            Some(p) => {
                p.tracks.push(path.to_string_lossy().to_string());
                Ok(())
            }
            None => Err("プレイリストが見つかりません".to_string()),
        })
    }

    /// 複数の曲をまとめて追加する（すでに入っている曲は飛ばす）。実際に追加した曲数を返す
    pub fn add_tracks(&self, playlist_id: i64, paths: &[PathBuf]) -> Result<usize, String> {
        self.store.mutate(|lib| match lib.playlists.iter_mut().find(|p| p.id == playlist_id) {
            Some(p) => {
                let mut have: std::collections::HashSet<String> =
                    p.tracks.iter().map(|t| crate::store::path_key(Path::new(t))).collect();
                let mut added = 0;
                for path in paths {
                    if have.insert(crate::store::path_key(path)) {
                        p.tracks.push(path.to_string_lossy().to_string());
                        added += 1;
                    }
                }
                Ok(added)
            }
            None => Err("プレイリストが見つかりません".to_string()),
        })
    }

    /// 複数の曲をまとめて外す。実際に外した曲数を返す
    pub fn remove_tracks(&self, playlist_id: i64, paths: &[PathBuf]) -> Result<usize, String> {
        self.store.mutate(|lib| match lib.playlists.iter_mut().find(|p| p.id == playlist_id) {
            Some(p) => {
                let drop: std::collections::HashSet<String> = paths.iter().map(|x| crate::store::path_key(x)).collect();
                let before = p.tracks.len();
                p.tracks.retain(|t| !drop.contains(&crate::store::path_key(Path::new(t))));
                Ok(before - p.tracks.len())
            }
            None => Err("プレイリストが見つかりません".to_string()),
        })
    }

    /// データが変わるたびに増える版番号
    pub fn version(&self) -> u64 {
        self.store.version()
    }

    /// プレイリストが参照している全ファイルのパス
    pub fn all_referenced_paths(&self) -> Vec<String> {
        self.store.read(|lib| lib.playlists.iter().flat_map(|p| p.tracks.iter().cloned()).collect())
    }

    /// ファイルの移動に合わせて、プレイリスト内のパスを付け替える
    pub fn relink_paths(&self, moves: &[(String, String)]) {
        if moves.is_empty() {
            return;
        }
        let map: std::collections::HashMap<String, &String> = moves
            .iter()
            .map(|(old, new)| (crate::store::path_key(Path::new(old)), new))
            .collect();
        self.store.mutate(|lib| {
            for p in lib.playlists.iter_mut() {
                for t in p.tracks.iter_mut() {
                    if let Some(new) = map.get(&crate::store::path_key(Path::new(t.as_str()))) {
                        *t = (*new).clone();
                    }
                }
            }
        });
    }

    pub fn export_m3u8(&self, playlist_id: i64, target_file: &Path) -> Result<(), String> {
        let tracks = self.get_playlist_tracks(playlist_id);
        let mut file = File::create(target_file).map_err(|e| e.to_string())?;
        writeln!(file, "#EXTM3U").map_err(|e| e.to_string())?;
        for t in tracks {
            writeln!(file, "{}", t.to_string_lossy()).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn import_m3u8(&self, name: &str, m3u_path: &Path) -> Result<i64, String> {
        // 大きすぎるファイルは読まない。文字コードが UTF-8 でなくても読めるように、バイト列から変換する
        const MAX_M3U_BYTES: u64 = 32 * 1024 * 1024;
        let mut bytes = Vec::new();
        File::open(m3u_path)
            .map_err(|e| e.to_string())?
            .take(MAX_M3U_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_M3U_BYTES {
            return Err("プレイリストファイルが大きすぎます".to_string());
        }
        let text = String::from_utf8_lossy(&bytes);
        let pid = self.create_playlist(name)?;

        let base_dir = m3u_path.parent().unwrap_or_else(|| Path::new("."));
        let mut tracks = Vec::new();
        for line in text.lines() {
            let l = line.trim().trim_start_matches('\u{feff}');
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            // ネットワーク共有のパスは、存在確認だけでも外部へ接続してしまうので取り込まない
            if crate::paths::is_unc(l) {
                continue;
            }
            let p = PathBuf::from(l);
            let full_path = if p.is_absolute() { p } else { base_dir.join(p) };
            if full_path.exists() {
                tracks.push(full_path);
            }
        }
        self.set_playlist_tracks(pid, &tracks)?;
        Ok(pid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playlist_ops() {
        let db = std::env::temp_dir().join(format!("pl_test_{}.bin", std::process::id()));
        let _ = std::fs::remove_file(&db);
        let store = Store::open(&db);
        let s = PlaylistService::new(store.clone());
        let names = |s: &PlaylistService| s.get_all_playlists().into_iter().map(|p| p.name).collect::<Vec<_>>();

        let a = s.create_playlist("a").unwrap();
        let b = s.create_playlist("b").unwrap();
        let c = s.create_playlist("吸血鬼").unwrap();
        assert!(s.create_playlist("A").unwrap_err().contains("同名"));
        assert_eq!(names(&s), ["a", "b", "吸血鬼"]);

        s.set_playlist_order(&[c, a, b]).unwrap();
        assert_eq!(names(&s), ["吸血鬼", "a", "b"]);

        s.add_track(c, Path::new("a.mp3")).unwrap();
        s.add_track(c, Path::new("b.mp3")).unwrap();
        s.set_playlist_tracks(c, &[PathBuf::from("b.mp3"), PathBuf::from("a.mp3")]).unwrap();
        assert_eq!(s.get_playlist_tracks(c), [PathBuf::from("b.mp3"), PathBuf::from("a.mp3")]);

        assert_eq!(s.add_tracks(c, &[PathBuf::from("A.mp3"), PathBuf::from("c.mp3"), PathBuf::from("c.mp3")]).unwrap(), 1);
        assert_eq!(s.remove_tracks(c, &[PathBuf::from("c.mp3"), PathBuf::from("zzz.mp3")]).unwrap(), 1);
        let d = s.duplicate_playlist(c).unwrap();
        let d2 = s.duplicate_playlist(c).unwrap();
        assert_ne!(d, d2);
        assert!(names(&s).contains(&"吸血鬼 のコピー".to_string()));
        assert!(names(&s).contains(&"吸血鬼 のコピー (2)".to_string()));
        s.rename_playlist(d, "renamed").unwrap();
        assert!(s.rename_playlist(d, "a").is_err());
        s.delete_playlist(d).unwrap();

        // 保存 → 再読み込み
        store.flush();
        let again = PlaylistService::new(Store::open(&db));
        assert_eq!(names(&again), names(&s));
        assert_eq!(again.get_playlist_tracks(c), s.get_playlist_tracks(c));
        let _ = std::fs::remove_file(&db);
    }
}

//! ファイル単位の永続キャッシュ（タグ情報・ジャケットサムネイル・波形用）。
//! キーはファイルパス。ファイルサイズと更新日時が一致するときだけ中身を信用するので、
//! ファイルが変更されても古い情報が表示されることはない。
//! 初回アクセス時に1ファイルを一括で読み込み、終了時（変更があったときだけ）まとめて書き出す。
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const MAGIC: &[u8; 4] = b"MMFC";
const VERSION: u32 = 1;

struct Entry {
    size: u64,
    mtime: u64,
    payload: Vec<u8>,
}

#[derive(Default)]
struct State {
    map: HashMap<String, Entry>,
    /// この起動中に参照（有効なキャッシュとして使用）または追加されたキー
    used: HashSet<String>,
}

pub struct FileCache {
    path: PathBuf,
    /// None = まだ読み込んでいない
    state: Mutex<Option<State>>,
    dirty: AtomicBool,
    /// false の間はキャッシュを使わない（読み込みも保存もしない）
    enabled: AtomicBool,
    /// 保存する最大件数（0 = 無制限）
    limit: AtomicUsize,
}

impl FileCache {
    pub fn new(path: &Path) -> Arc<Self> {
        Arc::new(Self {
            path: path.to_path_buf(),
            state: Mutex::new(None),
            dirty: AtomicBool::new(false),
            enabled: AtomicBool::new(true),
            limit: AtomicUsize::new(0),
        })
    }

    fn key(file: &Path) -> String {
        file.to_string_lossy().to_lowercase()
    }

    fn with_state<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let st = guard.get_or_insert_with(|| State { map: load(&self.path).unwrap_or_default(), used: HashSet::new() });
        f(st)
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
    }

    pub fn set_limit(&self, max_entries: usize) {
        self.limit.store(max_entries, Ordering::Relaxed);
    }

    /// (保存件数, ファイルサイズ[バイト])
    pub fn stats(&self) -> (usize, u64) {
        let count = self.state.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|s| s.map.len());
        let count = count.unwrap_or_else(|| load(&self.path).map_or(0, |m| m.len()));
        let bytes = std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
        (count, bytes)
    }

    /// 保存されているキャッシュをすべて消去する
    pub fn clear(&self) {
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = Some(State::default());
        self.dirty.store(false, Ordering::Relaxed);
        let _ = std::fs::remove_file(&self.path);
    }

    /// 起動時にバックグラウンドで呼んで、ファイルを先に読み込んでおく
    pub fn preload(&self) {
        self.with_state(|_| ());
    }

    /// サイズと更新日時が一致するキャッシュがあれば、その中身を返す
    pub fn get(&self, file: &Path, size: u64, mtime: u64) -> Option<Vec<u8>> {
        if !self.enabled.load(Ordering::Relaxed) {
            return None;
        }
        let key = Self::key(file);
        self.with_state(|st| {
            let payload = st.map.get(&key).filter(|e| e.size == size && e.mtime == mtime).map(|e| e.payload.clone());
            if payload.is_some() {
                st.used.insert(key);
            }
            payload
        })
    }

    pub fn put(&self, file: &Path, size: u64, mtime: u64, payload: Vec<u8>) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        let key = Self::key(file);
        self.with_state(|st| {
            st.used.insert(key.clone());
            st.map.insert(key, Entry { size, mtime, payload });
        });
        self.dirty.store(true, Ordering::Relaxed);
    }

    /// ファイルへ書き出す（一時ファイル → 置き換え）。
    /// - `prune = false`（定期の自動保存）: 読み込んだ分も含めて全部残す
    /// - `prune = true`（終了時）: この起動で使った・追加したデータだけを残し、
    ///   それ以前のデータは破棄する（キャッシュが肥大化しない）
    pub fn flush(&self, prune: bool) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        let bytes = {
            let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let Some(st) = guard.as_mut() else { return }; // 一度も使われていない
            let changed = self.dirty.swap(false, Ordering::Relaxed) || (prune && st.used.len() != st.map.len());
            if !changed {
                return;
            }
            if prune {
                let used = std::mem::take(&mut st.used);
                st.map.retain(|k, _| used.contains(k));
                st.used = used;
            }
            // 上限を超える分は破棄（上限は設定の「キャッシュ上限」）
            let limit = self.limit.load(Ordering::Relaxed);
            if limit > 0 && st.map.len() > limit {
                let extra: Vec<String> = st.map.keys().skip(limit).cloned().collect();
                for k in extra {
                    st.map.remove(&k);
                    st.used.remove(&k);
                }
            }
            let mut e = Enc::default();
            e.buf.extend_from_slice(MAGIC);
            e.u32(VERSION);
            e.u32(st.map.len() as u32);
            for (k, v) in &st.map {
                e.str(k);
                e.u64(v.size);
                e.u64(v.mtime);
                e.bytes(&v.payload);
            }
            e.buf
        };
        let tmp = self.path.with_extension("tmp");
        if std::fs::write(&tmp, &bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        }
    }
}

fn load(path: &Path) -> Option<HashMap<String, Entry>> {
    let bytes = std::fs::read(path).ok()?;
    let mut d = Dec { buf: &bytes, pos: 0 };
    if d.take(4)? != MAGIC || d.u32()? != VERSION {
        return None;
    }
    let n = d.u32()? as usize;
    let mut map = HashMap::with_capacity(n.min(bytes.len() / 20));
    for _ in 0..n {
        let key = d.str()?;
        let size = d.u64()?;
        let mtime = d.u64()?;
        let payload = d.bytes()?;
        map.insert(key, Entry { size, mtime, payload });
    }
    Some(map)
}

/// ファイルのサイズと更新日時(ナノ秒)。キャッシュの有効性判定に使う
pub fn file_stamp(path: &Path) -> Option<(u64, u64)> {
    let md = std::fs::metadata(path).ok()?;
    let mtime = md
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos() as u64;
    Some((md.len(), mtime))
}

// ---------- 簡易バイナリ入出力 (little endian) ----------

#[derive(Default)]
pub struct Enc {
    pub buf: Vec<u8>,
}

impl Enc {
    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    pub fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn bytes(&mut self, b: &[u8]) {
        self.u32(b.len() as u32);
        self.buf.extend_from_slice(b);
    }
    pub fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }
}

pub struct Dec<'a> {
    pub buf: &'a [u8],
    pub pos: usize,
}

impl<'a> Dec<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    pub fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let s = self.buf.get(self.pos..end)?;
        self.pos = end;
        Some(s)
    }
    pub fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    pub fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    pub fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    pub fn bytes(&mut self) -> Option<Vec<u8>> {
        let n = self.u32()? as usize;
        Some(self.take(n)?.to_vec())
    }
    pub fn str(&mut self) -> Option<String> {
        String::from_utf8(self.bytes()?).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_roundtrip_and_invalidation() {
        let f = std::env::temp_dir().join(format!("fc_test_{}.bin", std::process::id()));
        let _ = std::fs::remove_file(&f);
        let c = FileCache::new(&f);
        assert!(c.get(Path::new("C:/A/x.mp3"), 10, 20).is_none());
        c.put(Path::new("C:/A/x.mp3"), 10, 20, vec![1, 2, 3]);
        c.flush(true);

        let c2 = FileCache::new(&f);
        assert_eq!(c2.get(Path::new("c:/a/X.MP3"), 10, 20), Some(vec![1, 2, 3])); // 大文字小文字は無視
        assert!(c2.get(Path::new("C:/A/x.mp3"), 11, 20).is_none()); // サイズ違い → 無効
        assert!(c2.get(Path::new("C:/A/x.mp3"), 10, 21).is_none()); // 更新日時違い → 無効
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn exit_flush_keeps_only_this_sessions_data() {
        let f = std::env::temp_dir().join(format!("fc_prune_{}.bin", std::process::id()));
        let _ = std::fs::remove_file(&f);

        // 1回目の起動: a, b を保存
        let s1 = FileCache::new(&f);
        s1.put(Path::new("a.mp3"), 1, 1, vec![1]);
        s1.put(Path::new("b.mp3"), 2, 2, vec![2]);
        s1.flush(true);

        // 2回目: 起動時に読み込み、a だけ使って、c を追加 → 終了時の保存で b は破棄される
        let s2 = FileCache::new(&f);
        s2.preload();
        assert_eq!(s2.get(Path::new("a.mp3"), 1, 1), Some(vec![1]));
        s2.put(Path::new("c.mp3"), 3, 3, vec![3]);
        s2.flush(false); // 自動保存では何も捨てない
        let mid = FileCache::new(&f);
        assert!(mid.get(Path::new("b.mp3"), 2, 2).is_some());
        s2.flush(true); // 終了時

        let s3 = FileCache::new(&f);
        assert!(s3.get(Path::new("a.mp3"), 1, 1).is_some());
        assert!(s3.get(Path::new("c.mp3"), 3, 3).is_some());
        assert!(s3.get(Path::new("b.mp3"), 2, 2).is_none(), "使われなかったデータは破棄される");

        // 追加が無くても、使われなかったデータがあれば終了時に削られる
        let s4 = FileCache::new(&f);
        assert!(s4.get(Path::new("a.mp3"), 1, 1).is_some());
        s4.flush(true);
        let s5 = FileCache::new(&f);
        assert!(s5.get(Path::new("c.mp3"), 3, 3).is_none());
        assert!(s5.get(Path::new("a.mp3"), 1, 1).is_some());
        let _ = std::fs::remove_file(&f);
    }
}

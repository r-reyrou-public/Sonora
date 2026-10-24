//! ラベルとプレイリストの保存先。
//! 全データをメモリ上に持ち、変更時だけ独自の単純なバイナリ形式でファイルに書き出す。
//! 起動時は1ファイルを一括で読むだけ（SQLite のような接続・スキーマ確認・クエリは一切なし）。
//! 書き込みはバックグラウンドスレッドで行い、UI を止めない。
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};

const MAGIC: &[u8; 4] = b"MMLB";
const VERSION: u32 = 1;

#[derive(Clone, Debug)]
pub struct PlaylistData {
    pub id: i64,
    pub name: String,
    pub tracks: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Library {
    pub next_playlist_id: i64,
    /// 表示順に並んでいる
    pub playlists: Vec<PlaylistData>,
    /// ファイルパス(小文字) -> ラベル
    pub labels: HashMap<String, Vec<String>>,
    pub presets: BTreeSet<String>,
}

pub struct Store {
    data: Arc<Mutex<Library>>,
    path: PathBuf,
    /// 保存の依頼（中身は不要。書き込みスレッドが最新の状態を自分でエンコードして書く）
    tx: Sender<()>,
    /// 変更のたびに増える版番号。古い版が新しい版を上書きしないために使う
    version: Arc<AtomicU64>,
    /// ファイルに書き込み済みの最新の版（書き込み同士の排他も兼ねる）
    written: Arc<Mutex<u64>>,
}

impl Store {
    pub fn open(path: &Path) -> Arc<Self> {
        let data = match std::fs::read(path) {
            Ok(bytes) => match Library::decode(&bytes) {
                Some(lib) => lib,
                None => {
                    // 読めないファイルを空のデータで上書きして失わないよう、退避してから始める
                    let _ = std::fs::rename(path, path.with_extension("bin.corrupt"));
                    Library::default()
                }
            },
            Err(_) => Library::default(),
        };

        let (tx, rx) = channel::<()>();
        let written = Arc::new(Mutex::new(0u64));
        let data = Arc::new(Mutex::new(data));
        let version = Arc::new(AtomicU64::new(0));
        let target = path.to_path_buf();
        let (written_w, data_w, version_w) = (written.clone(), data.clone(), version.clone());
        std::thread::Builder::new()
            .name("StoreWriter".into())
            .spawn(move || {
                while rx.recv().is_ok() {
                    // 溜まっていたら、まとめて1回だけ書く
                    while rx.try_recv().is_ok() {}
                    // エンコードは、画面の処理ではなくこのスレッドで行う（データが大きくても操作が止まらない）
                    let (bytes, v) = {
                        let guard = data_w.lock().unwrap_or_else(|e| e.into_inner());
                        (guard.encode(), version_w.load(Ordering::SeqCst))
                    };
                    write_if_newer(&written_w, &target, v, &bytes);
                }
            })
            .ok();

        Arc::new(Self { data, path: path.to_path_buf(), tx, version, written })
    }

    /// データが変わるたびに増える版番号（画面側が「変わったか」を安く調べるため）
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::SeqCst)
    }

    /// バイト列が、このアプリのラベル・プレイリストデータとして読めるか（バックアップ復元時の検証用）
    pub fn is_valid_data(bytes: &[u8]) -> bool {
        Library::decode(bytes).is_some()
    }

    /// 他人から受け取ったデータ（バックアップ）用: ネットワーク共有のパスを取り除いた内容に作り直す。
    /// 読み込んだだけで外部サーバーへ接続しないようにするため
    pub fn sanitize_imported(bytes: &[u8]) -> Option<Vec<u8>> {
        let mut lib = Library::decode(bytes)?;
        lib.labels.retain(|k, _| !crate::paths::is_unc(k));
        for p in lib.playlists.iter_mut() {
            p.tracks.retain(|t| !crate::paths::is_unc(t));
        }
        Some(lib.encode())
    }

    /// 読み取り専用アクセス
    pub fn read<R>(&self, f: impl FnOnce(&Library) -> R) -> R {
        f(&self.data.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// 変更して、バックグラウンドで保存する
    pub fn mutate<R>(&self, f: impl FnOnce(&mut Library) -> R) -> R {
        let mut guard = self.data.lock().unwrap_or_else(|e| e.into_inner());
        let result = f(&mut guard);
        self.version.fetch_add(1, Ordering::SeqCst);
        drop(guard);
        let _ = self.tx.send(());
        result
    }

    /// 終了時: 未書き込みの変更を確実にファイルへ出す
    pub fn flush(&self) {
        let guard = self.data.lock().unwrap_or_else(|e| e.into_inner());
        let bytes = guard.encode();
        let version = self.version.load(Ordering::SeqCst);
        drop(guard);
        write_if_newer(&self.written, &self.path, version, &bytes);
    }
}

/// 既に書いた版以下なら何もしない（古いデータで新しいファイルを上書きしない）
fn write_if_newer(written: &Mutex<u64>, path: &Path, version: u64, bytes: &[u8]) {
    let mut last = written.lock().unwrap_or_else(|e| e.into_inner());
    if version <= *last && *last != 0 {
        return;
    }
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, bytes).is_ok() && std::fs::rename(&tmp, path).is_ok() {
        *last = version;
    }
}

// ---------- バイナリ形式 (little endian / 文字列は u32 長 + UTF-8) ----------

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn put_str(out: &mut Vec<u8>, s: &str) {
    put_u32(out, s.len() as u32);
    out.extend_from_slice(s.as_bytes());
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let s = self.buf.get(self.pos..end)?;
        self.pos = end;
        Some(s)
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn i64(&mut self) -> Option<i64> {
        Some(i64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    /// 件数 `n` の要素が最低 `min_each` バイトずつ残りに収まるか。壊れた件数で巨大なメモリを確保しないために使う
    fn cap(&self, n: usize, min_each: usize) -> usize {
        n.min((self.buf.len() - self.pos.min(self.buf.len())) / min_each.max(1))
    }
    fn str(&mut self) -> Option<String> {
        let n = self.u32()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }
}

impl Library {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4096);
        out.extend_from_slice(MAGIC);
        put_u32(&mut out, VERSION);

        put_u32(&mut out, self.presets.len() as u32);
        for p in &self.presets {
            put_str(&mut out, p);
        }

        put_u32(&mut out, self.labels.len() as u32);
        for (path, labels) in &self.labels {
            put_str(&mut out, path);
            put_u32(&mut out, labels.len() as u32);
            for l in labels {
                put_str(&mut out, l);
            }
        }

        out.extend_from_slice(&self.next_playlist_id.to_le_bytes());
        put_u32(&mut out, self.playlists.len() as u32);
        for p in &self.playlists {
            out.extend_from_slice(&p.id.to_le_bytes());
            put_str(&mut out, &p.name);
            put_u32(&mut out, p.tracks.len() as u32);
            for t in &p.tracks {
                put_str(&mut out, t);
            }
        }
        out
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let mut r = Reader { buf: bytes, pos: 0 };
        if r.take(4)? != MAGIC || r.u32()? != VERSION {
            return None;
        }
        let mut lib = Library::default();

        for _ in 0..r.u32()? {
            lib.presets.insert(r.str()?);
        }

        let n = r.u32()? as usize;
        lib.labels.reserve(r.cap(n, 8));
        for _ in 0..n {
            let path = r.str()?;
            let count = r.u32()?;
            let mut labels = Vec::with_capacity(r.cap(count as usize, 4));
            for _ in 0..count {
                labels.push(r.str()?);
            }
            lib.labels.insert(path, labels);
        }

        lib.next_playlist_id = r.i64()?;
        for _ in 0..r.u32()? {
            let id = r.i64()?;
            let name = r.str()?;
            let count = r.u32()? as usize;
            let mut tracks = Vec::with_capacity(r.cap(count, 4));
            for _ in 0..count {
                tracks.push(r.str()?);
            }
            lib.playlists.push(PlaylistData { id, name, tracks });
        }
        Some(lib)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut lib = Library::default();
        lib.next_playlist_id = 7;
        lib.presets.insert("fav".into());
        lib.labels.insert("c:/a.mp3".into(), vec!["fav".into(), "日本語".into()]);
        lib.playlists.push(PlaylistData { id: 3, name: "吸血鬼".into(), tracks: vec!["x.mp3".into(), "y.mp3".into()] });

        let back = Library::decode(&lib.encode()).unwrap();
        assert_eq!(back.next_playlist_id, 7);
        assert_eq!(back.labels["c:/a.mp3"], ["fav", "日本語"]);
        assert_eq!(back.playlists[0].name, "吸血鬼");
        assert_eq!(back.playlists[0].tracks, ["x.mp3", "y.mp3"]);
        assert!(back.presets.contains("fav"));

        // 壊れたデータは空として扱う
        assert!(Library::decode(&lib.encode()[..10]).is_none());
        assert!(Library::decode(b"junk").is_none());

        // 他人から受け取ったデータからは、ネットワーク共有のパスが取り除かれる
        let mut shared = Library::default();
        shared.labels.insert(r"\\evil\share\a.mp3".into(), vec!["x".into()]);
        shared.labels.insert(r"c:\ok\a.mp3".into(), vec!["x".into()]);
        shared.playlists.push(PlaylistData { id: 1, name: "p".into(), tracks: vec![r"\\evil\share\b.mp3".into(), r"c:\ok\b.mp3".into()] });
        let clean = Library::decode(&Store::sanitize_imported(&shared.encode()).unwrap()).unwrap();
        assert_eq!(clean.labels.len(), 1);
        assert_eq!(clean.playlists[0].tracks, [r"c:\ok\b.mp3"]);

        // 件数だけ巨大な壊れたデータでも、巨大なメモリを確保せずに失敗する
        let mut evil = Vec::new();
        evil.extend_from_slice(MAGIC);
        evil.extend_from_slice(&VERSION.to_le_bytes());
        evil.extend_from_slice(&0u32.to_le_bytes()); // presets
        evil.extend_from_slice(&1u32.to_le_bytes()); // labels: 1 件
        evil.extend_from_slice(&1u32.to_le_bytes());
        evil.push(b'a');
        evil.extend_from_slice(&u32::MAX.to_le_bytes()); // ラベル数が 42 億
        assert!(Library::decode(&evil).is_none());
    }
}

/// パスの同一判定用キー。Windows では大文字小文字・区切り文字(\ と /)・末尾の区切りの違いを無視する。
/// （同じファイルでも、開いた経路によって表記が違うことがあるため）
pub fn path_key(p: &std::path::Path) -> String {
    let s = p.to_string_lossy().replace('/', "\\").to_lowercase();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    s.trim_end_matches('\\').to_string()
}

#[cfg(test)]
mod path_key_tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn same_file_different_spelling() {
        let a = path_key(Path::new(r"M:\Video\BGM\01_導入.MP3"));
        assert_eq!(a, path_key(Path::new("m:/video/bgm/01_導入.mp3")));
        assert_eq!(a, path_key(Path::new(r"\\?\M:\Video\BGM\01_導入.mp3")));
        assert_ne!(a, path_key(Path::new(r"M:\Video\BGM\02.mp3")));
    }
}

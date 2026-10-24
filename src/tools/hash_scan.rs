//! 重複検出（ハッシュチェック）: タグを除いた音声データそのものを比べる。
//!
//! 各ファイルをコンテナとして読み、音声のパケット（MP3 のフレーム、FLAC のフレームなど）だけを
//! ハッシュにする。ID3・Vorbis コメント・ジャケット画像などのタグは含まれないので、
//! タグだけが違うコピーも同じ曲として見つかる。別の形式への変換や再エンコードは別のデータになる。
use crate::scanner::metadata::TrackItem;
use std::collections::HashMap;
use std::hash::Hasher;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// 重複していたファイル1件
#[derive(Clone, Debug)]
pub struct HashEntry {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    pub size_text: String,
}

/// 同じ音声データのファイルのまとまり
#[derive(Clone, Debug)]
pub struct HashGroup {
    pub entries: Vec<HashEntry>,
}

type Digest = (u64, u64, u64);

struct Shared {
    total: usize,
    next: AtomicUsize,
    done: AtomicUsize,
    failed: AtomicUsize,
    cancel: AtomicBool,
    running_workers: AtomicUsize,
    current: Mutex<String>,
    seen: Mutex<HashMap<Digest, Vec<HashEntry>>>,
}

/// 実行中（または完了した）ハッシュチェック
pub struct HashScan {
    shared: Arc<Shared>,
    started: Instant,
    finished_at: Mutex<Option<Instant>>,
}

pub struct Progress {
    pub total: usize,
    pub done: usize,
    pub failed: usize,
    pub current: String,
    pub running: bool,
    pub cancelled: bool,
    pub elapsed_secs: f32,
}

impl HashScan {
    pub fn start(tracks: &[TrackItem]) -> Self {
        let items: Vec<HashEntry> = tracks
            .iter()
            .map(|t| HashEntry {
                name: t.filename.clone(),
                path: t.file_path.clone(),
                size: t.file_size_bytes,
                size_text: t.formatted_size.clone(),
            })
            .collect();
        let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).clamp(1, 3);
        let shared = Arc::new(Shared {
            total: items.len(),
            next: AtomicUsize::new(0),
            done: AtomicUsize::new(0),
            failed: AtomicUsize::new(0),
            cancel: AtomicBool::new(false),
            running_workers: AtomicUsize::new(workers),
            current: Mutex::new(String::new()),
            seen: Mutex::new(HashMap::new()),
        });
        let items = Arc::new(items);
        for _ in 0..workers {
            let (sh, items) = (shared.clone(), items.clone());
            std::thread::spawn(move || {
                loop {
                    if sh.cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let i = sh.next.fetch_add(1, Ordering::SeqCst);
                    let Some(entry) = items.get(i) else { break };
                    if let Ok(mut c) = sh.current.lock() {
                        *c = entry.name.clone();
                    }
                    match audio_digest(&entry.path, &sh.cancel) {
                        Ok(d) => {
                            if let Ok(mut m) = sh.seen.lock() {
                                m.entry(d).or_default().push(entry.clone());
                            }
                        }
                        Err(_) => {
                            sh.failed.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    sh.done.fetch_add(1, Ordering::Relaxed);
                }
                sh.running_workers.fetch_sub(1, Ordering::SeqCst);
            });
        }
        Self { shared, started: Instant::now(), finished_at: Mutex::new(None) }
    }

    pub fn cancel(&self) {
        self.shared.cancel.store(true, Ordering::SeqCst);
    }

    pub fn progress(&self) -> Progress {
        let running = self.shared.running_workers.load(Ordering::SeqCst) > 0;
        let mut fin = self.finished_at.lock().unwrap_or_else(|e| e.into_inner());
        if !running && fin.is_none() {
            *fin = Some(Instant::now());
        }
        let end = fin.unwrap_or_else(Instant::now);
        Progress {
            total: self.shared.total,
            done: self.shared.done.load(Ordering::Relaxed).min(self.shared.total),
            failed: self.shared.failed.load(Ordering::Relaxed),
            current: self.shared.current.lock().map(|c| c.clone()).unwrap_or_default(),
            running,
            cancelled: self.shared.cancel.load(Ordering::Relaxed),
            elapsed_secs: end.duration_since(self.started).as_secs_f32(),
        }
    }

    /// ここまでに見つかった重複のまとまり（サイズの大きい順）
    pub fn groups(&self) -> Vec<HashGroup> {
        let Ok(map) = self.shared.seen.lock() else { return Vec::new() };
        let mut groups: Vec<HashGroup> = map
            .values()
            .filter(|v| v.len() > 1)
            .map(|v| {
                let mut entries = v.clone();
                entries.sort_by(|a, b| a.path.cmp(&b.path));
                HashGroup { entries }
            })
            .collect();
        groups.sort_by(|a, b| {
            b.entries[0].size.cmp(&a.entries[0].size).then_with(|| a.entries[0].path.cmp(&b.entries[0].path))
        });
        groups
    }
}

impl Drop for HashScan {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// 音声パケットだけを読んで、128 ビット相当のハッシュと総バイト数にする
fn audio_digest(path: &Path, cancel: &AtomicBool) -> Result<Digest, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| e.to_string())?;
    let mut format = probed.format;
    let track_id = format.default_track().map(|t| t.id).ok_or_else(|| "音声トラックがありません".to_string())?;

    let mut h1 = std::collections::hash_map::DefaultHasher::new();
    let mut h2 = std::collections::hash_map::DefaultHasher::new();
    h2.write_u64(0x9E37_79B9_7F4A_7C15);
    let mut bytes = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("中止".to_string());
        }
        match format.next_packet() {
            Ok(p) => {
                if p.track_id() != track_id {
                    continue;
                }
                h1.write(&p.data);
                h2.write(&p.data);
                bytes += p.data.len() as u64;
            }
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(Error::ResetRequired) => break,
            Err(e) => return Err(e.to_string()),
        }
    }
    if bytes == 0 {
        return Err("音声データがありません".to_string());
    }
    Ok((h1.finish(), h2.finish(), bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 16bit モノラル 8kHz の WAV。`extra` はデータの前に置くタグ代わりのチャンク
    fn wav(samples: &[i16], extra: &[u8]) -> Vec<u8> {
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut body = Vec::new();
        body.extend_from_slice(b"WAVE");
        body.extend_from_slice(b"fmt ");
        body.extend_from_slice(&16u32.to_le_bytes());
        for v in [1u16, 1] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        body.extend_from_slice(&8000u32.to_le_bytes());
        body.extend_from_slice(&16000u32.to_le_bytes());
        body.extend_from_slice(&2u16.to_le_bytes());
        body.extend_from_slice(&16u16.to_le_bytes());
        body.extend_from_slice(extra);
        body.extend_from_slice(b"data");
        body.extend_from_slice(&(data.len() as u32).to_le_bytes());
        body.extend_from_slice(&data);
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn tags_are_ignored_but_audio_is_compared() {
        let dir = std::env::temp_dir().join(format!("sonora_hash_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let samples: Vec<i16> = (0..4000).map(|i| ((i * 37) % 2000) as i16).collect();
        let mut other = samples.clone();
        other[100] += 1;
        let mut info = b"LIST".to_vec();
        info.extend_from_slice(&12u32.to_le_bytes());
        info.extend_from_slice(b"INFOINAM\x00\x00\x00\x00");
        let (a, b, c) = (dir.join("a.wav"), dir.join("b.wav"), dir.join("c.wav"));
        std::fs::write(&a, wav(&samples, &[])).unwrap();
        std::fs::write(&b, wav(&samples, &info)).unwrap();
        std::fs::write(&c, wav(&other, &[])).unwrap();
        let cancel = AtomicBool::new(false);
        let (da, db, dc) =
            (audio_digest(&a, &cancel).unwrap(), audio_digest(&b, &cancel).unwrap(), audio_digest(&c, &cancel).unwrap());
        assert_ne!(std::fs::metadata(&a).unwrap().len(), std::fs::metadata(&b).unwrap().len());
        assert_eq!(da, db, "タグ（追加チャンク）が違っても音声が同じなら一致する");
        assert_ne!(da, dc, "音声が違えば一致しない");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

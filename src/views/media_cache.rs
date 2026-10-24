use egui::{ColorImage, Context, TextureHandle, TextureOptions};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::filecache::{file_stamp, Dec, Enc, FileCache};
use crate::scanner::metadata::TrackItem;
use std::sync::Arc;

/// 波形として表示する先頭区間（秒）
static WAVEFORM_SECS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(15);

/// 出だし波形として表示・生成する秒数（設定「出だし波形プレビュー」。3〜120）
pub fn waveform_secs() -> f32 {
    WAVEFORM_SECS.load(std::sync::atomic::Ordering::Relaxed) as f32
}

pub fn set_waveform_secs(secs: u32) {
    WAVEFORM_SECS.store(secs.clamp(3, 120), std::sync::atomic::Ordering::Relaxed);
}
/// 波形のバー本数（0.2秒/本）
pub const WAVEFORM_BINS: usize = 75;
/// 曲全体波形のバー本数
pub const FULL_BINS: usize = 260;
const THUMB_SIZE: u32 = 48;
/// メモリに持つジャケット・波形の最大件数（超えたら捨てて、必要な分だけ作り直す）
const MAX_ENTRIES: usize = 1500;
const MAX_FULL: usize = 300;

#[derive(Clone)]
pub struct MediaData {
    pub art: Option<TextureHandle>,
    /// 先頭15秒の正規化ピーク (0.0..=1.0)。デコード失敗時は None
    pub peaks: Option<Vec<f32>>,
}

/// ジャケットサムネイルと先頭15秒波形をバックグラウンドで生成・保持する。
/// 画面に表示された行だけが `get` で要求するので、大量のファイルでも起動は遅くならない。
pub struct MediaCache {
    ctx: Context,
    /// ジャケットサムネイルと波形の永続キャッシュ（サイズ+更新日時が同じなら再利用）
    disk: Arc<FileCache>,
    entries: HashMap<PathBuf, Option<MediaData>>,
    tx: Sender<(PathBuf, MediaData)>,
    rx: Receiver<(PathBuf, MediaData)>,
    /// 曲全体の波形（情報パネル用。選択された曲だけ生成する）
    full: HashMap<PathBuf, Option<Vec<f32>>>,
    full_tx: Sender<(PathBuf, Vec<f32>)>,
    full_rx: Receiver<(PathBuf, Vec<f32>)>,
}

impl MediaCache {
    pub fn new(ctx: Context, disk: Arc<FileCache>) -> Self {
        let (tx, rx) = channel();
        let (full_tx, full_rx) = channel();
        Self { ctx, disk, entries: HashMap::new(), tx, rx, full: HashMap::new(), full_tx, full_rx }
    }

    /// 未生成なら生成を依頼し、完成済みならそのデータを返す
    pub fn get(&mut self, path: &Path) -> Option<MediaData> {
        while let Ok((p, data)) = self.rx.try_recv() {
            self.entries.insert(p, Some(data));
        }
        // 何万曲を見て回っても、画像（テクスチャ）がメモリに溜まり続けないよう、一定数を超えたら捨てる
        // （表示中の行の分は、ディスクキャッシュからすぐ作り直される）
        if self.entries.len() > MAX_ENTRIES {
            self.entries.clear();
        }

        match self.entries.get(path) {
            Some(entry) => entry.clone(),
            None => {
                self.entries.insert(path.to_path_buf(), None);
                let tx = self.tx.clone();
                let ctx = self.ctx.clone();
                let path = path.to_path_buf();
                let disk = self.disk.clone();
                rayon::spawn(move || {
                    let data = load_media(&ctx, &disk, &path);
                    let _ = tx.send((path, data));
                    ctx.request_repaint();
                });
                None
            }
        }
    }

    /// 曲全体の実波形（FULL_BINS 本）。未生成なら生成を依頼して None を返す
    pub fn get_full(&mut self, path: &Path, total_secs: f32) -> Option<Vec<f32>> {
        while let Ok((p, peaks)) = self.full_rx.try_recv() {
            self.full.insert(p, Some(peaks));
        }
        if self.full.len() > MAX_FULL {
            self.full.clear();
        }
        match self.full.get(path) {
            Some(entry) => entry.clone(),
            None => {
                self.full.insert(path.to_path_buf(), None);
                let tx = self.full_tx.clone();
                let ctx = self.ctx.clone();
                let path = path.to_path_buf();
                rayon::spawn(move || {
                    let peaks = compute_peaks(&path, total_secs.max(1.0), FULL_BINS).unwrap_or_default();
                    let _ = tx.send((path, peaks));
                    ctx.request_repaint();
                });
                None
            }
        }
    }

    /// ファイルが変更された場合などにキャッシュを破棄する
    pub fn clear(&mut self) {
        self.entries.clear();
        self.full.clear();
    }
}

/// ジャケットと先頭15秒波形。ディスクキャッシュにあればそれを使い、無ければ生成して保存する
fn load_media(ctx: &Context, disk: &FileCache, path: &Path) -> MediaData {
    let stamp = file_stamp(path);

    if let Some((size, mtime)) = stamp {
        if let Some(cached) = disk.get(path, size, mtime).and_then(|b| decode_payload(&b)) {
            let (jpeg, peaks_q, cached_secs) = cached;
            let art = jpeg.as_ref().and_then(|j| texture_from_image(ctx, path, image::load_from_memory(j).ok()?));
            if cached_secs == waveform_secs() as u32 {
                let peaks = peaks_q.map(|q| q.iter().map(|&v| v as f32 / 255.0).collect());
                return MediaData { art, peaks };
            }
            // 波形の秒数設定が変わっていたら、波形だけ作り直す（ジャケットはキャッシュのまま）
            let peaks = compute_peaks(path, waveform_secs(), WAVEFORM_BINS);
            let quantized = peaks.as_ref().map(|p| quantize(p));
            disk.put(path, size, mtime, encode_payload(jpeg.as_deref(), quantized.as_deref()));
            return MediaData { art, peaks };
        }
    }

    // サムネイル: テクスチャ化しつつ、キャッシュ用に小さな JPEG にも変換する
    let mut jpeg: Option<Vec<u8>> = None;
    let art = TrackItem::load_artwork(path)
        .and_then(|bytes| image::load_from_memory(&bytes).ok())
        .map(|img| img.thumbnail(THUMB_SIZE, THUMB_SIZE))
        .and_then(|thumb| {
            let mut out = Vec::new();
            if image::DynamicImage::ImageRgb8(thumb.to_rgb8())
                .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Jpeg)
                .is_ok()
            {
                jpeg = Some(out);
            }
            texture_from_image(ctx, path, thumb)
        });
    let peaks = compute_peaks(path, waveform_secs(), WAVEFORM_BINS);

    if let Some((size, mtime)) = stamp {
        let quantized = peaks.as_ref().map(|p| quantize(p));
        disk.put(path, size, mtime, encode_payload(jpeg.as_deref(), quantized.as_deref()));
    }
    MediaData { art, peaks }
}

fn texture_from_image(ctx: &Context, path: &Path, img: image::DynamicImage) -> Option<TextureHandle> {
    let img = img.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let color = ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    Some(ctx.load_texture(format!("art:{}", path.display()), color, TextureOptions::LINEAR))
}

fn quantize(p: &[f32]) -> Vec<u8> {
    p.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect()
}

fn encode_payload(jpeg: Option<&[u8]>, peaks: Option<&[u8]>) -> Vec<u8> {
    let mut e = Enc::default();
    for part in [jpeg, peaks] {
        match part {
            Some(b) => {
                e.u8(1);
                e.bytes(b);
            }
            None => e.u8(0),
        }
    }
    e.u32(waveform_secs() as u32); // この波形を作ったときの秒数
    e.buf
}

/// (サムネイルJPEG, 量子化した波形, 波形の秒数)
type Payload = (Option<Vec<u8>>, Option<Vec<u8>>, u32);

fn decode_payload(b: &[u8]) -> Option<Payload> {
    let mut d = Dec::new(b);
    let mut part = || -> Option<Option<Vec<u8>>> { if d.u8()? == 1 { Some(Some(d.bytes()?)) } else { Some(None) } };
    let jpeg = part()?;
    let peaks = part()?;
    let secs = d.u32().unwrap_or(15);
    Some((jpeg, peaks, secs))
}

/// 先頭 WAVEFORM_SECS 秒だけをデコードして、バーごとの最大振幅を求める
fn compute_peaks(path: &Path, secs: f32, bins: usize) -> Option<Vec<f32>> {
    let file = std::fs::File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .ok()?;
    let mut format = probed.format;
    let track = format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL)?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .ok()?;

    let mut peaks = vec![0f32; bins];
    let mut sample_buf: Option<SampleBuffer<f32>> = None;
    let mut frames_done: u64 = 0;
    let mut finished = false;

    while !finished {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(_) => break,
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(Error::DecodeError(_)) => continue,
            Err(_) => break,
        };

        let spec = *decoded.spec();
        let channels = spec.channels.count().max(1);
        let total_frames = (secs * spec.rate as f32) as u64;

        let needed = decoded.capacity() as u64 * channels as u64;
        if sample_buf.as_ref().is_none_or(|b| (b.capacity() as u64) < needed) {
            sample_buf = Some(SampleBuffer::<f32>::new(decoded.capacity() as u64, spec));
        }
        let buf = sample_buf.as_mut()?;
        buf.copy_interleaved_ref(decoded);

        for frame in buf.samples().chunks(channels) {
            if frames_done >= total_frames {
                finished = true;
                break;
            }
            let amp = frame.iter().fold(0f32, |m, s| m.max(s.abs()));
            let bin = ((frames_done * bins as u64 / total_frames) as usize).min(bins - 1);
            if amp > peaks[bin] {
                peaks[bin] = amp;
            }
            frames_done += 1;
        }
    }

    if frames_done == 0 {
        return None;
    }
    let max = peaks.iter().cloned().fold(0f32, f32::max).max(1e-3);
    Some(peaks.iter().map(|p| (p / max).sqrt()).collect())
}

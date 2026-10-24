use super::backend::{open_output, OpenedOutput};
use super::mixer::Deck;
use crate::settings::{AudioOutputEngine, LoudnessMode};
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// ラウドネス補正の目標値と上下限（C# 版 LoudnessHelper と同じ）
const TARGET_RMS_DB: f64 = -18.0;
const MIN_GAIN_DB: f64 = -15.0;
const MAX_GAIN_DB: f64 = 6.0;

/// 再生速度の範囲（音程も速度に合わせて変わる）
pub const MIN_SPEED: f32 = 0.5;
pub const MAX_SPEED: f32 = 2.0;
/// 逆再生で、一度にさかのぼって読み込む長さ（ミリ秒）
const REVERSE_SEGMENT_MS: u64 = 5_000;

pub enum PlayerCommand {
    /// crossfade=true: 曲の終わりに重ねて次の曲へ切り替える（クロスフェード設定が有効なとき）
    Play { path: PathBuf, crossfade: bool },
    Pause,
    Resume,
    Stop,
    Seek(Duration),
    SetVolume(f32),
    SetEngine(AudioOutputEngine),
    SetFade(u32),
    SetCrossfade(u32),
    SetLoudness(LoudnessMode),
    SetSpeed(f32),
    SetReverse(bool),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepeatMode {
    None,
    All,
    One,
}

/// UI スレッドとオーディオスレッドで共有する状態
impl Shared {
    fn notify_change(&self) {
        if let Some(f) = self.on_change.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            f();
        }
    }
}

struct Shared {
    is_playing: AtomicBool,
    is_paused: AtomicBool,
    track_ended: AtomicBool,
    /// 再生の開始・曲の終了を、画面側へ知らせる（画面の更新は、入力が無いと止まるため）
    on_change: Mutex<Option<Box<dyn Fn() + Send>>>,
    /// クロスフェード開始の時刻に達した（次の曲を重ねて始めてほしい）
    crossfade_due: AtomicBool,
    position_ms: AtomicU64,
    total_ms: AtomicU64,
    current_track: Mutex<Option<PathBuf>>,
    engine_name: Mutex<String>,
    is_fallback: AtomicBool,
    engine_error: Mutex<Option<String>>,
    /// 逆再生中か
    reverse: AtomicBool,
}

pub struct AudioPlayer {
    command_tx: crossbeam_channel::Sender<PlayerCommand>,
    shared: Arc<Shared>,
    pub volume: f32,
    pub repeat_mode: RepeatMode,
    pub shuffle: bool,
    /// 設定で選んだ出力方式（実際に使われている方式は `active_engine_name`）
    pub engine: AudioOutputEngine,
    pub fade_ms: u32,
    pub crossfade_ms: u32,
    /// 再生速度（1.0 が等倍）
    pub speed: f32,
}

impl AudioPlayer {
    pub fn new() -> Result<Self, String> {
        let (command_tx, command_rx) = crossbeam_channel::unbounded::<PlayerCommand>();
        let shared = Arc::new(Shared {
            is_playing: AtomicBool::new(false),
            is_paused: AtomicBool::new(false),
            track_ended: AtomicBool::new(false),
            on_change: Mutex::new(None),
            crossfade_due: AtomicBool::new(false),
            position_ms: AtomicU64::new(0),
            total_ms: AtomicU64::new(0),
            current_track: Mutex::new(None),
            engine_name: Mutex::new(String::new()),
            is_fallback: AtomicBool::new(false),
            engine_error: Mutex::new(None),
            reverse: AtomicBool::new(false),
        });

        let worker_shared = shared.clone();
        std::thread::Builder::new()
            .name("AudioThread".to_string())
            .spawn(move || audio_worker_loop(command_rx, worker_shared))
            .map_err(|e| e.to_string())?;

        Ok(Self {
            command_tx,
            shared,
            volume: 0.8,
            repeat_mode: RepeatMode::None,
            shuffle: false,
            engine: AudioOutputEngine::Auto,
            fade_ms: 0,
            crossfade_ms: 0,
            speed: 1.0,
        })
    }

    pub fn play(&self, path: &Path) {
        self.shared.track_ended.store(false, Ordering::Relaxed);
        self.shared.crossfade_due.store(false, Ordering::Relaxed);
        let _ = self.command_tx.send(PlayerCommand::Play { path: path.to_path_buf(), crossfade: false });
    }

    /// 現在の曲に重ねて再生を始める（クロスフェード）
    pub fn play_crossfade(&self, path: &Path) {
        self.shared.track_ended.store(false, Ordering::Relaxed);
        self.shared.crossfade_due.store(false, Ordering::Relaxed);
        let _ = self.command_tx.send(PlayerCommand::Play { path: path.to_path_buf(), crossfade: true });
    }

    pub fn pause(&self) {
        let _ = self.command_tx.send(PlayerCommand::Pause);
    }

    pub fn resume(&self) {
        let _ = self.command_tx.send(PlayerCommand::Resume);
    }

    pub fn stop(&self) {
        let _ = self.command_tx.send(PlayerCommand::Stop);
    }

    pub fn seek(&self, pos: Duration) {
        let _ = self.command_tx.send(PlayerCommand::Seek(pos));
    }

    pub fn set_volume(&mut self, vol: f32) {
        self.volume = vol.clamp(0.0, 1.0);
        let _ = self.command_tx.send(PlayerCommand::SetVolume(self.volume));
    }

    pub fn set_engine(&mut self, engine: AudioOutputEngine) {
        self.engine = engine;
        let _ = self.command_tx.send(PlayerCommand::SetEngine(engine));
    }

    pub fn set_fade_ms(&mut self, ms: u32) {
        self.fade_ms = ms;
        let _ = self.command_tx.send(PlayerCommand::SetFade(ms));
    }

    pub fn set_crossfade_ms(&mut self, ms: u32) {
        self.crossfade_ms = ms;
        let _ = self.command_tx.send(PlayerCommand::SetCrossfade(ms));
    }

    /// 再生速度を変える（0.5〜2.0 倍。再生中の曲にもすぐ反映する）
    pub fn set_speed(&mut self, speed: f32) {
        self.speed = (speed.clamp(MIN_SPEED, MAX_SPEED) * 100.0).round() / 100.0;
        let _ = self.command_tx.send(PlayerCommand::SetSpeed(self.speed));
    }

    /// 逆再生の入り切り（再生中の曲だけ。別の曲を再生し始めると、順方向に戻る）
    pub fn set_reverse(&self, on: bool) {
        let _ = self.command_tx.send(PlayerCommand::SetReverse(on));
    }

    pub fn is_reverse(&self) -> bool {
        self.shared.reverse.load(Ordering::Relaxed)
    }

    pub fn set_loudness(&self, mode: LoudnessMode) {
        let _ = self.command_tx.send(PlayerCommand::SetLoudness(mode));
    }

    /// 実際に使われている出力方式の名前（"WASAPI" など。出力が無ければ空）
    pub fn active_engine_name(&self) -> String {
        self.shared.engine_name.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// 希望した方式が使えず、別の方式で再生している
    pub fn engine_is_fallback(&self) -> bool {
        self.shared.is_fallback.load(Ordering::Relaxed)
    }

    #[allow(dead_code)]
    pub fn engine_error(&self) -> Option<String> {
        self.shared.engine_error.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn state(&self) -> PlaybackState {
        if self.shared.is_playing.load(Ordering::Relaxed) {
            if self.shared.is_paused.load(Ordering::Relaxed) {
                PlaybackState::Paused
            } else {
                PlaybackState::Playing
            }
        } else {
            PlaybackState::Stopped
        }
    }

    /// 再生の開始・曲の終了のたびに呼ばれる関数を登録する（画面の再描画を起こすために使う）
    pub fn set_on_change(&self, f: impl Fn() + Send + 'static) {
        *self.shared.on_change.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(f));
    }

    pub fn check_and_clear_ended(&self) -> bool {
        self.shared.track_ended.swap(false, Ordering::Relaxed)
    }

    /// クロスフェードの開始時刻になったか（一度だけ true になる）
    pub fn check_and_clear_crossfade_due(&self) -> bool {
        self.shared.crossfade_due.swap(false, Ordering::Relaxed)
    }

    pub fn current_position(&self) -> Duration {
        Duration::from_millis(self.shared.position_ms.load(Ordering::Relaxed))
    }

    pub fn total_duration(&self) -> Duration {
        Duration::from_millis(self.shared.total_ms.load(Ordering::Relaxed))
    }

    pub fn current_track_path(&self) -> Option<PathBuf> {
        self.shared.current_track.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn cycle_repeat_mode(&mut self) -> RepeatMode {
        self.repeat_mode = match self.repeat_mode {
            RepeatMode::None => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::None,
        };
        self.repeat_mode
    }

    pub fn toggle_shuffle(&mut self) -> bool {
        self.shuffle = !self.shuffle;
        self.shuffle
    }
}

// ---------------------------------------------------------------------------------------------
// オーディオスレッド
// ---------------------------------------------------------------------------------------------

/// 1曲分のデコード状態
struct Playing {
    deck: Arc<Deck>,
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    track_channels: usize,
    resampler: Resampler,
    sample_buf: Option<SampleBuffer<f32>>,
    /// シーク位置（再生位置 = これ + デッキが消費した時間）
    start_offset_ms: u64,
    total_ms: u64,
    path: PathBuf,
    track_rate: u32,
    speed: f64,
    reverse: bool,
    /// 逆再生: まだ再生していない部分の終わりの位置（ここから前へさかのぼる）
    rev_pos_ms: u64,
}

/// 線形補間のリサンプラー（トラックのサンプルレート → デバイスのサンプルレート）
struct Resampler {
    /// 出力1フレームあたりに進む入力フレーム数
    ratio: f64,
    pos: f64,
    carry: Vec<f32>,
    channels: usize,
}

impl Resampler {
    fn new(from_rate: u32, to_rate: u32, channels: usize, speed: f64) -> Self {
        Self { ratio: from_rate as f64 * speed / to_rate.max(1) as f64, pos: 0.0, carry: Vec::new(), channels }
    }

    /// 速度を変える（再生速度 = 入力を読み進める速さ）
    fn set_speed(&mut self, from_rate: u32, to_rate: u32, speed: f64) {
        self.ratio = from_rate as f64 * speed / to_rate.max(1) as f64;
    }

    fn reset(&mut self) {
        self.pos = 0.0;
        self.carry.clear();
    }

    fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if (self.ratio - 1.0).abs() < 1e-9 {
            out.extend_from_slice(input);
            return;
        }
        let ch = self.channels;
        self.carry.extend_from_slice(input);
        let frames = self.carry.len() / ch;
        while (self.pos as usize) + 1 < frames {
            let i = self.pos as usize;
            let frac = (self.pos - i as f64) as f32;
            for c in 0..ch {
                let a = self.carry[i * ch + c];
                let b = self.carry[(i + 1) * ch + c];
                out.push(a + (b - a) * frac);
            }
            self.pos += self.ratio;
        }
        let consumed = (self.pos as usize).min(frames);
        self.carry.drain(..consumed * ch);
        self.pos -= consumed as f64;
    }
}

/// チャンネル数をデバイスに合わせる（モノラルは複製、3ch以上は先頭2chを使用）
fn remap_channels(input: &[f32], in_ch: usize, out_ch: usize, out: &mut Vec<f32>) {
    if in_ch == out_ch {
        out.extend_from_slice(input);
        return;
    }
    for frame in input.chunks(in_ch) {
        for c in 0..out_ch {
            let v = if in_ch == 1 {
                frame[0]
            } else if c < in_ch {
                frame[c]
            } else {
                0.0
            };
            out.push(v);
        }
    }
}

struct WorkerState {
    out: Option<OpenedOutput>,
    engine: AudioOutputEngine,
    fade_ms: u32,
    crossfade_ms: u32,
    loudness: LoudnessMode,
    volume: f32,
    current: Option<Playing>,
    /// フェードアウト中の前の曲（クロスフェード）
    outgoing: Vec<Playing>,
    crossfade_signaled: bool,
    rms_cache: HashMap<PathBuf, f64>,
    speed: f64,
}

fn audio_worker_loop(rx: crossbeam_channel::Receiver<PlayerCommand>, shared: Arc<Shared>) {
    let mut st = WorkerState {
        out: None,
        engine: AudioOutputEngine::Auto,
        fade_ms: 0,
        crossfade_ms: 0,
        loudness: LoudnessMode::Off,
        volume: 0.8,
        current: None,
        outgoing: Vec::new(),
        crossfade_signaled: false,
        rms_cache: HashMap::new(),
        speed: 1.0,
    };
    reopen_output(&mut st, &shared);

    loop {
        // コマンドが無ければデコードを進めながら待つ
        loop {
            match rx.try_recv() {
                Ok(cmd) => handle_command(cmd, &mut st, &shared),
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => return,
            }
        }

        decode_and_update(&mut st, &shared);
        std::thread::sleep(Duration::from_millis(4));
    }
}

fn reopen_output(st: &mut WorkerState, shared: &Shared) {
    st.out = None; // 古い出力を先に閉じる
    match open_output(st.engine) {
        Ok(out) => {
            out.mixer.set_volume(st.volume);
            *shared.engine_name.lock().unwrap_or_else(|e| e.into_inner()) = out.backend.name().to_string();
            shared.is_fallback.store(out.is_fallback, Ordering::Relaxed);
            *shared.engine_error.lock().unwrap_or_else(|e| e.into_inner()) = None;
            st.out = Some(out);
        }
        Err(e) => {
            shared.engine_name.lock().unwrap_or_else(|e| e.into_inner()).clear();
            *shared.engine_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(e);
        }
    }
}

fn handle_command(cmd: PlayerCommand, st: &mut WorkerState, shared: &Shared) {
    match cmd {
        PlayerCommand::Play { path, crossfade } => start_track(st, shared, &path, crossfade, None),
        PlayerCommand::Pause => {
            if let Some(out) = &st.out {
                if st.current.is_some() && !shared.is_paused.load(Ordering::Relaxed) {
                    shared.is_paused.store(true, Ordering::Relaxed);
                    out.mixer.pause(st.fade_ms.min(1000));
                }
            }
        }
        PlayerCommand::Resume => {
            if let Some(out) = &st.out {
                if st.current.is_some() && shared.is_paused.load(Ordering::Relaxed) {
                    shared.is_paused.store(false, Ordering::Relaxed);
                    out.mixer.resume(st.fade_ms.min(1000));
                }
            }
        }
        PlayerCommand::Stop => {
            if let Some(out) = &st.out {
                let fading = shared.is_playing.load(Ordering::Relaxed) && !shared.is_paused.load(Ordering::Relaxed);
                out.mixer.stop(if fading { st.fade_ms } else { 0 });
            }
            st.current = None;
            st.outgoing.clear();
            shared.is_playing.store(false, Ordering::Relaxed);
            shared.is_paused.store(false, Ordering::Relaxed);
            shared.position_ms.store(0, Ordering::Relaxed);
            st.crossfade_signaled = false;
        }
        PlayerCommand::Seek(pos) => seek_current(st, shared, pos),
        PlayerCommand::SetVolume(v) => {
            st.volume = v;
            if let Some(out) = &st.out {
                out.mixer.set_volume(v);
            }
        }
        PlayerCommand::SetFade(ms) => st.fade_ms = ms,
        PlayerCommand::SetCrossfade(ms) => st.crossfade_ms = ms,
        PlayerCommand::SetLoudness(m) => st.loudness = m,
        PlayerCommand::SetSpeed(v) => {
            st.speed = (v as f64).clamp(MIN_SPEED as f64, MAX_SPEED as f64);
            let Some(rate) = st.out.as_ref().map(|o| o.mixer.rate) else { return };
            let speed = st.speed;
            if let Some(p) = st.current.as_mut() {
                let pos = shared.position_ms.load(Ordering::Relaxed);
                p.speed = speed;
                p.resampler.set_speed(p.track_rate, rate, speed);
                restart_at(p, pos, rate);
                shared.position_ms.store(p.start_offset_ms, Ordering::Relaxed);
            }
        }
        PlayerCommand::SetReverse(on) => {
            let Some(rate) = st.out.as_ref().map(|o| o.mixer.rate) else { return };
            if let Some(p) = st.current.as_mut() {
                if p.reverse == on {
                    return;
                }
                let mut pos = shared.position_ms.load(Ordering::Relaxed);
                // 曲の頭で逆再生にしても、すぐ終わってしまうので、終わりからさかのぼる
                if on && pos < 300 && p.total_ms > 0 {
                    pos = p.total_ms;
                }
                p.reverse = on;
                restart_at(p, pos, rate);
                shared.position_ms.store(p.start_offset_ms, Ordering::Relaxed);
                shared.reverse.store(on, Ordering::Relaxed);
                st.crossfade_signaled = false;
            }
        }
        PlayerCommand::SetEngine(engine) => {
            if engine == st.engine && st.out.is_some() {
                return;
            }
            st.engine = engine;
            // 再生中なら、同じ位置から新しい出力方式で再開する
            let resume = st.current.as_ref().map(|p| (p.path.clone(), shared.position_ms.load(Ordering::Relaxed)));
            let was_paused = shared.is_paused.load(Ordering::Relaxed);
            st.current = None;
            st.outgoing.clear();
            reopen_output(st, shared);
            if let Some((path, pos_ms)) = resume {
                start_track(st, shared, &path, false, Some(pos_ms));
                if was_paused {
                    if let Some(out) = &st.out {
                        shared.is_paused.store(true, Ordering::Relaxed);
                        out.mixer.pause(0);
                    }
                }
            }
        }
    }
}

/// ReplayGain タグ（"-6.50 dB" など）の値
fn read_replaygain_db(path: &Path) -> Option<f64> {
    use lofty::file::TaggedFileExt;
    use lofty::tag::ItemKey;
    let tagged = lofty::probe::Probe::open(path).ok()?.read().ok()?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
    let text = tag.get_string(&ItemKey::ReplayGainTrackGain)?;
    let num: String = text.trim().chars().take_while(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | '.')).collect();
    num.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// 冒頭 `secs` 秒の RMS（dBFS）を実測する
fn estimate_rms_db(path: &Path, secs: f32) -> Option<f64> {
    let file = File::open(path).ok()?;
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
    let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).ok()?;

    let (mut sum_sq, mut count) = (0f64, 0u64);
    let mut limit: Option<u64> = None;
    let mut buf: Option<SampleBuffer<f32>> = None;
    while limit.is_none_or(|l| count < l) {
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
        limit.get_or_insert((secs as f64 * spec.rate as f64 * spec.channels.count() as f64) as u64);
        let needed = decoded.capacity() as u64 * spec.channels.count() as u64;
        if buf.as_ref().is_none_or(|b| (b.capacity() as u64) < needed) {
            buf = Some(SampleBuffer::<f32>::new(decoded.capacity() as u64, spec));
        }
        let b = buf.as_mut()?;
        b.copy_interleaved_ref(decoded);
        for s in b.samples() {
            sum_sq += (*s as f64) * (*s as f64);
            count += 1;
        }
    }
    if count == 0 || sum_sq <= 0.0 {
        return None;
    }
    Some(10.0 * (sum_sq / count as f64).log10())
}

fn resolve_track_gain(st: &mut WorkerState, path: &Path) -> f32 {
    let db = match st.loudness {
        LoudnessMode::Off => None,
        mode => {
            // ReplayGain タグを優先。自動モードではタグが無ければ冒頭の音量から推定する
            read_replaygain_db(path).map(|g| g.clamp(MIN_GAIN_DB, MAX_GAIN_DB)).or_else(|| {
                if mode != LoudnessMode::Auto {
                    return None;
                }
                let rms = match st.rms_cache.get(path) {
                    Some(v) => Some(*v),
                    None => {
                        let v = estimate_rms_db(path, 15.0);
                        if let Some(v) = v {
                            st.rms_cache.insert(path.to_path_buf(), v);
                        }
                        v
                    }
                };
                rms.map(|r| (TARGET_RMS_DB - r).clamp(MIN_GAIN_DB, MAX_GAIN_DB))
            })
        }
    };
    db.map_or(1.0, |d| 10f64.powf(d / 20.0) as f32)
}

/// 曲を開いてデコードを始める。`start_ms` があればその位置から
fn start_track(st: &mut WorkerState, shared: &Shared, path: &Path, crossfade: bool, start_ms: Option<u64>) {
    if st.out.is_none() {
        reopen_output(st, shared);
    }
    let Some((rate, channels)) = st.out.as_ref().map(|o| (o.mixer.rate, o.mixer.channels)) else {
        shared.is_playing.store(false, Ordering::Relaxed);
        return;
    };

    let Ok(file) = File::open(path) else {
        shared.is_playing.store(false, Ordering::Relaxed);
        return;
    };
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let Ok(probed) = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
    else {
        shared.is_playing.store(false, Ordering::Relaxed);
        return;
    };
    let reader = probed.format;
    let Some(track) = reader.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL) else {
        shared.is_playing.store(false, Ordering::Relaxed);
        return;
    };
    let track_id = track.id;
    let track_rate = track.codec_params.sample_rate.unwrap_or(rate);
    let track_channels = track.codec_params.channels.map_or(2, |c| c.count().max(1));
    let total_ms = track
        .codec_params
        .n_frames
        .map(|f| (f as f64 / track_rate as f64 * 1000.0) as u64)
        .or_else(|| {
            // フレーム数が分からない形式（VBR の MP3 など）はタグ読み取りで長さを得る
            use lofty::file::AudioFile;
            lofty::probe::Probe::open(path).ok()?.read().ok().map(|t| t.properties().duration().as_millis() as u64)
        })
        .unwrap_or(0);
    let Ok(decoder) = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()) else {
        shared.is_playing.store(false, Ordering::Relaxed);
        return;
    };

    let gain = resolve_track_gain(st, path);

    // 切り替え時のフェード: 再生中の曲があれば前の曲をフェードアウトして重ねる
    let was_playing = st.current.is_some() && shared.is_playing.load(Ordering::Relaxed) && !shared.is_paused.load(Ordering::Relaxed);
    let switch_ms = if crossfade && st.crossfade_ms > 0 { st.crossfade_ms } else { st.fade_ms };
    let mixer = st.out.as_ref().unwrap().mixer.clone();

    if let Some(prev) = st.current.take() {
        if was_playing && switch_ms > 0 {
            prev.deck.fade_to(0.0, mixer.ms_to_frames(switch_ms));
            st.outgoing.push(prev);
        } else {
            st.outgoing.clear();
            mixer.clear_decks();
        }
    } else {
        st.outgoing.clear();
        mixer.clear_decks();
    }
    mixer.reset_master();

    let fade_in_ms = if was_playing { switch_ms } else { st.fade_ms.min(10_000) };
    let deck = Deck::new(gain, if fade_in_ms > 0 { 0.0 } else { 1.0 });
    if fade_in_ms > 0 {
        deck.fade_to(1.0, mixer.ms_to_frames(fade_in_ms));
    }
    mixer.add_deck(deck.clone());

    let mut playing = Playing {
        deck,
        reader,
        decoder,
        track_id,
        track_channels,
        resampler: Resampler::new(track_rate, rate, channels, st.speed),
        sample_buf: None,
        start_offset_ms: 0,
        total_ms,
        path: path.to_path_buf(),
        track_rate,
        speed: st.speed,
        reverse: false,
        rev_pos_ms: 0,
    };
    if let Some(ms) = start_ms.filter(|ms| *ms > 0) {
        seek_playing(&mut playing, Duration::from_millis(ms), rate);
    }

    shared.total_ms.store(total_ms, Ordering::Relaxed);
    shared.position_ms.store(playing.start_offset_ms, Ordering::Relaxed);
    *shared.current_track.lock().unwrap_or_else(|e| e.into_inner()) = Some(path.to_path_buf());
    shared.is_playing.store(true, Ordering::Relaxed);
    shared.notify_change();
    shared.is_paused.store(false, Ordering::Relaxed);
    shared.track_ended.store(false, Ordering::Relaxed);
    shared.reverse.store(false, Ordering::Relaxed);
    st.crossfade_signaled = false;
    st.current = Some(playing);
}

fn seek_playing(p: &mut Playing, pos: Duration, dev_rate: u32) {
    let sec = pos.as_secs();
    let frac = pos.subsec_nanos() as f64 / 1_000_000_000.0;
    let ok = p
        .reader
        .seek(
            SeekMode::Accurate,
            SeekTo::Time { time: symphonia::core::units::Time::new(sec, frac), track_id: Some(p.track_id) },
        )
        .is_ok();
    if ok {
        p.decoder.reset();
        p.deck.ring.lock().unwrap_or_else(|e| e.into_inner()).clear();
        p.deck.consumed_frames.store(0, Ordering::Relaxed);
        p.deck.decode_done.store(false, Ordering::Relaxed);
        p.resampler.reset();
        p.start_offset_ms = pos.as_millis() as u64;
        let _ = dev_rate;
    }
}

/// 指定の位置から、いまの向き（順 / 逆）と速度で読み直す
fn restart_at(p: &mut Playing, pos_ms: u64, dev_rate: u32) {
    if p.reverse {
        p.decoder.reset();
        p.deck.ring.lock().unwrap_or_else(|e| e.into_inner()).clear();
        p.deck.consumed_frames.store(0, Ordering::Relaxed);
        p.deck.decode_done.store(false, Ordering::Relaxed);
        p.resampler.reset();
        p.start_offset_ms = pos_ms;
        p.rev_pos_ms = pos_ms;
    } else {
        seek_playing(p, Duration::from_millis(pos_ms), dev_rate);
    }
}

fn seek_current(st: &mut WorkerState, shared: &Shared, pos: Duration) {
    let Some(rate) = st.out.as_ref().map(|o| o.mixer.rate) else { return };
    if let Some(p) = st.current.as_mut() {
        restart_at(p, pos.as_millis() as u64, rate);
        shared.position_ms.store(p.start_offset_ms, Ordering::Relaxed);
        st.crossfade_signaled = false;
    }
}

/// 1曲分のデコードを、バッファが溜まるまで進める。曲の最後まで読み切ったら true
fn decode_into_deck(p: &mut Playing, dev_rate: u32, dev_channels: usize) {
    let target = (dev_rate as usize * dev_channels) * 2 / 5; // 約 0.4 秒分
    let mut tmp_in: Vec<f32> = Vec::new();
    let mut tmp_out: Vec<f32> = Vec::new();

    if p.reverse {
        if !p.deck.decode_done.load(Ordering::Relaxed) && p.deck.buffered_samples() < target {
            decode_reverse_segment(p, dev_channels);
        }
        return;
    }

    while !p.deck.decode_done.load(Ordering::Relaxed) && p.deck.buffered_samples() < target {
        let packet = match p.reader.next_packet() {
            Ok(pk) => pk,
            Err(_) => {
                p.deck.decode_done.store(true, Ordering::Relaxed);
                break;
            }
        };
        if packet.track_id() != p.track_id {
            continue;
        }
        let decoded = match p.decoder.decode(&packet) {
            Ok(d) => d,
            Err(Error::DecodeError(_)) => continue,
            Err(_) => {
                p.deck.decode_done.store(true, Ordering::Relaxed);
                break;
            }
        };
        let spec = *decoded.spec();
        let ch = spec.channels.count().max(1);
        let needed = decoded.capacity() as u64 * ch as u64;
        if p.sample_buf.as_ref().is_none_or(|b| (b.capacity() as u64) < needed) {
            p.sample_buf = Some(SampleBuffer::<f32>::new(decoded.capacity() as u64, spec));
        }
        let Some(buf) = p.sample_buf.as_mut() else { break };
        buf.copy_interleaved_ref(decoded);

        tmp_in.clear();
        remap_channels(buf.samples(), ch, dev_channels, &mut tmp_in);
        tmp_out.clear();
        p.resampler.process(&tmp_in, &mut tmp_out);
        p.deck.ring.lock().unwrap_or_else(|e| e.into_inner()).extend(tmp_out.iter().copied());
        let _ = p.track_channels;
    }
}

/// 逆再生: 少し前の区間を読み込み、フレームの順番を逆にして、再生待ちの列に積む
fn decode_reverse_segment(p: &mut Playing, dev_channels: usize) {
    if p.rev_pos_ms == 0 {
        p.deck.decode_done.store(true, Ordering::Relaxed);
        return;
    }
    let seg_start = p.rev_pos_ms.saturating_sub(REVERSE_SEGMENT_MS);
    let want_frames = ((p.rev_pos_ms - seg_start) as u128 * p.track_rate as u128 / 1000) as usize;
    let time = symphonia::core::units::Time::new(seg_start / 1000, (seg_start % 1000) as f64 / 1000.0);
    let seeked = p.reader.seek(SeekMode::Accurate, SeekTo::Time { time, track_id: Some(p.track_id) });
    let Ok(seeked) = seeked else {
        p.deck.decode_done.store(true, Ordering::Relaxed);
        return;
    };
    p.decoder.reset();
    // シーク先がパケットの途中までしか合わないので、手前の余分なフレームは捨てる
    let mut skip = seeked.required_ts.saturating_sub(seeked.actual_ts) as usize;
    let mut seg: Vec<f32> = Vec::with_capacity(want_frames * dev_channels);
    let mut tmp: Vec<f32> = Vec::new();

    while seg.len() / dev_channels < want_frames {
        let Ok(packet) = p.reader.next_packet() else { break };
        if packet.track_id() != p.track_id {
            continue;
        }
        let decoded = match p.decoder.decode(&packet) {
            Ok(d) => d,
            Err(Error::DecodeError(_)) => continue,
            Err(_) => break,
        };
        let spec = *decoded.spec();
        let ch = spec.channels.count().max(1);
        let needed = decoded.capacity() as u64 * ch as u64;
        if p.sample_buf.as_ref().is_none_or(|b| (b.capacity() as u64) < needed) {
            p.sample_buf = Some(SampleBuffer::<f32>::new(decoded.capacity() as u64, spec));
        }
        let Some(buf) = p.sample_buf.as_mut() else { break };
        buf.copy_interleaved_ref(decoded);
        tmp.clear();
        remap_channels(buf.samples(), ch, dev_channels, &mut tmp);
        let frames = tmp.len() / dev_channels;
        if skip >= frames {
            skip -= frames;
            continue;
        }
        seg.extend_from_slice(&tmp[skip * dev_channels..]);
        skip = 0;
    }
    seg.truncate(want_frames * dev_channels);

    // フレーム単位で、後ろから前へ並べ替える
    let mut reversed: Vec<f32> = Vec::with_capacity(seg.len());
    for frame in seg.chunks_exact(dev_channels).rev() {
        reversed.extend_from_slice(frame);
    }
    let mut out: Vec<f32> = Vec::new();
    p.resampler.process(&reversed, &mut out);
    p.deck.ring.lock().unwrap_or_else(|e| e.into_inner()).extend(out.iter().copied());
    p.rev_pos_ms = seg_start;
    if seg.is_empty() {
        p.deck.decode_done.store(true, Ordering::Relaxed);
    }
}

fn decode_and_update(st: &mut WorkerState, shared: &Shared) {
    let Some((rate, channels)) = st.out.as_ref().map(|o| (o.mixer.rate, o.mixer.channels)) else { return };

    if let Some(p) = st.current.as_mut() {
        if !shared.is_paused.load(Ordering::Relaxed) || p.deck.buffered_samples() == 0 {
            decode_into_deck(p, rate, channels);
        }

        // 再生位置
        let consumed = p.deck.consumed_frames.load(Ordering::Relaxed);
        let moved_ms = (consumed as f64 * 1000.0 * p.speed / rate as f64) as u64;
        let pos_ms = if p.reverse { p.start_offset_ms.saturating_sub(moved_ms) } else { p.start_offset_ms + moved_ms };
        shared.position_ms.store(pos_ms, Ordering::Relaxed);

        // クロスフェード: 曲の終わりの手前で、次の曲を重ねて始めるよう知らせる
        let cf = st.crossfade_ms as u64;
        if cf > 0
            && !p.reverse
            && !st.crossfade_signaled
            && p.total_ms > cf * 2
            && p.total_ms.saturating_sub(pos_ms) <= cf
            && shared.is_playing.load(Ordering::Relaxed)
        {
            st.crossfade_signaled = true;
            shared.crossfade_due.store(true, Ordering::Relaxed);
        }

        // 曲の終わり
        if p.deck.is_drained() && shared.is_playing.load(Ordering::Relaxed) {
            shared.is_playing.store(false, Ordering::Relaxed);
            shared.track_ended.store(true, Ordering::Relaxed);
            shared.notify_change();
            st.current = None;
        }
    }

    // フェードアウト中の前の曲もデコードを続ける
    st.outgoing.retain_mut(|p| {
        decode_into_deck(p, rate, channels);
        !(p.deck.is_drained() || p.deck.is_faded_out())
    });
}

#[cfg(test)]
mod engine_tests {
    use super::*;

    fn wait_until(mut f: impl FnMut() -> bool, ms: u64) -> bool {
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_millis(ms) {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    /// 各出力方式で実際に再生を始め、再生位置が進むことを確認する（音そのものは聴けないが、
    /// デバイスがミキサーの音を取り出し続けている＝出力が動いている証拠になる）
    #[test]
    #[ignore] // 音声デバイスが必要なので、手動実行用: cargo test engines -- --ignored --nocapture
    fn engines_play_and_advance() {
        let wav = Path::new("M:/Video/Filmora/Voice01_short.mp3");
        if !wav.exists() {
            eprintln!("テスト用ファイルが無いためスキップ");
            return;
        }
        for engine in [AudioOutputEngine::Wasapi, AudioOutputEngine::DirectSound, AudioOutputEngine::WaveOut] {
            let mut p = AudioPlayer::new().unwrap();
            p.set_engine(engine);
            std::thread::sleep(Duration::from_millis(500));
            p.play(wav);
            let advanced = wait_until(|| p.current_position() > Duration::from_millis(600), 4000);
            eprintln!(
                "{:?}: 実際の方式={} fallback={} 位置={:?} 総時間={:?} 状態={:?} エラー={:?}",
                engine,
                p.active_engine_name(),
                p.engine_is_fallback(),
                p.current_position(),
                p.total_duration(),
                p.state(),
                p.engine_error()
            );
            assert!(advanced, "{:?} で再生位置が進みません", engine);
            assert_eq!(p.active_engine_name(), match engine {
                AudioOutputEngine::Wasapi => "WASAPI",
                AudioOutputEngine::DirectSound => "DirectSound",
                _ => "WaveOut",
            });
            // 一時停止 → 位置が止まる → 再開 → 進む
            p.pause();
            std::thread::sleep(Duration::from_millis(300));
            let paused_pos = p.current_position();
            std::thread::sleep(Duration::from_millis(400));
            assert_eq!(p.current_position(), paused_pos, "一時停止中に位置が進んでいます");
            p.resume();
            assert!(wait_until(|| p.current_position() > paused_pos + Duration::from_millis(300), 3000));
            // シーク
            p.seek(Duration::from_millis(2000));
            assert!(wait_until(|| p.current_position() >= Duration::from_millis(2000), 3000));
            p.stop();
            std::thread::sleep(Duration::from_millis(200));
            assert_eq!(p.state(), PlaybackState::Stopped);
        }
    }

    /// 12 秒の無音の WAV（16bit・44.1kHz・ステレオ）を一時フォルダに作る
    fn temp_wav(name: &str) -> PathBuf {
        let rate = 44_100u32;
        let frames = rate * 12;
        let data_len = frames * 4;
        let mut b: Vec<u8> = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data_len).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * 4).to_le_bytes());
        b.extend_from_slice(&4u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..frames {
            let v = (((i as f32) * 0.05).sin() * 3000.0) as i16;
            b.extend_from_slice(&v.to_le_bytes());
            b.extend_from_slice(&v.to_le_bytes());
        }
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, b).unwrap();
        path
    }

    /// 再生速度を上げると位置が速く進み、逆再生にすると位置が戻っていく
    #[test]
    fn speed_and_reverse_move_position() {
        let wav = temp_wav("sonora_speed_reverse_test.wav");
        let mut p = AudioPlayer::new().unwrap();
        std::thread::sleep(Duration::from_millis(500));
        p.play(&wav);
        assert!(wait_until(|| p.current_position() > Duration::from_millis(300), 4000), "再生が始まりません");

        // 2 倍速: 1 秒の間に、およそ 2 秒ぶん進む
        p.set_speed(2.0);
        std::thread::sleep(Duration::from_millis(300));
        let a = p.current_position();
        std::thread::sleep(Duration::from_millis(1000));
        let advanced = p.current_position().saturating_sub(a).as_millis();
        eprintln!("2 倍速で 1 秒の間に進んだ量: {advanced}ms");
        assert!((1500..=2600).contains(&advanced), "2 倍速になっていません: {advanced}ms");

        // 逆再生: 位置が戻っていく
        p.set_speed(1.0);
        p.seek(Duration::from_millis(8000));
        assert!(wait_until(|| p.current_position() >= Duration::from_millis(8000), 3000));
        p.set_reverse(true);
        assert!(wait_until(|| p.is_reverse(), 2000));
        std::thread::sleep(Duration::from_millis(400));
        let b = p.current_position();
        std::thread::sleep(Duration::from_millis(1000));
        let c = p.current_position();
        eprintln!("逆再生: {:?} → {:?}", b, c);
        assert!(c < b, "逆再生で位置が戻りません");
        let back = (b - c).as_millis();
        assert!((700..=1300).contains(&back), "逆再生の速さが違います: {back}ms");

        // 5 秒の区間をまたいでも途切れず戻り続ける
        assert!(wait_until(|| p.current_position() < Duration::from_millis(2500), 8000), "区間をまたいで戻れません");
        assert_eq!(p.state(), PlaybackState::Playing);

        // 順方向に戻す
        p.set_reverse(false);
        assert!(wait_until(|| !p.is_reverse(), 2000));
        let d = p.current_position();
        std::thread::sleep(Duration::from_millis(800));
        assert!(p.current_position() > d, "順方向に戻せません");
        p.stop();
        let _ = std::fs::remove_file(&wav);
    }

    /// フェード・クロスフェード・ラウドネス設定を有効にして再生しても動作する
    #[test]
    #[ignore]
    fn fade_crossfade_loudness() {
        let a = Path::new("M:/Video/Filmora/Voice01_short.mp3");
        let b = Path::new("M:/Video/Filmora/Preserved Roses.wav");
        if !a.exists() || !b.exists() {
            return;
        }
        let mut p = AudioPlayer::new().unwrap();
        p.set_fade_ms(500);
        p.set_crossfade_ms(1500);
        p.set_loudness(LoudnessMode::Auto);
        std::thread::sleep(Duration::from_millis(500));
        p.play(a);
        assert!(wait_until(|| p.current_position() > Duration::from_millis(500), 4000));
        p.play_crossfade(b);
        assert!(wait_until(|| p.current_track_path().as_deref() == Some(b), 3000));
        assert!(wait_until(|| p.current_position() > Duration::from_millis(1500), 5000));
        assert_eq!(p.state(), PlaybackState::Playing);
        p.stop();
    }

    /// (プライベートメモリ[バイト], ハンドル数)
    fn process_usage() -> (u64, u32) {
        use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessHandleCount};
        unsafe {
            let h = GetCurrentProcess();
            let mut c: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
            c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
            GetProcessMemoryInfo(h, &mut c as *mut _ as *mut _, c.cb);
            let mut handles = 0u32;
            GetProcessHandleCount(h, &mut handles);
            (c.PrivateUsage as u64, handles)
        }
    }

    /// 再生と停止を繰り返しても、プレイヤーを作って捨てることを繰り返しても、メモリ・ハンドルが増え続けない
    #[test]
    #[ignore]
    fn repeated_play_and_restart_does_not_leak() {
        let wav = Path::new("M:/Video/Filmora/Voice01_short.mp3");
        if !wav.exists() {
            return;
        }
        // 1) 同じプレイヤーで再生 → 停止 を繰り返す
        let mut p = AudioPlayer::new().unwrap();
        std::thread::sleep(Duration::from_millis(500));
        for _ in 0..5 {
            p.play(wav);
            std::thread::sleep(Duration::from_millis(150));
            p.stop();
        }
        std::thread::sleep(Duration::from_millis(300));
        let (mem0, h0) = process_usage();
        for _ in 0..80 {
            p.play(wav);
            std::thread::sleep(Duration::from_millis(120));
            p.stop();
        }
        std::thread::sleep(Duration::from_millis(500));
        let (mem1, h1) = process_usage();
        eprintln!("再生→停止 80回: メモリ {} KB -> {} KB, ハンドル {} -> {}", mem0 / 1024, mem1 / 1024, h0, h1);
        drop(p);

        // 2) プレイヤーごと作り直す（アプリの起動・終了に相当）を繰り返す
        for _ in 0..3 {
            let mut q = AudioPlayer::new().unwrap();
            std::thread::sleep(Duration::from_millis(300));
            q.play(wav);
            std::thread::sleep(Duration::from_millis(200));
        }
        std::thread::sleep(Duration::from_millis(500));
        let (mem2, h2) = process_usage();
        for _ in 0..25 {
            let mut q = AudioPlayer::new().unwrap();
            std::thread::sleep(Duration::from_millis(300));
            q.play(wav);
            std::thread::sleep(Duration::from_millis(200));
        }
        std::thread::sleep(Duration::from_millis(800));
        let (mem3, h3) = process_usage();
        eprintln!("プレイヤー作成→破棄 25回: メモリ {} KB -> {} KB, ハンドル {} -> {}", mem2 / 1024, mem3 / 1024, h2, h3);

        assert!(mem1.saturating_sub(mem0) < 20 * 1024 * 1024, "再生の繰り返しでメモリが増えています");
        assert!(h1.saturating_sub(h0) < 20, "再生の繰り返しでハンドルが増えています");
        assert!(mem3.saturating_sub(mem2) < 20 * 1024 * 1024, "プレイヤーの作り直しでメモリが増えています");
        assert!(h3.saturating_sub(h2) < 20, "プレイヤーの作り直しでハンドルが増えています");
    }
}

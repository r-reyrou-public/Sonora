//! 出力デバイス（WASAPI / DirectSound / WaveOut）のコールバックから呼ばれるミキサー。
//! デッキごとのリングバッファから音を取り出して重ね合わせ、フェード（曲ごと・全体）と音量を掛ける。
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// 直線的に変化するゲイン（フェードイン／アウト用）
pub struct Fade {
    cur: f32,
    target: f32,
    step: f32,
}

impl Fade {
    pub fn new(value: f32) -> Self {
        Self { cur: value, target: value, step: 0.0 }
    }

    /// `frames` フレームかけて `target` まで変化させる（0 なら即時）
    pub fn start(&mut self, target: f32, frames: u64) {
        self.target = target;
        if frames == 0 {
            self.cur = target;
            self.step = 0.0;
        } else {
            self.step = (target - self.cur) / frames as f32;
        }
    }

    pub fn set(&mut self, value: f32) {
        self.cur = value;
        self.target = value;
        self.step = 0.0;
    }

    /// 1フレーム進めて、そのフレームのゲインを返す
    pub fn next(&mut self) -> f32 {
        if self.step != 0.0 {
            self.cur += self.step;
            if (self.step > 0.0 && self.cur >= self.target) || (self.step < 0.0 && self.cur <= self.target) {
                self.cur = self.target;
                self.step = 0.0;
            }
        }
        self.cur
    }

    pub fn is_silent(&self) -> bool {
        self.cur <= 0.0 && self.target <= 0.0
    }
}

/// 1曲分の再生状態（デコード済みの PCM を溜めるリングバッファ）
pub struct Deck {
    /// デバイスのサンプルレート・チャンネル数に変換済みの交互配置サンプル
    pub ring: Mutex<VecDeque<f32>>,
    gain: AtomicU32,
    fade: Mutex<Fade>,
    /// これまでにミキサーが取り出したフレーム数
    pub consumed_frames: AtomicU64,
    /// これ以上デコードするデータが無い
    pub decode_done: AtomicBool,
}

impl Deck {
    pub fn new(track_gain: f32, initial_fade: f32) -> Arc<Self> {
        Arc::new(Self {
            ring: Mutex::new(VecDeque::new()),
            gain: AtomicU32::new(track_gain.to_bits()),
            fade: Mutex::new(Fade::new(initial_fade)),
            consumed_frames: AtomicU64::new(0),
            decode_done: AtomicBool::new(false),
        })
    }

    pub fn fade_to(&self, target: f32, frames: u64) {
        self.fade.lock().unwrap_or_else(|e| e.into_inner()).start(target, frames);
    }

    pub fn buffered_samples(&self) -> usize {
        self.ring.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// フェードアウトし終えて無音になっている
    pub fn is_faded_out(&self) -> bool {
        self.fade.lock().unwrap_or_else(|e| e.into_inner()).is_silent()
    }

    /// デコード済みで、リングも空（再生し終えた）
    pub fn is_drained(&self) -> bool {
        self.decode_done.load(Ordering::Relaxed) && self.ring.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
    }
}

pub struct Mixer {
    pub channels: usize,
    pub rate: u32,
    volume: AtomicU32,
    decks: Mutex<Vec<Arc<Deck>>>,
    paused: AtomicBool,
    /// 全体フェード（一時停止・停止・再開のなめらかな切り替え用）
    master: Mutex<Fade>,
    pending_pause: AtomicBool,
    pending_stop: AtomicBool,
}

impl Mixer {
    pub fn new(channels: usize, rate: u32) -> Arc<Self> {
        Arc::new(Self {
            channels: channels.max(1),
            rate,
            volume: AtomicU32::new(0.8f32.to_bits()),
            decks: Mutex::new(Vec::new()),
            paused: AtomicBool::new(false),
            master: Mutex::new(Fade::new(1.0)),
            pending_pause: AtomicBool::new(false),
            pending_stop: AtomicBool::new(false),
        })
    }

    pub fn set_volume(&self, v: f32) {
        self.volume.store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn add_deck(&self, deck: Arc<Deck>) {
        self.decks.lock().unwrap_or_else(|e| e.into_inner()).push(deck);
    }

    pub fn clear_decks(&self) {
        self.decks.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    pub fn ms_to_frames(&self, ms: u32) -> u64 {
        self.rate as u64 * ms as u64 / 1000
    }

    /// 一時停止（fade_ms > 0 ならフェードアウトしてから止める）
    pub fn pause(&self, fade_ms: u32) {
        if fade_ms == 0 {
            self.paused.store(true, Ordering::Relaxed);
            return;
        }
        self.pending_stop.store(false, Ordering::Relaxed);
        self.pending_pause.store(true, Ordering::Relaxed);
        self.master.lock().unwrap_or_else(|e| e.into_inner()).start(0.0, self.ms_to_frames(fade_ms));
    }

    pub fn resume(&self, fade_ms: u32) {
        self.pending_pause.store(false, Ordering::Relaxed);
        self.pending_stop.store(false, Ordering::Relaxed);
        let mut m = self.master.lock().unwrap_or_else(|e| e.into_inner());
        if fade_ms > 0 {
            m.start(1.0, self.ms_to_frames(fade_ms));
        } else {
            m.set(1.0);
        }
        self.paused.store(false, Ordering::Relaxed);
    }

    /// 停止（fade_ms > 0 ならフェードアウト後に全デッキを破棄）
    pub fn stop(&self, fade_ms: u32) {
        if fade_ms == 0 || self.paused.load(Ordering::Relaxed) {
            self.clear_decks();
            self.pending_pause.store(false, Ordering::Relaxed);
            self.pending_stop.store(false, Ordering::Relaxed);
            self.master.lock().unwrap_or_else(|e| e.into_inner()).set(1.0);
            self.paused.store(false, Ordering::Relaxed);
            return;
        }
        self.pending_stop.store(true, Ordering::Relaxed);
        self.master.lock().unwrap_or_else(|e| e.into_inner()).start(0.0, self.ms_to_frames(fade_ms));
    }

    /// 新しい曲を始めるとき: 全体フェードを通常状態に戻す
    pub fn reset_master(&self) {
        self.pending_pause.store(false, Ordering::Relaxed);
        self.pending_stop.store(false, Ordering::Relaxed);
        self.master.lock().unwrap_or_else(|e| e.into_inner()).set(1.0);
        self.paused.store(false, Ordering::Relaxed);
    }

    /// 出力デバイスのコールバックから呼ぶ。`out` は交互配置の f32（チャンネル数 = self.channels）
    pub fn fill(&self, out: &mut [f32]) {
        for s in out.iter_mut() {
            *s = 0.0;
        }
        if self.paused.load(Ordering::Relaxed) {
            return;
        }

        let ch = self.channels;
        let frames = out.len() / ch;
        let vol = f32::from_bits(self.volume.load(Ordering::Relaxed));

        {
            let decks = self.decks.lock().unwrap_or_else(|e| e.into_inner());
            for deck in decks.iter() {
                let mut ring = deck.ring.lock().unwrap_or_else(|e| e.into_inner());
                let mut fade = deck.fade.lock().unwrap_or_else(|e| e.into_inner());
                let gain = f32::from_bits(deck.gain.load(Ordering::Relaxed));
                let mut used = 0u64;
                for f in 0..frames {
                    if ring.len() < ch {
                        break; // データ不足（アンダーラン）: 残りは無音
                    }
                    let g = fade.next() * gain;
                    for c in 0..ch {
                        out[f * ch + c] += ring.pop_front().unwrap_or(0.0) * g;
                    }
                    used += 1;
                }
                deck.consumed_frames.fetch_add(used, Ordering::Relaxed);
            }
        }

        // 全体フェード + 音量 + クリップ防止
        let mut master = self.master.lock().unwrap_or_else(|e| e.into_inner());
        for f in 0..frames {
            let m = master.next() * vol;
            for c in 0..ch {
                let s = &mut out[f * ch + c];
                *s = (*s * m).clamp(-1.0, 1.0);
            }
        }
        let silent = master.is_silent();
        drop(master);
        if silent {
            if self.pending_pause.swap(false, Ordering::Relaxed) {
                self.paused.store(true, Ordering::Relaxed);
            }
            if self.pending_stop.swap(false, Ordering::Relaxed) {
                self.clear_decks();
                self.master.lock().unwrap_or_else(|e| e.into_inner()).set(1.0);
                return;
            }
        }

        // フェードアウトし終えた／再生し終えたデッキを片付ける
        let mut decks = self.decks.lock().unwrap_or_else(|e| e.into_inner());
        decks.retain(|d| {
            let faded_out = d.fade.lock().unwrap_or_else(|e| e.into_inner()).is_silent();
            !(faded_out || d.is_drained())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixes_and_fades() {
        let mixer = Mixer::new(2, 1000);
        mixer.set_volume(1.0);
        let deck = Deck::new(1.0, 1.0);
        {
            let mut r = deck.ring.lock().unwrap_or_else(|e| e.into_inner());
            for _ in 0..100 {
                r.push_back(0.5);
                r.push_back(0.5);
            }
        }
        deck.decode_done.store(true, Ordering::Relaxed);
        mixer.add_deck(deck.clone());

        let mut out = vec![0.0f32; 20]; // 10フレーム
        mixer.fill(&mut out);
        assert!(out.iter().all(|&s| (s - 0.5).abs() < 1e-6));
        assert_eq!(deck.consumed_frames.load(Ordering::Relaxed), 10);

        // 一時停止（フェードなし）→ 無音で、消費されない
        mixer.pause(0);
        mixer.fill(&mut out);
        assert!(out.iter().all(|&s| s == 0.0));
        assert_eq!(deck.consumed_frames.load(Ordering::Relaxed), 10);
        mixer.resume(0);

        // 20ms(=20フレーム)かけてフェードアウトする停止: 最終的に全デッキが消える
        mixer.stop(20);
        let mut out2 = vec![0.0f32; 80];
        mixer.fill(&mut out2);
        assert!(out2[0] > out2[38], "フェードアウトで小さくなる");
        assert!(mixer.decks.lock().unwrap_or_else(|e| e.into_inner()).is_empty());
    }

    #[test]
    fn fade_reaches_target() {
        let mut f = Fade::new(0.0);
        f.start(1.0, 4);
        let v: Vec<f32> = (0..5).map(|_| f.next()).collect();
        assert!((v[3] - 1.0).abs() < 1e-6 && (v[4] - 1.0).abs() < 1e-6);
        assert!(v[0] < v[1] && v[1] < v[2]);
    }
}

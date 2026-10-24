//! 音声の出力方式。WASAPI（cpal）・DirectSound・WaveOut の3つを持ち、
//! 指定した方式が使えなければ他の方式へ自動的に切り替える（C# 版の「多層フォールバック」と同じ考え方）。
use super::mixer::Mixer;
use crate::settings::AudioOutputEngine;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::any::Any;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Wasapi,
    DirectSound,
    WaveOut,
}

impl Backend {
    pub fn name(self) -> &'static str {
        match self {
            Backend::Wasapi => "WASAPI",
            Backend::DirectSound => "DirectSound",
            Backend::WaveOut => "WaveOut",
        }
    }
}

/// 開いた出力。`keep` を捨てると出力が止まる
pub struct OpenedOutput {
    pub backend: Backend,
    pub mixer: Arc<Mixer>,
    /// 希望した方式が使えず、別の方式で再生している
    pub is_fallback: bool,
    #[allow(dead_code)] // 持っている間だけ出力が動く（読まれないが、捨てると止まる）
    pub keep: Box<dyn Any>,
}

fn candidates(pref: AudioOutputEngine) -> [Backend; 3] {
    match pref {
        AudioOutputEngine::Auto | AudioOutputEngine::Wasapi => [Backend::Wasapi, Backend::DirectSound, Backend::WaveOut],
        AudioOutputEngine::DirectSound => [Backend::DirectSound, Backend::Wasapi, Backend::WaveOut],
        AudioOutputEngine::WaveOut => [Backend::WaveOut, Backend::Wasapi, Backend::DirectSound],
    }
}

/// 希望の方式で開く。失敗したら他の方式を順に試す
pub fn open_output(pref: AudioOutputEngine) -> Result<OpenedOutput, String> {
    let order = candidates(pref);
    let preferred = order[0];
    let mut last_err = String::from("出力デバイスが見つかりません");
    for backend in order {
        let started = match backend {
            Backend::Wasapi => start_wasapi(),
            Backend::DirectSound => start_directsound(),
            Backend::WaveOut => start_waveout(),
        };
        match started {
            Ok((mixer, keep)) => {
                return Ok(OpenedOutput { backend, mixer, is_fallback: backend != preferred, keep });
            }
            Err(e) => last_err = format!("{}: {}", backend.name(), e),
        }
    }
    Err(last_err)
}

// ================= WASAPI (cpal) =================

fn start_wasapi() -> Result<(Arc<Mixer>, Box<dyn Any>), String> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("既定の出力デバイスがありません")?;
    let config = device.default_output_config().map_err(|e| e.to_string())?;
    let rate = config.sample_rate().0;
    let channels = config.channels() as usize;
    let mixer = Mixer::new(channels, rate);
    let err_cb = |e| eprintln!("Audio stream error: {}", e);

    let m = mixer.clone();
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => {
            device.build_output_stream(&config.into(), move |data: &mut [f32], _: &cpal::OutputCallbackInfo| m.fill(data), err_cb, None)
        }
        cpal::SampleFormat::I16 => {
            let mut tmp: Vec<f32> = Vec::new();
            device.build_output_stream(
                &config.into(),
                move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    tmp.resize(data.len(), 0.0);
                    m.fill(&mut tmp);
                    for (o, s) in data.iter_mut().zip(tmp.iter()) {
                        *o = (s * i16::MAX as f32) as i16;
                    }
                },
                err_cb,
                None,
            )
        }
        cpal::SampleFormat::U16 => {
            let mut tmp: Vec<f32> = Vec::new();
            device.build_output_stream(
                &config.into(),
                move |data: &mut [u16], _: &cpal::OutputCallbackInfo| {
                    tmp.resize(data.len(), 0.0);
                    m.fill(&mut tmp);
                    for (o, s) in data.iter_mut().zip(tmp.iter()) {
                        *o = ((s * 0.5 + 0.5) * u16::MAX as f32) as u16;
                    }
                },
                err_cb,
                None,
            )
        }
        other => return Err(format!("未対応のサンプル形式です: {:?}", other)),
    }
    .map_err(|e| e.to_string())?;

    stream.play().map_err(|e| e.to_string())?;
    Ok((mixer, Box::new(stream)))
}

// ================= 共通: 専用スレッドで動く出力の後始末 =================

/// 専用スレッドを持つ出力。捨てるとスレッドを止めて、デバイスを閉じる
struct ThreadedOutput {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for ThreadedOutput {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

const OUT_RATE: u32 = 48_000;
const OUT_CHANNELS: usize = 2;

// ================= WaveOut (winmm) =================

#[cfg(windows)]
fn start_waveout() -> Result<(Arc<Mixer>, Box<dyn Any>), String> {
    use std::sync::mpsc::channel;
    use windows_sys::Win32::Media::Audio::*;

    let mixer = Mixer::new(OUT_CHANNELS, OUT_RATE);
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = channel::<Result<(), String>>();

    let (m, st) = (mixer.clone(), stop.clone());
    let thread = std::thread::Builder::new()
        .name("WaveOutThread".into())
        .spawn(move || unsafe {
            const BUFFERS: usize = 4;
            const FRAMES: usize = (OUT_RATE as usize) / 20; // 1バッファ 50ms

            // 32bit float を試し、だめなら 16bit PCM
            let mut use_float = true;
            let mut hwo: HWAVEOUT = std::mem::zeroed();
            let mut opened = false;
            for float in [true, false] {
                let (tag, bits) = if float { (3u16, 32u16) } else { (WAVE_FORMAT_PCM as u16, 16u16) };
                let block = (OUT_CHANNELS as u16) * bits / 8;
                let fmt = WAVEFORMATEX {
                    wFormatTag: tag,
                    nChannels: OUT_CHANNELS as u16,
                    nSamplesPerSec: OUT_RATE,
                    nAvgBytesPerSec: OUT_RATE * block as u32,
                    nBlockAlign: block,
                    wBitsPerSample: bits,
                    cbSize: 0,
                };
                if waveOutOpen(&mut hwo, WAVE_MAPPER, &fmt, 0, 0, 0) == 0 {
                    use_float = float;
                    opened = true;
                    break;
                }
            }
            if !opened {
                let _ = tx.send(Err("waveOutOpen に失敗しました".into()));
                return;
            }

            let bytes_per_frame = OUT_CHANNELS * if use_float { 4 } else { 2 };
            let mut data: Vec<Vec<u8>> = (0..BUFFERS).map(|_| vec![0u8; FRAMES * bytes_per_frame]).collect();
            let mut hdrs: Vec<Box<WAVEHDR>> = Vec::new();
            for d in data.iter_mut() {
                let mut h: Box<WAVEHDR> = Box::new(std::mem::zeroed());
                h.lpData = d.as_mut_ptr();
                h.dwBufferLength = d.len() as u32;
                waveOutPrepareHeader(hwo, &mut *h, std::mem::size_of::<WAVEHDR>() as u32);
                hdrs.push(h);
            }
            let _ = tx.send(Ok(()));

            let mut tmp = vec![0.0f32; FRAMES * OUT_CHANNELS];
            let mut queued = [false; BUFFERS];
            while !st.load(Ordering::Relaxed) {
                for i in 0..BUFFERS {
                    // ドライバが書き換えるフラグなので、読む前にメモリの同期を取る
                    std::sync::atomic::fence(Ordering::SeqCst);
                    let flags = std::ptr::addr_of!(hdrs[i].dwFlags).read_unaligned();
                    let done = flags & WHDR_DONE != 0;
                    if !queued[i] || done {
                        m.fill(&mut tmp);
                        let dst = &mut data[i];
                        if use_float {
                            for (k, s) in tmp.iter().enumerate() {
                                dst[k * 4..k * 4 + 4].copy_from_slice(&s.to_le_bytes());
                            }
                        } else {
                            for (k, s) in tmp.iter().enumerate() {
                                let v = (s * i16::MAX as f32) as i16;
                                dst[k * 2..k * 2 + 2].copy_from_slice(&v.to_le_bytes());
                            }
                        }
                        waveOutWrite(hwo, &mut *hdrs[i], std::mem::size_of::<WAVEHDR>() as u32);
                        queued[i] = true;
                    }
                }
                std::thread::sleep(Duration::from_millis(5));
            }

            waveOutReset(hwo);
            for h in hdrs.iter_mut() {
                waveOutUnprepareHeader(hwo, &mut **h, std::mem::size_of::<WAVEHDR>() as u32);
            }
            waveOutClose(hwo);
        })
        .map_err(|e| e.to_string())?;

    match rx.recv_timeout(Duration::from_secs(3)) {
        Ok(Ok(())) => Ok((mixer, Box::new(ThreadedOutput { stop, thread: Some(thread) }))),
        Ok(Err(e)) => {
            let _ = thread.join();
            Err(e)
        }
        Err(_) => {
            stop.store(true, Ordering::Relaxed);
            Err("WaveOut の初期化がタイムアウトしました".into())
        }
    }
}

#[cfg(not(windows))]
fn start_waveout() -> Result<(Arc<Mixer>, Box<dyn Any>), String> {
    Err("Windows のみ対応です".into())
}

// ================= DirectSound (dsound.dll を、他の API と同じ通常のインポートで使う) =================

#[cfg(windows)]
mod ds {
    use std::ffi::c_void;

    #[repr(C)]
    pub struct DsBufferDesc {
        pub size: u32,
        pub flags: u32,
        pub buffer_bytes: u32,
        pub reserved: u32,
        pub format: *mut c_void,
        pub guid_3d: [u8; 16],
    }

    pub const DSSCL_PRIORITY: u32 = 2;
    pub const DSBCAPS_GETCURRENTPOSITION2: u32 = 0x0001_0000;
    pub const DSBCAPS_GLOBALFOCUS: u32 = 0x0000_8000;
    pub const DSBPLAY_LOOPING: u32 = 1;

    pub type CreateFn = unsafe extern "system" fn(*const c_void, *mut *mut c_void, *mut c_void) -> i32;

    // raw-dylib: インポートライブラリ無しで、実行ファイルのインポートテーブルに載せる（実行時の API 解決をしない）
    #[link(name = "dsound", kind = "raw-dylib")]
    extern "system" {
        pub fn DirectSoundCreate8(device: *const c_void, ds: *mut *mut c_void, outer: *mut c_void) -> i32;
    }
    type CreateBufferFn = unsafe extern "system" fn(*mut c_void, *const DsBufferDesc, *mut *mut c_void, *mut c_void) -> i32;
    type SetCoopFn = unsafe extern "system" fn(*mut c_void, *mut c_void, u32) -> i32;
    type GetPosFn = unsafe extern "system" fn(*mut c_void, *mut u32, *mut u32) -> i32;
    type LockFn = unsafe extern "system" fn(*mut c_void, u32, u32, *mut *mut c_void, *mut u32, *mut *mut c_void, *mut u32, u32) -> i32;
    type PlayFn = unsafe extern "system" fn(*mut c_void, u32, u32, u32) -> i32;
    type StopFn = unsafe extern "system" fn(*mut c_void) -> i32;
    type UnlockFn = unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, u32) -> i32;
    type ReleaseFn = unsafe extern "system" fn(*mut c_void) -> u32;

    unsafe fn vtbl(obj: *mut c_void, index: usize) -> *const c_void {
        let table = *(obj as *const *const *const c_void);
        *table.add(index)
    }

    pub unsafe fn release(obj: *mut c_void) {
        if !obj.is_null() {
            let f: ReleaseFn = std::mem::transmute(vtbl(obj, 2));
            f(obj);
        }
    }
    pub unsafe fn set_cooperative_level(ds: *mut c_void, hwnd: *mut c_void) -> i32 {
        let f: SetCoopFn = std::mem::transmute(vtbl(ds, 6));
        f(ds, hwnd, DSSCL_PRIORITY)
    }
    pub unsafe fn create_buffer(ds: *mut c_void, desc: &DsBufferDesc, out: &mut *mut c_void) -> i32 {
        let f: CreateBufferFn = std::mem::transmute(vtbl(ds, 3));
        f(ds, desc, out, std::ptr::null_mut())
    }
    pub unsafe fn get_position(buf: *mut c_void, play: &mut u32, write: &mut u32) -> i32 {
        let f: GetPosFn = std::mem::transmute(vtbl(buf, 4));
        f(buf, play, write)
    }
    pub unsafe fn lock(buf: *mut c_void, offset: u32, bytes: u32, p1: &mut *mut c_void, n1: &mut u32, p2: &mut *mut c_void, n2: &mut u32) -> i32 {
        let f: LockFn = std::mem::transmute(vtbl(buf, 11));
        f(buf, offset, bytes, p1, n1, p2, n2, 0)
    }
    pub unsafe fn play(buf: *mut c_void) -> i32 {
        let f: PlayFn = std::mem::transmute(vtbl(buf, 12));
        f(buf, 0, 0, DSBPLAY_LOOPING)
    }
    pub unsafe fn stop(buf: *mut c_void) -> i32 {
        let f: StopFn = std::mem::transmute(vtbl(buf, 18));
        f(buf)
    }
    pub unsafe fn unlock(buf: *mut c_void, p1: *mut c_void, n1: u32, p2: *mut c_void, n2: u32) -> i32 {
        let f: UnlockFn = std::mem::transmute(vtbl(buf, 19));
        f(buf, p1, n1, p2, n2)
    }
}

#[cfg(windows)]
fn start_directsound() -> Result<(Arc<Mixer>, Box<dyn Any>), String> {
    use std::ffi::c_void;
    use std::sync::mpsc::channel;
    use windows_sys::Win32::Media::Audio::{WAVEFORMATEX, WAVE_FORMAT_PCM};
    use windows_sys::Win32::UI::WindowsAndMessaging::GetDesktopWindow;

    let mixer = Mixer::new(OUT_CHANNELS, OUT_RATE);
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = channel::<Result<(), String>>();

    let (m, st) = (mixer.clone(), stop.clone());
    let thread = std::thread::Builder::new()
        .name("DirectSoundThread".into())
        .spawn(move || unsafe {
            let create: ds::CreateFn = ds::DirectSoundCreate8;

            let mut dsobj: *mut c_void = std::ptr::null_mut();
            if create(std::ptr::null(), &mut dsobj, std::ptr::null_mut()) < 0 || dsobj.is_null() {
                let _ = tx.send(Err("DirectSound デバイスを作成できません".into()));
                return;
            }
            if ds::set_cooperative_level(dsobj, GetDesktopWindow()) < 0 {
                ds::release(dsobj);
                let _ = tx.send(Err("協調レベルを設定できません".into()));
                return;
            }

            // 16bit PCM のストリーミング用循環バッファ（約 200ms）
            const BYTES_PER_FRAME: usize = OUT_CHANNELS * 2;
            let buffer_frames = (OUT_RATE as usize) / 5;
            let buffer_bytes = (buffer_frames * BYTES_PER_FRAME) as u32;
            let mut fmt = WAVEFORMATEX {
                wFormatTag: WAVE_FORMAT_PCM as u16,
                nChannels: OUT_CHANNELS as u16,
                nSamplesPerSec: OUT_RATE,
                nAvgBytesPerSec: OUT_RATE * BYTES_PER_FRAME as u32,
                nBlockAlign: BYTES_PER_FRAME as u16,
                wBitsPerSample: 16,
                cbSize: 0,
            };
            let desc = ds::DsBufferDesc {
                size: std::mem::size_of::<ds::DsBufferDesc>() as u32,
                flags: ds::DSBCAPS_GETCURRENTPOSITION2 | ds::DSBCAPS_GLOBALFOCUS,
                buffer_bytes,
                reserved: 0,
                format: &mut fmt as *mut WAVEFORMATEX as *mut c_void,
                guid_3d: [0; 16],
            };
            let mut buf: *mut c_void = std::ptr::null_mut();
            if ds::create_buffer(dsobj, &desc, &mut buf) < 0 || buf.is_null() {
                ds::release(dsobj);
                let _ = tx.send(Err("サウンドバッファを作成できません".into()));
                return;
            }

            // バッファ全体を一度埋めてから、ループ再生を始める
            let mut tmp: Vec<f32> = Vec::new();
            let mut pcm: Vec<u8> = Vec::new();
            let mut write_pos: u32 = 0;
            let write_chunk = |buf: *mut c_void, offset: u32, bytes: u32, tmp: &mut Vec<f32>, pcm: &mut Vec<u8>| {
                let frames = bytes as usize / BYTES_PER_FRAME;
                tmp.resize(frames * OUT_CHANNELS, 0.0);
                m.fill(tmp);
                pcm.clear();
                for s in tmp.iter() {
                    pcm.extend_from_slice(&((s * i16::MAX as f32) as i16).to_le_bytes());
                }
                let (mut p1, mut p2): (*mut c_void, *mut c_void) = (std::ptr::null_mut(), std::ptr::null_mut());
                let (mut n1, mut n2) = (0u32, 0u32);
                if ds::lock(buf, offset, bytes, &mut p1, &mut n1, &mut p2, &mut n2) >= 0 {
                    if !p1.is_null() {
                        std::ptr::copy_nonoverlapping(pcm.as_ptr(), p1 as *mut u8, n1 as usize);
                    }
                    if !p2.is_null() {
                        std::ptr::copy_nonoverlapping(pcm.as_ptr().add(n1 as usize), p2 as *mut u8, n2 as usize);
                    }
                    ds::unlock(buf, p1, n1, p2, n2);
                }
            };
            write_chunk(buf, 0, buffer_bytes, &mut tmp, &mut pcm);
            if ds::play(buf) < 0 {
                ds::release(buf);
                ds::release(dsobj);
                let _ = tx.send(Err("再生を開始できません".into()));
                return;
            }
            let _ = tx.send(Ok(()));

            let chunk_min = (OUT_RATE as usize / 100 * BYTES_PER_FRAME) as u32; // 10ms
            while !st.load(Ordering::Relaxed) {
                let (mut play_cursor, mut write_cursor) = (0u32, 0u32);
                if ds::get_position(buf, &mut play_cursor, &mut write_cursor) >= 0 {
                    // 再生カーソルより手前（再生済み）の領域を新しい音で埋め直す
                    let free = (play_cursor + buffer_bytes - write_pos) % buffer_bytes;
                    let free = free - free % BYTES_PER_FRAME as u32;
                    if free >= chunk_min {
                        write_chunk(buf, write_pos, free, &mut tmp, &mut pcm);
                        write_pos = (write_pos + free) % buffer_bytes;
                    }
                }
                std::thread::sleep(Duration::from_millis(4));
            }

            ds::stop(buf);
            ds::release(buf);
            ds::release(dsobj);
        })
        .map_err(|e| e.to_string())?;

    match rx.recv_timeout(Duration::from_secs(3)) {
        Ok(Ok(())) => Ok((mixer, Box::new(ThreadedOutput { stop, thread: Some(thread) }))),
        Ok(Err(e)) => {
            let _ = thread.join();
            Err(e)
        }
        Err(_) => {
            stop.store(true, Ordering::Relaxed);
            Err("DirectSound の初期化がタイムアウトしました".into())
        }
    }
}

#[cfg(not(windows))]
fn start_directsound() -> Result<(Arc<Mixer>, Box<dyn Any>), String> {
    Err("Windows のみ対応です".into())
}

#[cfg(all(test, windows))]
mod tests {
    /// DirectSound を通常のインポートで呼べること（出力デバイスが無い環境では何もしない）
    #[test]
    fn directsound_opens() {
        match super::start_directsound() {
            Ok((_mixer, _keep)) => {}
            Err(e) => eprintln!("DirectSound を開けませんでした（デバイス無しの環境かもしれません）: {e}"),
        }
    }
}

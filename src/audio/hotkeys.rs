//! メディアキー・グローバルホットキー（他のアプリを操作中でも再生操作できる）。C# 版 HotkeyService 相当。
//! 専用スレッドでキーを登録し、メッセージループで受けた操作を UI スレッドへ渡す。
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

pub use super::hotkeys_defs::HotkeyAction;
use super::hotkeys_defs::{default_bindings, Binding};

const MOD_NOREPEAT: u32 = 0x4000;

pub struct HotkeyService {
    thread: Option<JoinHandle<()>>,
    thread_id: Arc<AtomicU32>,
    rx: Receiver<HotkeyAction>,
    tx: Sender<HotkeyAction>,
    /// 登録に失敗したキー（他のアプリが使用中など）
    failed: Arc<Mutex<Vec<String>>>,
    ctx: egui::Context,
    /// 登録するキーの一覧（設定で変えられる）
    bindings: Vec<Binding>,
}

impl HotkeyService {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, rx) = channel();
        Self {
            thread: None,
            thread_id: Arc::new(AtomicU32::new(0)),
            rx,
            tx,
            failed: Arc::new(Mutex::new(Vec::new())),
            ctx,
            bindings: default_bindings(),
        }
    }

    /// 割り当てを入れ替える。有効なときは、登録し直す
    pub fn set_bindings(&mut self, bindings: Vec<Binding>) {
        if bindings == self.bindings {
            return;
        }
        self.bindings = bindings;
        if self.is_enabled() {
            self.stop();
            self.start();
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.thread.is_some()
    }

    pub fn failed_keys(&self) -> Vec<String> {
        self.failed.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn poll(&self) -> Vec<HotkeyAction> {
        self.rx.try_iter().collect()
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        if enabled == self.is_enabled() {
            return;
        }
        if enabled {
            self.start();
        } else {
            self.stop();
        }
    }

    #[cfg(windows)]
    fn start(&mut self) {
        use windows_sys::Win32::System::Threading::GetCurrentThreadId;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey};
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY};

        self.failed.lock().unwrap_or_else(|e| e.into_inner()).clear();
        let tx = self.tx.clone();
        let failed = self.failed.clone();
        let tid_slot = self.thread_id.clone();
        let ctx = self.ctx.clone();
        let bindings = self.bindings.clone();
        let (ready_tx, ready_rx) = channel::<()>();

        self.thread = std::thread::Builder::new()
            .name("HotkeyThread".into())
            .spawn(move || unsafe {
                tid_slot.store(GetCurrentThreadId(), Ordering::SeqCst);
                let mut registered = Vec::new();
                for (i, b) in bindings.iter().enumerate() {
                    let id = (0x5A00 + i) as i32;
                    if RegisterHotKey(std::ptr::null_mut(), id, b.combo.modifiers | MOD_NOREPEAT, b.combo.key) != 0 {
                        registered.push(id);
                    } else {
                        failed.lock().unwrap_or_else(|e| e.into_inner()).push(b.combo.name());
                    }
                }
                let _ = ready_tx.send(());

                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                    if msg.message == WM_HOTKEY {
                        let idx = msg.wParam as i32 - 0x5A00;
                        if idx >= 0 && (idx as usize) < bindings.len() {
                            let _ = tx.send(bindings[idx as usize].action);
                            ctx.request_repaint();
                        }
                    }
                }
                for id in registered {
                    UnregisterHotKey(std::ptr::null_mut(), id);
                }
            })
            .ok();
        let _ = ready_rx.recv_timeout(std::time::Duration::from_secs(2));
    }

    #[cfg(not(windows))]
    fn start(&mut self) {}

    fn stop(&mut self) {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
            let tid = self.thread_id.load(Ordering::SeqCst);
            if tid != 0 {
                PostThreadMessageW(tid, WM_QUIT, 0, 0);
            }
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        self.thread_id.store(0, Ordering::SeqCst);
    }
}

impl Drop for HotkeyService {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn enable_disable_cycle() {
        let mut hk = HotkeyService::new(egui::Context::default());
        assert!(!hk.is_enabled());
        hk.set_enabled(true);
        assert!(hk.is_enabled());
        // 登録に失敗するキーがあっても（他のアプリが使用中など）、スレッドは動き続ける
        eprintln!("登録できなかったキー: {:?}", hk.failed_keys());
        assert!(hk.failed_keys().len() <= 12);
        hk.set_enabled(false); // メッセージループを止めて、スレッドが終了する
        assert!(!hk.is_enabled());
        hk.set_enabled(true); // 再度有効化できる
        assert!(hk.is_enabled());
        drop(hk);
    }
}

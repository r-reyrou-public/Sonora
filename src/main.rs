#![windows_subsystem = "windows"]
// 画面描画の関数は、部品ごとの状態と通知を引数で受け取る作りなので、引数が多くなる
#![allow(clippy::too_many_arguments)]

mod app;
mod audio;
mod backup;
mod db;
mod filecache;
mod lyrics;
mod paths;
mod playlist;
mod relink;
mod scanner;
mod search;
mod store;
mod trash;
mod theme;
mod update;
mod settings;
mod tools;
mod views;

use app::MusicManagerApp;
use eframe::egui::{FontData, FontDefinitions, FontFamily};
use settings::AppSettings;

const WINDOW_TITLE: &str = "Sonora — 音楽管理";

/// ウィンドウ・タスクバー用のアイコン（EXE のアイコンと同じ絵）
fn window_icon() -> Option<egui::IconData> {
    let img = image::load_from_memory(include_bytes!("../assets/sonora.png")).ok()?.to_rgba8();
    let (width, height) = img.dimensions();
    Some(egui::IconData { rgba: img.into_raw(), width, height })
}

/// 多重起動を防ぐ。すでに起動中なら、そのウィンドウを前面に出して `false` を返す
#[cfg(windows)]
fn acquire_single_instance() -> bool {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE,
    };

    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    unsafe {
        // ログオン中のユーザーごとに1つ（他のユーザーの起動は妨げない）
        let name = wide("Local\\Sonora.SingleInstance");
        let handle = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        if handle.is_null() {
            return true; // 作れなかったときは、起動を止めない
        }
        if GetLastError() != ERROR_ALREADY_EXISTS {
            // 終了までハンドルを持ち続ける（閉じない）
            return true;
        }
        let title = wide(WINDOW_TITLE);
        let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
        if !hwnd.is_null() {
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            SetForegroundWindow(hwnd);
        }
        false
    }
}

#[cfg(not(windows))]
fn acquire_single_instance() -> bool {
    true
}

/// 予期しない異常（パニック）で終了するとき、原因を `crash.log`（データ保存先）に書き残す。
/// 画面のないアプリなので、何も残らないと原因が分からない。書くのは、場所・内容・スレッド名・時刻だけ
fn install_crash_log() {
    std::panic::set_hook(Box::new(|info| {
        use std::io::Write;
        let path = paths::data_file("crash.log");
        // 溜まりすぎないよう、大きくなったら作り直す
        if std::fs::metadata(&path).map(|m| m.len() > 512 * 1024).unwrap_or(false) {
            let _ = std::fs::remove_file(&path);
        }
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(内容なし)".to_string());
        let place = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())).unwrap_or_default();
        let thread = std::thread::current().name().unwrap_or("(名前なし)").to_string();
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "[unix {}] Sonora {} パニック thread={} at {} : {}", secs, update::APP_VERSION, thread, place, payload);
        }
    }));
}

fn main() -> eframe::Result<()> {
    if !acquire_single_instance() {
        return Ok(());
    }
    install_crash_log();

    // 日本語フォントのファイル読み込み(数MB)は、ウィンドウ/GL初期化と並行して先に始めておく
    let font_loader = std::thread::spawn(read_japanese_font);

    let settings = AppSettings::load(&paths::settings_file());

    let mut viewport = egui::ViewportBuilder::default().with_title(WINDOW_TITLE);
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(icon);
    }
    let mut native_options = eframe::NativeOptions {
        viewport: viewport
            .with_inner_size([settings.window_width, settings.window_height])
            .with_min_inner_size([720.0, 480.0]),
        ..Default::default()
    };

    // 保存されていた位置が、今のモニター配置の外（モニターを外した後など）なら使わない
    if settings.window_left > 0.0 && settings.window_top > 0.0 && on_screen(settings.window_left, settings.window_top) {
        native_options.viewport = native_options
            .viewport
            .with_position([settings.window_left, settings.window_top]);
    }

    if settings.is_maximized {
        native_options.viewport = native_options.viewport.with_maximized(true);
    }

    eframe::run_native(
        "MusicManagerApp",
        native_options,
        Box::new(|cc| {
            setup_japanese_fonts(&cc.egui_ctx, font_loader.join().ok().flatten());
            Ok(Box::new(MusicManagerApp::new(cc)))
        }),
    )
}

/// ウィンドウの左上が、全モニターを合わせた範囲に十分収まっているか（タイトルバーを掴める位置か）
#[cfg(windows)]
fn on_screen(left: f32, top: f32) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    };
    let (x, y, w, h) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN) as f32,
            GetSystemMetrics(SM_YVIRTUALSCREEN) as f32,
            GetSystemMetrics(SM_CXVIRTUALSCREEN) as f32,
            GetSystemMetrics(SM_CYVIRTUALSCREEN) as f32,
        )
    };
    if w <= 0.0 || h <= 0.0 {
        return true; // 取得できなければ従来どおり
    }
    // 論理座標と物理座標の差があっても外れにくいよう、余裕を持って判定する
    left >= x - 100.0 && top >= y - 10.0 && left < x + w - 150.0 && top < y + h - 100.0
}

#[cfg(not(windows))]
fn on_screen(_left: f32, _top: f32) -> bool {
    true
}

fn read_japanese_font() -> Option<Vec<u8>> {
    [
        r"C:\Windows\Fonts\meiryo.ttc",
        r"C:\Windows\Fonts\YuGothM.ttc",
        r"C:\Windows\Fonts\msgothic.ttc",
    ]
    .iter()
    .find_map(|path| std::fs::read(path).ok())
}

fn setup_japanese_fonts(ctx: &egui::Context, font_bytes: Option<Vec<u8>>) {
    let mut fonts = FontDefinitions::default();

    if let Some(font_bytes) = font_bytes {
        fonts.font_data.insert("JapaneseFont".to_owned(), FontData::from_owned(font_bytes));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts.families.entry(family).or_default().insert(0, "JapaneseFont".to_owned());
        }
    }

    ctx.set_fonts(fonts);
}

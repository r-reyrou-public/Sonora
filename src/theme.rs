//! カラーテーマ（システム準拠 / ライト / ダーク）。
//! UI のコードはダーク用の色を `theme::c(r, g, b)` で指定し、ライトテーマのときだけ対応する明るい色に置き換える。
use crate::settings::AppThemeMode;
use egui::{Color32, Context, Stroke, Visuals};
use std::sync::atomic::{AtomicBool, Ordering};

static DARK: AtomicBool = AtomicBool::new(true);

pub fn is_dark() -> bool {
    DARK.load(Ordering::Relaxed)
}

/// ダークテーマ基準の色を、現在のテーマに合わせて返す
pub fn c(r: u8, g: u8, b: u8) -> Color32 {
    if is_dark() {
        return Color32::from_rgb(r, g, b);
    }
    let (r2, g2, b2) = match (r, g, b) {
        // 背景・面・罫線
        (15, 23, 42) => (248, 250, 252),
        (30, 41, 59) => (226, 232, 240),
        (51, 65, 85) => (203, 213, 225),
        (71, 85, 105) => (176, 190, 208),
        (40, 60, 90) | (30, 64, 100) => (191, 219, 254),
        (40, 50, 70) => (226, 232, 240),
        // 文字
        (241, 245, 249) => (15, 23, 42),
        (226, 232, 240) => (30, 41, 59),
        (203, 213, 225) => (51, 65, 85),
        (148, 163, 184) | (100, 116, 139) => (100, 116, 139),
        // アクセント色（明るい背景でも読める濃さにする）
        (34, 197, 94) => (22, 163, 74),
        (248, 113, 113) => (220, 38, 38),
        (147, 197, 253) => (29, 78, 216),
        (96, 165, 250) => (37, 99, 235),
        (251, 146, 60) => (234, 88, 12),
        (56, 189, 248) => (2, 132, 199),
        (251, 191, 36) => (180, 83, 9),
        (52, 211, 153) => (5, 150, 105),
        (250, 204, 21) => (161, 98, 7),
        (71, 140, 200) => (59, 130, 246),
        other => other,
    };
    Color32::from_rgb(r2, g2, b2)
}

/// Windows のアプリ用テーマ設定（ダークか）。取得できなければダーク
pub fn system_is_dark() -> bool {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
        let sub: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\0".encode_utf16().collect();
        let val: Vec<u16> = "AppsUseLightTheme\0".encode_utf16().collect();
        let mut data: u32 = 1;
        let mut size = std::mem::size_of::<u32>() as u32;
        let rc = RegGetValueW(
            HKEY_CURRENT_USER,
            sub.as_ptr(),
            val.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            &mut data as *mut u32 as *mut _,
            &mut size,
        );
        if rc == 0 {
            return data == 0;
        }
    }
    true
}

pub fn resolve_dark(mode: AppThemeMode) -> bool {
    match mode {
        AppThemeMode::Dark => true,
        AppThemeMode::Light => false,
        AppThemeMode::System => system_is_dark(),
    }
}

/// テーマを適用する（egui の既定ビジュアルと、`c()` の切り替え）
pub fn apply(ctx: &Context, mode: AppThemeMode) {
    let dark = resolve_dark(mode);
    DARK.store(dark, Ordering::Relaxed);

    let mut v = if dark { Visuals::dark() } else { Visuals::light() };
    v.override_text_color = Some(c(241, 245, 249));
    v.panel_fill = c(15, 23, 42);
    v.window_fill = c(15, 23, 42);
    v.extreme_bg_color = if dark { Color32::from_rgb(10, 10, 10) } else { Color32::WHITE };
    v.widgets.noninteractive.bg_fill = c(30, 41, 59);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, c(51, 65, 85));
    v.widgets.inactive.bg_fill = c(30, 41, 59);
    v.widgets.hovered.bg_fill = c(51, 65, 85);
    v.widgets.active.bg_fill = c(71, 85, 105);
    if !dark {
        v.selection.bg_fill = Color32::from_rgb(191, 219, 254);
        v.selection.stroke = Stroke::new(1.0_f32, Color32::from_rgb(29, 78, 216));
    }
    ctx.set_visuals(v);
}

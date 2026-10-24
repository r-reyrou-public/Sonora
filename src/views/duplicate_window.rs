//! 重複検出（ハッシュチェック）の別ウィンドウ: 進捗・重複ファイルの名前とフルパス・聴き比べ
use crate::tools::hash_scan::{HashGroup, HashScan};
use egui::{RichText, ScrollArea, Ui};
use std::path::PathBuf;
use std::time::Instant;

/// 画面を作り直す間隔（検索中の重複一覧を、毎フレーム作り直さないため）
const REFRESH_SECS: f32 = 0.5;

pub struct DuplicateWindow {
    pub scan: HashScan,
    /// ビューポート ID を新しくするための世代
    pub gen: u64,
    pub frames: u32,
    groups: Vec<HashGroup>,
    refreshed: Option<Instant>,
    /// 検査が終わったあとの最終の一覧を作ったか
    final_done: bool,
}

pub enum DuplicateAction {
    Play(PathBuf),
    Stop,
    Reveal(PathBuf),
}

impl DuplicateWindow {
    pub fn new(scan: HashScan, gen: u64) -> Self {
        Self { scan, gen, frames: 0, groups: Vec::new(), refreshed: None, final_done: false }
    }
}

/// `now_playing`: 再生エンジンが実際に流している曲（再生中のみ）
pub fn render_duplicate_window(
    ui: &mut Ui,
    w: &mut DuplicateWindow,
    now_playing: Option<&PathBuf>,
) -> Option<DuplicateAction> {
    let mut action = None;
    let prog = w.scan.progress();

    // 検索中は一定間隔で、終わったら最後に一度、重複の一覧を作り直す
    let stale = w.refreshed.is_none_or(|t| t.elapsed().as_secs_f32() >= REFRESH_SECS);
    let due = if prog.running { stale } else { !w.final_done };
    if due {
        w.groups = w.scan.groups();
        w.refreshed = Some(Instant::now());
        w.final_done = !prog.running;
    }
    if prog.running {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }

    // ---- 進捗 ----
    let frac = if prog.total == 0 { 1.0 } else { prog.done as f32 / prog.total as f32 };
    ui.horizontal(|ui| {
        let state = if prog.running {
            "検査中…"
        } else if prog.cancelled {
            "中止しました"
        } else {
            "完了"
        };
        ui.label(RichText::new(state).strong());
        ui.label(format!("{} / {} ファイル", prog.done, prog.total));
        ui.label(RichText::new(format!("経過 {}", fmt_time(prog.elapsed_secs))).weak());
        if prog.failed > 0 {
            ui.label(
                RichText::new(format!("読めなかったファイル {}", prog.failed))
                    .color(crate::theme::c(251, 191, 36))
                    .size(12.0),
            )
            .on_hover_text("音声として読めなかったファイルは、比較の対象から外しています");
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if prog.running && ui.button("⏹ 中止").on_hover_text("検出を途中でやめます").clicked() {
                w.scan.cancel();
            }
        });
    });
    ui.add(egui::ProgressBar::new(frac).show_percentage());
    if prog.running {
        ui.label(RichText::new(format!("確認中: {}", prog.current)).weak().size(11.0));
    }
    ui.separator();

    // ---- 結果 ----
    let files: usize = w.groups.iter().map(|g| g.entries.len()).sum();
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("重複グループ: {} 件（{} ファイル）", w.groups.len(), files)).strong());
        ui.label(RichText::new("タグを除いた音声データが同じファイルです").weak().size(11.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if now_playing.is_some() && ui.button("⏹ 再生を止める").on_hover_text("聴き比べの再生を止めます").clicked() {
                action = Some(DuplicateAction::Stop);
            }
        });
    });
    ui.add_space(4.0);

    if w.groups.is_empty() {
        let msg = if prog.running { "ここまでに重複は見つかっていません…" } else { "重複するファイルは見つかりませんでした。" };
        ui.label(RichText::new(msg).weak());
        return action;
    }

    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for (gi, g) in w.groups.iter().enumerate() {
            ui.group(|ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("G-{:02}", gi + 1)).strong().color(crate::theme::c(251, 146, 60)));
                    ui.label(RichText::new(format!("{} ファイル", g.entries.len())).weak());
                });
                for e in &g.entries {
                    let on = now_playing.is_some_and(|p| crate::store::path_key(p) == crate::store::path_key(&e.path));
                    ui.horizontal(|ui| {
                        let (icon, tip) = if on { ("⏹", "再生を止める") } else { ("▶", "この曲を再生して聴き比べる") };
                        if ui.add_sized([28.0, 22.0], egui::Button::new(icon)).on_hover_text(tip).clicked() {
                            action = Some(if on { DuplicateAction::Stop } else { DuplicateAction::Play(e.path.clone()) });
                        }
                        ui.vertical(|ui| {
                            let name = RichText::new(&e.name).strong();
                            ui.label(if on { name.color(crate::theme::c(34, 197, 94)) } else { name });
                            ui.label(RichText::new(e.path.to_string_lossy().to_string()).weak().size(11.0));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("📂 ツリーで開く").clicked() {
                                action = Some(DuplicateAction::Reveal(e.path.clone()));
                            }
                            ui.label(RichText::new(&e.size_text).weak());
                        });
                    });
                }
            });
            ui.add_space(4.0);
        }
    });
    action
}

fn fmt_time(secs: f32) -> String {
    let s = secs as u32;
    if s >= 3600 {
        format!("{}時間{:02}分{:02}秒", s / 3600, s % 3600 / 60, s % 60)
    } else {
        format!("{}分{:02}秒", s / 60, s % 60)
    }
}

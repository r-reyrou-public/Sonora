//! ミニプレイヤー（常に手前に表示できる小さな操作パネル）。C# 版 MiniPlayerWindow 相当。
use crate::audio::player::PlaybackState;
use crate::scanner::metadata::TrackItem;
use egui::{RichText, TextureHandle, Ui, Vec2};
use std::time::Duration;

pub enum MiniAction {
    PlayPause,
    Prev,
    Next,
    Stop,
    Seek(Duration),
    /// 「常に手前に表示」の切り替え
    Topmost(bool),
}

fn fmt(d: Duration) -> String {
    let s = d.as_secs();
    format!("{:02}:{:02}", s / 60, s % 60)
}

pub fn render_mini_player(
    ui: &mut Ui,
    track: Option<&TrackItem>,
    art: Option<&TextureHandle>,
    state: PlaybackState,
    position: Duration,
    total: Duration,
    topmost: bool,
) -> Option<MiniAction> {
    let mut action = None;

    // 曲情報
    ui.horizontal(|ui| {
        match art {
            Some(tex) => {
                ui.add(egui::Image::new((tex.id(), Vec2::splat(36.0))).rounding(4.0));
            }
            None => {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(36.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 4.0, crate::theme::c(30, 41, 59));
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "🎵", egui::FontId::proportional(16.0), crate::theme::c(100, 116, 139));
            }
        }
        ui.vertical(|ui| {
            let (title, artist) = match track {
                Some(t) => (if t.tag_title.is_empty() { t.filename.clone() } else { t.tag_title.clone() }, t.artist.clone()),
                None => ("再生停止中".to_string(), String::new()),
            };
            ui.add(egui::Label::new(RichText::new(title).strong().size(13.0)).truncate());
            ui.add(egui::Label::new(RichText::new(artist).weak().size(11.0)).truncate());
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            let pin = ui.selectable_label(topmost, "📌").on_hover_text("常に手前に表示");
            if pin.clicked() {
                action = Some(MiniAction::Topmost(!topmost));
            }
        });
    });

    // シークバー
    ui.horizontal(|ui| {
        ui.label(RichText::new(fmt(position)).weak().size(10.0));
        let total_secs = total.as_secs_f64().max(1.0);
        let mut sec = position.as_secs_f64().min(total_secs);
        ui.scope(|ui| {
            ui.visuals_mut().widgets.inactive.bg_fill = crate::theme::c(71, 85, 105);
            ui.spacing_mut().slider_width = (ui.available_width() - 50.0).max(60.0);
            let resp = ui.add(egui::Slider::new(&mut sec, 0.0..=total_secs).show_value(false).trailing_fill(true));
            if resp.drag_stopped() || (resp.clicked() && !resp.dragged()) {
                action = Some(MiniAction::Seek(Duration::from_secs_f64(sec)));
            }
        });
        ui.label(RichText::new(fmt(total)).weak().size(10.0));
    });

    // 操作ボタン
    ui.horizontal(|ui| {
        let w = ui.available_width();
        ui.add_space(((w - (40.0 * 3.0 + 48.0 + 18.0)) / 2.0).max(0.0));
        if ui.add_sized([40.0, 26.0], egui::Button::new("⏮")).on_hover_text("前の曲").clicked() {
            action = Some(MiniAction::Prev);
        }
        let icon = if state == PlaybackState::Playing { "⏸" } else { "▶" };
        if ui.add_sized([48.0, 26.0], egui::Button::new(RichText::new(icon).strong())).on_hover_text("再生 / 一時停止").clicked() {
            action = Some(MiniAction::PlayPause);
        }
        if ui.add_sized([40.0, 26.0], egui::Button::new("⏹")).on_hover_text("停止").clicked() {
            action = Some(MiniAction::Stop);
        }
        if ui.add_sized([40.0, 26.0], egui::Button::new("⏭")).on_hover_text("次の曲").clicked() {
            action = Some(MiniAction::Next);
        }
    });

    action
}

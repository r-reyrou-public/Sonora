use crate::scanner::metadata::TrackItem;
use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
use crate::views::columns::{ColumnConfig, ColumnId};
use crate::views::media_cache::{waveform_secs, MediaCache, WAVEFORM_BINS};
use egui_extras::{Column, TableBuilder};
use crate::audio::player::{PlaybackState, RepeatMode};
use std::collections::HashSet;
use std::time::Duration;

#[derive(Clone, Debug)]
pub enum TrackAction {
    Play(usize),
    Select(usize),
    EditTags(usize),
    /// 複数選択中の曲のタグを一括編集
    EditTagsMulti,
    EditLabels(usize),
    ShowDetails(usize),
    OpenExplorer(usize),
    CopyPath(usize),
    /// この曲をお気に入りに追加
    AddFavorite(usize),
    Delete(usize),
    /// 一覧に表示したフォルダ行を開く
    OpenFolder(std::path::PathBuf),
    /// フォルダ行の右クリックメニュー（お気に入り・ラベル）
    Folder(crate::views::folder_tree::FavAction),
    /// ラベル一覧のフォルダ行をクリック: 配下のファイルの展開 / 折りたたみ
    ToggleFolder(std::path::PathBuf),
    /// プレイリストペイン: 一覧からこの曲を外す
    RemoveFromPlaylist(usize),
    /// ラベル列をクリック: ラベル選択ポップアップを開く（曲の番号, 表示位置）
    OpenLabelPicker(usize, egui::Pos2),
    /// 情報パネルの操作
    ToggleShuffle,
    CycleRepeat,
    TogglePlay(usize),
    Prev,
    Next,
    Stop,
    /// 逆再生の入り切り
    ToggleReverse,
    /// 再生速度（1.0 が等倍）
    SetSpeed(f32),
    Seek(Duration),
    /// 選択中のプレイリストに追加
    AddToPlaylist(usize),
    /// 複数選択中の曲をまとめてプレイリストに追加（すべて登録済みなら、まとめて削除）
    AddMultiToPlaylist,
}

/// Ctrl/Shift で複数選択した曲（メイン一覧のみ。パスで持つので、並びが変わってもずれない）
pub fn multi_sel_id() -> egui::Id {
    egui::Id::new("track_list_multi_selection")
}

/// 列の幅を覚えておく場所（egui のメモリ。設定ファイルへは app 側が保存する）
pub fn col_widths_id() -> egui::Id {
    egui::Id::new("track_list_col_widths")
}

pub fn render_track_list(
    ui: &mut Ui,
    tracks: &[TrackItem],
    selected_track: &Option<usize>,
    current_playing: &Option<std::path::PathBuf>,
    current_position: Duration,
    total_duration: Duration,
    playback: PlaybackState,
    shuffle: bool,
    repeat_mode: RepeatMode,
    reverse: bool,
    speed: f32,
    media_cache: &mut MediaCache,
    columns: &ColumnConfig,
    folders: &[(String, std::path::PathBuf)],
    playlist_name: Option<&str>,
    // 選択中プレイリストに登録済みの曲（メイン一覧で背景色を変える）
    playlist_paths: Option<&HashSet<String>>,
    // true: プレイリストペイン用（情報パネルなし・メニューも専用）
    is_pane: bool,
    // true: ラベルで絞り込んだ一覧（フォルダ行のクリックで展開 / 折りたたみ）
    label_view: bool,
    on_action: &mut impl FnMut(TrackAction),
) {
    // --- 1. 上部: 選択中または再生中トラックの「波形・再生時間・ID3タグ・ラベル」区切りインフォパネル ---
    let active_idx = (if is_pane { None } else { selected_track.filter(|&i| i < tracks.len()) }).or_else(|| {
        if is_pane {
            return None;
        }
        current_playing
            .as_ref()
            .and_then(|p| tracks.iter().position(|t| &t.file_path == p))
    });

    // メイン領域が狭くても（右ペイン表示中など）内容が隠れないよう、波形幅は可変にして折り返す
    // プレイリストペイン表示中か（メイン一覧側の情報パネルの配置を固定するために使う）
    let pane_open = playlist_paths.is_some() && !is_pane;
    let wave_w = (ui.available_width() * 0.28).clamp(110.0, 260.0);

    if let Some((track, active_idx)) = active_idx.map(|i| (&tracks[i], i)) {
        let is_playing = current_playing
            .as_ref()
            .map(|p| p == &track.file_path)
            .unwrap_or(false);
        let total_secs = if is_playing && total_duration.as_secs_f32() > 0.0 {
            total_duration.as_secs_f32()
        } else {
            track.duration.as_secs_f32()
        };
        let progress = if is_playing && total_secs > 0.0 {
            (current_position.as_secs_f32() / total_secs).clamp(0.0, 1.0)
        } else {
            0.0
        };
        // 全体波形はバックグラウンドで生成（完成までは中央線のみ）
        let full_peaks = media_cache.get_full(&track.file_path, track.duration.as_secs_f32());
        let art = media_cache.get(&track.file_path).and_then(|m| m.art);

        egui::Frame::none()
            .fill(crate::theme::c(30, 41, 59))
            .stroke(Stroke::new(1.0_f32, crate::theme::c(51, 65, 85)))
            .inner_margin(egui::Margin::symmetric(10.0, 8.0))
            .rounding(6.0)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    // 左側: ジャケットまたはアイコン
                    match &art {
                        Some(tex) => {
                            ui.add(egui::Image::new((tex.id(), Vec2::splat(44.0))).rounding(4.0));
                        }
                        None => {
                            ui.allocate_ui_with_layout(
                                Vec2::new(44.0, 44.0),
                                egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                                |ui| {
                                    let icon = if is_playing { "🔊" } else { "🎵" };
                                    ui.label(RichText::new(icon).size(26.0));
                                },
                            );
                        }
                    }

                    ui.separator();

                    // タイトル ＆ アーティスト ＆ 再生時間
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&track.filename).strong().size(13.0).color(crate::theme::c(241, 245, 249)));
                            ui.label(RichText::new(&track.formatted_duration).size(12.0).color(crate::theme::c(56, 189, 248)));
                        });
                        let artist_line = if !track.artist.is_empty() {
                            format!("{}{} • {}", if track.tag_title.is_empty() { String::new() } else { format!("{} / ", track.tag_title) }, track.artist, track.format)
                        } else {
                            track.format.clone()
                        };
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(artist_line).weak().size(11.0));
                            if track.has_lyrics {
                                lyrics_pill(ui);
                            }
                        });
                    });

                    ui.separator();

                    // 曲全体の実波形（クリックでその位置へシーク）
                    let wave = draw_waveform(
                        ui,
                        Vec2::new(wave_w, 40.0),
                        full_peaks.as_deref(),
                        progress,
                        Sense::click(),
                    );
                    if wave.hovered() && is_playing {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if wave.clicked() && is_playing {
                        if let Some(pos) = wave.interact_pointer_pos() {
                            let frac = ((pos.x - wave.rect.left()) / wave.rect.width()).clamp(0.0, 1.0);
                            on_action(TrackAction::Seek(Duration::from_secs_f32(frac * total_secs)));
                        }
                    }

                    ui.separator();

                    // ID3タグ ＆ ラベル
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("ID3:").weak().size(11.0));
                            let tag_text = format!(
                                "{} | {}",
                                if track.genre.is_empty() { "ジャンル未設定" } else { &track.genre },
                                if track.year == 0 { "年未設定".to_string() } else { format!("{}年", track.year) }
                            );
                            ui.label(RichText::new(tag_text).size(11.0).color(crate::theme::c(203, 213, 225)));
                        });

                        ui.horizontal(|ui| {
                            ui.label(RichText::new("ラベル:").weak().size(11.0));
                            if track.custom_tags.is_empty() {
                                ui.label(RichText::new("なし").weak().size(11.0));
                            } else {
                                for t in &track.custom_tags {
                                    let badge = RichText::new(format!("[{}]", t))
                                        .size(11.0)
                                        .color(crate::theme::c(251, 146, 60));
                                    ui.label(badge);
                                }
                            }
                        });
                    });

                    // プレイリストペインが開いている間は、幅に関わらず常に「上段: 曲情報、下段: 再生ボタンとシークバー」の2段にする
                    if pane_open {
                        ui.end_row();
                    } else {
                        ui.separator();
                    }

                    // 右側: 再生ボタン群 ＆ シークバー
                    ui.vertical(|ui| {
                        let buttons_w = ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            let btn = |text: &str| egui::Button::new(RichText::new(text.to_string()).size(15.0));

                            // シャッフル
                            let shuffle_color = if shuffle { crate::theme::c(34, 197, 94) } else { crate::theme::c(148, 163, 184) };
                            if ui
                                .add_sized([32.0, 26.0], egui::Button::new(RichText::new("🔀").size(14.0).color(shuffle_color)))
                                .on_hover_text(if shuffle { "シャッフル: ON" } else { "シャッフル: OFF" })
                                .clicked()
                            {
                                on_action(TrackAction::ToggleShuffle);
                            }
                            // 逆再生
                            let rev_color = if reverse { crate::theme::c(56, 189, 248) } else { crate::theme::c(148, 163, 184) };
                            if ui
                                .add_enabled(
                                    is_playing,
                                    egui::Button::new(RichText::new("⏪").size(14.0).color(rev_color)).min_size(Vec2::new(32.0, 26.0)),
                                )
                                .on_hover_text(if reverse { "逆再生: オン（クリックで順方向に戻す）" } else { "逆再生: オフ（クリックで、いまの位置から逆向きに再生）" })
                                .on_disabled_hover_text("逆再生（曲を再生しているときに使えます）")
                                .clicked()
                            {
                                on_action(TrackAction::ToggleReverse);
                            }
                            if ui.add_sized([32.0, 26.0], btn("⏮")).on_hover_text("前の曲").clicked() {
                                on_action(TrackAction::Prev);
                            }
                            let play_icon = if is_playing && playback == PlaybackState::Playing { "⏸" } else { "▶" };
                            let play_btn = egui::Button::new(
                                RichText::new(play_icon).size(16.0).color(crate::theme::c(34, 197, 94)),
                            );
                            if ui
                                .add_sized([38.0, 26.0], play_btn)
                                .on_hover_text(if is_playing && playback == PlaybackState::Playing { "一時停止" } else { "再生" })
                                .clicked()
                            {
                                on_action(TrackAction::TogglePlay(active_idx));
                            }
                            if ui.add_sized([32.0, 26.0], btn("⏹")).on_hover_text("停止").clicked() {
                                on_action(TrackAction::Stop);
                            }
                            if ui.add_sized([32.0, 26.0], btn("⏭")).on_hover_text("次の曲").clicked() {
                                on_action(TrackAction::Next);
                            }
                            // リピート
                            let (rep_icon, rep_text, rep_color) = match repeat_mode {
                                RepeatMode::None => ("🔁", "リピート: なし", crate::theme::c(148, 163, 184)),
                                RepeatMode::All => ("🔁", "リピート: 全曲", crate::theme::c(34, 197, 94)),
                                RepeatMode::One => ("🔂", "リピート: 1曲", crate::theme::c(56, 189, 248)),
                            };
                            if ui
                                .add_sized([32.0, 26.0], egui::Button::new(RichText::new(rep_icon).size(14.0).color(rep_color)))
                                .on_hover_text(rep_text)
                                .clicked()
                            {
                                on_action(TrackAction::CycleRepeat);
                            }
                            // 再生速度（クリックで候補を開く）
                            let speed_color = if (speed - 1.0).abs() < 0.001 { crate::theme::c(148, 163, 184) } else { crate::theme::c(251, 146, 60) };
                            let speed_btn = ui.menu_button(RichText::new(format!("×{:.2}", speed)).size(12.0).color(speed_color), |ui| {
                                ui.label(RichText::new("再生速度（音程も変わります）").weak().size(11.0));
                                for preset in [0.5_f32, 0.75, 1.0, 1.25, 1.5, 2.0] {
                                    let label = if preset == 1.0 { "×1.00（等倍）".to_string() } else { format!("×{:.2}", preset) };
                                    if ui.selectable_label((speed - preset).abs() < 0.001, label).clicked() {
                                        on_action(TrackAction::SetSpeed(preset));
                                        ui.close_menu();
                                    }
                                }
                                ui.separator();
                                ui.horizontal(|ui| {
                                    if ui.small_button("－0.05").on_hover_text("少しゆっくりにする").clicked() {
                                        on_action(TrackAction::SetSpeed(speed - 0.05));
                                    }
                                    if ui.small_button("＋0.05").on_hover_text("少し速くする").clicked() {
                                        on_action(TrackAction::SetSpeed(speed + 0.05));
                                    }
                                });
                            });
                            speed_btn.response.on_hover_text(format!("再生速度: ×{:.2}（クリックで変更。音程も変わります）", speed));
                        }).response.rect.width();

                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            ui.label(RichText::new(fmt_time(if is_playing { current_position } else { Duration::ZERO })).weak().size(11.0));

                            let mut sec = if is_playing { current_position.as_secs_f64() } else { 0.0 };
                            let slider = egui::Slider::new(&mut sec, 0.0..=(total_secs as f64).max(1.0))
                                .show_value(false)
                                .trailing_fill(true);
                            let resp = ui
                                .scope(|ui| {
                                    ui.visuals_mut().widgets.inactive.bg_fill = crate::theme::c(71, 85, 105);
                                    // 再生ボタン群の幅いっぱい（時間表示の分を除く）まで伸ばす。2段表示のときは行の残り幅いっぱい
                                    let row_w = if pane_open { ui.available_width() } else { buttons_w };
                                    ui.spacing_mut().slider_width = (row_w - 110.0).max(120.0);
                                    ui.add_enabled(is_playing, slider)
                                })
                                .inner;
                            if is_playing && (resp.drag_stopped() || (resp.clicked() && !resp.dragged())) {
                                on_action(TrackAction::Seek(Duration::from_secs_f64(sec)));
                            }

                            ui.label(RichText::new(fmt_time(Duration::from_secs_f32(total_secs))).weak().size(11.0));
                        });
                    });
                });
            });

        ui.add_space(6.0);
    }

    // --- 2. 下部: 楽曲一覧テーブル（列の表示・並び順は設定で変更可能） ---
    let row_height: f32 = 40.0;
    let art_size = 36.0;
    let table_height = ui.available_height();
    let mut visible: Vec<ColumnId> = columns.visible().collect();
    if visible.is_empty() {
        visible.push(ColumnId::FileName); // 列が0個だと表を作れないので、最低1列は出す
    }

    let row_ctx = ui.ctx().clone();
    let mods = ui.input(|i| i.modifiers);
    let multi: HashSet<std::path::PathBuf> = if is_pane {
        HashSet::new()
    } else {
        ui.ctx().data(|d| d.get_temp(multi_sel_id())).unwrap_or_default()
    };

    // 列が多いので横スクロールを許可（縦スクロールはテーブル自身が担当）
    egui::ScrollArea::horizontal().auto_shrink([false, false]).show(ui, |ui| {
        // 表内の文字をテキスト選択の対象にしない（文字の上でも入力カーソルにならず、行クリックが効く）
        ui.style_mut().interaction.selectable_labels = false;
        // 列幅は列の種類ごとに覚える。表示項目を移動・追加・削除しても幅が初期値に戻らないよう、
        // 並び順が変わったら表の状態を作り直し、覚えておいた幅から始める
        let width_key = |c: ColumnId| format!("{}.{}", if is_pane { "p" } else { "m" }, c.key());
        let mut widths: std::collections::BTreeMap<String, f32> =
            ui.ctx().data(|d| d.get_temp(col_widths_id())).unwrap_or_default();
        let order_salt: String = visible.iter().map(|c| c.key()).collect::<Vec<_>>().join(",");
        let mut table = TableBuilder::new(ui)
            .id_salt(("track_table", is_pane, order_salt))
            .striped(true)
            .resizable(true)
            .sense(Sense::click())
            .max_scroll_height(table_height)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center));

        let stored = |c: ColumnId, default: f32| widths.get(&width_key(c)).copied().unwrap_or(default);
        // 波形の列は、幅を変えると波形の大きさ（高さ）も合わせて変わり、行の高さもそれに合わせる
        let wave_default_w = WAVEFORM_BINS as f32 * 2.0 + 16.0;
        let wave_h = ((stored(ColumnId::Waveform, wave_default_w) - 16.0) * WAVE_ASPECT).clamp(28.0, 110.0);
        let row_height = if visible.contains(&ColumnId::Waveform) { row_height.max(wave_h + 12.0) } else { row_height };
        for &c in &visible {
            table = table.column(match c {
                ColumnId::Index => Column::exact(36.0),
                ColumnId::Art => Column::exact(art_size + 8.0),
                ColumnId::FileName => Column::initial(stored(c, 240.0)).at_least(120.0),
                ColumnId::Waveform => Column::initial(stored(c, wave_default_w)).at_least(70.0),
                ColumnId::Title => Column::initial(stored(c, 180.0)).at_least(80.0),
                ColumnId::Artist => Column::initial(stored(c, 130.0)).at_least(80.0),
                ColumnId::Album => Column::initial(stored(c, 150.0)).at_least(80.0),
                ColumnId::AlbumArtist => Column::initial(stored(c, 120.0)).at_least(80.0),
                ColumnId::Genre => Column::initial(stored(c, 90.0)).at_least(60.0),
                ColumnId::Year => Column::exact(50.0),
                ColumnId::TrackNo => Column::exact(50.0),
                ColumnId::Format => Column::exact(96.0),
                ColumnId::Labels => Column::initial(stored(c, 120.0)).at_least(80.0),
                ColumnId::Duration => Column::exact(65.0),
                ColumnId::Size => Column::exact(70.0),
            });
        }

        let mut measured: Vec<(ColumnId, f32)> = Vec::new();
        table
            .header(24.0, |mut header| {
                for &c in &visible {
                    header.col(|ui| {
                        if matches!(c, ColumnId::FileName | ColumnId::Waveform | ColumnId::Title | ColumnId::Artist | ColumnId::Album | ColumnId::AlbumArtist | ColumnId::Genre | ColumnId::Labels) {
                            measured.push((c, ui.max_rect().width()));
                        }
                        ui.strong(match c {
                            ColumnId::Index => "#",
                            ColumnId::Art => "画像",
                            ColumnId::FileName => "ファイル名",
                            ColumnId::Waveform => "波形",
                            ColumnId::Title => "タイトル",
                            ColumnId::Artist => "アーティスト",
                            ColumnId::Album => "アルバム",
                            ColumnId::AlbumArtist => "アルバムアーティスト",
                            ColumnId::Genre => "ジャンル",
                            ColumnId::Year => "年",
                            ColumnId::TrackNo => "Trk",
                            ColumnId::Format => "形式",
                            ColumnId::Labels => "ラベル",
                            ColumnId::Duration => "時間",
                            ColumnId::Size => "サイズ",
                        });
                    });
                }
            })
            .body(|body| {
                body.rows(row_height, folders.len() + tracks.len(), |mut row| {
                    let row_index = row.index();

                    // 先頭にフォルダ行（".." と サブフォルダ）を並べる
                    if row_index < folders.len() {
                        let (name, path) = &folders[row_index];
                        // 名前を出す列: ファイル名列が無ければタイトル列、それも無ければ先頭の列（空行に見えないように）
                        let name_col = [ColumnId::FileName, ColumnId::Title]
                            .into_iter()
                            .find(|c| visible.contains(c))
                            .or_else(|| visible.iter().copied().find(|c| *c != ColumnId::Art));
                        for &c in &visible {
                            row.col(|ui| match c {
                                _ if Some(c) == name_col => {
                                    ui.add(egui::Label::new(RichText::new(format!(
                                        "{}{}",
                                        if crate::views::folder_tree::is_favorite(ui.ctx(), path) { "⭐ " } else { "" },
                                        if visible.contains(&ColumnId::Art) { name.clone() } else { format!("📁 {}", name) }
                                    )).strong().color(crate::theme::c(251, 191, 36))).truncate());
                                }
                                ColumnId::Art => {
                                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(art_size), Sense::hover());
                                    ui.painter().text(
                                        rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        "📁",
                                        egui::FontId::proportional(24.0),
                                        crate::theme::c(251, 191, 36),
                                    );
                                }
                                _ => {}
                            });
                        }
                        let folder_resp = row.response();
                        if folder_resp.hovered() {
                            ui_cursor_hand(&folder_resp);
                        }
                        folder_resp.context_menu(|ui| {
                            crate::views::folder_tree::folder_menu(ui, path, &mut |a| on_action(TrackAction::Folder(a)));
                        });
                        if folder_resp.double_clicked() {
                            on_action(TrackAction::OpenFolder(path.clone()));
                        } else if label_view && folder_resp.clicked() {
                            on_action(TrackAction::ToggleFolder(path.clone()));
                        }
                        return;
                    }

                    let idx = row_index - folders.len();
                    let track = &tracks[idx];

                    let in_multi = !multi.is_empty() && multi.contains(&track.file_path);
                    let is_selected = selected_track.map(|s| s == idx).unwrap_or(false) || in_multi;
                    let is_playing = current_playing
                        .as_ref()
                        .map(|p| p == &track.file_path)
                        .unwrap_or(false);

                    row.set_selected(is_selected);
                    let in_playlist = playlist_paths.is_some_and(|s| s.contains(&crate::store::path_key(&track.file_path)));

                    // ジャケット/波形の列が表示中の行だけ、バックグラウンド生成を要求する
                    let needs_media = visible.iter().any(|c| matches!(c, ColumnId::Art | ColumnId::Waveform));
                    let media = if needs_media { media_cache.get(&track.file_path) } else { None };

                    let mut label_cell: Option<egui::Rect> = None;
                    for &c in &visible {
                        let (_cell_rect, _cell) = row.col(|ui| {
                            // プレイリスト登録済みの曲は行の背景色を変える（選択中は選択色を優先）
                            if in_playlist && !is_selected {
                                let r = ui.max_rect().expand2(egui::vec2(ui.spacing().item_spacing.x / 2.0 + 1.0, 1.0));
                                ui.painter().rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(34, 197, 94, 30));
                            }
                            match c {
                            ColumnId::Index => {
                                if is_playing {
                                    ui.label(RichText::new("▶").color(crate::theme::c(34, 197, 94)).strong());
                                } else {
                                    ui.label(RichText::new(format!("{}", idx + 1)).weak());
                                }
                            }
                            ColumnId::Art => match media.as_ref().and_then(|m| m.art.as_ref()) {
                                Some(tex) => {
                                    ui.add(egui::Image::new((tex.id(), Vec2::splat(art_size))).rounding(3.0));
                                }
                                None => {
                                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(art_size), Sense::hover());
                                    ui.painter().rect_filled(rect, 3.0, crate::theme::c(30, 41, 59));
                                    ui.painter().text(
                                        rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        "🎵",
                                        egui::FontId::proportional(16.0),
                                        crate::theme::c(100, 116, 139),
                                    );
                                }
                            },
                            ColumnId::FileName => {
                                let name = if crate::views::folder_tree::is_favorite(ui.ctx(), &track.file_path) {
                                    format!("⭐ {}", track.filename)
                                } else {
                                    track.filename.clone()
                                };
                                let text = RichText::new(name).color(if is_playing {
                                    crate::theme::c(34, 197, 94)
                                } else if is_selected {
                                    crate::theme::c(147, 197, 253)
                                } else {
                                    crate::theme::c(241, 245, 249)
                                });

                                ui.add(egui::Label::new(text).truncate());
                            }
                            ColumnId::Waveform => {
                                let progress = if is_playing && total_duration.as_secs_f32() > 0.0 {
                                    (current_position.as_secs_f32() / waveform_secs()).clamp(0.0, 1.0)
                                } else {
                                    0.0
                                };
                                let peaks = media.as_ref().and_then(|m| m.peaks.as_deref());
                                let w = (ui.available_width() - 4.0).max(40.0);
                                draw_waveform(ui, Vec2::new(w, wave_h), peaks, progress, Sense::hover());
                            }
                            ColumnId::Title => weak_label(ui, &track.tag_title),
                            ColumnId::Artist => weak_label(ui, &track.artist),
                            ColumnId::Album => weak_label(ui, &track.album),
                            ColumnId::AlbumArtist => weak_label(ui, &track.album_artist),
                            ColumnId::Genre => weak_label(ui, &track.genre),
                            ColumnId::Year => {
                                if track.year > 0 {
                                    ui.label(RichText::new(track.year.to_string()).weak());
                                }
                            }
                            ColumnId::TrackNo => {
                                if track.track_number > 0 {
                                    ui.label(RichText::new(track.track_number.to_string()).weak());
                                }
                            }
                            ColumnId::Format => {
                                ui.label(RichText::new(&track.format).size(11.0).color(crate::theme::c(148, 163, 184)));
                                if track.has_lyrics {
                                    lyrics_pill(ui);
                                }
                            }
                            ColumnId::Labels => {
                                // クリック判定は、文字の幅ではなくセル全体の範囲で行う
                                label_cell = Some(ui.max_rect());
                                if track.custom_tags.is_empty() {
                                    ui.label(RichText::new("＋ ラベル").weak().size(11.0));
                                } else {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(track.custom_tags.join(", "))
                                                .size(11.0)
                                                .color(crate::theme::c(251, 146, 60)),
                                        )
                                        .truncate(),
                                    );
                                }
                            }
                            ColumnId::Duration => {
                                ui.label(RichText::new(&track.formatted_duration).weak());
                            }
                            ColumnId::Size => {
                                ui.label(RichText::new(&track.formatted_size).weak());
                            }
                            }
                        });
                        // ラベル列のクリックで、登録済みラベルの一覧（複数選択）を開く
                    }

                    // 行全体がクリック(選択)・ダブルクリック(再生)・右クリック(メニュー)の対象
                    let row_resp = row.response();
                    if row_resp.hovered() {
                        ui_cursor_hand(&row_resp);
                    }
                    row_resp.context_menu(|ui| {
                        if in_multi && multi.len() > 1 {
                            if ui.button(RichText::new(format!("🏷 選択中の {} 曲のタグを一括編集", multi.len())).strong()).clicked() {
                                on_action(TrackAction::EditTagsMulti);
                                ui.close_menu();
                            }
                            ui.separator();
                        }
                        if ui.button("▶ 今すぐ再生").clicked() {
                            on_action(TrackAction::Play(idx));
                            ui.close_menu();
                        }
                        if is_pane {
                            if ui.button("➖ プレイリストから外す").clicked() {
                                on_action(TrackAction::RemoveFromPlaylist(idx));
                                ui.close_menu();
                            }
                        } else {
                        match playlist_name {
                            Some(name) if in_multi && multi.len() > 1 => {
                                let all_in = playlist_paths.is_some_and(|s| {
                                    tracks
                                        .iter()
                                        .filter(|t| multi.contains(&t.file_path))
                                        .all(|t| s.contains(&crate::store::path_key(&t.file_path)))
                                });
                                let label = if all_in {
                                    format!("➖ [{}]から {} 曲を削除", name, multi.len())
                                } else {
                                    format!("📑 [{}]に {} 曲を追加", name, multi.len())
                                };
                                if ui.button(label).clicked() {
                                    on_action(TrackAction::AddMultiToPlaylist);
                                    ui.close_menu();
                                }
                            }
                            Some(name) => {
                                let label = if in_playlist {
                                    format!("➖ [{}]から削除", name)
                                } else {
                                    format!("📑 [{}]に追加", name)
                                };
                                if ui.button(label).clicked() {
                                    on_action(TrackAction::AddToPlaylist(idx));
                                    ui.close_menu();
                                }
                            }
                            None => {
                                ui.add_enabled(false, egui::Button::new("📑 プレイリスト未選択"))
                                    .on_disabled_hover_text("上部のプレイリストボタンからプレイリストを選択してください");
                            }
                        }
                        }
                        ui.separator();
                        if ui.button("🏷 タグ情報を編集").clicked() {
                            on_action(TrackAction::EditTags(idx));
                            ui.close_menu();
                        }
                        if ui.button("🏷 ラベルを編集").clicked() {
                            on_action(TrackAction::EditLabels(idx));
                            ui.close_menu();
                        }
                        if ui.button("ℹ 楽曲の詳細プロパティ").clicked() {
                            on_action(TrackAction::ShowDetails(idx));
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("📂 ツリーで開く").clicked() {
                            on_action(TrackAction::OpenExplorer(idx));
                            ui.close_menu();
                        }
                        if ui.button("📋 ファイルパスをコピー").clicked() {
                            on_action(TrackAction::CopyPath(idx));
                            ui.close_menu();
                        }
                        if ui.button("⭐ お気に入りに追加").clicked() {
                            on_action(TrackAction::AddFavorite(idx));
                            ui.close_menu();
                        }
                        if !is_pane {
                            ui.separator();
                            if ui.button(RichText::new("🗑 ファイルを削除").color(crate::theme::c(248, 113, 113))).clicked() {
                                on_action(TrackAction::Delete(idx));
                                ui.close_menu();
                            }
                        }
                    });
                    // ラベル列のクリックで、登録済みラベルの一覧（複数選択）を開く
                    let on_label_cell = label_cell
                        .zip(row_resp.hover_pos().or_else(|| row_resp.interact_pointer_pos()))
                        .is_some_and(|(r, p)| r.contains(p));
                    if on_label_cell && row_resp.hovered() {
                        ui_cursor_hand(&row_resp);
                    }
                    if row_resp.double_clicked() && !on_label_cell {
                        on_action(TrackAction::Play(idx));
                    } else if row_resp.clicked() {
                        if !is_pane {
                            let (ctrl, shift) = (mods.command, mods.shift);
                            let mut set = multi.clone();
                            let mut ranged = false;
                            if let Some(a) = selected_track.filter(|&a| shift && a < tracks.len()) {
                                // 起点の曲から、クリックした曲までを選択
                                let (lo, hi) = (a.min(idx), a.max(idx));
                                set = tracks[lo..=hi].iter().map(|t| t.file_path.clone()).collect();
                                ranged = true;
                            } else if ctrl {
                                if set.is_empty() {
                                    if let Some(t) = selected_track.and_then(|a| tracks.get(a)) {
                                        set.insert(t.file_path.clone());
                                    }
                                }
                                if !set.remove(&track.file_path) {
                                    set.insert(track.file_path.clone());
                                }
                            } else {
                                set.clear();
                            }
                            if set != multi {
                                row_ctx.data_mut(|d| d.insert_temp(multi_sel_id(), set));
                            }
                            if ranged {
                                return; // 範囲選択のときは、起点を動かさない（起点がまだ無いときは、ここを起点にする）
                            }
                        }
                        on_action(TrackAction::Select(idx));
                        if on_label_cell {
                            if let Some(r) = label_cell {
                                on_action(TrackAction::OpenLabelPicker(idx, r.left_bottom()));
                            }
                        }
                    }
                });
            });

        // ドラッグで変えた幅を覚える
        let mut changed = false;
        for (c, w) in measured {
            let e = widths.entry(width_key(c)).or_insert(0.0);
            if (*e - w).abs() > 0.5 {
                *e = w;
                changed = true;
            }
        }
        if changed {
            ui.ctx().data_mut(|d| d.insert_temp(col_widths_id(), widths));
        }
    });
}

fn weak_label(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).weak()).truncate());
}

/// 先頭15秒の実波形を描画する。`peaks` が未生成/デコード不能なら中央線だけ描く
/// 波形の高さ / 幅（幅に比例して高さも変える）
const WAVE_ASPECT: f32 = 28.0 / 150.0;

fn draw_waveform(ui: &mut Ui, size: Vec2, peaks: Option<&[f32]>, progress: f32, sense: Sense) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, crate::theme::c(15, 23, 42));

    let cy = rect.center().y;
    let Some(peaks) = peaks else {
        painter.line_segment(
            [Pos2::new(rect.left() + 2.0, cy), Pos2::new(rect.right() - 2.0, cy)],
            Stroke::new(1.0_f32, crate::theme::c(51, 65, 85)),
        );
        return response;
    };

    let bar_w = rect.width() / peaks.len() as f32;
    let half = rect.height() / 2.0 - 2.0;
    for (i, p) in peaks.iter().enumerate() {
        let x = rect.left() + i as f32 * bar_w;
        let h = (p * half).max(1.0);
        let color = if (i as f32 / peaks.len() as f32) <= progress {
            crate::theme::c(34, 197, 94)
        } else {
            crate::theme::c(71, 140, 200)
        };
        painter.rect_filled(
            Rect::from_min_max(Pos2::new(x, cy - h), Pos2::new(x + (bar_w - 0.6).max(1.0), cy + h)),
            0.0,
            color,
        );
    }
    response
}

fn fmt_time(d: Duration) -> String {
    let s = d.as_secs();
    format!("{:02}:{:02}", s / 60, s % 60)
}

/// 行にマウスが乗っているときは人差し指カーソルにする
fn ui_cursor_hand(resp: &egui::Response) {
    resp.ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
}

/// 歌詞ファイル（.lrc）がある曲に付ける、「歌詞」のピル（丸い印）
fn lyrics_pill(ui: &mut egui::Ui) {
    egui::Frame::none()
        .fill(crate::theme::c(30, 64, 100))
        .rounding(8.0)
        .inner_margin(egui::Margin::symmetric(5.0, 0.0))
        .show(ui, |ui| {
            ui.label(RichText::new("歌詞").size(10.0).color(crate::theme::c(125, 211, 252)));
        })
        .response
        .on_hover_text("歌詞ファイル（.lrc）があります");
}

//! 設定ウィンドウ（C# 版 SettingsDialog の全項目）。
//! カラーテーマ / 再生エンジン / 出だし波形 / 再生設定 / タグ表示 / ラベル管理 / 高速起動キャッシュ /
//! データ保存先 / バックアップと復元 / マウスホイール
use super::MusicManagerApp;
use crate::relink::RelinkResult;
use crate::audio::hotkeys_defs::{default_bindings, Binding, Combo, HotkeyAction};
use crate::settings::{AppThemeMode, AudioOutputEngine, LoudnessMode, TagDisplayMode};
use egui::{Color32, RichText, ScrollArea, Ui};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Receiver;
use std::sync::Arc;

#[derive(Default)]
pub struct SettingsUi {
    pub new_label: String,
    pub selected_label: Option<String>,
    /// ラベル名の変更中 (元の名前, 入力中の新しい名前)
    pub rename: Option<(String, String)>,
    pub delete_confirm: Option<String>,
    pub relink_running: bool,
    pub relink_rx: Option<Receiver<RelinkResult>>,
    pub relink_cancel: Option<Arc<AtomicBool>>,
    /// 検索結果の確認待ち
    pub relink_pending: Option<RelinkResult>,
    pub confirm_clear_cache: bool,
    pub message: Option<String>,
}

enum Action {
    AddLabel(String),
    RenameLabel(String, String),
    DeleteLabel(String),
    StartRelink,
    ApplyRelink,
    CancelRelink,
    ClearCache,
    ChangeDataDir,
    ResetDataDir,
    CreateBackup,
    RestoreBackup,
    CheckUpdate,
    OpenReleasePage,
}

fn heading(ui: &mut Ui, title: &str, desc: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(title).strong().size(14.0).color(crate::theme::c(251, 191, 36)));
    if !desc.is_empty() {
        ui.label(RichText::new(desc).weak().size(11.0));
    }
    ui.add_space(4.0);
}

fn combo_u32(ui: &mut Ui, id: &str, value: &mut u32, options: &[(u32, &str)]) {
    let shown = options.iter().find(|(v, _)| v == value).map_or("-", |(_, l)| *l);
    egui::ComboBox::from_id_salt(id).selected_text(shown).width(190.0).show_ui(ui, |ui| {
        for (v, label) in options {
            ui.selectable_value(value, *v, *label);
        }
    });
}

impl MusicManagerApp {
    pub(super) fn render_settings_window(&mut self, ctx: &egui::Context) {
        // 検索が終わった再紐付けの結果を受け取る
        if let Some(rx) = &self.sui.relink_rx {
            if let Ok(result) = rx.try_recv() {
                self.sui.relink_running = false;
                self.sui.relink_rx = None;
                if result.missing_count == 0 {
                    self.sui.message = Some("見失ったファイルはありません。すべてのラベル・プレイリストは正しく紐付いています。".into());
                } else if result.relinked.is_empty() {
                    self.sui.message = Some(format!(
                        "見失ったファイル: {} 件。選んだフォルダ内に同じファイル名の曲が見つかりませんでした。{}",
                        result.missing_count,
                        if result.ambiguous > 0 { format!("（同名の候補が複数あり判断できなかった曲: {} 件）", result.ambiguous) } else { String::new() }
                    ));
                } else {
                    self.sui.relink_pending = Some(result);
                }
            }
        }

        if !self.show_settings {
            return;
        }

        let mut open = true;
        let mut actions: Vec<Action> = Vec::new();
        let mut s = self.settings.clone();
        let mut sui = std::mem::take(&mut self.sui);
        let labels = self.label_service.get_labels_with_usage();
        let (tag_count, tag_bytes) = self.tag_cache.stats();
        let (_, media_bytes) = self.media_disk_cache.stats();
        let active_engine = self.player.active_engine_name();
        let failed_keys = self.hotkeys.failed_keys();
        let update_status = self.update_status.clone();

        egui::Window::new("⚙ アプリケーション設定")
            .open(&mut open)
            .collapsible(false)
            .default_size([640.0, 700.0])
            .show(ctx, |ui| {
                ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.label(RichText::new("カラーテーマ、再生エンジン、出だし波形、ラベルの管理などが行えます。").weak().size(12.0));

                    // ---- バージョン情報 ----
                    heading(ui, "ℹ Sonora について", "");
                    egui::Grid::new("about_grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                        ui.label("バージョン:");
                        ui.label(RichText::new(format!("{}  (ベータ版)", crate::update::APP_VERSION)).strong());
                        ui.end_row();
                        ui.label("制作:");
                        ui.label(RichText::new(crate::update::APP_AUTHOR).strong());
                        ui.end_row();
                        ui.label("リポジトリ:");
                        ui.horizontal(|ui| {
                            let ok = crate::update::valid_repo(s.update_repo.trim());
                            ui.add(egui::TextEdit::singleline(&mut s.update_repo).desired_width(260.0).hint_text("owner/repo"));
                            if !ok {
                                ui.label(RichText::new("⚠ owner/repo の形で入力").color(crate::theme::c(248, 113, 113)).size(11.0));
                            } else if ui.button("🌐 開く").on_hover_text("リリースのページをブラウザで開きます").clicked() {
                                actions.push(Action::OpenReleasePage);
                            }
                        });
                        ui.end_row();
                    });
                    ui.checkbox(&mut s.check_updates, "起動して読み込みが終わったあと、新しいバージョンがないか確認する");
                    ui.horizontal(|ui| {
                        let checking = matches!(update_status, crate::update::UpdateStatus::Checking);
                        if ui
                            .add_enabled(!checking && crate::update::valid_repo(s.update_repo.trim()), egui::Button::new("🔄 今すぐ確認"))
                            .clicked()
                        {
                            actions.push(Action::CheckUpdate);
                        }
                        use crate::update::UpdateStatus as U;
                        match &update_status {
                            U::Idle => {}
                            U::Checking => {
                                ui.label(RichText::new("確認中…").weak());
                            }
                            U::UpToDate { latest } => {
                                ui.label(RichText::new(format!("✔ 最新です（公開中の最新: {}）", latest)).color(crate::theme::c(34, 197, 94)));
                            }
                            U::Available { latest } => {
                                ui.label(RichText::new(format!("🔔 新しいバージョン {} があります", latest)).color(crate::theme::c(251, 191, 36)).strong());
                                if ui.button("ダウンロードページを開く").clicked() {
                                    actions.push(Action::OpenReleasePage);
                                }
                            }
                            U::Failed(e) => {
                                ui.label(RichText::new(format!("確認できませんでした: {}", e)).color(crate::theme::c(248, 113, 113)).size(11.0));
                            }
                        }
                    });

                    // ---- カラーテーマ ----
                    heading(ui, "カラーテーマ", "");
                    ui.radio_value(&mut s.theme, AppThemeMode::System, "🖥 システム準拠 (Windows設定に合わせる)");
                    ui.radio_value(&mut s.theme, AppThemeMode::Light, "☀ ライトテーマ (清潔感のある白基調)");
                    ui.radio_value(&mut s.theme, AppThemeMode::Dark, "🌙 ダークテーマ (目に優しい黒・濃灰基調)");

                    // ---- オーディオ再生エンジン ----
                    heading(ui, "オーディオ再生エンジン (高音質 & 代替手段)", "WASAPIが動作しない環境でも安全に代替再生できます。");
                    ui.radio_value(&mut s.audio_engine, AudioOutputEngine::Auto, "⚡ 自動 (推奨: WASAPI優先、不可時は自動代替)");
                    ui.radio_value(&mut s.audio_engine, AudioOutputEngine::Wasapi, "💎 WASAPI (最高音質・低遅延・ピュアオーディオ)");
                    ui.radio_value(&mut s.audio_engine, AudioOutputEngine::DirectSound, "🔊 DirectSound (Windows標準・高互換性)");
                    ui.radio_value(&mut s.audio_engine, AudioOutputEngine::WaveOut, "🛡 WaveOut (仮想マシンや古いドライバ向け最終互換)");
                    ui.label(
                        RichText::new(format!(
                            "現在の出力: {}",
                            if active_engine.is_empty() { "なし（出力デバイスを開けません）".to_string() } else { active_engine.clone() }
                        ))
                        .weak()
                        .size(11.0),
                    );

                    // ---- 出だし波形 ----
                    heading(ui, "出だし波形プレビュー設定", "楽曲の出だしとして生成・表示する波形の長さを設定します。");
                    ui.horizontal(|ui| {
                        ui.label("波形の長さ (出だし秒数):");
                        combo_u32(
                            ui,
                            "wf_secs",
                            &mut s.waveform_secs,
                            &[(5, "5 秒"), (10, "10 秒"), (15, "15 秒 (デフォルト)"), (30, "30 秒"), (45, "45 秒"), (60, "60 秒"), (90, "90 秒"), (120, "120 秒")],
                        );
                    });

                    // ---- 再生設定 ----
                    heading(ui, "🎚 再生設定", "曲の切り替えや停止を自然にし、曲ごとの音量差をそろえます。");
                    ui.horizontal(|ui| {
                        ui.label("フェードイン / アウト (再生開始・停止・一時停止):");
                        combo_u32(ui, "fade", &mut s.fade_ms, &[(0, "なし"), (500, "0.5 秒"), (1000, "1 秒"), (2000, "2 秒"), (3000, "3 秒"), (5000, "5 秒")]);
                    });
                    ui.horizontal(|ui| {
                        ui.label("クロスフェード (曲の切り替え時に重ねる):");
                        combo_u32(ui, "cross", &mut s.crossfade_ms, &[(0, "なし"), (1000, "1 秒"), (2000, "2 秒"), (3000, "3 秒"), (5000, "5 秒"), (8000, "8 秒")]);
                    });
                    ui.horizontal(|ui| {
                        ui.label("音量の自動調整 (ラウドネス補正):");
                        let label = match s.loudness_mode {
                            LoudnessMode::Off => "オフ",
                            LoudnessMode::ReplayGain => "ReplayGain タグのみ使用",
                            LoudnessMode::Auto => "自動 (タグ優先・なければ推定)",
                        };
                        egui::ComboBox::from_id_salt("loud").selected_text(label).width(230.0).show_ui(ui, |ui| {
                            ui.selectable_value(&mut s.loudness_mode, LoudnessMode::Off, "オフ");
                            ui.selectable_value(&mut s.loudness_mode, LoudnessMode::ReplayGain, "ReplayGain タグのみ使用");
                            ui.selectable_value(&mut s.loudness_mode, LoudnessMode::Auto, "自動 (タグ優先・なければ推定)");
                        });
                    });
                    ui.checkbox(&mut s.enable_global_hotkeys, "メディアキー・グローバルホットキーを有効にする (他のアプリ操作中も有効)");
                    hotkey_editor(ui, &mut s.hotkeys);
                    if s.enable_global_hotkeys && !failed_keys.is_empty() {
                        ui.label(
                            RichText::new(format!("登録できなかったキー（他のアプリが使用中）: {}", failed_keys.join(" / ")))
                                .color(crate::theme::c(251, 146, 60))
                                .size(11.0),
                        );
                    }

                    // ---- タグ表示 ----
                    heading(ui, "タグ表示", "一覧に表示するタグの種類です（ID3タグ / ラベル）。");
                    ui.radio_value(&mut s.tag_display_mode, TagDisplayMode::Both, "両方（ID3タグ + ラベル）");
                    ui.radio_value(&mut s.tag_display_mode, TagDisplayMode::Id3Only, "ID3情報のみ");
                    ui.radio_value(&mut s.tag_display_mode, TagDisplayMode::CustomOnly, "ラベルのみ");

                    // ---- マウスホイール ----
                    heading(ui, "マウスホイールのスクロール量", "ホイール1ノッチで動くピクセル数です（大きいほど速く動きます）。");
                    ui.horizontal(|ui| {
                        ui.add(egui::Slider::new(&mut s.scroll_speed, 5.0..=200.0).suffix(" px"));
                        if ui.button("初期値 (40)").on_hover_text("スクロール量を、初期値に戻します").clicked() {
                            s.scroll_speed = crate::settings::DEFAULT_SCROLL_SPEED;
                        }
                    });

                    // ---- ラベルの編集・管理 ----
                    heading(ui, "🏷 ラベルの編集・管理", "楽曲に付与できるラベルの一覧です。追加、名称変更、削除が行えます。");
                    ui.horizontal(|ui| {
                        let edit = ui.add(egui::TextEdit::singleline(&mut sui.new_label).hint_text("新しいラベル名").desired_width(300.0));
                        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if (ui.button("＋ 追加").clicked() || enter) && !sui.new_label.trim().is_empty() {
                            actions.push(Action::AddLabel(std::mem::take(&mut sui.new_label)));
                        }
                    });
                    egui::Frame::none().fill(crate::theme::c(30, 41, 59)).inner_margin(6.0).rounding(4.0).show(ui, |ui| {
                        ScrollArea::vertical().id_salt("labels_list").max_height(130.0).auto_shrink([false, true]).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            if labels.is_empty() {
                                ui.label(RichText::new("（ラベルは未登録です）").weak());
                            }
                            for (name, count) in &labels {
                                let text = if *count > 0 { format!("{}  ({} 曲で使用中)", name, count) } else { name.clone() };
                                let selected = sui.selected_label.as_deref() == Some(name.as_str());
                                if ui.selectable_label(selected, text).clicked() {
                                    sui.selected_label = Some(name.clone());
                                }
                            }
                        });
                    });
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("{} 個のラベルが登録されています", labels.len())).weak().size(11.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let has_sel = sui.selected_label.is_some();
                            if ui.add_enabled(has_sel, egui::Button::new("🗑 削除")).clicked() {
                                sui.delete_confirm = sui.selected_label.clone();
                            }
                            if ui.add_enabled(has_sel, egui::Button::new("✏ 名前変更")).clicked() {
                                if let Some(l) = sui.selected_label.clone() {
                                    sui.rename = Some((l.clone(), l));
                                }
                            }
                        });
                    });
                    // ラベル・プレイリストの再紐付け
                    ui.add_space(6.0);
                    egui::Frame::none().fill(crate::theme::c(30, 41, 59)).inner_margin(8.0).rounding(6.0).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("エクスプローラー等でファイルを移動・名前変更して外れたラベルとプレイリストを、同じファイル名の曲を探して付け直します。")
                                    .weak()
                                    .size(11.0),
                            );
                        });
                        ui.horizontal(|ui| {
                            if sui.relink_running {
                                ui.spinner();
                                ui.label("検索中...");
                                if ui.button("中止").clicked() {
                                    actions.push(Action::CancelRelink);
                                }
                            } else if ui.button("🔗 再紐付け...").on_hover_text("移動・名前変更されたファイルを探して、ラベルなどの情報を付け直します").clicked() {
                                actions.push(Action::StartRelink);
                            }
                        });
                    });

                    // ---- 歌詞の色 ----
                    heading(ui, "🎤 歌詞の色", "歌詞ペインで、再生位置（タイムスタンプ）に合わせて行の色が変わります。色は自由に選べます。");
                    egui::Grid::new("lyrics_color_grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                        ui.label("いまの行（歌い終えた部分）:");
                        ui.color_edit_button_srgb(&mut s.lyrics_color_current);
                        ui.end_row();
                        ui.label("これから歌う行（いまの行の残り）:");
                        ui.color_edit_button_srgb(&mut s.lyrics_color_upcoming);
                        ui.end_row();
                        ui.label("歌い終えた行:");
                        ui.color_edit_button_srgb(&mut s.lyrics_color_past);
                        ui.end_row();
                    });
                    ui.checkbox(&mut s.lyrics_sweep, "いまの行の中でも、時刻に合わせて色を進める（カラオケ風）");
                    ui.horizontal(|ui| {
                        // 見本（前の行・いまの行の途中・次の行）
                        let rgb = |c: [u8; 3]| egui::Color32::from_rgb(c[0], c[1], c[2]);
                        ui.label(RichText::new("見本: ").weak());
                        ui.label(RichText::new("前の行 ").color(rgb(s.lyrics_color_past)));
                        ui.label(RichText::new("いまの").color(rgb(s.lyrics_color_current)));
                        ui.label(RichText::new("行の途中 ").color(if s.lyrics_sweep { rgb(s.lyrics_color_upcoming) } else { rgb(s.lyrics_color_current) }));
                        ui.label(RichText::new("次の行").color(rgb(s.lyrics_color_upcoming)));
                    });
                    if ui.button("初期の色に戻す").on_hover_text("歌詞の色を、初期の色に戻します").clicked() {
                        s.lyrics_color_current = crate::settings::LYRICS_COLOR_CURRENT;
                        s.lyrics_color_past = crate::settings::LYRICS_COLOR_PAST;
                        s.lyrics_color_upcoming = crate::settings::LYRICS_COLOR_UPCOMING;
                        s.lyrics_sweep = true;
                    }
                    // ---- 高速起動キャッシュ ----
                    heading(ui, "⚡ 高速起動キャッシュ設定", "楽曲メタデータと出だし波形・ジャケットを保存し、起動時・再アクセス時に即時描画します。");
                    ui.checkbox(&mut s.enable_cache, "高速メタデータ＆波形キャッシュを有効にする (推奨)");
                    ui.horizontal(|ui| {
                        ui.label("キャッシュ上限 (最大保存曲数):");
                        combo_u32(
                            ui,
                            "cache_max",
                            &mut s.max_cache_tracks,
                            &[(1000, "1,000 曲"), (3000, "3,000 曲"), (5000, "5,000 曲 (推奨)"), (10000, "10,000 曲"), (30000, "30,000 曲"), (0, "無制限")],
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "キャッシュ状況: {} 曲 ({:.1} MB)",
                                tag_count,
                                (tag_bytes + media_bytes) as f64 / 1024.0 / 1024.0
                            ))
                            .weak()
                            .size(11.0),
                        );
                        if ui.button("🗑 キャッシュを消去").on_hover_text("保存してあるメタデータと波形のキャッシュを消します（次回の読み込みが遅くなります）").clicked() {
                            sui.confirm_clear_cache = true;
                        }
                    });

                    // ---- データ保存先 ----
                    heading(
                        ui,
                        "📁 データの保存先フォルダ",
                        "設定ファイル (settings.ini) は、常に EXE と同じフォルダに置きます（変更できません）。ラベル・プレイリスト (library.bin) とキャッシュは、EXE の直下にあればそれを使い、なければ下で指定したフォルダへ取りに行きます。",
                    );
                    let mut ini_text = crate::paths::settings_file().to_string_lossy().to_string();
                    ui.horizontal(|ui| {
                        ui.label("設定ファイル:");
                        ui.add(egui::TextEdit::singleline(&mut ini_text).desired_width(f32::INFINITY).interactive(false));
                    });
                    let mut dir_text = if s.data_folder.is_empty() { "（指定なし: EXE と同じフォルダ）".to_string() } else { s.data_folder.clone() };
                    ui.horizontal(|ui| {
                        ui.label("データの取りに行き先:");
                        ui.add(egui::TextEdit::singleline(&mut dir_text).desired_width(f32::INFINITY).interactive(false));
                    });
                    ui.horizontal(|ui| {
                        let mut on = !s.data_folder.is_empty() && s.data_folder_move != "return";
                        let was = on;
                        ui.checkbox(&mut on, "データを指定したフォルダへ移して使う（EXE のフォルダには、実行ファイルと設定ファイルだけが残ります）");
                        if on != was {
                            actions.push(if on { Action::ChangeDataDir } else { Action::ResetDataDir });
                        }
                    });
                    ui.horizontal(|ui| {
                        if !s.data_folder_move.is_empty() {
                            ui.label(RichText::new("⏳ 次回の起動時に反映します").size(11.0).color(crate::theme::c(251, 191, 36)));
                        }
                    });
                    ui.label(
                        RichText::new("※ 指定したフォルダを使う場合、起動のたびにそのフォルダを確認するため、起動まで少し遅くなります（ネットワークドライブでは、最大で数秒）。EXE の直下にデータがそろっているときは、確認しないので遅くなりません。")
                            .size(11.0)
                            .color(crate::theme::c(251, 191, 36)),
                    );

                    // ---- バックアップと復元 ----
                    heading(ui, "💾 バックアップと復元", "ラベル・プレイリスト・設定を1つのファイル (.zip) に保存し、PC移行や再インストール時に復元できます。");
                    ui.horizontal(|ui| {
                        if ui.button("💾 バックアップを作成...").on_hover_text("ラベルとプレイリストを、ファイルに保存します").clicked() {
                            actions.push(Action::CreateBackup);
                        }
                        if ui.button("📂 バックアップから復元...").on_hover_text("保存してあるバックアップから、ラベルとプレイリストを戻します").clicked() {
                            actions.push(Action::RestoreBackup);
                        }
                    });
                    ui.checkbox(&mut s.enable_auto_backup, "起動時にラベル・プレイリストを自動バックアップする (1日1回・7日分を保持)");

                    if let Some(msg) = &sui.message {
                        ui.add_space(8.0);
                        ui.label(RichText::new(msg).color(crate::theme::c(34, 197, 94)));
                    }
                    ui.add_space(8.0);
                });
            });

        // ---- 小さな確認・入力ウィンドウ ----
        if let Some((old, mut input)) = sui.rename.clone() {
            let mut close = false;
            egui::Window::new("ラベル名の変更").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.label(format!("ラベル「{}」の新しい名称を入力してください:", old));
                let edit = ui.add(egui::TextEdit::singleline(&mut input).desired_width(260.0));
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.horizontal(|ui| {
                    if (ui.button("OK").clicked() || enter) && !input.trim().is_empty() {
                        actions.push(Action::RenameLabel(old.clone(), input.trim().to_string()));
                        close = true;
                    }
                    if ui.button("キャンセル").clicked() {
                        close = true;
                    }
                });
            });
            sui.rename = if close { None } else { Some((old, input)) };
        }
        if let Some(label) = sui.delete_confirm.clone() {
            let mut close = false;
            egui::Window::new("ラベルの削除確認").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.label(format!("ラベル「{}」を削除しますか？\n付与されているすべての楽曲からも解除されます。", label));
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("🗑 削除する").color(crate::theme::c(248, 113, 113))).clicked() {
                        actions.push(Action::DeleteLabel(label.clone()));
                        close = true;
                    }
                    if ui.button("キャンセル").clicked() {
                        close = true;
                    }
                });
            });
            if close {
                sui.delete_confirm = None;
            }
        }
        if sui.confirm_clear_cache {
            let mut close = false;
            egui::Window::new("キャッシュの消去").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.label("保存されているすべての楽曲メタデータおよび波形キャッシュを削除しますか？\n（次回のフォルダ読み込み時に再解析が行われます）");
                ui.horizontal(|ui| {
                    if ui.button("消去する").clicked() {
                        actions.push(Action::ClearCache);
                        close = true;
                    }
                    if ui.button("キャンセル").clicked() {
                        close = true;
                    }
                });
            });
            if close {
                sui.confirm_clear_cache = false;
            }
        }
        if let Some(result) = &sui.relink_pending {
            let mut close = false;
            let mut apply = false;
            let preview: Vec<String> = result
                .relinked
                .iter()
                .take(5)
                .map(|(old, new)| {
                    let name = std::path::Path::new(old).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    format!("・{}\n    → {}", name, new)
                })
                .collect();
            egui::Window::new("再紐付けの確認").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.label(format!(
                    "見失ったファイル {} 件のうち、{} 件の移動先が見つかりました。",
                    result.missing_count,
                    result.relinked.len()
                ));
                ui.label(preview.join("\n"));
                if result.relinked.len() > 5 {
                    ui.label(format!("…ほか {} 件", result.relinked.len() - 5));
                }
                if result.ambiguous > 0 {
                    ui.label(format!("※同名の候補が複数あるため変更しない曲: {} 件", result.ambiguous));
                }
                if result.limit_reached {
                    ui.label("※検索ファイル数の上限に達したため、一部のフォルダは検索していません。");
                }
                ui.label("ラベルとプレイリストを新しい場所に付け直しますか？");
                ui.horizontal(|ui| {
                    if ui.button("付け直す").clicked() {
                        apply = true;
                        close = true;
                    }
                    if ui.button("キャンセル").clicked() {
                        close = true;
                    }
                });
            });
            if apply {
                actions.push(Action::ApplyRelink);
            } else if close {
                sui.relink_pending = None;
            }
        }

        // ---- 変更された設定の反映 ----
        let old = self.settings.clone();
        let mut changed = false;
        if s.theme != old.theme {
            crate::theme::apply(ctx, s.theme);
            changed = true;
        }
        if s.audio_engine != old.audio_engine {
            self.player.set_engine(s.audio_engine);
            changed = true;
        }
        if s.waveform_secs != old.waveform_secs {
            crate::views::media_cache::set_waveform_secs(s.waveform_secs);
            self.media_cache.clear();
            changed = true;
        }
        if s.fade_ms != old.fade_ms {
            self.player.set_fade_ms(s.fade_ms);
            changed = true;
        }
        if s.crossfade_ms != old.crossfade_ms {
            self.player.set_crossfade_ms(s.crossfade_ms);
            changed = true;
        }
        if s.loudness_mode != old.loudness_mode {
            self.player.set_loudness(s.loudness_mode);
            changed = true;
        }
        if s.enable_global_hotkeys != old.enable_global_hotkeys {
            self.hotkeys.set_enabled(s.enable_global_hotkeys);
            changed = true;
        }
        if s.hotkeys != old.hotkeys {
            self.hotkeys.set_bindings(s.hotkeys.clone());
            changed = true;
        }
        if s.enable_cache != old.enable_cache || s.max_cache_tracks != old.max_cache_tracks {
            self.apply_cache_settings(&s);
            changed = true;
        }
        if s.scroll_speed != old.scroll_speed {
            ctx.options_mut(|o| o.line_scroll_speed = s.scroll_speed);
            changed = true;
        }
        if s.tag_display_mode != old.tag_display_mode || s.enable_auto_backup != old.enable_auto_backup {
            changed = true;
        }
        self.settings = s;
        if changed {
            self.settings.save(&self.settings_path);
        }

        // ---- 操作の実行 ----
        for action in actions {
            match action {
                Action::AddLabel(name) => {
                    self.label_service.add_preset_label(&name);
                    self.refresh_labels_in_view();
                }
                Action::RenameLabel(old, new) => {
                    self.label_service.rename_label(&old, &new);
                    sui.selected_label = Some(new);
                    self.refresh_labels_in_view();
                }
                Action::DeleteLabel(name) => {
                    self.label_service.delete_preset_label(&name);
                    sui.selected_label = None;
                    self.refresh_labels_in_view();
                }
                Action::StartRelink => {
                    if let Some(root) = rfd::FileDialog::new().set_title("移動先の音楽フォルダを選択（サブフォルダも検索します）").pick_folder() {
                        let (tx, rx) = std::sync::mpsc::channel();
                        let cancel = Arc::new(AtomicBool::new(false));
                        let (tags, lists, c, ctx2) = (self.label_service.clone(), self.playlist_service.clone(), cancel.clone(), ctx.clone());
                        std::thread::spawn(move || {
                            let missing = crate::relink::find_missing_files(&tags, &lists, &c);
                            let result = crate::relink::find_new_locations(&missing, &root, &c);
                            let _ = tx.send(result);
                            ctx2.request_repaint();
                        });
                        sui.relink_running = true;
                        sui.relink_rx = Some(rx);
                        sui.relink_cancel = Some(cancel);
                        sui.message = None;
                    }
                }
                Action::CancelRelink => {
                    if let Some(c) = &sui.relink_cancel {
                        c.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                Action::ApplyRelink => {
                    if let Some(result) = sui.relink_pending.take() {
                        crate::relink::apply(&result, &self.label_service, &self.playlist_service);
                        sui.message = Some(format!("{} 件のファイルを再紐付けしました。", result.relinked.len()));
                        self.refresh_labels_in_view();
                        self.refresh_current_view();
                    }
                }
                Action::ClearCache => {
                    self.tag_cache.clear();
                    self.media_disk_cache.clear();
                    self.media_cache.clear();
                    sui.message = Some("キャッシュを消去しました。".into());
                }
                Action::ChangeDataDir => {
                    if let Some(dir) = rfd::FileDialog::new().set_title("データ（ラベル・プレイリスト・キャッシュ）の保存先フォルダを選択").pick_folder() {
                        let ok = rfd::MessageDialog::new()
                            .set_title("データの保存先の変更")
                            .set_description(format!(
                                "ラベル・プレイリスト・キャッシュ・バックアップ・crash.log を、以下のフォルダへ移動します:\n{}\n\n移動先に同じ名前のファイルがあるときは、そちらに「.old」を付けて残してから移動します。\n移動したあと、EXE のフォルダには、実行ファイルと設定ファイル (settings.ini) だけが残ります。\n\n実際の移動は、次回の起動時に行います。起動まで少し遅くなります。",
                                dir.display()
                            ))
                            .set_buttons(rfd::MessageButtons::OkCancel)
                            .show();
                        if ok == rfd::MessageDialogResult::Ok {
                            self.store.flush();
                            self.settings.data_folder = dir.to_string_lossy().to_string();
                            self.settings.data_folder_move = "move".to_string();
                            self.settings.save(&self.settings_path);
                            sui.message = Some("移動を設定しました。変更を反映するため、アプリケーションを再起動してください。".into());
                        }
                    }
                }
                Action::ResetDataDir => {
                    let yes = rfd::MessageDialog::new()
                        .set_title("データを EXE のフォルダへ戻す")
                        .set_description("データを、指定フォルダから EXE と同じフォルダへ移動して戻しますか？\n（EXE のフォルダに同じ名前のファイルがあるときは、そちらに「.old」を付けて残します）\n実際の移動は、次回の起動時に行います。")
                        .set_buttons(rfd::MessageButtons::YesNo)
                        .show();
                    if yes == rfd::MessageDialogResult::Yes {
                        self.store.flush();
                        self.settings.data_folder_move = "return".to_string();
                        self.settings.save(&self.settings_path);
                        sui.message = Some("指定の解除を設定しました。変更を反映するため、アプリケーションを再起動してください。".into());
                    }
                }
                Action::CheckUpdate => self.start_update_check(),
                Action::OpenReleasePage => crate::update::open_release_page(&self.settings.update_repo),
                Action::CreateBackup => {
                    let default_name = format!("MusicManager_backup_{}.zip", crate::backup::timestamp_for_filename());
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title("バックアップの保存先")
                        .add_filter("音楽管理ツール バックアップ (*.zip)", &["zip"])
                        .set_file_name(default_name)
                        .save_file()
                    {
                        self.settings.save(&self.settings_path);
                        sui.message = Some(match crate::backup::create_backup(&path, &self.store, &self.settings_path) {
                            Ok(()) => format!("バックアップを作成しました: {}", path.display()),
                            Err(e) => format!("バックアップの作成に失敗しました: {}", e),
                        });
                    }
                }
                Action::RestoreBackup => {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title("復元するバックアップを選択")
                        .add_filter("音楽管理ツール バックアップ (*.zip)", &["zip"])
                        .pick_file()
                    {
                        let yes = rfd::MessageDialog::new()
                            .set_title("復元の確認")
                            .set_description("現在のラベル・プレイリスト・設定をバックアップの内容で置き換えます。\n（置き換え前のファイルは *.before-restore として残ります）\n\n復元後、アプリを再起動してください。よろしいですか？")
                            .set_buttons(rfd::MessageButtons::YesNo)
                            .show();
                        if yes == rfd::MessageDialogResult::Yes {
                            sui.message = Some(match crate::backup::restore_backup(&path, &self.settings_path) {
                                Ok(()) => {
                                    // 終了時の保存で、復元したファイルを上書きしないようにする
                                    self.restored_backup = true;
                                    "復元が完了しました。アプリを再起動してください。".to_string()
                                }
                                Err(e) => format!("復元に失敗しました: {}", e),
                            });
                        }
                    }
                }
            }
        }

        self.sui = sui;
        self.show_settings = open;
        let _ = Color32::WHITE;
    }

    pub(super) fn apply_cache_settings(&mut self, s: &crate::settings::AppSettings) {
        self.tag_cache.set_enabled(s.enable_cache);
        self.media_disk_cache.set_enabled(s.enable_cache);
        self.tag_cache.set_limit(s.max_cache_tracks as usize);
        self.media_disk_cache.set_limit(s.max_cache_tracks as usize);
    }

    /// ラベルを変更したあと、表示中のラベル絞り込みを更新する
    fn refresh_labels_in_view(&mut self) {
        if let super::ViewSelection::Labels(_) = self.active_selection {
            self.refresh_current_view();
        }
    }
}

/// ホットキーを割り当て中の操作を覚えておく場所（割り当て中は、アプリ側がグローバルホットキーを一時的に止める）
pub fn hotkey_capture_id() -> egui::Id {
    egui::Id::new("hotkey_capture_action")
}

fn hotkey_editor(ui: &mut Ui, bindings: &mut Vec<Binding>) {
    let ctx = ui.ctx().clone();
    let mut capture: Option<HotkeyAction> = ctx.data(|d| d.get_temp(hotkey_capture_id()));
    let msg_id = egui::Id::new("hotkey_message");
    let mut message: Option<String> = ctx.data(|d| d.get_temp(msg_id));

    ui.collapsing("ホットキーの割り当て", |ui| {
        ui.label(
            RichText::new(
                "操作ごとに、キーを自由に追加・削除できます（複数可）。「＋ キーを追加」を押してから、割り当てたいキーを押してください。\
                 Ctrl・Alt のどちらかと一緒に押す必要があります（F1〜F24 とメディアキーは単独でも可）。Esc で中止します。",
            )
            .weak()
            .size(11.0),
        );
        for action in HotkeyAction::ALL {
            ui.horizontal_wrapped(|ui| {
                ui.add_sized([150.0, 20.0], egui::Label::new(action.label()));
                let mut remove: Option<usize> = None;
                for (i, b) in bindings.iter().enumerate() {
                    if b.action != action {
                        continue;
                    }
                    let text = b.combo.name();
                    if ui.small_button(format!("{text}  ✖")).on_hover_text("クリックで、この割り当てを外す").clicked() {
                        remove = Some(i);
                    }
                }
                if let Some(i) = remove {
                    bindings.remove(i);
                    message = None;
                }

                if capture == Some(action) {
                    ui.label(RichText::new("キーを押してください…").color(crate::theme::c(251, 146, 60)));
                    let events = ui.input(|i| i.events.clone());
                    for ev in events {
                        let egui::Event::Key { key, pressed: true, modifiers, .. } = ev else { continue };
                        if key == egui::Key::Escape {
                            capture = None;
                            message = None;
                            break;
                        }
                        let Some(combo) = Combo::from_egui(key, modifiers) else {
                            message = Some("このキーは割り当てられません。".to_string());
                            continue;
                        };
                        match assign(bindings, action, combo) {
                            Ok(()) => {
                                capture = None;
                                message = None;
                            }
                            Err(e) => message = Some(e),
                        }
                        break;
                    }
                    // 押されたキーは、ほかの操作に使われないようにする
                    ui.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Key { .. })));
                    if ui.small_button("中止").on_hover_text("割り当てをやめます（Esc キーでも中止できます）").clicked() {
                        capture = None;
                        message = None;
                    }
                } else {
                    if ui.small_button("＋ キーを追加").on_hover_text("押したキーをこの操作に割り当てます").clicked() {
                        capture = Some(action);
                        message = None;
                    }
                    ui.menu_button(RichText::new("＋ メディアキー").size(12.0), |ui| {
                        for (vk, name) in [
                            (0xB3, "再生 / 一時停止キー"),
                            (0xB0, "次の曲キー"),
                            (0xB1, "前の曲キー"),
                            (0xB2, "停止キー"),
                        ] {
                            if ui.button(name).clicked() {
                                match assign(bindings, action, Combo::new(0, vk)) {
                                    Ok(()) => message = None,
                                    Err(e) => message = Some(e),
                                }
                                ui.close_menu();
                            }
                        }
                    })
                    .response
                    .on_hover_text("キーボードのメディアキーを割り当てます");
                }
            });
        }
        if let Some(m) = &message {
            ui.label(RichText::new(m).color(crate::theme::c(251, 146, 60)).size(11.0));
        }
        if ui.button("↺ 既定の割り当てに戻す").on_hover_text("すべての操作のキーを、初期の割り当てに戻します").clicked() {
            *bindings = default_bindings();
            capture = None;
            message = None;
        }
    });

    ctx.data_mut(|d| {
        match capture {
            Some(a) => d.insert_temp(hotkey_capture_id(), a),
            None => d.remove::<HotkeyAction>(hotkey_capture_id()),
        }
        match message {
            Some(m) => d.insert_temp(msg_id, m),
            None => d.remove::<String>(msg_id),
        }
    });
}

/// 操作にキーを追加する。ほかの操作と重なるとき、使えないキーのときは理由を返す
fn assign(bindings: &mut Vec<Binding>, action: HotkeyAction, combo: Combo) -> Result<(), String> {
    combo.check().map_err(|e| e.to_string())?;
    if let Some(b) = bindings.iter().find(|b| b.combo == combo) {
        return Err(format!("{} は、すでに「{}」に割り当てられています。", combo.name(), b.action.label()));
    }
    bindings.push(Binding { action, combo });
    Ok(())
}

use crate::settings::AudioOutputEngine;
use egui::{Key, RichText, Ui};

/// これより狭いウィンドウでは、上部の操作を複数の段に分けて並べる（重なり防止）
const NARROW_WIDTH: f32 = 1250.0;

pub enum HeaderToolAction {
    BatchRename,
    FindDuplicates,
    FindDuplicatesHash,
    Lyrics,
    ExportPlaylist,
    ImportPlaylist,
}

/// ヘッダーのプレイリストメニューで選ばれた操作
pub enum PlaylistMenu {
    /// このプレイリストを選ぶ（右ペインに開く）
    Select(i64),
    /// 選択を解除して、右ペインを閉じる
    Deselect,
    /// 管理ウィンドウ（別ウィンドウ）を開く
    OpenWindow,
    /// 新しいプレイリストを作って選ぶ
    Create(String),
}

pub fn render_header(
    ui: &mut Ui,
    search_query: &mut String,
    folder_path_input: &mut String,
    include_subfolders: &mut bool,
    track_count: usize,
    total_size: &str,
    playlist_label: &str,
    playlists: &[crate::playlist::PlaylistInfo],
    selected_playlist: Option<i64>,
    // 一覧で複数選択している曲数（2以上のとき、追加・削除はまとめて行う）
    multi_count: usize,
    playlist_selected: bool,
    can_add_track: bool,
    track_in_playlist: bool,
    playlist_anchor: &mut Option<egui::Pos2>,
    copy_feedback: bool,
    volume: f32,
    engine_pref: AudioOutputEngine,
    active_engine: &str,
    engine_fallback: bool,
    on_open_folder: impl FnOnce(),
    mut on_copy: impl FnMut(),
    mut on_navigate: impl FnMut(String),
    mut on_tool: impl FnMut(HeaderToolAction),
    mut on_open_column_settings: impl FnMut(),
    mut on_open_help: impl FnMut(),
    mut on_add_to_playlist: impl FnMut(),
    mut on_playlist_menu: impl FnMut(PlaylistMenu),
    mut on_open_settings: impl FnMut(),
    // 「Sonora について」: 更新確認のオン/オフ、確認先、リポジトリを開く操作
    check_updates: &mut bool,
    update_repo: &str,
    mut on_open_repo: impl FnMut(),
    mut on_volume: impl FnMut(f32),
    mut on_cycle_mode: impl FnMut(),
    mut on_toggle_mini: impl FnMut(),
) {
    // 検索欄・音量スライダーの幅は、ウィンドウ幅に対する割合で決める
    let window_w = ui.ctx().screen_rect().width();
    let volume_w = (window_w * 0.07).clamp(60.0, 160.0);

    // 各まとまりの実際の幅（前のフレームで測った値）から、1段に収まるかを判断する。
    // 文字が長い・フォントが違う場合でも、重ならないようにするため
    let measure_id = egui::Id::new("header_measure");
    let (m_left, m_menu, m_status) =
        ui.ctx().data(|d| d.get_temp::<(f32, f32, f32)>(measure_id)).unwrap_or((NARROW_WIDTH * 0.4, 40.0, 340.0));
    let avail = ui.available_width();
    let extras = 110.0; // 区切り線・🔍・✖・❓ と余白
    let narrow = m_left + m_menu + m_status + extras + 220.0 > avail;
    let search_w = if narrow {
        (window_w * 0.30).clamp(150.0, (avail - m_status - extras).max(150.0))
    } else {
        (window_w * 0.30).clamp(200.0, (avail - m_left - m_menu - m_status - extras).max(200.0))
    };
    let measured = std::cell::Cell::new((m_left, m_menu, m_status));
    let help_requested = std::cell::Cell::new(false);

    // 各まとまり（幅が足りないときは段を分けて並べる）
    let left_part = |ui: &mut Ui| {
    let x0 = ui.next_widget_position().x;
    // フォルダ選択ダイアログを開くボタン
    if ui.button(RichText::new("📁 フォルダ選択").size(13.0)).on_hover_text("音楽が入っているフォルダを選んで、一覧に表示します").clicked() {
        on_open_folder();
    }

    // サブフォルダを含めるチェックボックス
    ui.checkbox(include_subfolders, "サブフォルダを含める")
        .on_hover_text("オンにすると、フォルダの中のフォルダにある曲も、まとめて表示します");

    ui.separator();

    // ツールメニュー
    ui.menu_button(RichText::new("🔧 ツール").size(13.0), |ui| {
        if ui.button("🔄 一括ファイル名リネーム...").on_hover_text("タグの内容から、複数のファイル名をまとめて付け直します").clicked() {
            on_tool(HeaderToolAction::BatchRename);
            ui.close_menu();
        }
        if ui.button("🔍 重複ファイル検出（簡易: サイズ・タイトル）...").on_hover_text("ファイルサイズとタイトルが同じ曲を探します（速いが、簡易的な判定）").clicked() {
            on_tool(HeaderToolAction::FindDuplicates);
            ui.close_menu();
        }
        if ui.button("🧬 重複ファイル検出（ハッシュ: 音声データ比較）...").on_hover_text("音声データそのものを比べて、同じ曲を探します（時間がかかります）").clicked() {
            on_tool(HeaderToolAction::FindDuplicatesHash);
            ui.close_menu();
        }
        ui.separator();
        if ui.button("🎤 歌詞を表示・編集...").on_hover_text("画面の右に歌詞ペインを開きます。再生に合わせた表示と、タイムスタンプの編集ができます").clicked() {
            on_tool(HeaderToolAction::Lyrics);
            ui.close_menu();
        }
        ui.separator();
        if ui.button("💾 M3U8プレイリストとして保存...").on_hover_text("表示中の曲の一覧を、M3U8 形式のプレイリストファイルとして保存します").clicked() {
            on_tool(HeaderToolAction::ExportPlaylist);
            ui.close_menu();
        }
        if ui.button("📥 M3U8プレイリストをインポート...").on_hover_text("M3U8 形式のプレイリストファイルを読み込んで、プレイリストにします").clicked() {
            on_tool(HeaderToolAction::ImportPlaylist);
            ui.close_menu();
        }
    })
    .response
    .on_hover_text("曲の一覧の書き出し、重複の検出、歌詞の編集などのツール");

    // プレイリスト: メニューから選ぶと、右側にそのプレイリストが開く（別ウィンドウは「管理...」から）
    let pl_btn = ui.menu_button(RichText::new(format!("📑 {} ▾", playlist_label)).size(13.0), |ui| {
        ui.set_min_width(220.0);
        if playlists.is_empty() {
            ui.label(RichText::new("（プレイリストがありません）").weak());
        }
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            for p in playlists {
                let on = selected_playlist == Some(p.id);
                let text = format!("{}{} ({})", if on { "✔ " } else { "　" }, p.name, p.count);
                if ui.selectable_label(on, text).clicked() {
                    on_playlist_menu(PlaylistMenu::Select(p.id));
                    ui.close_menu();
                }
            }
        });
        ui.separator();
        // 新規作成（名前を入れて Enter か「作成」）
        let name_id = egui::Id::new("header_new_playlist_name");
        let mut name: String = ui.data(|d| d.get_temp(name_id)).unwrap_or_default();
        ui.horizontal(|ui| {
            let edit = ui.add(
                egui::TextEdit::singleline(&mut name)
                    .lock_focus(crate::views::ime::ime_composing(ui.ctx()))
                    .hint_text("新しいプレイリスト名")
                    .desired_width(140.0),
            );
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if (ui.button("➕ 作成").on_hover_text("入力した名前で、新しいプレイリストを作ります").clicked() || enter) && !name.trim().is_empty() {
                on_playlist_menu(PlaylistMenu::Create(name.trim().to_string()));
                name.clear();
                ui.close_menu();
            }
        });
        ui.data_mut(|d| d.insert_temp(name_id, name));
        ui.separator();
        if ui.button("📋 プレイリストの管理...（別ウィンドウ）").on_hover_text("名前の変更・コピー・削除・曲の並び替えなどを、別ウィンドウで行います").clicked() {
            on_playlist_menu(PlaylistMenu::OpenWindow);
            ui.close_menu();
        }
        if ui.add_enabled(selected_playlist.is_some(), egui::Button::new("✖ 選択を解除（右ペインを閉じる）")).on_hover_text("プレイリストの選択をやめて、右のペインを閉じます").clicked() {
            on_playlist_menu(PlaylistMenu::Deselect);
            ui.close_menu();
        }
    });
    pl_btn.response.clone().on_hover_text("プレイリストを選んで、右のペインに開きます");
    *playlist_anchor = Some(pl_btn.response.rect.left_bottom());
    // プレイリスト選択中は、名前の横に「追加」ボタン（一覧で選択中の曲を追加する）
    if playlist_selected {
        // 選択中の曲がすでにプレイリストに入っていれば「削除」、入っていなければ「追加」
        let count = if multi_count > 1 { format!(" ({}曲)", multi_count) } else { String::new() };
        let (label, hover) = if track_in_playlist {
            (format!("➖ 削除{}", count), "一覧で選択中の曲を、選択中のプレイリストから削除")
        } else {
            (format!("➕ 追加{}", count), "一覧で選択中の曲を、選択中のプレイリストに追加（Ctrl/Shift+クリックで複数選択）")
        };
        let add = ui
            .add_enabled(can_add_track, egui::Button::new(RichText::new(label).size(13.0)))
            .on_hover_text(hover)
            .on_disabled_hover_text("一覧で曲を選択すると追加・削除できます");
        if add.clicked() {
            on_add_to_playlist();
        }
    }

    // ミニプレイヤー（常に手前に表示できる小さな操作パネル）
    if ui.button(RichText::new("🗗").size(13.0)).on_hover_text("ミニプレイヤーを表示 (常に手前に表示できる小さな操作パネル)").clicked() {
        on_toggle_mini();
    }

    let (_, mm, ms) = measured.get();
    measured.set(((ui.next_widget_position().x - x0).max(0.0), mm, ms));
    };
    let mut search_part = |ui: &mut Ui| {
    // 検索ボックス
    ui.label(RichText::new("🔍").size(13.0));
    let _search_edit = ui.add(
        egui::TextEdit::singleline(search_query)
            .lock_focus(crate::views::ime::ime_composing(ui.ctx()))
            .hint_text("曲名・アーティスト・アルバム・タグで検索...")
            .desired_width(search_w),
    );

    if !search_query.is_empty() && ui.small_button("✖").clicked() {
        search_query.clear();
    }

    if ui.small_button("❓").on_hover_text("検索の使い方（ヘルプ）").clicked() {
        help_requested.set(true);
    }
    };
    let mut menu_part = |ui: &mut Ui| {
    let x0 = ui.next_widget_position().x;
    ui.menu_button(RichText::new("☰").size(16.0), |ui| {
        if ui.button("📋 表示項目・並び順の設定...").on_hover_text("一覧に出す項目（列）と、並び順を変えます").clicked() {
            on_open_column_settings();
            ui.close_menu();
        }
        if ui.button("⚙ 設定...").on_hover_text("アプリの設定を開きます").clicked() {
            on_open_settings();
            ui.close_menu();
        }
        if ui.button("❓ ヘルプ（検索の使い方）").on_hover_text("検索ボックスの使い方を表示します").clicked() {
            help_requested.set(true);
            ui.close_menu();
        }
        ui.separator();
        ui.menu_button("ℹ Sonora について", |ui| {
            ui.set_min_width(300.0);
            egui::Grid::new("about_menu_grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label("バージョン:");
                ui.label(RichText::new(format!("{}（ベータ版）", crate::update::APP_VERSION)).strong());
                ui.end_row();
                ui.label("制作:");
                ui.label(RichText::new(crate::update::APP_AUTHOR).strong());
                ui.end_row();
                ui.label("公開リポジトリ:");
                if ui
                    .link(format!("github.com/{}", update_repo))
                    .on_hover_text("リリースのページをブラウザで開きます")
                    .clicked()
                {
                    on_open_repo();
                }
                ui.end_row();
            });
            ui.separator();
            ui.checkbox(check_updates, "起動後、公開リポジトリで新しいバージョンを確認する")
                .on_hover_text("オンのときだけ、GitHub に問い合わせます（通知だけで、自動ダウンロードはしません）");
        });
    })
    .response
    .on_hover_text("表示項目・設定・ヘルプ・バージョン情報");
    let (ml, _, ms) = measured.get();
    measured.set((ml, (x0 - ui.next_widget_position().x).max(0.0), ms));
    };
    let mut status_part = |ui: &mut Ui| {
    let x0 = ui.next_widget_position().x;
    ui.label(
        RichText::new(format!("{} 曲 ({})", track_count, total_size))
            .weak()
            .size(12.0),
    );

    // 再生モード（曲数の左隣）: クリックで切り替え。モードごとに文字色が変わる
    let (r, g, b) = match active_engine {
        "WASAPI" => (147, 197, 253),
        "DirectSound" => (251, 191, 36),
        "WaveOut" => (251, 146, 60),
        _ => (248, 113, 113),
    };
    let shown = if active_engine.is_empty() { "出力なし" } else { active_engine };
    let suffix = match (engine_pref == AudioOutputEngine::Auto, engine_fallback) {
        (_, true) => " ⚠",
        (true, false) => " (自動)",
        _ => "",
    };
    let tooltip = if active_engine.is_empty() {
        "出力デバイスを開けませんでした".to_string()
    } else if engine_fallback {
        format!("{} が使えないため、{} で再生しています（設定: {}）", "指定の出力方式", active_engine, engine_pref.as_str())
    } else {
        format!("{} で再生中（設定: {}）", active_engine, engine_pref.as_str())
    };
    let mode_label = ui
        .add(
            egui::Label::new(
                RichText::new(format!("{}{}", shown, suffix)).size(11.0).strong().color(crate::theme::c(r, g, b)),
            )
            .sense(egui::Sense::click()),
        )
        .on_hover_text(format!("{}\nクリックで出力方式を切り替え（自動 → WASAPI → DirectSound → WaveOut）", tooltip));
    if mode_label.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if mode_label.clicked() {
        on_cycle_mode();
    }

    // 音量スライダー（再生モードの左隣・幅はウィンドウ幅の割合）
    let mut vol = volume;
    let changed = ui
        .scope(|ui| {
            ui.visuals_mut().widgets.inactive.bg_fill = crate::theme::c(71, 85, 105);
            ui.spacing_mut().slider_width = volume_w;
            ui.add(egui::Slider::new(&mut vol, 0.0..=1.0).show_value(false).trailing_fill(true))
                .on_hover_text(format!("音量 {}%", (vol * 100.0).round() as u32))
                .changed()
        })
        .inner;
    if changed {
        on_volume(vol);
    }
    ui.label(RichText::new(if vol <= 0.0 { "🔇" } else { "🔊" }).size(14.0));
    let (ml, mm, _) = measured.get();
    measured.set((ml, mm, (x0 - ui.next_widget_position().x).max(0.0)));
    };

    if narrow {
        // 狭いとき: 1段目 ボタン類 / 2段目 検索 ＋ 音量・再生モード・曲数
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            left_part(ui);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| menu_part(ui));
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            search_part(ui);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| status_part(ui));
        });
    } else {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            left_part(ui);
            ui.separator();
            search_part(ui);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                menu_part(ui);
                status_part(ui);
            });
        });
    }
    // 測った幅を覚える（変わったら次のフレームで配置し直す）
    let now = measured.get();
    if (now.0 - m_left).abs() > 1.0 || (now.1 - m_menu).abs() > 1.0 || (now.2 - m_status).abs() > 1.0 {
        ui.ctx().data_mut(|d| d.insert_temp(measure_id, now));
        ui.ctx().request_repaint();
    }
    if help_requested.get() {
        on_open_help();
    }

    // 2段目: パスコピーボタン ＆ 直接パス入力欄
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(RichText::new("現在地:").weak().size(11.0));

        // コピーボタン
        let copy_btn_text = if copy_feedback {
            RichText::new("✔ コピー完了").color(crate::theme::c(34, 197, 94)).size(11.0)
        } else {
            RichText::new("📋 パスコピー").size(11.0)
        };

        if ui.button(copy_btn_text).on_hover_text("現在地パスをクリップボードにコピー").clicked() {
            ui.output_mut(|o| o.copied_text = folder_path_input.clone());
            on_copy();
        }

        // 直接パスを入力・編集できる欄 (Enterキーまたは「移動」ボタンで移動)
        let available_w = ui.available_width() - 170.0;
        let path_edit = ui.add(
            egui::TextEdit::singleline(folder_path_input)
                .lock_focus(crate::views::ime::ime_composing(ui.ctx()))
                .hint_text("フォルダパスを直接入力して Enter で移動...")
                .desired_width(available_w.max(200.0)),
        );

        if path_edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            on_navigate(folder_path_input.clone());
        }

        if ui.button(RichText::new("移動 ➜").size(11.0)).on_hover_text("入力したフォルダへ移動して、一覧を表示します").clicked() {
            on_navigate(folder_path_input.clone());
        }
    });
}

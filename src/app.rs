mod settings_ui;

use crate::audio::hotkeys::{HotkeyAction, HotkeyService};
use crate::audio::player::{AudioPlayer, PlaybackState, RepeatMode};
use settings_ui::SettingsUi;
use crate::db::labels::CustomTagService;
use crate::filecache::{file_stamp, FileCache};
use crate::scanner::metadata::TagMeta;
use crate::store::Store;
use crate::playlist::PlaylistService;
use crate::scanner::metadata::{format_size, TrackItem};
use crate::scanner::scanner::list_audio_files;
use crate::settings::AppSettings;
use crate::tools::duplicates::find_duplicates;
use crate::tools::renamer::{generate_rename_plan, RenamePattern};
use crate::views::dialogs::{render_dialogs, ActiveDialog, MultiTagForm, TagForm};
use crate::views::folder_tree::{render_folder_tree, FavAction, LeftPanelTab, ViewSelection};
use crate::views::header::{render_header, HeaderToolAction, PlaylistMenu};
use crate::search::SearchQuery;
use crate::views::help::render_help;
use crate::views::media_cache::MediaCache;
use crate::views::playlist_window::{
    render_playlist_window, render_reorder_window, PlaylistWindowAction, PlaylistWindowState, ReorderOutcome,
};
use crate::views::track_list::{render_track_list, TrackAction};
use eframe::egui::{self, Color32, Margin, RichText, Stroke};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

pub struct MusicManagerApp {
    settings: AppSettings,
    settings_path: PathBuf,
    current_folder: PathBuf,
    folder_path_input: String,
    active_selection: ViewSelection,
    left_panel_tab: LeftPanelTab,
    tracks: Vec<TrackItem>,
    filtered_indices: Vec<usize>,
    search_query: String,
    selected_track: Option<usize>,
    current_playing_index: Option<usize>,
    /// 右ペインのプレイリストから再生しているか（次の曲の決め方が変わる）
    play_from_pane: bool,
    /// 重複検出ウィンドウでの聴き比べ中か（曲が終わっても一覧の次の曲へ進まない）
    audition: bool,
    /// 重複検出（ハッシュチェック）のウィンドウ
    dup_win: Option<crate::views::duplicate_window::DuplicateWindow>,
    dup_gen: u64,
    /// 歌詞ペイン（右からスライドして出る）。状態は閉じても残す
    lyrics_win: Option<crate::views::lyrics_window::LyricsWindow>,
    show_lyrics: bool,
    /// 歌詞ペインの幅の初期値（ユーザーが幅を変えたかの判定に使う）
    lyrics_default_w: f32,
    player: AudioPlayer,
    total_size_formatted: String,
    copy_feedback_instant: Option<Instant>,
    label_service: CustomTagService,
    playlist_service: PlaylistService,
    active_dialog: ActiveDialog,
    media_cache: MediaCache,
    show_column_settings: bool,
    show_help: bool,
    pl_pane: PlaylistPane,
    pl_tx: std::sync::mpsc::Sender<(u64, Vec<TrackItem>)>,
    /// ペインの読み込みスレッドが「自分が最新か」を調べるための世代番号
    pl_load_gen: std::sync::Arc<std::sync::atomic::AtomicU64>,
    pl_rx: std::sync::mpsc::Receiver<(u64, Vec<TrackItem>)>,
    /// 列設定ウィンドウで編集中の対象（false=メイン一覧 / true=プレイリストペイン）
    column_target_pane: bool,
    /// 最後に曲を選択したのがプレイリストペインか（Delete キーの対象を決める）
    pane_has_focus: bool,
    show_settings: bool,
    sui: SettingsUi,
    hotkeys: HotkeyService,
    /// バックアップから復元した（終了時の保存で、復元したファイルを上書きしない）
    restored_backup: bool,
    /// 新しい版の確認（起動後、他の読み込みが終わってから行う）
    update_status: crate::update::UpdateStatus,
    update_rx: Option<std::sync::mpsc::Receiver<crate::update::UpdateStatus>>,
    /// 読み込みが終わった時刻。ここから少し待って確認を始める
    update_ready_at: Option<f64>,
    update_started: bool,
    /// 「新しい版があります」のお知らせを出す / すでに出した
    show_update_notice: bool,
    update_notified: bool,
    show_mini: bool,
    /// ラベル選択ポップアップ (対象ファイル, 表示位置, 新規ラベル入力)
    label_picker: Option<LabelPicker>,
    /// カスタム保存先にアクセスできなかった起動時の警告
    data_dir_warning: bool,
    /// お気に入りの表示名を変更中 (番号, 入力中の名前)
    fav_rename: Option<(usize, String)>,
    /// 削除確認中のラベル
    label_delete: Option<String>,
    /// ごみ箱へ移動する前の確認中のファイル
    delete_confirm: Option<PathBuf>,
    /// 追加するラベルの名前を入力中
    label_add: Option<String>,
    /// 一覧の読み込み後に選択（必要なら再生）する曲（お気に入りの曲を開いたとき）
    pending_select: Option<(PathBuf, bool)>,
    /// ラベル一覧で展開しているフォルダ（path_key）
    label_expanded: std::collections::HashSet<String>,
    /// ミニプレイヤーの最後の位置（終了時に設定へ保存）
    mini_pos: Option<egui::Pos2>,
    /// true の間に始めた再生はクロスフェードで重ねる
    crossfade_next: bool,
    /// 「〜に追加しました」通知 (メッセージ, 表示開始時刻)
    toast: Option<(String, f64)>,
    /// 通知を出す位置（プレイリストボタンの左下）
    toast_anchor: egui::Pos2,
    show_playlist_window: bool,
    /// 別ウィンドウは開くたびに別の ID にする（閉じた直後の再オープンで、古い窓の状態を引きずらないため）
    pl_win_gen: u64,
    pl_win_frames: u32,
    mini_gen: u64,
    mini_frames: u32,
    /// プレイリストウィンドウの初期位置（開く時にメインウィンドウ中央から算出）
    playlist_window_pos: Option<egui::Pos2>,
    playlist_window: PlaylistWindowState,
    /// 一覧の先頭に出すフォルダ行 (表示名, パス)。フォルダ表示のときだけ使う
    folder_entries: Vec<(String, PathBuf)>,
    label_match_all: bool,
    /// 現在のフォルダまでツリーを展開・スクロールするフレーム数（起動時・パス移動時）
    reveal_frames: u8,
    /// 現在の絞り込みに反映済みの検索文字列（search_query との差分を毎フレーム検出する）
    applied_query: String,
    ctx: egui::Context,
    /// フォルダ読み込み（バックグラウンド）
    scan_tx: std::sync::mpsc::Sender<ScanMsg>,
    scan_rx: std::sync::mpsc::Receiver<ScanMsg>,
    /// 最新のフォルダ読み込み要求の番号（古い要求のスレッドは自分で打ち切る）
    scan_gen: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// 読み込み済みトラックのパス -> tracks 内の位置（タグ読み込み結果の差し込み用）
    track_index: std::collections::HashMap<PathBuf, usize>,
    store: std::sync::Arc<Store>,
    /// 起動を速くするための永続キャッシュ（タグ情報 / ジャケット・波形）
    tag_cache: std::sync::Arc<FileCache>,
    media_disk_cache: std::sync::Arc<FileCache>,
}

/// 一覧に出す曲の取得元
enum TrackSource {
    Folder { folder: PathBuf, include_sub: bool },
    Labels { labels: Vec<String>, match_all: bool, expanded: std::collections::HashSet<String> },
    /// プレイリストの曲（並びはそのまま）
    Paths(Vec<PathBuf>),
}

/// ラベル列をクリックしたときに出す、ラベル選択ポップアップの状態
struct LabelPicker {
    path: PathBuf,
    pos: egui::Pos2,
    new_label: String,
    /// 開いたフレームのクリックで、すぐ閉じないようにする
    just_opened: bool,
}

/// 右ペインに出す、選択中プレイリストの曲一覧
#[derive(Default)]
struct PlaylistPane {
    id: Option<i64>,
    paths: Vec<PathBuf>,
    /// 背景色の判定用（paths と同じ内容）
    set: HashSet<String>,
    items: Vec<TrackItem>,
    selected: Option<usize>,
    /// ペインから再生している行（同じ曲が複数入っていても、次の曲を正しく決めるため）
    playing: Option<usize>,
    gen: u64,
    /// 最後に読み込んだときのデータの版（変わっていなければ、何も複製・比較しない）
    version: u64,
    /// 一覧の中身（ラベル・タグ）が変わったので、曲が同じでも読み直す
    force_reload: bool,
}

/// バックグラウンドで読み込んだフォルダの内容
enum ScanMsg {
    /// 1段階目: ファイル名とサイズだけの一覧（すぐ表示できる）
    Listed { gen: u64, tracks: Vec<TrackItem>, entries: Vec<(String, PathBuf)> },
    /// 2段階目: タグ・再生時間まで読み込めたトラック（少しずつ届く）
    Tags { gen: u64, items: Vec<TrackItem> },
}

/// 新しい別ウィンドウは、最初は隠しておき、中身を描いてから表示する（真っ白な画面が一瞬出るのを防ぐ）
fn reveal_when_painted(ctx: &egui::Context, frames: &mut u32) {
    // 作られてから何回描いたかを数える。表示するかどうかは、呼び出し側が `*frames >= REVEAL_AFTER` で
    // ウィンドウの設定（visible）に反映する（その変化を egui が表示の命令に変えて実行する）
    let created = ctx.input(|i| i.viewport().inner_rect.is_some());
    if created && *frames < REVEAL_AFTER + 2 {
        *frames += 1;
    }
    // 別ウィンドウの描画は、メインウィンドウの描画の中で行われる。
    // メイン側の描画を続けて起こさないと、数回描く前に止まって、ウィンドウが表示されないままになる
    if *frames < REVEAL_AFTER + 2 {
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
        ctx.request_repaint_of(egui::ViewportId::ROOT);
    }
}

/// 別ウィンドウは、作ってからこの回数描くまで隠しておく
const REVEAL_AFTER: u32 = 3;

impl MusicManagerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings_path = crate::paths::settings_file();
        let settings = AppSettings::load(&settings_path);

        let store = Store::open(&crate::paths::data_file("library.bin"));
        let label_service = CustomTagService::new(store.clone());
        let playlist_service = PlaylistService::new(store.clone());
        let (scan_tx, scan_rx) = std::sync::mpsc::channel();
        let (pl_tx, pl_rx) = std::sync::mpsc::channel();
        let tag_cache = FileCache::new(&crate::paths::data_file("tags.cache"));
        let media_disk_cache = FileCache::new(&crate::paths::data_file("media.cache"));
        // 起動時にバックグラウンドで読み込んでおく（ウィンドウ表示は待たない）
        for c in [tag_cache.clone(), media_disk_cache.clone()] {
            std::thread::spawn(move || c.preload());
        }

        // マウスホイールのスクロール量（設定値）
        cc.egui_ctx.options_mut(|o| o.line_scroll_speed = settings.scroll_speed);
        // 列の幅（一覧側が読み書きし、終了時などに設定へ保存する）
        cc.egui_ctx.data_mut(|d| d.insert_temp(crate::views::track_list::col_widths_id(), settings.column_widths.clone()));

        // カラーテーマ（システム準拠 / ライト / ダーク）
        crate::theme::apply(&cc.egui_ctx, settings.theme);

        let mut player = AudioPlayer::new().expect("Failed to initialize audio engine");
        player.set_volume(settings.volume);
        {
            // 再生の開始・曲の終了で画面の更新を起こす（最小化中など、入力が無くても、リピートが止まらないように）
            let ctx = cc.egui_ctx.clone();
            player.set_on_change(move || ctx.request_repaint());
        }
        player.set_engine(settings.audio_engine);
        player.set_fade_ms(settings.fade_ms);
        player.set_crossfade_ms(settings.crossfade_ms);
        player.set_loudness(settings.loudness_mode);

        let initial_folder = if !settings.last_folder.is_empty() && Path::new(&settings.last_folder).is_dir() {
            PathBuf::from(&settings.last_folder)
        } else {
            dirs::audio_dir().unwrap_or_else(|| PathBuf::from("."))
        };

        let initial_path_str = initial_folder.to_string_lossy().to_string();

        // 保存済みの設定を各機能へ反映
        crate::views::media_cache::set_waveform_secs(settings.waveform_secs);
        cc.egui_ctx.options_mut(|o| o.line_scroll_speed = settings.scroll_speed);

        let mut app = Self {
            settings,
            settings_path,
            current_folder: initial_folder.clone(),
            folder_path_input: initial_path_str,
            active_selection: ViewSelection::Folder(initial_folder.clone()),
            left_panel_tab: LeftPanelTab::Explorer,
            tracks: Vec::new(),
            filtered_indices: Vec::new(),
            search_query: String::new(),
            selected_track: None,
            current_playing_index: None,
            play_from_pane: false,
            audition: false,
            dup_win: None,
            dup_gen: 0,
            lyrics_win: None,
            show_lyrics: false,
            lyrics_default_w: 0.0,
            player,
            total_size_formatted: "0 B".to_string(),
            copy_feedback_instant: None,
            label_service,
            playlist_service,
            active_dialog: ActiveDialog::None,
            media_cache: MediaCache::new(cc.egui_ctx.clone(), media_disk_cache.clone()),
            show_column_settings: false,
            show_help: false,
            pl_pane: PlaylistPane::default(),
            pl_tx,
            pl_load_gen: Default::default(),
            pl_rx,
            column_target_pane: false,
            pane_has_focus: false,
            show_settings: false,
            sui: SettingsUi::default(),
            hotkeys: HotkeyService::new(cc.egui_ctx.clone()),
            restored_backup: false,
            update_status: crate::update::UpdateStatus::Idle,
            update_rx: None,
            update_ready_at: None,
            update_started: false,
            show_update_notice: false,
            update_notified: false,
            show_mini: false,
            label_picker: None,
            data_dir_warning: crate::paths::custom_location_unavailable(),
            fav_rename: None,
            label_delete: None,
            delete_confirm: None,
            label_add: None,
            pending_select: None,
            label_expanded: Default::default(),
            mini_pos: None,
            crossfade_next: false,
            toast: None,
            toast_anchor: egui::Pos2::new(300.0, 40.0),
            show_playlist_window: false,
            pl_win_gen: 0,
            pl_win_frames: 0,
            mini_gen: 0,
            mini_frames: 0,
            playlist_window_pos: None,
            playlist_window: PlaylistWindowState::default(),
            folder_entries: Vec::new(),
            label_match_all: true,
            reveal_frames: 4,
            applied_query: String::new(),
            ctx: cc.egui_ctx.clone(),
            scan_tx,
            scan_rx,
            scan_gen: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            track_index: std::collections::HashMap::new(),
            store,
            tag_cache,
            media_disk_cache,
        };

        app.apply_cache_settings(&app.settings.clone());
        app.hotkeys.set_bindings(app.settings.hotkeys.clone());
        app.hotkeys.set_enabled(app.settings.enable_global_hotkeys);
        if app.settings.enable_auto_backup && !crate::paths::custom_location_unavailable() {
            std::thread::spawn(crate::backup::run_auto_backup);
        }

        app.open_folder(initial_folder);
        app
    }

    /// 一覧にある同じファイルの曲のラベル表示を、最新の内容に更新する
    fn apply_labels_to_lists(&mut self, path: &Path, labels: &[String]) {
        let key = crate::store::path_key(path);
        for t in self.tracks.iter_mut().chain(self.pl_pane.items.iter_mut()) {
            if crate::store::path_key(&t.file_path) == key {
                t.custom_tags = labels.to_vec();
            }
        }
    }

    /// ラベル列をクリックしたときのポップアップ: 登録済みのラベルを複数選択できる
    fn render_label_picker(&mut self, ctx: &egui::Context) {
        let Some(mut picker) = self.label_picker.take() else { return };
        let all = self.label_service.get_all_preset_labels();
        let mut current = self.label_service.get_labels_for_file(&picker.path);
        let before = current.clone();
        let mut close = false;
        let is_dir = picker.path.is_dir();

        let file_name = picker.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let area = egui::Area::new(egui::Id::new("label_picker"))
            .order(egui::Order::Foreground)
            .fixed_pos(picker.pos + egui::vec2(0.0, 2.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(220.0);
                    ui.label(RichText::new("🏷 ラベルを選択（複数可）").strong().size(12.0));
                    ui.label(RichText::new(if is_dir { format!("📁 {}（配下の全ファイルにも反映）", file_name) } else { file_name.clone() }).weak().size(10.0));
                    ui.separator();
                    egui::ScrollArea::vertical().max_height(240.0).auto_shrink([true, true]).show(ui, |ui| {
                        if all.is_empty() {
                            ui.label(RichText::new("（ラベルが未登録です。下で追加できます）").weak().size(11.0));
                        }
                        for label in &all {
                            let mut checked = current.iter().any(|l| l == label);
                            if ui.checkbox(&mut checked, label.as_str()).changed() {
                                if checked {
                                    current.push(label.clone());
                                } else {
                                    current.retain(|l| l != label);
                                }
                            }
                        }
                    });
                    ui.separator();
                    ui.horizontal(|ui| {
                        let edit = ui.add(egui::TextEdit::singleline(&mut picker.new_label).hint_text("新しいラベル").desired_width(140.0));
                        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let name = picker.new_label.trim().to_string();
                        if (ui.button("＋ 追加").clicked() || enter) && !name.is_empty() {
                            if !current.contains(&name) {
                                current.push(name);
                            }
                            picker.new_label.clear();
                        }
                        if ui.button("閉じる").clicked() {
                            close = true;
                        }
                    });
                });
            });

        // ポップアップの外をクリックしたら閉じる（開いた直後のクリックは無視）
        if !picker.just_opened {
            let clicked_outside = ctx.input(|i| i.pointer.any_pressed())
                && ctx.input(|i| i.pointer.interact_pos()).is_some_and(|p| !area.response.rect.contains(p));
            if clicked_outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                close = true;
            }
        }
        picker.just_opened = false;

        if current != before && is_dir {
            // フォルダ: 追加・解除したラベルを、配下の全ファイルにも同じように反映する
            let add: Vec<String> = current.iter().filter(|l| !before.contains(l)).cloned().collect();
            let remove: Vec<String> = before.iter().filter(|l| !current.contains(l)).cloned().collect();
            let files = list_audio_files(&picker.path, true);
            self.label_service.apply_folder_labels(&picker.path, &files, &add, &remove);
            self.refresh_current_view();
        } else if current != before {
            self.label_service.set_labels_for_file(&picker.path, &current);
            self.apply_labels_to_lists(&picker.path, &current);
            if let ViewSelection::Labels(_) = self.active_selection {
                self.refresh_current_view();
            }
        }
        if !close {
            self.label_picker = Some(picker);
        }
    }

    /// アプリ内のツリーで開く（左を「ツリー」タブにして、フォルダを展開・表示する。曲は選択状態にする）
    fn reveal_in_tree(&mut self, path: PathBuf) {
        self.left_panel_tab = LeftPanelTab::Explorer;
        self.reveal_frames = 4;
        if path.is_file() {
            self.handle_fav_action(FavAction::OpenFile(path, false));
        } else if path.is_dir() {
            self.open_folder(path);
        }
    }

    /// お気に入りフォルダの追加・削除・表示名の変更
    fn handle_fav_action(&mut self, action: FavAction) {
        let now = self.ctx.input(|i| i.time);
        let add = |s: &mut Self, path: PathBuf| {
            let key = crate::store::path_key(&path);
            if s.settings.favorites.iter().any(|(_, p)| crate::store::path_key(Path::new(p)) == key) {
                s.toast = Some(("既にお気に入りに登録されています".to_string(), now));
                return;
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string_lossy().to_string());
            s.settings.favorites.push((name, path.to_string_lossy().to_string()));
            s.settings.save(&s.settings_path);
            s.toast = Some((format!("お気に入りに追加しました: {}", path.display()), now));
        };
        match action {
            FavAction::AddFolder => {
                if let Some(dir) = rfd::FileDialog::new().set_title("お気に入りに追加するフォルダを選択").pick_folder() {
                    add(self, dir);
                }
            }
            FavAction::AddCurrent => add(self, self.current_folder.clone()),
            FavAction::AddFile => {
                if let Some(f) = rfd::FileDialog::new()
                    .set_title("お気に入りに追加する曲を選択")
                    .add_filter("音楽ファイル", &["mp3", "flac", "wav", "ogg", "m4a", "aac", "wma", "opus", "aiff", "ape"])
                    .pick_file()
                {
                    add(self, f);
                }
            }
            FavAction::AddPath(p) => add(self, p),
            FavAction::Explorer(p) => {
                self.reveal_in_tree(p);
            }
            FavAction::DeleteLabel(name) => self.label_delete = Some(name),
            FavAction::AddLabel => self.label_add = Some(String::new()),
            FavAction::Toggle(p) => {
                let key = crate::store::path_key(&p);
                if let Some(i) = self.settings.favorites.iter().position(|(_, q)| crate::store::path_key(Path::new(q)) == key) {
                    self.settings.favorites.remove(i);
                    self.settings.save(&self.settings_path);
                    self.toast = Some(("お気に入りから外しました".to_string(), now));
                } else {
                    add(self, p);
                }
            }
            FavAction::Labels(p, pos) => {
                self.label_picker = Some(LabelPicker { path: p, pos, new_label: String::new(), just_opened: true });
            }
            FavAction::ClearLabels(p) => {
                let labels = self.label_service.get_labels_for_file(&p);
                if !labels.is_empty() {
                    let files = list_audio_files(&p, true);
                    self.label_service.apply_folder_labels(&p, &files, &[], &labels);
                    self.refresh_current_view();
                    self.toast = Some((format!("{} 個のファイルからラベルを外しました", files.len()), now));
                }
            }
            FavAction::OpenFile(p, play) => {
                if !p.is_file() {
                    self.toast = Some(("ファイルが見つかりません".to_string(), now));
                    return;
                }
                if let Some(parent) = p.parent() {
                    // 同じフォルダを開いていれば、読み込み直さずに選択だけする
                    let same = self.current_folder == parent && matches!(self.active_selection, ViewSelection::Folder(_));
                    let key = crate::store::path_key(&p);
                    let pos = if same {
                        self.filtered_indices.iter().position(|&i| crate::store::path_key(&self.tracks[i].file_path) == key)
                    } else {
                        None
                    };
                    match pos {
                        Some(pos) => {
                            self.selected_track = Some(pos);
                            if play {
                                self.play_track_at_index(pos);
                            }
                        }
                        None => {
                            self.pending_select = Some((p.clone(), play));
                            self.open_folder(parent.to_path_buf());
                        }
                    }
                }
            }
            FavAction::Remove(i) => {
                if i < self.settings.favorites.len() {
                    self.settings.favorites.remove(i);
                    self.settings.save(&self.settings_path);
                    self.toast = Some(("お気に入りを削除しました".to_string(), now));
                }
            }
            FavAction::Rename(i) => {
                if let Some((name, _)) = self.settings.favorites.get(i) {
                    self.fav_rename = Some((i, name.clone()));
                }
            }
        }
    }

    /// 指定のファイルが、いま再生中（一時停止中は含まない）か
    fn is_playing_now(&self, path: Option<&Path>) -> bool {
        match (path, self.player.current_track_path()) {
            (Some(p), Some(cur)) => {
                self.player.state() == PlaybackState::Playing
                    && crate::store::path_key(p) == crate::store::path_key(&cur)
            }
            _ => false,
        }
    }

    /// 再生速度を変える（0.05 刻み。範囲外は端に丸める）
    fn set_speed(&mut self, speed: f32) {
        self.player.set_speed(speed);
    }

    /// 逆再生の入り切り（再生中の曲だけ）
    fn toggle_reverse(&mut self) {
        if self.player.state() != PlaybackState::Stopped {
            self.player.set_reverse(!self.player.is_reverse());
        }
    }

    /// 再生中なら一時停止、一時停止中なら再開、停止中なら選択曲（無ければ先頭）を再生
    fn toggle_play_pause(&mut self) {
        match self.player.state() {
            PlaybackState::Playing => self.player.pause(),
            PlaybackState::Paused => self.player.resume(),
            PlaybackState::Stopped => {
                if let Some(sel) = self.selected_track {
                    self.play_track_at_index(sel);
                } else {
                    self.play_next();
                }
            }
        }
    }

    /// ウィンドウの位置・大きさ・最大化状態を設定へ書き戻す（次回起動時に復元する）
    fn capture_window_state(&mut self) {
        // 列の幅
        if let Some(w) = self.ctx.data(|d| d.get_temp::<std::collections::BTreeMap<String, f32>>(crate::views::track_list::col_widths_id())) {
            self.settings.column_widths = w;
        }

        // ミニプレイヤーの位置
        if let Some(p) = self.mini_pos {
            self.settings.mini_player_left = p.x;
            self.settings.mini_player_top = p.y;
        }

        // 左パネルの幅
        if let Some(state) = egui::containers::panel::PanelState::load(&self.ctx, egui::Id::new("left_tree_v2")) {
            self.settings.left_panel_width = state.rect.width().max(240.0);
        }

        // 歌詞ペインの幅（ユーザーが幅を変えたときだけ記録する。変えていなければ、メインペインの半分が既定）
        if let Some(state) = egui::containers::panel::PanelState::load(&self.ctx, egui::Id::new("lyrics_pane")) {
            let w = state.rect.width();
            if self.lyrics_default_w > 0.0 && (w - self.lyrics_default_w).abs() > 1.5 {
                self.settings.lyrics_pane_width = w.max(200.0);
            }
        }

        let vp = self.ctx.input(|i| i.viewport().clone());
        let maximized = vp.maximized.unwrap_or(false);
        self.settings.is_maximized = maximized;
        if !maximized {
            if let Some(inner) = vp.inner_rect {
                self.settings.window_width = inner.width();
                self.settings.window_height = inner.height();
            }
            if let Some(outer) = vp.outer_rect {
                self.settings.window_left = outer.min.x;
                self.settings.window_top = outer.min.y;
            }
        }
    }

    pub fn open_folder(&mut self, folder: PathBuf) {
        if !folder.is_dir() {
            return;
        }

        self.current_folder = folder.clone();
        self.folder_path_input = folder.to_string_lossy().to_string();
        self.active_selection = ViewSelection::Folder(folder.clone());
        self.settings.last_folder = folder.to_string_lossy().to_string();

        // フォルダの中身の精査(タグ読み取り・ラベル付与・サブフォルダ列挙)はバックグラウンドで行い、
        // 画面は即座に表示する。結果は update() で受け取って反映する。
        let include_sub = self.settings.include_subfolders;
        self.spawn_track_loader(TrackSource::Folder { folder, include_sub });
    }

    /// 一覧に出す曲をバックグラウンドで読み込む（フォルダ / ラベル絞り込み共通）。
    /// 一覧は先に出し、タグはキャッシュ → 無い曲だけ並列で読む。メモリには今の一覧分しか持たない
    fn spawn_track_loader(&mut self, source: TrackSource) {
        use std::sync::atomic::Ordering;
        let gen = self.scan_gen.fetch_add(1, Ordering::SeqCst) + 1;
        self.total_size_formatted = "読み込み中…".to_string();
        let latest = self.scan_gen.clone();
        let labels = self.label_service.clone();
        let tag_cache = self.tag_cache.clone();
        let tx = self.scan_tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            use rayon::prelude::*;

            // 1段階目: 一覧をすぐ出す。タグがキャッシュ(サイズ+更新日時が一致)にある曲は、
            // この時点でタグ込みで表示し、無い曲だけ名前とサイズで仮表示して後で読む
            let mut label_entries: Vec<(String, PathBuf)> = Vec::new();
            let paths = match &source {
                TrackSource::Paths(list) => list.clone(),
                TrackSource::Folder { folder, include_sub } => list_audio_files(folder, *include_sub),
                TrackSource::Labels { labels: wanted, match_all, expanded } => {
                    // ラベルを付けた曲・フォルダだけをメモリ上のラベル情報から選ぶ（フォルダは走査しない）
                    let mut v: Vec<PathBuf> = Vec::new();
                    let mut dirs: Vec<(String, PathBuf)> = Vec::new();
                    let mut files: Vec<(String, PathBuf)> = Vec::new();
                    for key in labels.all_tagged_paths() {
                        let tags = labels.get_labels_for_file(Path::new(&key));
                        let hit = if *match_all { wanted.iter().all(|l| tags.contains(l)) } else { wanted.iter().any(|l| tags.contains(l)) };
                        if !hit {
                            continue;
                        }
                        // キーは小文字化されているので、実際の大文字小文字のパスに戻す（無いファイルは飛ばす）
                        if let Ok(real) = std::fs::canonicalize(&key) {
                            let real = PathBuf::from(real.to_string_lossy().trim_start_matches(r"\\?\"));
                            if real.is_dir() {
                                dirs.push((key, real));
                            } else {
                                files.push((key, real));
                            }
                        }
                    }
                    // 折りたたまれたフォルダの配下のファイルは出さない（フォルダ行をクリックで展開）
                    files.retain(|(k, _)| !dirs.iter().any(|(d, _)| !expanded.contains(d) && k.starts_with(&format!("{}\\", d))));
                    dirs.sort_by(|a, b| a.0.cmp(&b.0));
                    for (key, real) in &dirs {
                        let name = real.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| real.to_string_lossy().to_string());
                        label_entries.push((format!("{} {}", if expanded.contains(key) { "▼" } else { "▶" }, name), real.clone()));
                    }
                    v.extend(files.into_iter().map(|(_, p)| p));
                    v.sort_by_key(|p| p.to_string_lossy().to_lowercase());
                    v
                }
            };
            let mut tracks: Vec<TrackItem> = Vec::with_capacity(paths.len());
            let mut misses: Vec<(std::path::PathBuf, Option<(u64, u64)>)> = Vec::new();
            for p in &paths {
                let stamp = file_stamp(p);
                // プレイリストの曲で、ファイルが無くなっているものは一覧に出さない
                if stamp.is_none() && matches!(source, TrackSource::Paths(_)) {
                    continue;
                }
                let cached = stamp.and_then(|(size, mtime)| {
                    let meta = TagMeta::decode(&tag_cache.get(p, size, mtime)?)?;
                    TrackItem::from_meta(p, size, &meta)
                });
                let mut t = match cached {
                    Some(t) => t,
                    None => {
                        misses.push((p.clone(), stamp));
                        match TrackItem::basic(p) {
                            Some(t) => t,
                            None => continue,
                        }
                    }
                };
                t.custom_tags = labels.get_labels_for_file(&t.file_path);
                tracks.push(t);
            }
            let entries = match &source {
                TrackSource::Folder { folder, .. } => list_folder_entries(folder),
                TrackSource::Labels { .. } => label_entries,
                TrackSource::Paths(_) => Vec::new(),
            };
            let _ = tx.send(ScanMsg::Listed { gen, tracks, entries });
            ctx.request_repaint();

            // 2段階目: キャッシュに無い曲だけタグ・再生時間を並列に読み、64件ずつ届ける
            // （別フォルダに移ったら打ち切り）。読めた内容はキャッシュに保存する
            misses.par_chunks(64).for_each(|chunk| {
                if latest.load(Ordering::Relaxed) != gen {
                    return;
                }
                let mut items = Vec::with_capacity(chunk.len());
                for (p, stamp) in chunk {
                    let meta = TagMeta::read(p);
                    let size = match stamp {
                        Some((size, mtime)) => {
                            tag_cache.put(p, *size, *mtime, meta.encode());
                            *size
                        }
                        None => std::fs::metadata(p).map(|m| m.len()).unwrap_or(0),
                    };
                    if let Some(t) = TrackItem::from_meta(p, size, &meta) {
                        items.push(t);
                    }
                }
                let _ = tx.send(ScanMsg::Tags { gen, items });
                ctx.request_repaint();
            });
        });
    }

    /// バックグラウンド読み込みの結果を反映する（最新の要求のものだけ）
    fn apply_scan_results(&mut self) {
        use std::sync::atomic::Ordering;
        let current_gen = self.scan_gen.load(Ordering::SeqCst);
        let mut tags_merged = false;
        while let Ok(msg) = self.scan_rx.try_recv() {
            match msg {
                ScanMsg::Listed { gen, tracks, entries } if gen == current_gen => {
                    let total_bytes: u64 = tracks.iter().map(|t| t.file_size_bytes).sum();
                    self.total_size_formatted = format_size(total_bytes);
                    self.track_index = tracks.iter().enumerate().map(|(i, t)| (t.file_path.clone(), i)).collect();
                    self.tracks = tracks;
                    self.folder_entries = entries;
                    self.apply_filter();
                    self.selected_track = None;
                    if let Some((p, play)) = self.pending_select.take() {
                        let key = crate::store::path_key(&p);
                        let pos = self.filtered_indices.iter().position(|&i| crate::store::path_key(&self.tracks[i].file_path) == key);
                        if let Some(pos) = pos {
                            self.selected_track = Some(pos);
                            if play {
                                self.play_track_at_index(pos);
                            }
                        }
                    }
                }
                ScanMsg::Tags { gen, items } if gen == current_gen => {
                    for item in items {
                        if let Some(&i) = self.track_index.get(&item.file_path) {
                            if let Some(slot) = self.tracks.get_mut(i) {
                                let labels = std::mem::take(&mut slot.custom_tags);
                                *slot = item;
                                slot.custom_tags = labels;
                                tags_merged = true;
                            }
                        }
                    }
                }
                _ => {} // 古い要求の結果は捨てる
            }
        }
        // 検索中なら、読み込めたタグで絞り込み結果を更新（選択は維持する）
        if tags_merged && !self.search_query.trim().is_empty() {
            let selected_path = self
                .selected_track
                .and_then(|s| self.filtered_indices.get(s))
                .and_then(|&i| self.tracks.get(i))
                .map(|t| t.file_path.clone());
            self.apply_filter();
            self.selected_track = selected_path
                .and_then(|p| self.filtered_indices.iter().position(|&i| self.tracks[i].file_path == p));
        }
    }

    /// 選択中プレイリストの内容が変わったら、右ペイン用の曲一覧を（バックグラウンドで）作り直す
    fn refresh_playlist_pane(&mut self) {
        let Some(id) = self.playlist_window.selected else {
            if self.pl_pane.id.is_some() {
                // 世代番号は巻き戻さない（閉じる前の読み込みの結果が、開き直した後の一覧に混ざらないように）
                self.pl_load_gen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let gen = self.pl_pane.gen + 1;
                self.pl_pane = PlaylistPane { gen, ..Default::default() };
            }
            return;
        };

        while let Ok((gen, items)) = self.pl_rx.try_recv() {
            if gen == self.pl_pane.gen {
                self.pl_pane.items = items;
                let len = self.pl_pane.items.len();
                if self.pl_pane.selected.is_some_and(|s| s >= len) {
                    self.pl_pane.selected = None;
                }
                if self.pl_pane.playing.is_some_and(|s| s >= len) {
                    self.pl_pane.playing = None;
                }
            }
        }

        // データが変わっていなければ、曲の一覧を複製して比べることもしない
        let version = self.playlist_service.version();
        if self.pl_pane.id == Some(id) && self.pl_pane.version == version {
            return;
        }
        let paths = self.playlist_service.get_playlist_tracks(id);
        self.pl_pane.version = version;
        if self.pl_pane.id == Some(id) && self.pl_pane.paths == paths && !self.pl_pane.force_reload {
            return;
        }
        self.pl_pane.force_reload = false;
        if self.pl_pane.id != Some(id) {
            self.pl_pane.items.clear();
            self.pl_pane.selected = None;
        }
        self.pl_pane.id = Some(id);
        self.pl_pane.gen += 1;
        self.pl_load_gen.store(self.pl_pane.gen, std::sync::atomic::Ordering::SeqCst);
        self.pl_pane.set = paths.iter().map(|p| crate::store::path_key(p)).collect();
        self.pl_pane.paths = paths.clone();

        let gen = self.pl_pane.gen;
        let latest = self.pl_load_gen.clone();
        let tag_cache = self.tag_cache.clone();
        let labels = self.label_service.clone();
        let tx = self.pl_tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let mut items: Vec<TrackItem> = Vec::with_capacity(paths.len());
            for p in &paths {
                // 新しい読み込みが始まった（ペインを閉じた・別のプレイリストにした）ら、残りは読まない
                if latest.load(std::sync::atomic::Ordering::SeqCst) != gen {
                    return;
                }
                let stamp = file_stamp(p);
                let meta = stamp
                    .and_then(|(size, mtime)| TagMeta::decode(&tag_cache.get(p, size, mtime)?))
                    .unwrap_or_else(|| {
                        let meta = TagMeta::read(p);
                        if let Some((size, mtime)) = stamp {
                            tag_cache.put(p, size, mtime, meta.encode());
                        }
                        meta
                    });
                if let Some(mut t) = TrackItem::from_meta(p, stamp.map_or(0, |s| s.0), &meta) {
                    t.custom_tags = labels.get_labels_for_file(p);
                    items.push(t);
                }
            }
            let _ = tx.send((gen, items));
            ctx.request_repaint();
        });
    }

    /// 新しい版の確認を別スレッドで始める
    fn start_update_check(&mut self) {
        if matches!(self.update_status, crate::update::UpdateStatus::Checking) {
            return;
        }
        let repo = self.settings.update_repo.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.update_rx = Some(rx);
        self.update_status = crate::update::UpdateStatus::Checking;
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(crate::update::check(&repo));
            ctx.request_repaint();
        });
    }

    /// 確認の開始（起動後、読み込みが終わってから少し待つ）・結果の受け取り・お知らせの表示
    fn update_check_tick(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        // 一覧の読み込みが終わったら、その3秒後に確認する（起動直後の他の処理と重ならないように）
        if !self.update_started && self.settings.check_updates && self.total_size_formatted != "読み込み中…" {
            let at = *self.update_ready_at.get_or_insert(now + 3.0);
            if now >= at {
                self.update_started = true;
                self.start_update_check();
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(500));
            }
        }
        if let Some(rx) = &self.update_rx {
            if let Ok(status) = rx.try_recv() {
                self.update_rx = None;
                if matches!(status, crate::update::UpdateStatus::Available { .. }) && !self.update_notified {
                    self.update_notified = true;
                    self.show_update_notice = true;
                }
                self.update_status = status;
            }
        }
        if self.show_update_notice {
            if let crate::update::UpdateStatus::Available { latest } = self.update_status.clone() {
                let mut close = false;
                egui::Window::new("🔔 新しいバージョンがあります")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.label(format!(
                            "Sonora {} が公開されています（お使いのバージョン: {}）。",
                            latest,
                            crate::update::APP_VERSION
                        ));
                        ui.label(RichText::new("ダウンロードは、リリースのページから行えます。").weak().size(11.0));
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if ui.button(RichText::new("🌐 ダウンロードページを開く").strong()).clicked() {
                                crate::update::open_release_page(&self.settings.update_repo);
                                close = true;
                            }
                            if ui.button("あとで").clicked() {
                                close = true;
                            }
                        });
                    });
                if close {
                    self.show_update_notice = false;
                }
            } else {
                self.show_update_notice = false;
            }
        }
    }

    /// 右ペインの曲情報（タグ・ラベル）を、次の描画で読み直させる
    fn reload_playlist_pane(&mut self) {
        self.pl_pane.force_reload = true;
        self.pl_pane.version = u64::MAX;
    }

    /// プレイリストペイン上の曲操作（再生・詳細・タグ編集など）
    fn handle_pane_action(&mut self, ctx: &egui::Context, action: TrackAction) {
        let item = |s: &Self, i: usize| s.pl_pane.items.get(i).cloned();
        match action {
            TrackAction::OpenLabelPicker(i, pos) => {
                self.pl_pane.selected = Some(i);
                self.pane_has_focus = true;
                if let Some(t) = item(self, i) {
                    self.label_picker = Some(LabelPicker { path: t.file_path, pos, new_label: String::new(), just_opened: true });
                }
            }
            TrackAction::Select(i) => {
                self.pl_pane.selected = Some(i);
                self.pane_has_focus = true;
            }
            TrackAction::Play(i) => {
                if let Some(t) = item(self, i) {
                    self.pl_pane.selected = Some(i);
                    self.pane_has_focus = true;
                    if self.is_playing_now(Some(&t.file_path)) {
                        self.player.stop(); // 流れている曲と同じ曲のダブルクリックは停止
                    } else {
                        self.current_playing_index = None;
                        self.play_from_pane = true;
                        self.pl_pane.playing = Some(i);
                        self.audition = false;
                        self.player.play(&t.file_path);
                    }
                }
            }
            TrackAction::RemoveFromPlaylist(i) => {
                if let Some(pid) = self.pl_pane.id {
                    let mut paths = self.pl_pane.paths.clone();
                    // 一覧の行番号ではなく、その行の曲そのものを外す（同じ曲が複数あれば、同じ順番のものを外す）
                    let target = self.pl_pane.items.get(i).map(|t| crate::store::path_key(&t.file_path));
                    let pos = target.and_then(|key| {
                        let nth = self.pl_pane.items[..i].iter().filter(|t| crate::store::path_key(&t.file_path) == key).count();
                        paths.iter().enumerate().filter(|(_, p)| crate::store::path_key(p) == key).nth(nth).map(|(k, _)| k)
                    });
                    if let Some(pos) = pos {
                        paths.remove(pos);
                        self.pl_pane.playing = match self.pl_pane.playing {
                            Some(p) if p == i => None,
                            Some(p) if p > i => Some(p - 1),
                            other => other,
                        };
                        let _ = self.playlist_service.set_playlist_tracks(pid, &paths);
                        // 続けて Delete で消せるよう、同じ位置（末尾なら一つ前）の曲を選択状態にする
                        self.pl_pane.selected = if paths.is_empty() { None } else { Some(i.min(paths.len() - 1)) };
                        if self.active_selection == ViewSelection::Playlist(pid) {
                            self.refresh_current_view();
                        }
                        // 消えた行に紐づくメニュー/操作状態が残って入力を塞がないようにする
                        ctx.memory_mut(|m| m.close_popup());
                        ctx.request_repaint();
                        let name = self
                            .playlist_service
                            .get_all_playlists()
                            .into_iter()
                            .find(|p| p.id == pid)
                            .map(|p| p.name)
                            .unwrap_or_default();
                        self.toast = Some((format!("[{}]から削除しました", name), ctx.input(|i| i.time)));
                    }
                }
            }
            TrackAction::EditTags(i) => {
                if let Some(t) = item(self, i) {
                    self.active_dialog = ActiveDialog::EditTags { form: Box::new(TagForm::from_track(&t)), track: t };
                }
            }
            TrackAction::EditLabels(i) => {
                if let Some(t) = item(self, i) {
                    let current_labels = self.label_service.get_labels_for_file(&t.file_path);
                    self.active_dialog = ActiveDialog::EditLabels { track: t, current_labels, new_label_input: String::new() };
                }
            }
            TrackAction::ShowDetails(i) => {
                if let Some(mut t) = item(self, i) {
                    t.artwork_bytes = TrackItem::load_artwork(&t.file_path);
                    self.active_dialog = ActiveDialog::TrackDetails { track: t };
                }
            }
            TrackAction::OpenExplorer(i) => {
                if let Some(t) = item(self, i) {
                    self.reveal_in_tree(t.file_path);
                }
            }
            TrackAction::AddFavorite(i) => {
                if let Some(t) = item(self, i) {
                    self.handle_fav_action(FavAction::AddPath(t.file_path));
                }
            }
            TrackAction::CopyPath(i) => {
                if let Some(t) = item(self, i) {
                    ctx.output_mut(|o| o.copied_text = t.file_path.to_string_lossy().to_string());
                }
            }
            _ => {}
        }
    }

    /// Ctrl/Shift で複数選択している曲（一覧の並び順）。2曲未満なら空
    fn multi_selected_paths(&self, ctx: &egui::Context) -> Vec<PathBuf> {
        let sel: HashSet<PathBuf> = ctx.data(|d| d.get_temp(crate::views::track_list::multi_sel_id())).unwrap_or_default();
        if sel.len() < 2 {
            return Vec::new();
        }
        self.filtered_indices
            .iter()
            .filter_map(|&i| self.tracks.get(i))
            .filter(|t| sel.contains(&t.file_path))
            .map(|t| t.file_path.clone())
            .collect()
    }

    /// 選択中の複数の曲を、選択中のプレイリストへまとめて追加する（全部すでに入っていれば、まとめて外す）
    fn add_multi_to_selected_playlist(&mut self, ctx: &egui::Context, paths: &[PathBuf]) {
        let Some(pid) = self.playlist_window.selected else { return };
        if paths.is_empty() {
            return;
        }
        let all_in = paths.iter().all(|p| self.pl_pane.set.contains(&crate::store::path_key(p)));
        let name = self
            .playlist_service
            .get_all_playlists()
            .into_iter()
            .find(|p| p.id == pid)
            .map(|p| p.name)
            .unwrap_or_default();
        let msg = if all_in {
            match self.playlist_service.remove_tracks(pid, paths) {
                Ok(n) => format!("[{}]から {} 曲を削除しました", name, n),
                Err(e) => e,
            }
        } else {
            match self.playlist_service.add_tracks(pid, paths) {
                Ok(n) if n == paths.len() => format!("[{}]に {} 曲を追加しました", name, n),
                Ok(n) => format!("[{}]に {} 曲を追加しました（{} 曲は登録済み）", name, n, paths.len() - n),
                Err(e) => e,
            }
        };
        self.toast = Some((msg, ctx.input(|i| i.time)));
        if self.active_selection == ViewSelection::Playlist(pid) {
            self.refresh_current_view();
        }
    }

    /// 一覧の idx 番目の曲を、選択中のプレイリストに追加して「追加しました」を表示する
    fn add_to_selected_playlist(&mut self, ctx: &egui::Context, idx: usize) {
        let Some(pid) = self.playlist_window.selected else { return };
        let Some(path) = self.filtered_indices.get(idx).and_then(|&i| self.tracks.get(i)).map(|t| t.file_path.clone()) else {
            return;
        };
        // すでに入っていれば削除、入っていなければ追加（ヘッダーの「追加／削除」ボタンと同じ動き）
        let current = self.playlist_service.get_playlist_tracks(pid);
        let key = crate::store::path_key(&path);
        let removing = current.iter().any(|p| crate::store::path_key(p) == key);
        let ok = if removing {
            let kept: Vec<PathBuf> = current.into_iter().filter(|p| crate::store::path_key(p) != key).collect();
            self.playlist_service.set_playlist_tracks(pid, &kept).is_ok()
        } else {
            self.playlist_service.add_track(pid, &path).is_ok()
        };
        if !ok {
            return;
        }
        let name = self
            .playlist_service
            .get_all_playlists()
            .into_iter()
            .find(|p| p.id == pid)
            .map(|p| p.name)
            .unwrap_or_default();
        let msg = if removing { format!("[{}]から削除しました", name) } else { format!("[{}]に追加しました", name) };
        self.toast = Some((msg, ctx.input(|i| i.time)));
        // 表示中のプレイリストへ追加した場合は一覧も更新
        if self.active_selection == ViewSelection::Playlist(pid) {
            self.refresh_current_view();
        }
    }

    pub fn open_labels_view(&mut self, labels: Vec<String>) {
        if labels.is_empty() {
            // 全ラベルの選択が外れたらフォルダ表示に戻る
            self.open_folder(self.current_folder.clone());
            return;
        }
        let match_all = self.label_match_all;
        self.active_selection = ViewSelection::Labels(labels.clone());
        self.folder_entries.clear();
        let expanded = self.label_expanded.clone();
        self.spawn_track_loader(TrackSource::Labels { labels, match_all, expanded });
    }

    pub fn open_playlist_view(&mut self, playlist_id: i64) {
        self.active_selection = ViewSelection::Playlist(playlist_id);
        self.playlist_window.selected = Some(playlist_id);
        self.folder_entries.clear();
        // タグの読み込みはバックグラウンドで行う（曲数が多い・ディスクが遅くても画面を止めない）
        let paths = self.playlist_service.get_playlist_tracks(playlist_id);
        self.spawn_track_loader(TrackSource::Paths(paths));
    }

    pub fn refresh_current_view(&mut self) {
        match self.active_selection.clone() {
            ViewSelection::Folder(p) => self.open_folder(p),
            ViewSelection::Labels(l) => self.open_labels_view(l),
            ViewSelection::Playlist(id) => self.open_playlist_view(id),
        }
    }

    pub fn apply_filter(&mut self) {
        self.applied_query = self.search_query.clone();
        let query = SearchQuery::parse(&self.search_query);
        self.filtered_indices = if query.is_empty() {
            (0..self.tracks.len()).collect()
        } else {
            self.tracks
                .iter()
                .enumerate()
                .filter(|(_, t)| query.matches(t))
                .map(|(i, _)| i)
                .collect()
        };
        // 絞り込み後は並びが変わるため、古いインデックスの選択は無効にする
        self.selected_track = None;
    }

    pub fn play_track_at_index(&mut self, filtered_idx: usize) {
        if let Some(&original_idx) = self.filtered_indices.get(filtered_idx) {
            if let Some(track) = self.tracks.get(original_idx) {
                if self.crossfade_next {
                    self.player.play_crossfade(&track.file_path);
                } else {
                    self.player.play(&track.file_path);
                }
                self.current_playing_index = Some(original_idx);
                self.play_from_pane = false;
                self.audition = false;
                self.selected_track = Some(filtered_idx);
            }
        }
    }

    pub fn play_next(&mut self) {
        // 右ペインのプレイリストから再生している間は、そのプレイリストの次の曲へ進む
        if self.play_from_pane {
            if let Some(cur) = self.player.current_track_path() {
                let key = crate::store::path_key(&cur);
                let items = &self.pl_pane.items;
                // 同じ曲が複数入っていても迷わないよう、再生を始めた行を優先する
                let playing = self.pl_pane.playing.filter(|&p| items.get(p).is_some_and(|t| crate::store::path_key(&t.file_path) == key));
                if let Some(pos) = playing.or_else(|| items.iter().position(|t| crate::store::path_key(&t.file_path) == key)) {
                    let next = if self.player.shuffle && !items.is_empty() {
                        Some(rand_simple(items.len()))
                    } else if pos + 1 < items.len() {
                        Some(pos + 1)
                    } else if self.player.repeat_mode == RepeatMode::All {
                        Some(0)
                    } else {
                        None
                    };
                    if let Some(n) = next {
                        let path = items[n].file_path.clone();
                        self.pl_pane.selected = Some(n);
                        self.pl_pane.playing = Some(n);
                        if self.crossfade_next {
                            self.player.play_crossfade(&path);
                        } else {
                            self.player.play(&path);
                        }
                    }
                    return;
                }
            }
            self.play_from_pane = false;
        }

        if self.filtered_indices.is_empty() {
            return;
        }

        if self.player.shuffle {
            let next_idx = rand_simple(self.filtered_indices.len());
            self.play_track_at_index(next_idx);
            return;
        }

        // フォーカス（選択）ではなく、いま再生している曲の次を再生する
        let playing_pos = self.player.current_track_path().and_then(|cur| {
            let key = crate::store::path_key(&cur);
            self.filtered_indices.iter().position(|&i| {
                self.tracks.get(i).is_some_and(|t| crate::store::path_key(&t.file_path) == key)
            })
        });
        match playing_pos.or(self.selected_track) {
            Some(sel) => {
                if sel + 1 < self.filtered_indices.len() {
                    self.play_track_at_index(sel + 1);
                } else if self.player.repeat_mode == RepeatMode::All {
                    self.play_track_at_index(0); // 全曲ループ
                }
            }
            None => self.play_track_at_index(0),
        }
    }

    pub fn play_prev(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }

        if let Some(sel) = self.selected_track {
            if sel > 0 {
                self.play_track_at_index(sel - 1);
            } else if !self.filtered_indices.is_empty() {
                self.play_track_at_index(self.filtered_indices.len() - 1);
            }
        }
    }
}

impl eframe::App for MusicManagerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.apply_scan_results();
        self.refresh_playlist_pane();

        // 重複検出（ハッシュチェック）の開始要望（確認ダイアログから）
        if ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::new("dup_hash_start"))).unwrap_or(false) {
            self.dup_gen += 1;
            let scan = crate::tools::hash_scan::HashScan::start(&self.tracks);
            // 前の検査が残っていれば、置き換えで中止される
            self.dup_win = Some(crate::views::duplicate_window::DuplicateWindow::new(scan, self.dup_gen));
        }

        // ダイアログ等からの「ツリーで開く」要望
        if let Some(p) = ctx.data_mut(|d| d.remove_temp::<PathBuf>(egui::Id::new("reveal_request"))) {
            self.reveal_in_tree(p);
        }

        // お気に入りに登録済みのフォルダ・曲（一覧とツリーで ⭐ を付けるために共有する）
        {
            let keys: std::collections::HashSet<String> =
                self.settings.favorites.iter().map(|(_, p)| crate::store::path_key(Path::new(p))).collect();
            ctx.data_mut(|d| d.insert_temp(crate::views::folder_tree::fav_keys_id(), std::sync::Arc::new(keys)));
        }

        // キーの割り当て中は、押したキーを受け取れるよう、グローバルホットキーを一時的に止める
        if !self.show_settings {
            ctx.data_mut(|d| d.remove::<HotkeyAction>(crate::app::settings_ui::hotkey_capture_id()));
        }
        let capturing = ctx.data(|d| d.get_temp::<HotkeyAction>(crate::app::settings_ui::hotkey_capture_id())).is_some();
        self.hotkeys.set_enabled(self.settings.enable_global_hotkeys && !capturing);

        // メディアキー・グローバルホットキー
        for action in self.hotkeys.poll() {
            match action {
                HotkeyAction::PlayPause => self.toggle_play_pause(),
                HotkeyAction::Next => self.play_next(),
                HotkeyAction::Previous => self.play_prev(),
                HotkeyAction::Stop => self.player.stop(),
                HotkeyAction::Reverse => {
                    self.toggle_reverse();
                    let msg = if self.player.is_reverse() { "逆再生: オン" } else { "逆再生: オフ" };
                    self.toast = Some((msg.to_string(), ctx.input(|i| i.time)));
                }
                HotkeyAction::SpeedUp | HotkeyAction::SpeedDown | HotkeyAction::SpeedReset => {
                    let speed = match action {
                        HotkeyAction::SpeedUp => self.player.speed + 0.05,
                        HotkeyAction::SpeedDown => self.player.speed - 0.05,
                        _ => 1.0,
                    };
                    self.set_speed(speed);
                    self.toast = Some((format!("再生速度: ×{:.2}", self.player.speed), ctx.input(|i| i.time)));
                }
            }
        }

        // クロスフェード: 曲の終わりの手前で次の曲を重ねて始める
        if self.player.check_and_clear_crossfade_due() && self.player.repeat_mode != RepeatMode::One && !self.audition {
            self.crossfade_next = true;
            self.play_next();
            self.crossfade_next = false;
        }

        // 曲終了検知と自動送り
        if self.player.check_and_clear_ended() && !self.audition {
            match self.player.repeat_mode {
                RepeatMode::One => {
                    if let Some(cur) = self.player.current_track_path() {
                        self.player.play(&cur);
                    } else if let Some(sel) = self.selected_track {
                        self.play_track_at_index(sel);
                    }
                }
                _ => self.play_next(),
            }
        }

        // 再生進行中のUI再描画（開始・終了の直後は、プレイヤー側からの通知で更新される）
        if self.player.state() == crate::audio::player::PlaybackState::Playing {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        let copy_feedback = self
            .copy_feedback_instant
            .map(|t| t.elapsed().as_secs_f32() < 2.0)
            .unwrap_or(false);

        // 選択中のプレイリスト名（ヘッダーのボタン表示・右クリック追加に使う）
        let active_playlist_name: Option<String> = self.playlist_window.selected.and_then(|id| {
            self.playlist_service.get_all_playlists().into_iter().find(|p| p.id == id).map(|p| p.name)
        });

        // --- 1. トップバー（ヘッダー・検索・直接パス入力・ツール） ---
        egui::TopBottomPanel::top("top_header")
            .frame(egui::Frame::none().fill(crate::theme::c(30, 41, 59)).inner_margin(Margin::same(8.0)))
            .show(ctx, |ui| {
                let mut open_folder_requested = false;
                let mut copy_clicked = false;
                let mut navigate_to: Option<String> = None;
                let mut tool_selected: Option<HeaderToolAction> = None;
                let mut open_column_settings = false;
                let mut open_help = false;
                let mut open_settings = false;
                let active_engine_name = self.player.active_engine_name();
                let mut new_volume: Option<f32> = None;
                let mut cycle_mode = false;
                let mut toggle_mini = false;
                let mut add_selected_requested = false;
                let mut playlist_menu: Option<PlaylistMenu> = None;
                let playlists_info = self.playlist_service.get_all_playlists();
                let multi_paths = self.multi_selected_paths(ctx);
                let multi_all_in_playlist =
                    !multi_paths.is_empty() && multi_paths.iter().all(|p| self.pl_pane.set.contains(&crate::store::path_key(p)));
                // 最後にペインで曲を選んだ場合、ヘッダーのボタンはペインで選んだ曲（=登録済み）が対象になる
                let pane_target = self.pane_has_focus
                    && self.pl_pane.selected.is_some_and(|i| i < self.pl_pane.items.len());
                let mut playlist_anchor: Option<egui::Pos2> = None;
                let playlist_label = match active_playlist_name.as_deref() {
                    Some(n) => n.to_string(),
                    None => "プレイリスト[選択なし]".to_string(),
                };
                let mut open_repo = false;
                let prev_check_updates = self.settings.check_updates;
                let prev_inc = self.settings.include_subfolders;
                let prev_query = self.search_query.clone();

                render_header(
                    ui,
                    &mut self.search_query,
                    &mut self.folder_path_input,
                    &mut self.settings.include_subfolders,
                    self.filtered_indices.len(),
                    &self.total_size_formatted,
                    &playlist_label,
                    &playlists_info,
                    self.playlist_window.selected,
                    if pane_target { 0 } else { multi_paths.len() },
                    active_playlist_name.is_some(),
                    pane_target || !multi_paths.is_empty() || self.selected_track.is_some_and(|i| i < self.filtered_indices.len()),
                    pane_target
                        || if multi_paths.is_empty() {
                            self.selected_track
                                .and_then(|i| self.filtered_indices.get(i))
                                .and_then(|&o| self.tracks.get(o))
                                .is_some_and(|t| self.pl_pane.set.contains(&crate::store::path_key(&t.file_path)))
                        } else {
                            multi_all_in_playlist
                        },
                    &mut playlist_anchor,
                    copy_feedback,
                    self.player.volume,
                    self.player.engine,
                    &active_engine_name,
                    self.player.engine_is_fallback(),
                    || open_folder_requested = true,
                    || copy_clicked = true,
                    |path| navigate_to = Some(path),
                    |tool| tool_selected = Some(tool),
                    || open_column_settings = true,
                    || open_help = true,
                    || add_selected_requested = true,
                    |m| playlist_menu = Some(m),
                    || open_settings = true,
                    &mut self.settings.check_updates,
                    &self.settings.update_repo,
                    || open_repo = true,
                    |v| new_volume = Some(v),
                    || cycle_mode = true,
                    || toggle_mini = true,
                );

                if toggle_mini {
                    self.show_mini = !self.show_mini;
                    if !self.show_mini {
                        self.mini_gen += 1;
                        self.mini_frames = 0;
                    }
                }

                if let Some(v) = new_volume {
                    self.player.set_volume(v);
                }
                if cycle_mode {
                    use crate::settings::AudioOutputEngine as E;
                    let next = match self.player.engine {
                        E::Auto => E::Wasapi,
                        E::Wasapi => E::DirectSound,
                        E::DirectSound => E::WaveOut,
                        E::WaveOut => E::Auto,
                    };
                    self.player.set_engine(next);
                    self.settings.audio_engine = next;
                }

                if let Some(pos) = playlist_anchor {
                    self.toast_anchor = pos;
                }
                if open_settings {
                    self.show_settings = true;
                }
                if open_repo {
                    crate::update::open_release_page(&self.settings.update_repo);
                }
                if self.settings.check_updates != prev_check_updates {
                    self.settings.save(&self.settings_path);
                }
                if let Some(m) = playlist_menu {
                    match m {
                        PlaylistMenu::Select(id) => self.playlist_window.selected = Some(id),
                        PlaylistMenu::Deselect => self.playlist_window.selected = None,
                        PlaylistMenu::OpenWindow => {
                            if self.show_playlist_window {
                                // すでに開いているときは、前面に出す
                                let id = egui::ViewportId::from_hash_of(("playlist_window", self.pl_win_gen));
                                ctx.send_viewport_cmd_to(id, egui::ViewportCommand::Focus);
                            }
                            self.show_playlist_window = true;
                        }
                        PlaylistMenu::Create(name) => match self.playlist_service.create_playlist(&name) {
                            Ok(id) => {
                                self.playlist_window.selected = Some(id);
                                self.toast = Some((format!("[{}]を作成しました", name), ctx.input(|i| i.time)));
                            }
                            Err(e) => self.toast = Some((e, ctx.input(|i| i.time))),
                        },
                    }
                }
                if add_selected_requested {
                    if !pane_target && !multi_paths.is_empty() {
                        self.add_multi_to_selected_playlist(ctx, &multi_paths);
                    } else if pane_target {
                        // ペインで選んだ曲 → プレイリストから削除
                        if let Some(sel) = self.pl_pane.selected {
                            self.handle_pane_action(ctx, TrackAction::RemoveFromPlaylist(sel));
                        }
                    } else if let Some(idx) = self.selected_track {
                        self.add_to_selected_playlist(ctx, idx);
                    }
                }

                if open_help {
                    self.show_help = true;
                }

                if open_column_settings {
                    self.show_column_settings = true;
                }

                if copy_clicked {
                    self.copy_feedback_instant = Some(Instant::now());
                }

                if let Some(path_str) = navigate_to {
                    let p = PathBuf::from(path_str.trim());
                    if p.is_dir() {
                        self.open_folder(p);
                        self.reveal_frames = 4;
                    }
                }

                if self.search_query != prev_query {
                    self.apply_filter();
                }

                if self.settings.include_subfolders != prev_inc {
                    self.refresh_current_view();
                }

                if open_folder_requested {
                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                        self.open_folder(folder);
                    }
                }

                if let Some(action) = tool_selected {
                    match action {
                        HeaderToolAction::BatchRename => {
                            let plan = generate_rename_plan(&self.tracks, RenamePattern::ArtistTitle);
                            self.active_dialog = ActiveDialog::BatchRename {
                                tracks: self.tracks.clone(),
                                pattern: RenamePattern::ArtistTitle,
                                plan,
                                result_message: None,
                            };
                        }
                        HeaderToolAction::FindDuplicates => {
                            let groups = find_duplicates(&self.tracks);
                            self.active_dialog = ActiveDialog::DuplicateFinder { groups };
                        }
                        HeaderToolAction::Lyrics => {
                            self.show_lyrics = !self.show_lyrics;
                            if self.show_lyrics && self.lyrics_win.is_none() {
                                self.lyrics_win = Some(crate::views::lyrics_window::LyricsWindow::new());
                            }
                        }
                        HeaderToolAction::FindDuplicatesHash => {
                            if self.tracks.is_empty() {
                                self.toast = Some(("検査する曲がありません（先にフォルダを開いてください）".to_string(), ctx.input(|i| i.time)));
                            } else {
                                let bytes: u64 = self.tracks.iter().map(|t| t.file_size_bytes).sum();
                                self.active_dialog = ActiveDialog::DuplicateHashWarn {
                                    count: self.tracks.len(),
                                    total_size: crate::scanner::metadata::format_size(bytes),
                                };
                            }
                        }
                        HeaderToolAction::ExportPlaylist => {
                            if let Some(save_file) = rfd::FileDialog::new()
                                .add_filter("M3U8 Playlist", &["m3u8", "m3u"])
                                .set_file_name("playlist.m3u8")
                                .save_file()
                            {
                                if let Ok(mut f) = std::fs::File::create(&save_file) {
                                    use std::io::Write;
                                    let _ = writeln!(f, "#EXTM3U");
                                    for &idx in &self.filtered_indices {
                                        if let Some(t) = self.tracks.get(idx) {
                                            let _ = writeln!(f, "{}", t.file_path.to_string_lossy());
                                        }
                                    }
                                }
                            }
                        }
                        HeaderToolAction::ImportPlaylist => {
                            if let Some(open_file) = rfd::FileDialog::new()
                                .add_filter("M3U8 Playlist", &["m3u8", "m3u"])
                                .pick_file()
                            {
                                let stem = open_file.file_stem().unwrap_or_default().to_string_lossy().to_string();
                                if let Ok(pid) = self.playlist_service.import_m3u8(&stem, &open_file) {
                                    self.open_playlist_view(pid);
                                }
                            }
                        }
                    }
                }
            });

        // --- 3. 左サイドバー（エクスプローラー・お気に入り・ラベルの3タブ） ---
        // ID を変更: 旧バージョンで永続化された巨大なパネル幅を引き継がない
        egui::SidePanel::left("left_tree_v2")
            .default_width(self.settings.left_panel_width)
            .width_range(200.0..=600.0)
            .resizable(true)
            .frame(egui::Frame::none().fill(crate::theme::c(15, 23, 42)).inner_margin(Margin::same(8.0)))
            .show(ctx, |ui| {
                let mut select_view: Option<ViewSelection> = None;
                let mut new_pl_name: Option<String> = None;
                let match_before = self.label_match_all;
                let mut fav_action: Option<FavAction> = None;

                render_folder_tree(
                    ui,
                    &self.settings.favorites,
                    &self.current_folder,
                    &self.active_selection,
                    &mut self.left_panel_tab,
                    &mut self.label_match_all,
                    self.reveal_frames > 0,
                    &self.label_service,
                    &self.playlist_service,
                    &mut |v| select_view = Some(v),
                    &mut |name| new_pl_name = Some(name),
                    &mut |a| fav_action = Some(a),
                );

                if let Some(a) = fav_action {
                    self.handle_fav_action(a);
                }

                if match_before != self.label_match_all {
                    if let ViewSelection::Labels(l) = self.active_selection.clone() {
                        self.open_labels_view(l);
                    }
                }

                if let Some(name) = new_pl_name {
                    let _ = self.playlist_service.create_playlist(&name);
                }

                if let Some(v) = select_view {
                    match v {
                        ViewSelection::Folder(p) => self.open_folder(p),
                        ViewSelection::Labels(l) => self.open_labels_view(l),
                        ViewSelection::Playlist(id) => self.open_playlist_view(id),
                    }
                }
            });

        if self.reveal_frames > 0 {
            self.reveal_frames -= 1;
            ctx.request_repaint();
        }

        // 検索欄の内容と絞り込み結果がずれていたら必ず再絞り込みする
        if self.search_query != self.applied_query {
            self.apply_filter();
        }

        // Delete キー: 選択中の曲（ペイン／メイン一覧の登録済みの曲）をプレイリストから削除（文字入力中は無効）
        // 歌詞のタイムスタンプ打ちの最中は、Delete は歌詞の操作に使う
        let lyrics_keys = ctx.data(|d| d.get_temp::<bool>(egui::Id::new("lyrics_keys_active"))).unwrap_or(false);
        if !lyrics_keys && !ctx.wants_keyboard_input() && ctx.input(|i| i.key_pressed(egui::Key::Delete)) {
            if self.pane_has_focus {
                // ペインで選んだ曲 → プレイリストから外す
                if let (Some(sel), true) = (self.pl_pane.selected, self.pl_pane.id.is_some()) {
                    self.handle_pane_action(ctx, TrackAction::RemoveFromPlaylist(sel));
                }
            } else if let Some(sel) = self.selected_track {
                // メイン一覧で選んだ曲が、選択中プレイリストに入っている（緑の行）なら外す
                let in_playlist = self
                    .filtered_indices
                    .get(sel)
                    .and_then(|&i| self.tracks.get(i))
                    .is_some_and(|t| self.pl_pane.set.contains(&crate::store::path_key(&t.file_path)));
                if in_playlist {
                    self.add_to_selected_playlist(ctx, sel); // 入っている曲に対しては「削除」として働く
                }
            }
        }

        // --- 3a. 歌詞ペイン（メイン画面の右からスライドして出る。幅の既定はメインペインの半分） ---
        if !self.show_lyrics {
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("lyrics_keys_active"), false));
        }
        if self.lyrics_win.is_some() {
            use crate::views::lyrics_window::{render_lyrics_window, LyricsAction};
            let avail = ctx.available_rect().width();
            let default_w = if self.settings.lyrics_pane_width > 0.0 {
                self.settings.lyrics_pane_width
            } else {
                (avail / 3.0).max(260.0)
            };
            self.lyrics_default_w = default_w;
            let playing_path = self.player.current_track_path();
            let selected_path = self
                .selected_track
                .and_then(|s| self.filtered_indices.get(s))
                .and_then(|&i| self.tracks.get(i))
                .map(|t| t.file_path.clone());
            let now = playing_path.clone().or(selected_path);
            let (pos, total, state) = (self.player.current_position(), self.player.total_duration(), self.player.state());
            let rgb = |c: [u8; 3]| egui::Color32::from_rgb(c[0], c[1], c[2]);
            let style = crate::views::lyrics_window::LyricsStyle {
                current: rgb(self.settings.lyrics_color_current),
                past: rgb(self.settings.lyrics_color_past),
                upcoming: rgb(self.settings.lyrics_color_upcoming),
                sweep: self.settings.lyrics_sweep,
            };
            let mut act: Option<LyricsAction> = None;
            if let Some(w) = self.lyrics_win.as_mut() {
                egui::SidePanel::right("lyrics_pane")
                    .default_width(default_w)
                    .width_range(220.0..=(avail * 0.8).max(360.0))
                    .resizable(true)
                    .frame(egui::Frame::none().fill(crate::theme::c(15, 23, 42)).inner_margin(Margin::same(8.0)))
                    .show_animated(ctx, self.show_lyrics, |ui| {
                        act = render_lyrics_window(ui, w, now.as_ref(), playing_path.as_ref(), pos, total, state, &style);
                    });
            }
            match act {
                Some(LyricsAction::Seek(d)) => self.player.seek(d),
                Some(LyricsAction::TogglePause) => self.toggle_play_pause(),
                Some(LyricsAction::Close) => self.show_lyrics = false,
                None => {}
            }
            // 保存した曲は、一覧の「歌詞」の印を更新する
            if let Some(path) = self.lyrics_win.as_mut().and_then(|w| w.take_saved()) {
                let key = crate::store::path_key(&path);
                for t in self.tracks.iter_mut().chain(self.pl_pane.items.iter_mut()) {
                    if crate::store::path_key(&t.file_path) == key {
                        t.has_lyrics = true;
                    }
                }
            }
        }
        // --- 3b. 右ペイン: 選択中プレイリストの曲一覧（既定の幅はメインペインの半分） ---
        if let Some(pid) = self.pl_pane.id {
            let avail = ctx.available_rect().width();
            let name = self
                .playlist_service
                .get_all_playlists()
                .into_iter()
                .find(|p| p.id == pid)
                .map(|p| p.name)
                .unwrap_or_default();
            let current_playing_path = self.player.current_track_path();
            let current_pos = self.player.current_position();
            let total_dur = self.player.total_duration();
            let playback = self.player.state();
            let mut pane_action: Option<TrackAction> = None;
            let pane_columns = self.settings.playlist_columns.with_tag_mode(self.settings.tag_display_mode);
            let mut close_pane = false;

            egui::SidePanel::right("playlist_pane")
                // メインペイン : 右ペイン = 2 : 1（右ペインはメインの半分）
                .default_width((avail / 3.0).max(240.0))
                .width_range(200.0..=(avail * 0.8).max(320.0))
                .resizable(true)
                .frame(egui::Frame::none().fill(crate::theme::c(15, 23, 42)).inner_margin(Margin::same(6.0)))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("📑 {}", name)).strong().size(14.0));
                        ui.label(RichText::new(format!("{} 曲", self.pl_pane.paths.len())).weak());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("✖").on_hover_text("ペインを閉じる（プレイリストの選択を解除）").clicked() {
                                close_pane = true;
                            }
                        });
                    });
                    ui.separator();
                    render_track_list(
                        ui,
                        &self.pl_pane.items,
                        &self.pl_pane.selected,
                        &current_playing_path,
                        current_pos,
                        total_dur,
                        playback,
                        self.player.shuffle,
                        self.player.repeat_mode,
                        self.player.is_reverse(),
                        self.player.speed,
                        &mut self.media_cache,
                        &pane_columns,
                        &[],
                        None,
                        None,
                        true,
                        false,
                        &mut |act| pane_action = Some(act),
                    );
                });

            if close_pane {
                self.playlist_window.selected = None;
            }
            if let Some(act) = pane_action {
                self.handle_pane_action(ctx, act);
            }
        }

        // --- 4. 中央メイン（波形・タグ区切り ＆ 楽曲一覧テーブル） ---
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(crate::theme::c(15, 23, 42)).inner_margin(Margin::same(6.0)))
            .show(ctx, |ui| {
                let filtered_tracks: Vec<TrackItem> = self
                    .filtered_indices
                    .iter()
                    .filter_map(|&i| self.tracks.get(i).cloned())
                    .collect();

                let current_playing_path = self.player.current_track_path();
                let current_pos = self.player.current_position();
                let total_dur = self.player.total_duration();
                let mut track_action: Option<TrackAction> = None;
                let main_columns = self.settings.columns.with_tag_mode(self.settings.tag_display_mode);

                render_track_list(
                    ui,
                    &filtered_tracks,
                    &self.selected_track,
                    &current_playing_path,
                    current_pos,
                    total_dur,
                    self.player.state(),
                    self.player.shuffle,
                    self.player.repeat_mode,
                    self.player.is_reverse(),
                    self.player.speed,
                    &mut self.media_cache,
                    &main_columns,
                    if self.search_query.trim().is_empty() { &self.folder_entries[..] } else { &[][..] },
                    active_playlist_name.as_deref(),
                    if self.pl_pane.id.is_some() { Some(&self.pl_pane.set) } else { None },
                    false,
                    matches!(self.active_selection, ViewSelection::Labels(_)),
                    &mut |act| track_action = Some(act),
                );

                if let Some(action) = track_action {
                    match action {
                        TrackAction::RemoveFromPlaylist(_) => {} // プレイリストペイン専用
                        TrackAction::AddToPlaylist(idx) => {
                            self.add_to_selected_playlist(ctx, idx);
                        }
                        TrackAction::AddMultiToPlaylist => {
                            let paths = self.multi_selected_paths(ctx);
                            self.add_multi_to_selected_playlist(ctx, &paths);
                        }
                        TrackAction::TogglePlay(idx) => {
                            let path = self
                                .filtered_indices
                                .get(idx)
                                .and_then(|&i| self.tracks.get(i))
                                .map(|t| t.file_path.clone());
                            let is_current = path.is_some() && self.player.current_track_path() == path;
                            match (is_current, self.player.state()) {
                                (true, crate::audio::player::PlaybackState::Playing) => self.player.pause(),
                                (true, crate::audio::player::PlaybackState::Paused) => self.player.resume(),
                                _ => self.play_track_at_index(idx),
                            }
                        }
                        TrackAction::OpenLabelPicker(idx, pos) => {
                            self.pane_has_focus = false;
                            self.selected_track = Some(idx);
                            if let Some(t) = self.filtered_indices.get(idx).and_then(|&i| self.tracks.get(i)) {
                                self.label_picker = Some(LabelPicker { path: t.file_path.clone(), pos, new_label: String::new(), just_opened: true });
                            }
                        }
                        TrackAction::ToggleShuffle => {
                            self.player.toggle_shuffle();
                        }
                        TrackAction::CycleRepeat => {
                            self.player.cycle_repeat_mode();
                        }
                        TrackAction::Prev => self.play_prev(),
                        TrackAction::Next => self.play_next(),
                        TrackAction::Stop => self.player.stop(),
                        TrackAction::ToggleReverse => self.toggle_reverse(),
                        TrackAction::SetSpeed(v) => self.set_speed(v),
                        TrackAction::Seek(d) => self.player.seek(d),
                        TrackAction::OpenFolder(p) => {
                            self.open_folder(p);
                            self.reveal_frames = 4;
                        }
                        TrackAction::Play(idx) => {
                            // 流れている曲と同じ曲をダブルクリックしたら、止める
                            let path = self.filtered_indices.get(idx).and_then(|&i| self.tracks.get(i)).map(|t| t.file_path.clone());
                            if self.is_playing_now(path.as_deref()) {
                                self.player.stop();
                            } else {
                                self.play_track_at_index(idx);
                            }
                        }
                        TrackAction::Select(idx) => {
                            self.pane_has_focus = false;
                            self.selected_track = Some(idx);
                        }
                        TrackAction::EditTags(idx) => {
                            if let Some(&orig) = self.filtered_indices.get(idx) {
                                if let Some(t) = self.tracks.get(orig) {
                                    self.active_dialog =
                                        ActiveDialog::EditTags { form: Box::new(TagForm::from_track(t)), track: t.clone() };
                                }
                            }
                        }
                        TrackAction::EditTagsMulti => {
                            let sel: std::collections::HashSet<PathBuf> =
                                ctx.data(|d| d.get_temp(crate::views::track_list::multi_sel_id())).unwrap_or_default();
                            let picked: Vec<TrackItem> = self
                                .filtered_indices
                                .iter()
                                .filter_map(|&i| self.tracks.get(i))
                                .filter(|t| sel.contains(&t.file_path))
                                .cloned()
                                .collect();
                            if !picked.is_empty() {
                                self.active_dialog =
                                    ActiveDialog::EditTagsMulti { form: Box::new(MultiTagForm::from_tracks(&picked)), tracks: picked };
                            }
                        }
                        TrackAction::EditLabels(idx) => {
                            if let Some(&orig) = self.filtered_indices.get(idx) {
                                if let Some(t) = self.tracks.get(orig) {
                                    let current_labels = self.label_service.get_labels_for_file(&t.file_path);
                                    self.active_dialog = ActiveDialog::EditLabels {
                                        track: t.clone(),
                                        current_labels,
                                        new_label_input: String::new(),
                                    };
                                }
                            }
                        }
                        TrackAction::ShowDetails(idx) => {
                            if let Some(&orig) = self.filtered_indices.get(idx) {
                                if let Some(t) = self.tracks.get(orig) {
                                    let mut track = t.clone();
                                    track.artwork_bytes = TrackItem::load_artwork(&track.file_path);
                                    self.active_dialog = ActiveDialog::TrackDetails { track };
                                }
                            }
                        }
                        TrackAction::OpenExplorer(idx) => {
                            if let Some(p) = self.filtered_indices.get(idx).and_then(|&i| self.tracks.get(i)).map(|t| t.file_path.clone()) {
                                self.reveal_in_tree(p);
                            }
                        }
                        TrackAction::Folder(a) => self.handle_fav_action(a),
                        TrackAction::ToggleFolder(p) => {
                            let key = crate::store::path_key(&p);
                            if !self.label_expanded.remove(&key) {
                                self.label_expanded.insert(key);
                            }
                            self.refresh_current_view();
                        }
                        TrackAction::AddFavorite(idx) => {
                            if let Some(t) = self.filtered_indices.get(idx).and_then(|&i| self.tracks.get(i)) {
                                let p = t.file_path.clone();
                                self.handle_fav_action(FavAction::AddPath(p));
                            }
                        }
                        TrackAction::CopyPath(idx) => {
                            if let Some(&orig) = self.filtered_indices.get(idx) {
                                if let Some(t) = self.tracks.get(orig) {
                                    ui.output_mut(|o| o.copied_text = t.file_path.to_string_lossy().to_string());
                                }
                            }
                        }
                        TrackAction::Delete(idx) => {
                            if let Some(&orig) = self.filtered_indices.get(idx) {
                                if let Some(t) = self.tracks.get(orig) {
                                    // 確認してからごみ箱へ移動する（その場では消さない）
                                    self.delete_confirm = Some(t.file_path.clone());
                                }
                            }
                        }
                    }
                }
            });

        // --- 4c. プレイリスト（別ウィンドウ） ---
        if self.show_playlist_window {
            let mut close = false;
            let mut action: Option<PlaylistWindowAction> = None;
            const PL_SIZE: [f32; 2] = [780.0, 500.0];
            let pos = *self.playlist_window_pos.get_or_insert_with(|| {
                let main = ctx.input(|i| i.viewport().outer_rect).unwrap_or(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1260.0, 800.0)));
                main.center() - egui::vec2(PL_SIZE[0], PL_SIZE[1]) / 2.0
            });
            let service = &self.playlist_service;
            let state = &mut self.playlist_window;
            let reveal = self.pl_win_frames >= REVEAL_AFTER;
            let frames = &mut self.pl_win_frames;
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of(("playlist_window", self.pl_win_gen)),
                egui::ViewportBuilder::default()
                    .with_visible(reveal)
                    .with_title("プレイリスト")
                    .with_inner_size(PL_SIZE)
                    .with_position(pos)
                    .with_min_inner_size([480.0, 300.0]),
                |ctx, _class| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::none().fill(crate::theme::c(15, 23, 42)).inner_margin(Margin::same(8.0)))
                        .show(ctx, |ui| {
                            action = render_playlist_window(ui, service, state);
                        });
                    reveal_when_painted(ctx, frames);
                    if ctx.input(|i| i.viewport().close_requested()) {
                        close = true;
                    }
                },
            );
            if close {
                self.show_playlist_window = false;
                self.playlist_window_pos = None;
                self.pl_win_gen += 1;
                self.pl_win_frames = 0;
            }
            match action {
                Some(PlaylistWindowAction::ShowInMain(id)) => self.open_playlist_view(id),
                Some(PlaylistWindowAction::Changed(id)) => {
                    // 表示中のプレイリストが編集・削除された場合は一覧を更新する
                    if self.active_selection == ViewSelection::Playlist(id) {
                        if self.playlist_service.get_all_playlists().iter().any(|p| p.id == id) {
                            self.refresh_current_view();
                        } else {
                            // 削除されたので、フォルダ表示に戻す
                            self.open_folder(self.current_folder.clone());
                        }
                    }
                    // 削除されたプレイリストの編集・並び替えが開いたままなら閉じる
                    if !self.playlist_service.get_all_playlists().iter().any(|p| p.id == id) {
                        if self.playlist_window.reorder.as_ref().is_some_and(|r| r.id == id) {
                            self.playlist_window.reorder = None;
                        }
                        if self.playlist_window.edit.as_ref().is_some_and(|e| e.id == id) {
                            self.playlist_window.edit = None;
                        }
                    }
                }
                Some(PlaylistWindowAction::Play { playlist_id, path }) => {
                    // 一覧の読み込みは裏で行われるので、右ペインに出して、そこから再生する
                    self.playlist_window.selected = Some(playlist_id);
                    self.pane_has_focus = true;
                    self.current_playing_index = None;
                    self.play_from_pane = true;
                    self.pl_pane.playing = None;
                    self.audition = false;
                    self.player.play(&path);
                }
                None => {}
            }
        }

        // --- 4c-1. 重複検出（ハッシュチェック）: 進捗・結果・聴き比べ（別ウィンドウ） ---
        if self.dup_win.is_some() {
            use crate::views::duplicate_window::{render_duplicate_window, DuplicateAction};
            let now_playing = if self.player.state() == PlaybackState::Playing { self.player.current_track_path() } else { None };
            let mut act: Option<DuplicateAction> = None;
            let mut close = false;
            if let Some(w) = self.dup_win.as_mut() {
                let mut frames = w.frames;
                let reveal = frames >= REVEAL_AFTER;
                let id = egui::ViewportId::from_hash_of(("duplicate_window", w.gen));
                ctx.show_viewport_immediate(
                    id,
                    egui::ViewportBuilder::default()
                        .with_visible(reveal)
                        .with_title("重複検出（ハッシュチェック）")
                        .with_inner_size([760.0, 560.0])
                        .with_min_inner_size([480.0, 300.0]),
                    |ctx, _class| {
                        egui::CentralPanel::default()
                            .frame(egui::Frame::none().fill(crate::theme::c(15, 23, 42)).inner_margin(Margin::same(10.0)))
                            .show(ctx, |ui| {
                                act = render_duplicate_window(ui, w, now_playing.as_ref());
                            });
                        reveal_when_painted(ctx, &mut frames);
                        if ctx.input(|i| i.viewport().close_requested()) {
                            close = true;
                        }
                    },
                );
                w.frames = frames;
            }
            match act {
                Some(DuplicateAction::Play(p)) => {
                    self.audition = true;
                    self.current_playing_index = None;
                    self.play_from_pane = false;
                    self.player.play(&p);
                }
                Some(DuplicateAction::Stop) => self.player.stop(),
                Some(DuplicateAction::Reveal(p)) => {
                    ctx.data_mut(|d| d.insert_temp(egui::Id::new("reveal_request"), p));
                }
                None => {}
            }
            if close {
                // 検査を止めて、聴き比べ中の再生も止める
                if self.audition {
                    self.player.stop();
                    self.audition = false;
                }
                self.dup_win = None; // 破棄すると検査も中止される
            }
        }

        // --- 4c-2. 曲の並び替え（別ウィンドウ） ---
        if self.playlist_window.reorder.is_some() {
            let mut outcome: Option<ReorderOutcome> = None;
            let state = &mut self.playlist_window;
            if let Some(rs) = state.reorder.as_mut() {
                let mut frames = rs.frames;
                let reveal = frames >= REVEAL_AFTER;
                ctx.show_viewport_immediate(
                    egui::ViewportId::from_hash_of(("playlist_reorder", rs.id, rs.gen)),
                    egui::ViewportBuilder::default()
                        .with_visible(reveal)
                        .with_title(format!("曲の並び替え - {}", rs.name))
                        .with_inner_size([560.0, 600.0])
                        .with_min_inner_size([360.0, 300.0]),
                    |ctx, _class| {
                        egui::CentralPanel::default()
                            .frame(egui::Frame::none().fill(crate::theme::c(15, 23, 42)).inner_margin(Margin::same(8.0)))
                            .show(ctx, |ui| {
                                outcome = render_reorder_window(ui, rs);
                            });
                        reveal_when_painted(ctx, &mut frames);
                        if ctx.input(|i| i.viewport().close_requested()) {
                            outcome = Some(ReorderOutcome::Cancel);
                        }
                    },
                );
                rs.frames = frames;
            }
            match outcome {
                Some(ReorderOutcome::Save) => {
                    if let Some(rs) = self.playlist_window.reorder.take() {
                        // 並び替え中に追加・削除された曲を失わないよう、いまの内容に合わせてから保存する
                        let now = self.playlist_service.get_playlist_tracks(rs.id);
                        let now_keys: HashSet<String> = now.iter().map(|p| crate::store::path_key(p)).collect();
                        let ordered_keys: HashSet<String> = rs.items.iter().map(|p| crate::store::path_key(p)).collect();
                        let mut items: Vec<PathBuf> =
                            rs.items.iter().filter(|p| now_keys.contains(&crate::store::path_key(p))).cloned().collect();
                        items.extend(now.into_iter().filter(|p| !ordered_keys.contains(&crate::store::path_key(p))));
                        if let Err(e) = self.playlist_service.set_playlist_tracks(rs.id, &items) {
                            self.playlist_window.message = Some(e);
                        } else if self.active_selection == ViewSelection::Playlist(rs.id) {
                            self.refresh_current_view();
                        }
                    }
                }
                Some(ReorderOutcome::Cancel) => self.playlist_window.reorder = None,
                None => {}
            }
        }

        // --- 4d. 「追加しました」通知（プレイリストボタンの下にスライドイン／スライドアウト） ---
        if let Some((msg, start)) = self.toast.clone() {
            const SLIDE: f64 = 0.28; // スライドの所要時間(秒)
            const TOTAL: f64 = 2.6; // 表示全体(秒)
            let t = ctx.input(|i| i.time) - start;
            if t >= TOTAL {
                self.toast = None;
            } else {
                let ease = |x: f64| (x * x * (3.0 - 2.0 * x)).clamp(0.0, 1.0);
                let p = if t < SLIDE {
                    ease(t / SLIDE)
                } else if t > TOTAL - SLIDE {
                    ease((TOTAL - t) / SLIDE)
                } else {
                    1.0
                } as f32;
                // 上から滑り出して(イン)、同じ向きに引っ込む(アウト)
                let pos = self.toast_anchor + egui::vec2(0.0, 4.0 - (1.0 - p) * 22.0);
                egui::Area::new(egui::Id::new("add_toast"))
                    .order(egui::Order::Tooltip)
                    .fixed_pos(pos)
                    .interactable(false)
                    .show(ctx, |ui| {
                        ui.set_opacity(p);
                        egui::Frame::none()
                            .fill(crate::theme::c(22, 101, 52))
                            .stroke(Stroke::new(1.0_f32, crate::theme::c(34, 197, 94)))
                            .rounding(6.0)
                            .inner_margin(Margin::symmetric(12.0, 6.0))
                            .show(ui, |ui| {
                                ui.label(RichText::new(format!("✔ {}", msg)).color(Color32::WHITE).size(13.0));
                            });
                    });
                ctx.request_repaint();
            }
        }

        // --- 4d-2. ミニプレイヤー（別ウィンドウ） ---
        if self.show_mini {
            let mini_id = egui::ViewportId::from_hash_of(("mini_player", self.mini_gen));
            let path = self.player.current_track_path();
            let track = path.as_ref().and_then(|p| {
                self.tracks
                    .iter()
                    .find(|t| &t.file_path == p)
                    .or_else(|| self.pl_pane.items.iter().find(|t| &t.file_path == p))
                    .cloned()
            });
            let art = path.as_ref().and_then(|p| self.media_cache.get(p)).and_then(|m| m.art);
            let (state, pos, total) = (self.player.state(), self.player.current_position(), self.player.total_duration());
            let topmost = self.settings.mini_player_topmost;

            let mut builder = egui::ViewportBuilder::default()
                .with_visible(self.mini_frames >= REVEAL_AFTER)
                .with_title("ミニプレイヤー")
                .with_inner_size([380.0, 150.0])
                .with_min_inner_size([300.0, 150.0])
                .with_max_inner_size([2000.0, 150.0])
                .with_taskbar(false);
            if topmost {
                builder = builder.with_always_on_top();
            }
            if !self.settings.mini_player_left.is_nan() && !self.settings.mini_player_top.is_nan() {
                builder = builder.with_position([self.settings.mini_player_left, self.settings.mini_player_top]);
            }

            let mut mini_action = None;
            let mut close = false;
            let mut last_pos = None;
            let frames = &mut self.mini_frames;
            ctx.show_viewport_immediate(mini_id, builder, |ctx, _| {
                reveal_when_painted(ctx, frames);
                egui::CentralPanel::default()
                    .frame(egui::Frame::none().fill(crate::theme::c(15, 23, 42)).inner_margin(Margin::symmetric(10.0, 8.0)))
                    .show(ctx, |ui| {
                        mini_action = crate::views::mini_player::render_mini_player(ui, track.as_ref(), art.as_ref(), state, pos, total, topmost);
                    });
                last_pos = ctx.input(|i| i.viewport().outer_rect).map(|r| r.min);
                if ctx.input(|i| i.viewport().close_requested()) {
                    close = true;
                }
            });
            if last_pos.is_some() {
                self.mini_pos = last_pos;
            }
            if let Some(a) = mini_action {
                use crate::views::mini_player::MiniAction;
                match a {
                    MiniAction::PlayPause => self.toggle_play_pause(),
                    MiniAction::Prev => self.play_prev(),
                    MiniAction::Next => self.play_next(),
                    MiniAction::Stop => self.player.stop(),
                    MiniAction::Seek(d) => self.player.seek(d),
                    MiniAction::Topmost(on) => {
                        self.settings.mini_player_topmost = on;
                        let level = if on { egui::viewport::WindowLevel::AlwaysOnTop } else { egui::viewport::WindowLevel::Normal };
                        ctx.send_viewport_cmd_to(mini_id, egui::ViewportCommand::WindowLevel(level));
                    }
                }
            }
            if close {
                self.show_mini = false;
                self.mini_gen += 1;
                self.mini_frames = 0;
                self.settings.save(&self.settings_path);
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }

        self.update_check_tick(ctx);

        // --- 4d-5. ラベル選択ポップアップ（複数選択） ---
        self.render_label_picker(ctx);

        // --- 4d-4. 保存先フォルダにアクセスできない場合の警告 ---
        if self.data_dir_warning {
            let mut ok = false;
            egui::Window::new("⚠ データ保存先にアクセスできません")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(format!(
                        "設定で指定されたデータ保存先:
{}
に接続できませんでした（ネットワーク切断など）。

ラベル・プレイリスト・設定は、保存先に接続できるまで読み込めません。
接続を確認してから、アプリを再起動してください。",
                        crate::paths::data_dir().display()
                    ));
                    if ui.button("OK").clicked() {
                        ok = true;
                    }
                });
            if ok {
                self.data_dir_warning = false;
            }
        }

        // --- 4d-3. お気に入りの表示名の変更 ---
        if let Some((idx, mut input)) = self.fav_rename.clone() {
            let mut close = false;
            egui::Window::new("表示名の変更")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("お気に入りの表示名を入力してください:");
                    let edit = ui.add(egui::TextEdit::singleline(&mut input).desired_width(280.0));
                    let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.horizontal(|ui| {
                        if (ui.button("OK").clicked() || enter) && !input.trim().is_empty() {
                            if let Some(fav) = self.settings.favorites.get_mut(idx) {
                                fav.0 = input.trim().to_string();
                            }
                            self.settings.save(&self.settings_path);
                            close = true;
                        }
                        if ui.button("キャンセル").clicked() {
                            close = true;
                        }
                    });
                });
            self.fav_rename = if close { None } else { Some((idx, input)) };
        }

        // --- 4d-7. ファイル削除の確認（ごみ箱へ移動） ---
        if let Some(path) = self.delete_confirm.clone() {
            let mut close = false;
            let mut go = false;
            egui::Window::new("ファイルの削除")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("次のファイルをごみ箱へ移動します。");
                    ui.label(RichText::new(path.to_string_lossy()).strong());
                    ui.label(RichText::new("ごみ箱から元に戻せます。").weak());
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("🗑 ごみ箱へ移動").color(crate::theme::c(248, 113, 113))).clicked() {
                            go = true;
                            close = true;
                        }
                        if ui.button("キャンセル").clicked() {
                            close = true;
                        }
                    });
                });
            if go {
                if self.is_playing_now(Some(&path)) {
                    self.player.stop(); // 再生中のファイルは使用中で削除できない
                }
                let now = ctx.input(|i| i.time);
                match crate::trash::move_to_trash(&path) {
                    Ok(()) => {
                        self.toast = Some(("ごみ箱へ移動しました".to_string(), now));
                        self.refresh_current_view();
                    }
                    Err(e) => self.toast = Some((format!("削除できませんでした: {}", e), now)),
                }
            }
            if close {
                self.delete_confirm = None;
            }
        }

        // --- 4d-6. ラベルの削除確認 / 追加 ---
        if let Some(name) = self.label_delete.clone() {
            let used = self.label_service.get_labels_with_usage().into_iter().find(|(n, _)| *n == name).map_or(0, |(_, c)| c);
            let mut close = false;
            let mut delete = false;
            egui::Window::new("ラベルの削除")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(format!("ラベル「{}」を削除します。", name));
                    ui.label(RichText::new(if used > 0 { format!("このラベルが付いている {} 件からも外れます。", used) } else { "付いている曲はありません。".to_string() }).weak());
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("🗑 削除").color(crate::theme::c(248, 113, 113))).clicked() {
                            delete = true;
                            close = true;
                        }
                        if ui.button("キャンセル").clicked() {
                            close = true;
                        }
                    });
                });
            if delete {
                self.label_service.delete_preset_label(&name);
                if let ViewSelection::Labels(sel) = self.active_selection.clone() {
                    let rest: Vec<String> = sel.into_iter().filter(|l| *l != name).collect();
                    self.open_labels_view(rest);
                } else {
                    self.refresh_current_view();
                }
                self.reload_playlist_pane(); // 右ペインのラベル表示も更新させる
                self.toast = Some((format!("ラベル「{}」を削除しました", name), ctx.input(|i| i.time)));
            }
            if close {
                self.label_delete = None;
            }
        }
        if let Some(mut input) = self.label_add.clone() {
            let mut close = false;
            egui::Window::new("ラベルを追加")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("追加するラベルの名前:");
                    let edit = ui.add(egui::TextEdit::singleline(&mut input).desired_width(240.0));
                    // 開いた最初の1回だけ入力欄にフォーカスを当てる（毎回当てると Enter で確定できなくなる）
                    let focus_id = egui::Id::new("label_add_focus");
                    if !ui.data(|d| d.get_temp::<bool>(focus_id)).unwrap_or(false) {
                        edit.request_focus();
                        ui.data_mut(|d| d.insert_temp(focus_id, true));
                    }
                    let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.horizontal(|ui| {
                        if (ui.button("追加").clicked() || enter) && !input.trim().is_empty() {
                            self.label_service.add_preset_label(&input);
                            close = true;
                        }
                        if ui.button("キャンセル").clicked() {
                            close = true;
                        }
                    });
                });
            if close {
                ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::new("label_add_focus")));
            }
            self.label_add = if close { None } else { Some(input) };
        }

        // --- 4e. 設定ウィンドウ ---
        self.render_settings_window(ctx);

        // --- 4a. ヘルプ ---
        if self.show_help {
            let mut open = true;
            egui::Window::new("ヘルプ")
                .open(&mut open)
                .collapsible(false)
                .default_width(620.0)
                .default_height(520.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, render_help);
                });
            self.show_help = open;
        }

        // --- 4b. 表示項目・並び順の設定ウィンドウ ---
        if self.show_column_settings {
            let columns_before = (self.settings.columns.clone(), self.settings.playlist_columns.clone());
            let mut open = true;
            let mut reset = false;
            let mut move_req: Option<(usize, usize)> = None;
            egui::Window::new("表示項目・並び順の設定")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    // ドラッグ中に文字が選択されたり、カーソルが入力用に変わったりしないようにする
                    ui.style_mut().interaction.selectable_labels = false;
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.column_target_pane, false, "メイン一覧");
                        ui.selectable_value(&mut self.column_target_pane, true, "プレイリストペイン");
                    });
                    ui.label("チェックで表示/非表示、左の ☰ をドラッグして並び替えます（変更は即時反映・自動保存）");
                    ui.add_space(6.0);
                    let cfg = if self.column_target_pane { &mut self.settings.playlist_columns } else { &mut self.settings.columns };
                    let drag_id = egui::Id::new("column_settings_dnd");
                    let mut dragging: Option<usize> = ui.data(|d| d.get_temp(drag_id));
                    let mut rects: Vec<egui::Rect> = Vec::with_capacity(cfg.items.len());
                    for (i, (col, visible)) in cfg.items.iter_mut().enumerate() {
                        let row = ui.horizontal(|ui| {
                            let grip = ui
                                .add(egui::Label::new(egui::RichText::new("☰").weak()).selectable(false).sense(egui::Sense::drag()))
                                .on_hover_cursor(egui::CursorIcon::PointingHand);
                            if grip.drag_started() {
                                dragging = Some(i);
                                ui.data_mut(|d| d.insert_temp(drag_id, i));
                            }
                            ui.checkbox(visible, col.label());
                        });
                        rects.push(row.response.rect);
                    }
                    if let (Some(from), Some(pointer)) = (dragging, ui.input(|i| i.pointer.interact_pos())) {
                        if let (Some(first), Some(last)) = (rects.first(), rects.last()) {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            // 挿入位置: ポインタより下に中心がある最初の行の手前
                            let target = rects.iter().position(|r| pointer.y < r.center().y).unwrap_or(rects.len());
                            let y = if target < rects.len() { rects[target].top() } else { last.bottom() };
                            ui.painter().hline(first.x_range(), y, egui::Stroke::new(2.0_f32, crate::theme::c(56, 189, 248)));
                            if ui.input(|i| i.pointer.any_released()) {
                                let to = if target > from { target - 1 } else { target };
                                if to != from {
                                    move_req = Some((from, to));
                                }
                                ui.data_mut(|d| d.remove::<usize>(drag_id));
                            }
                        }
                    } else if dragging.is_some() {
                        ui.data_mut(|d| d.remove::<usize>(drag_id));
                    }
                    ui.add_space(6.0);
                    if ui.button("初期状態に戻す").clicked() {
                        reset = true;
                    }
                });
            if let Some((from, to)) = move_req {
                let cfg = if self.column_target_pane { &mut self.settings.playlist_columns } else { &mut self.settings.columns };
                if from < cfg.items.len() && to < cfg.items.len() {
                    let item = cfg.items.remove(from);
                    cfg.items.insert(to, item);
                }
            }
            if reset {
                if self.column_target_pane {
                    self.settings.playlist_columns = crate::views::columns::ColumnConfig::playlist_default();
                } else {
                    self.settings.columns = Default::default();
                }
            }
            self.show_column_settings = open;
            // 表示列の変更は、その場で保存する（終了前にアプリが落ちても失われない）
            if columns_before != (self.settings.columns.clone(), self.settings.playlist_columns.clone()) {
                self.settings.save(&self.settings_path);
            }
        }

        // --- 5. モーダルダイアログの描画 ---
        let mut refresh_needed = false;
        let mut locate_path: Option<PathBuf> = None;

        render_dialogs(
            ctx,
            &mut self.active_dialog,
            &self.label_service,
            &mut || refresh_needed = true,
            &mut |p| locate_path = Some(p),
        );

        if refresh_needed {
            self.refresh_current_view();
            self.reload_playlist_pane(); // タグを書き換えた曲が右ペインにあれば、そちらも読み直す
        }
        if let Some(p) = locate_path {
            if let Some(parent) = p.parent() {
                self.open_folder(parent.to_path_buf());
            }
        }
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        if self.restored_backup {
            return; // 復元したファイルを上書きしない
        }
        self.settings.volume = self.player.volume;
        self.capture_window_state();
        self.settings.save(&self.settings_path);
        // 定期の自動保存は別スレッドで行う（ディスクが遅くても画面の操作を止めない）。
        // ラベル・プレイリストは変更のたびに保存済みなので、ここでは書かない（終了時に確定する）。
        // キャッシュは何も捨てない（異常終了しても前回までの内容が残る）
        let (tags, media) = (self.tag_cache.clone(), self.media_disk_cache.clone());
        std::thread::spawn(move || {
            tags.flush(false);
            media.flush(false);
        });
    }

    /// 終了時: ラベル等を確定し、キャッシュはこの起動で使ったデータだけを残して保存する
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.restored_backup {
            return;
        }
        self.settings.volume = self.player.volume;
        self.capture_window_state();
        self.settings.save(&self.settings_path);
        self.store.flush();
        self.tag_cache.flush(true);
        self.media_disk_cache.flush(true);
    }
}

fn rand_simple(max: usize) -> usize {
    use std::time::SystemTime;
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as usize;
    nanos % max
}

/// フォルダ表示用の行: サブフォルダ(名前順)
fn list_folder_entries(folder: &Path) -> Vec<(String, PathBuf)> {
    // 「..」(親フォルダ) の行は、空行に見えるため出さない（親フォルダへはツリー・パス欄から移動できる）
    let mut entries = Vec::new();

    let mut subdirs: Vec<(String, PathBuf)> = std::fs::read_dir(folder)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.starts_with('$') || name.starts_with('.') || name == "System Volume Information" {
                        None
                    } else {
                        Some((name, e.path()))
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    subdirs.sort_by_key(|(n, _)| n.to_lowercase());
    entries.extend(subdirs);
    entries
}

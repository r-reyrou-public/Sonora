use crate::db::labels::CustomTagService;
use crate::playlist::PlaylistService;
use egui::{CollapsingHeader, RichText, ScrollArea, Sense, Ui};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeftPanelTab {
    Explorer,
    Favorites,
    Labels,
}

/// お気に入りに登録済みのパス（path_key）を一覧・ツリーへ渡す場所（egui のメモリ）
pub fn fav_keys_id() -> egui::Id {
    egui::Id::new("favorite_path_keys")
}

/// お気に入りに登録されているか（登録済みなら名前の頭に ⭐ を付ける）
pub fn is_favorite(ctx: &egui::Context, path: &Path) -> bool {
    let key = crate::store::path_key(path);
    ctx.data(|d| d.get_temp::<std::sync::Arc<std::collections::HashSet<String>>>(fav_keys_id()))
        .is_some_and(|set| set.contains(&key))
}

/// お気に入りタブの操作
#[derive(Clone, Debug)]
pub enum FavAction {
    /// フォルダ選択ダイアログから追加
    AddFolder,
    /// 表示中のフォルダを追加
    AddCurrent,
    /// ファイル選択ダイアログから曲を追加
    AddFile,
    /// 曲のパスを追加
    AddPath(PathBuf),
    /// お気に入りの曲を開く（true なら再生も行う）
    OpenFile(PathBuf, bool),
    Remove(usize),
    Rename(usize),
    /// フォルダのお気に入り登録 / 解除
    Toggle(PathBuf),
    /// フォルダのラベル選択を開く（配下の全ファイルにも付与）
    Labels(PathBuf, egui::Pos2),
    /// フォルダと配下の全ファイルから、そのフォルダのラベルを全て外す
    ClearLabels(PathBuf),
    /// アプリのツリーで開く（曲は選択状態で、フォルダは中身を表示）
    Explorer(PathBuf),
    /// ラベルの削除（確認のあと、付いている全ての曲から外す）
    DeleteLabel(String),
    /// ラベルを新しく追加する（名前の入力を出す）
    AddLabel,
}

/// フォルダの右クリックメニュー（ツリー・一覧で共通）
pub fn folder_menu(ui: &mut Ui, path: &Path, on_fav_action: &mut impl FnMut(FavAction)) {
    let fav = is_favorite(ui.ctx(), path);
    if ui.button(if fav { "⭐ お気に入りから外す" } else { "⭐ お気に入りに追加" }).clicked() {
        on_fav_action(FavAction::Toggle(path.to_path_buf()));
        ui.close_menu();
    }
    let pos = ui.min_rect().left_top();
    if ui.button("🏷 ラベルを付ける（配下の全ファイルにも）").clicked() {
        on_fav_action(FavAction::Labels(path.to_path_buf(), pos));
        ui.close_menu();
    }
    if ui.button("🗑 ラベルを一括で外す").clicked() {
        on_fav_action(FavAction::ClearLabels(path.to_path_buf()));
        ui.close_menu();
    }
    ui.separator();
    if ui.button("📂 ツリーで開く").clicked() {
        on_fav_action(FavAction::Explorer(path.to_path_buf()));
        ui.close_menu();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewSelection {
    Folder(PathBuf),
    /// 選択中のラベル（複数選択）。空のときはフォルダ表示に戻る
    Labels(Vec<String>),
    Playlist(i64),
}

pub fn render_folder_tree(
    ui: &mut Ui,
    favorites: &[(String, String)],
    current_folder: &Path,
    active_selection: &ViewSelection,
    active_tab: &mut LeftPanelTab,
    label_match_all: &mut bool,
    reveal_current: bool,
    label_service: &CustomTagService,
    playlist_service: &PlaylistService,
    on_select_view: &mut impl FnMut(ViewSelection),
    on_create_playlist: &mut impl FnMut(String),
    on_fav_action: &mut impl FnMut(FavAction),
) {
    ui.vertical(|ui| {
        // --- 1. 下部タブ切り替えバーの高さを除いたメインコンテンツ領域 ---
        let content_height = (ui.available_height() - 34.0).max(100.0);

        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), content_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                // 深い階層で内容が横にはみ出してもパネル幅を押し広げないよう、横スクロールも有効にする
                ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;

                        match active_tab {
                            LeftPanelTab::Explorer => {
                                // ドライブ ＆ フォルダツリー
                                ui.label(RichText::new("📁 PC / ドライブ").strong().size(13.0));
                                ui.add_space(2.0);

                                for drive_path in list_drives() {
                                    {
                                        render_dir_node(ui, &drive_path, current_folder, active_selection, on_select_view, on_fav_action, 0, reveal_current);
                                    }
                                }

                                ui.add_space(8.0);
                                ui.separator();
                                ui.add_space(4.0);

                                // プレイリスト
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("📑 プレイリスト").strong().size(13.0));
                                    if ui.small_button("➕ 新規").on_hover_text("新しいラベルを作ります").clicked() {
                                        let default_name = format!("プレイリスト {}", chrono_simple_now());
                                        on_create_playlist(default_name);
                                    }
                                });
                                ui.add_space(2.0);

                                let playlists = playlist_service.get_all_playlists();
                                if playlists.is_empty() {
                                    ui.label(RichText::new("（プレイリストなし）").weak().size(11.0));
                                } else {
                                    for p in playlists {
                                        let is_selected = *active_selection == ViewSelection::Playlist(p.id);
                                        let text = RichText::new(format!("📑 {} ({})", p.name, p.count))
                                            .size(12.0)
                                            .color(if is_selected {
                                                crate::theme::c(52, 211, 153)
                                            } else {
                                                crate::theme::c(226, 232, 240)
                                            });

                                        if ui.selectable_label(is_selected, text).clicked() {
                                            on_select_view(ViewSelection::Playlist(p.id));
                                        }
                                    }
                                }
                            }

                            LeftPanelTab::Favorites => {
                                ui.label(RichText::new("⭐ お気に入り").strong().size(13.0));
                                ui.horizontal_wrapped(|ui| {
                                    if ui.small_button("📁 フォルダ追加").on_hover_text("フォルダを選んでお気に入りに追加").clicked() {
                                        on_fav_action(FavAction::AddFolder);
                                    }
                                    if ui.small_button("🎵 ファイル追加").on_hover_text("曲のファイルを選んでお気に入りに追加").clicked() {
                                        on_fav_action(FavAction::AddFile);
                                    }
                                    if ui.small_button("📂 表示中のフォルダを追加").on_hover_text("一覧に表示しているフォルダを、お気に入りに追加します").clicked() {
                                        on_fav_action(FavAction::AddCurrent);
                                    }
                                });
                                ui.add_space(4.0);

                                if favorites.is_empty() {
                                    ui.label(RichText::new("（お気に入りがありません）").weak().size(11.0));
                                } else {
                                    for (fav_index, (name, path_str)) in favorites.iter().enumerate() {
                                        let path = PathBuf::from(path_str);
                                        // 毎フレーム通信やディスクに触れないよう、拡張子で曲ファイルかを判断する
                                        let is_file = crate::scanner::metadata::is_audio_ext(&path);
                                        let is_selected = !is_file && *active_selection == ViewSelection::Folder(path.clone());
                                        let label = RichText::new(format!("{} {}", if is_file { "🎵" } else { "⭐" }, name))
                                            .size(12.0)
                                            .color(if is_selected {
                                                crate::theme::c(96, 165, 250)
                                            } else {
                                                crate::theme::c(226, 232, 240)
                                            });

                                        let resp = ui.selectable_label(is_selected, label).on_hover_text(path_str.as_str());
                                        resp.context_menu(|ui| {
                                            if ui.button("✏ 表示名の変更").clicked() {
                                                on_fav_action(FavAction::Rename(fav_index));
                                                ui.close_menu();
                                            }
                                            if ui.button("🗑 お気に入りから削除").clicked() {
                                                on_fav_action(FavAction::Remove(fav_index));
                                                ui.close_menu();
                                            }
                                            ui.separator();
                                            let pos = ui.min_rect().left_top();
                                            let note = if is_file { "🏷 ラベルを付ける" } else { "🏷 ラベルを付ける（配下の全ファイルにも）" };
                                            if ui.button(note).clicked() {
                                                on_fav_action(FavAction::Labels(path.clone(), pos));
                                                ui.close_menu();
                                            }
                                            if !is_file && ui.button("🗑 ラベルを一括で外す").clicked() {
                                                on_fav_action(FavAction::ClearLabels(path.clone()));
                                                ui.close_menu();
                                            }
                                            ui.separator();
                                            if ui.button("📂 ツリーで開く").clicked() {
                                                on_fav_action(FavAction::Explorer(path.clone()));
                                                ui.close_menu();
                                            }
                                        });
                                        if is_file {
                                            if resp.double_clicked() {
                                                on_fav_action(FavAction::OpenFile(path, true));
                                            } else if resp.clicked() {
                                                on_fav_action(FavAction::OpenFile(path, false));
                                            }
                                        } else if resp.clicked() {
                                            on_select_view(ViewSelection::Folder(path));
                                        }
                                    }
                                }
                            }

                            LeftPanelTab::Labels => {
                                ui.label(RichText::new("🏷 ラベル").strong().size(13.0));
                                ui.label(RichText::new("クリックで複数選択 / 選択中を再クリックで解除").weak().size(10.0));
                                ui.horizontal(|ui| {
                                    ui.selectable_value(label_match_all, true, "すべて含む");
                                    ui.selectable_value(label_match_all, false, "いずれか");
                                });
                                ui.add_space(4.0);
                                let selected_labels: Vec<String> = match active_selection {
                                    ViewSelection::Labels(v) => v.clone(),
                                    _ => Vec::new(),
                                };

                                // 何もない所を右クリックしたら、ラベルの追加を出す（個々のラベルは後から重ねるので、そちらが優先される）
                                let bg = ui.interact(ui.clip_rect(), ui.id().with("labels_bg"), Sense::click());
                                bg.context_menu(|ui| {
                                    if ui.button("➕ ラベルを追加").clicked() {
                                        on_fav_action(FavAction::AddLabel);
                                        ui.close_menu();
                                    }
                                });

                                let labels = label_service.get_all_preset_labels();
                                if labels.is_empty() {
                                    ui.label(RichText::new("（ラベル未登録）").weak().size(11.0));
                                } else {
                                    for l in labels {
                                        let is_selected = selected_labels.contains(&l);
                                        let label_text = RichText::new(format!("🏷 {}", l))
                                            .size(12.0)
                                            .color(if is_selected {
                                                crate::theme::c(251, 146, 60)
                                            } else {
                                                crate::theme::c(226, 232, 240)
                                            });

                                        let lresp = ui.selectable_label(is_selected, label_text);
                                        lresp.context_menu(|ui| {
                                            if ui.button("🗑 このラベルを削除").clicked() {
                                                on_fav_action(FavAction::DeleteLabel(l.clone()));
                                                ui.close_menu();
                                            }
                                            if ui.button("➕ ラベルを追加").clicked() {
                                                on_fav_action(FavAction::AddLabel);
                                                ui.close_menu();
                                            }
                                        });
                                        if lresp.clicked() {
                                            // 選択中なら除外、未選択なら追加
                                            let mut next = selected_labels.clone();
                                            if is_selected {
                                                next.retain(|x| x != &l);
                                            } else {
                                                next.push(l);
                                            }
                                            on_select_view(ViewSelection::Labels(next));
                                        }
                                    }
                                }
                            }
                        }
                    });
            },
        );

        ui.separator();

        // --- 2. 下部タブ切り替えバー ---
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let total_w = ui.available_width();            let tab_w = ((total_w - 8.0) / 3.0).max(60.0);

            let is_exp = *active_tab == LeftPanelTab::Explorer;
            if ui.add_sized([tab_w, 24.0], egui::SelectableLabel::new(is_exp, RichText::new("📁 ツリー").size(11.0))).clicked() {
                *active_tab = LeftPanelTab::Explorer;
            }

            let is_fav = *active_tab == LeftPanelTab::Favorites;
            if ui.add_sized([tab_w, 24.0], egui::SelectableLabel::new(is_fav, RichText::new("⭐ お気に入り").size(11.0))).clicked() {
                *active_tab = LeftPanelTab::Favorites;
            }

            let is_lbl = *active_tab == LeftPanelTab::Labels;
            if ui.add_sized([tab_w, 24.0], egui::SelectableLabel::new(is_lbl, RichText::new("🏷 ラベル").size(11.0))).clicked() {
                *active_tab = LeftPanelTab::Labels;
            }
        });
    });
}

fn render_dir_node(
    ui: &mut Ui,
    dir: &Path,
    current_folder: &Path,
    active_selection: &ViewSelection,
    on_select_view: &mut impl FnMut(ViewSelection),
    on_fav_action: &mut impl FnMut(FavAction),
    depth: usize,
    reveal_current: bool,
) {
    if depth > 8 {
        return;
    }

    let dir_name = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| dir.to_string_lossy().to_string());

    if depth > 0 && (dir_name.starts_with('$') || dir_name.starts_with('.')) {
        return;
    }

    let is_selected = *active_selection == ViewSelection::Folder(dir.to_path_buf());
    let is_parent_of_current = current_folder.starts_with(dir);

    let header_id = ui.make_persistent_id(dir.to_string_lossy().to_string());

    let is_current = current_folder == dir;
    // 選択中のフォルダは、名前の右隣に緑の「✔ 選択中」を並べる
    let font = egui::FontId::proportional(12.0);
    let mut title = egui::text::LayoutJob::default();
    title.append(
        &format!("{}📁 {}", if is_favorite(ui.ctx(), dir) { "⭐" } else { "" }, dir_name),
        0.0,
        egui::TextFormat {
            font_id: font.clone(),
            color: if is_selected { crate::theme::c(96, 165, 250) } else { crate::theme::c(226, 232, 240) },
            ..Default::default()
        },
    );
    if is_selected {
        title.append(
            "  ✔ 選択中",
            0.0,
            egui::TextFormat {
                font_id: egui::FontId::proportional(11.0),
                color: crate::theme::c(34, 197, 94),
                ..Default::default()
            },
        );
    }
    let mut header = CollapsingHeader::new(title)
    .id_salt(header_id)
    .default_open(is_parent_of_current);

    // 起動時などは、保存されている開閉状態に関わらず現在のフォルダまで展開する
    if reveal_current && is_parent_of_current && !is_current {
        header = header.open(Some(true));
    }

    let resp = header.show(ui, |ui| {
        if !is_selected {
            ui.horizontal(|ui| {
                if ui.small_button(RichText::new("📂 このフォルダを開く").size(11.0)).on_hover_text("このフォルダを開いて、曲を一覧に表示します").clicked() {
                    on_select_view(ViewSelection::Folder(dir.to_path_buf()));
                }
            });
        }

        for sub in cached_subdirs(ui.ctx(), dir).iter() {
            render_dir_node(ui, sub, current_folder, active_selection, on_select_view, on_fav_action, depth + 1, reveal_current);
        }
    });

    resp.header_response.context_menu(|ui| folder_menu(ui, dir, on_fav_action));

    if reveal_current && is_current {
        resp.header_response.scroll_to_me(Some(egui::Align::Center));
    }
}

/// サブフォルダ一覧。再描画のたびに read_dir しないよう 3 秒キャッシュする
fn cached_subdirs(ctx: &egui::Context, dir: &Path) -> std::sync::Arc<Vec<PathBuf>> {
    let id = egui::Id::new(("subdirs_cache", dir));
    let now = ctx.input(|i| i.time);
    if let Some((t, list)) = ctx.data(|d| d.get_temp::<(f64, std::sync::Arc<Vec<PathBuf>>)>(id)) {
        if now - t < 3.0 {
            return list;
        }
    }

    let mut subdirs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let name = entry.file_name().to_string_lossy().to_string();
                if !name.starts_with('$') && !name.starts_with('.') {
                    subdirs.push(entry.path());
                }
            }
        }
    }
    subdirs.sort();
    let list = std::sync::Arc::new(subdirs);
    ctx.data_mut(|d| d.insert_temp(id, (now, list.clone())));
    list
}

/// 接続されている全ドライブ (A:〜Z:)。固定リストだと N: などが漏れるため OS に問い合わせる
#[cfg(windows)]
fn list_drives() -> Vec<PathBuf> {
    let mask = unsafe { windows_sys::Win32::Storage::FileSystem::GetLogicalDrives() };
    (0..26u32)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| PathBuf::from(format!("{}:\\", (b'A' + i as u8) as char)))
        .filter(|p| p.exists()) // 空の光学ドライブなどは除外
        .collect()
}

#[cfg(not(windows))]
fn list_drives() -> Vec<PathBuf> {
    vec![PathBuf::from("/")]
}

fn chrono_simple_now() -> String {
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{}", (now % 10000))
}

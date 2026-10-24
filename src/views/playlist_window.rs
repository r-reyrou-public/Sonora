use crate::playlist::PlaylistService;
use egui::{Color32, Id, Pos2, Rect, RichText, ScrollArea, Sense, Stroke, Ui};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// (プレイリストID, データの版)
type CacheKey = (i64, u64);

#[derive(Default)]
pub struct PlaylistWindowState {
    pub selected: Option<i64>,
    pub new_name: String,
    pub message: Option<String>,
    /// テキスト編集中のプレイリスト
    pub edit: Option<EditState>,
    /// 削除確認待ちのプレイリスト (id, 名前)
    pub confirm_delete: Option<(i64, String)>,
    /// 曲の並び替え（別ウィンドウ）
    pub reorder: Option<ReorderState>,
    /// 毎フレーム全曲を複製・確認しないための控え
    tracks_cache: Option<(CacheKey, Arc<Vec<PathBuf>>)>,
    /// ファイルが見つからない曲（バックグラウンドで調べる。画面の処理でディスクに触らない）
    missing: Arc<Mutex<Arc<HashSet<PathBuf>>>>,
    missing_key: Option<CacheKey>,
    missing_at: Option<Instant>,
    missing_busy: Arc<AtomicBool>,
}

impl PlaylistWindowState {
    /// プレイリストの曲一覧。内容が変わっていなければ、前回の複製をそのまま使う
    fn tracks(&mut self, service: &PlaylistService, id: i64) -> Arc<Vec<PathBuf>> {
        let key = (id, service.version());
        if let Some((k, t)) = &self.tracks_cache {
            if *k == key {
                return t.clone();
            }
        }
        let t = Arc::new(service.get_playlist_tracks(id));
        self.tracks_cache = Some((key, t.clone()));
        t
    }

    /// 見つからない曲の一覧。曲が変わったときと、10秒ごとに、別スレッドで調べ直す（外付けドライブの接続変化に追従）
    fn missing_set(&mut self, ctx: &egui::Context, id: i64, version: u64, tracks: &Arc<Vec<PathBuf>>) -> Arc<HashSet<PathBuf>> {
        let key = (id, version);
        let stale = self.missing_at.is_none_or(|t| t.elapsed().as_secs() >= 10);
        if (self.missing_key != Some(key) || stale) && !self.missing_busy.load(Ordering::SeqCst) {
            self.missing_key = Some(key);
            self.missing_at = Some(Instant::now());
            self.missing_busy.store(true, Ordering::SeqCst);
            let (out, busy, tracks, ctx) = (self.missing.clone(), self.missing_busy.clone(), tracks.clone(), ctx.clone());
            std::thread::spawn(move || {
                let set: HashSet<PathBuf> = tracks.iter().filter(|p| !p.exists()).cloned().collect();
                *out.lock().unwrap_or_else(|e| e.into_inner()) = Arc::new(set);
                busy.store(false, Ordering::SeqCst);
                ctx.request_repaint();
            });
        }
        self.missing.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

pub struct EditState {
    pub id: i64,
    pub name: String,
    /// 1行1ファイルパス
    pub text: String,
}

pub struct ReorderState {
    pub id: i64,
    pub name: String,
    pub items: Vec<PathBuf>,
    /// 開くたびに変える ID と、表示してからの描画回数（app 側が使う）
    pub gen: u64,
    pub frames: u32,
}

fn next_window_gen() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub enum PlaylistWindowAction {
    /// 指定プレイリストの指定ファイルを再生
    Play { playlist_id: i64, path: PathBuf },
    /// メイン画面の一覧にプレイリストを表示
    ShowInMain(i64),
    /// 内容が変わった（メイン画面が表示中なら更新が必要）
    Changed(i64),
}

pub enum ReorderOutcome {
    Save,
    Cancel,
}

/// ドラッグ＆ドロップで並び替えできる縦リスト。
/// `draw_row` で各行の見た目を描き、`on_row` で各行の Response（クリック・右クリック用）を受け取る。
/// ドロップされたら (移動元, 移動後の位置) を返す。
fn reorder_list(
    ui: &mut Ui,
    id: Id,
    count: usize,
    mut draw_row: impl FnMut(&mut Ui, usize, bool),
    mut on_row: impl FnMut(usize, &egui::Response),
) -> Option<(usize, usize)> {
    let mut dragging: Option<usize> = ui.data(|d| d.get_temp(id));
    let mut rects: Vec<Rect> = Vec::with_capacity(count);

    for i in 0..count {
        let inner = ui.scope(|ui| {
            ui.style_mut().interaction.selectable_labels = false;
            draw_row(ui, i, dragging == Some(i));
        });
        // 行の幅いっぱいを当たり判定にする
        let r = inner.response.rect;
        let rect = Rect::from_min_max(r.min, Pos2::new(ui.max_rect().right().max(r.max.x), r.max.y));
        let resp = ui.interact(rect, id.with(("row", i)), Sense::click_and_drag());
        if resp.hovered() && dragging.is_none() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.drag_started() {
            dragging = Some(i);
            ui.data_mut(|d| d.insert_temp(id, i));
        }
        on_row(i, &resp);
        rects.push(rect);
    }

    let mut result = None;
    if let (Some(from), Some(pointer)) = (dragging, ui.input(|i| i.pointer.interact_pos())) {
        if !rects.is_empty() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            // 挿入位置: ポインタより下に中心がある最初の行の手前
            let target = rects.iter().position(|r| pointer.y < r.center().y).unwrap_or(count);
            let y = if target < count { rects[target].top() } else { rects[count - 1].bottom() };
            let x = rects[0].x_range();
            ui.painter().hline(x, y, Stroke::new(2.0_f32, crate::theme::c(56, 189, 248)));

            if ui.input(|i| i.pointer.any_released()) {
                let to = if target > from { target - 1 } else { target };
                if to != from {
                    result = Some((from, to));
                }
                ui.data_mut(|d| d.remove::<usize>(id));
            }
        }
    } else if dragging.is_some() {
        ui.data_mut(|d| d.remove::<usize>(id));
    }
    result
}

/// 別ウィンドウに表示するプレイリスト管理画面
pub fn render_playlist_window(
    ui: &mut Ui,
    service: &PlaylistService,
    state: &mut PlaylistWindowState,
) -> Option<PlaylistWindowAction> {
    let mut action = None;
    let playlists = service.get_all_playlists();

    // 選択中のプレイリストが削除されていたら選択なしに戻す
    if state.selected.is_some_and(|id| !playlists.iter().any(|p| p.id == id)) {
        state.selected = None;
    }

    // 右クリックメニューから選ばれた操作 (対象ID, 操作)
    let mut menu_action: Option<(i64, ListMenu)> = None;

    egui::SidePanel::left("playlist_list")
        .default_width(260.0)
        .width_range(180.0..=420.0)
        .show_inside(ui, |ui| {
            ui.label(RichText::new("📑 プレイリスト").strong().size(14.0));
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut state.new_name)
                        .lock_focus(crate::views::ime::ime_composing(ui.ctx()))
                        .hint_text("新しいプレイリスト名")
                        .desired_width(ui.available_width() - 56.0),
                );
                // Enter キーでも作成できる（入力欄でフォーカスを失った瞬間に Enter が押されていた場合）
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (ui.button("➕ 作成").on_hover_text("入力した名前で、新しいプレイリストを作ります").clicked() || enter) && !state.new_name.trim().is_empty() {
                    match service.create_playlist(&state.new_name) {
                        Ok(id) => {
                            state.selected = Some(id);
                            state.new_name.clear();
                            state.message = None;
                            if enter {
                                edit.request_focus(); // 続けて入力できるよう入力欄にフォーカスを戻す
                            }
                        }
                        Err(e) => state.message = Some(e),
                    }
                }
            });
            if let Some(m) = &state.message {
                ui.label(RichText::new(m).color(crate::theme::c(248, 113, 113)).size(11.0));
            }
            ui.label(RichText::new("ドラッグで並び替え / 右クリックでメニュー").weak().size(10.0));
            ui.separator();

            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                if playlists.is_empty() {
                    ui.label(RichText::new("（プレイリストがありません）").weak());
                    return;
                }
                let selected = state.selected;
                let mut new_selected = None;

                let moved = reorder_list(
                    ui,
                    Id::new("playlist_list_dnd"),
                    playlists.len(),
                    |ui, i, is_dragged| {
                        let p = &playlists[i];
                        let fill = if selected == Some(p.id) {
                            crate::theme::c(30, 64, 100)
                        } else if is_dragged {
                            crate::theme::c(40, 50, 70)
                        } else {
                            Color32::TRANSPARENT
                        };
                        egui::Frame::none().fill(fill).inner_margin(egui::Margin::symmetric(6.0, 4.0)).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(format!("📑 {} ({})", p.name, p.count));
                        });
                    },
                    |i, resp| {
                        let p = &playlists[i];
                        if resp.clicked() {
                            new_selected = Some(p.id);
                        }
                        if resp.double_clicked() {
                            menu_action = Some((p.id, ListMenu::ShowInMain));
                        }
                        resp.context_menu(|ui| {
                            new_selected = Some(p.id);
                            if ui.button("📋 メイン画面で開く").on_hover_text("このプレイリストの曲を、メイン画面の一覧に表示します").clicked() {
                                menu_action = Some((p.id, ListMenu::ShowInMain));
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button("✏ テキストで編集...").on_hover_text("プレイリストの曲を、ファイルのパスの一覧として直接編集します").clicked() {
                                menu_action = Some((p.id, ListMenu::Edit));
                                ui.close_menu();
                            }
                            if ui.button("↕ 曲の並び替え...").on_hover_text("曲の順番を入れ替えます").clicked() {
                                menu_action = Some((p.id, ListMenu::Reorder));
                                ui.close_menu();
                            }
                            if ui.button("📄 コピー（複製）").on_hover_text("このプレイリストを複製して、新しいプレイリストを作ります").clicked() {
                                menu_action = Some((p.id, ListMenu::Duplicate));
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button(RichText::new("🗑 削除...").color(crate::theme::c(248, 113, 113))).clicked() {
                                menu_action = Some((p.id, ListMenu::Delete));
                                ui.close_menu();
                            }
                        });
                    },
                );

                if let Some(id) = new_selected {
                    state.selected = Some(id);
                }
                if let Some((from, to)) = moved {
                    let mut ids: Vec<i64> = playlists.iter().map(|p| p.id).collect();
                    let id = ids.remove(from);
                    ids.insert(to, id);
                    if let Err(e) = service.set_playlist_order(&ids) {
                        state.message = Some(e);
                    }
                }
            });
        });

    // --- 右クリックメニューの処理 ---
    if let Some((id, act)) = menu_action {
        let name = playlists.iter().find(|p| p.id == id).map(|p| p.name.clone()).unwrap_or_default();
        match act {
            ListMenu::ShowInMain => action = Some(PlaylistWindowAction::ShowInMain(id)),
            ListMenu::Edit => {
                let text = service
                    .get_playlist_tracks(id)
                    .iter()
                    .map(|p| p.to_string_lossy().to_string())
                    .collect::<Vec<_>>()
                    .join("\n");
                state.edit = Some(EditState { id, name, text });
            }
            ListMenu::Reorder => {
                state.reorder = Some(ReorderState { id, name, items: service.get_playlist_tracks(id), gen: next_window_gen(), frames: 0 });
            }
            ListMenu::Duplicate => match service.duplicate_playlist(id) {
                Ok(new_id) => {
                    state.selected = Some(new_id);
                    state.message = None;
                }
                Err(e) => state.message = Some(e),
            },
            ListMenu::Delete => state.confirm_delete = Some((id, name)),
        }
    }

    // --- 削除の確認 ---
    if let Some((id, name)) = state.confirm_delete.clone() {
        let mut close = false;
        egui::Window::new("プレイリストの削除")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ui.ctx(), |ui| {
                ui.label(format!("「{}」を削除しますか？（曲ファイル自体は削除されません）", name));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("🗑 削除する").color(crate::theme::c(248, 113, 113))).clicked() {
                        match service.delete_playlist(id) {
                            Ok(()) => {
                                if state.selected == Some(id) {
                                    state.selected = None;
                                }
                                action = Some(PlaylistWindowAction::Changed(id));
                            }
                            Err(e) => state.message = Some(e),
                        }
                        close = true;
                    }
                    if ui.button("キャンセル").clicked() {
                        close = true;
                    }
                });
            });
        if close {
            state.confirm_delete = None;
        }
    }

    // --- テキスト編集 ---
    if let Some(edit) = state.edit.as_mut() {
        let mut close = false;
        let mut save = false;
        egui::Window::new(format!("プレイリストの編集 - {}", edit.id))
            .id(Id::new("playlist_text_edit"))
            .collapsible(false)
            .default_size([620.0, 460.0])
            .show(ui.ctx(), |ui| {
                ui.horizontal(|ui| {
                    ui.label("名前:");
                    ui.add(egui::TextEdit::singleline(&mut edit.name).lock_focus(crate::views::ime::ime_composing(ui.ctx())).desired_width(f32::INFINITY));
                });
                ui.label(
                    RichText::new("1行に1ファイルのパスを書きます。行の追加・削除・入れ替え・コピペが自由にできます。空行は無視します。")
                        .weak()
                        .size(11.0),
                );
                ui.add_space(4.0);
                ScrollArea::vertical().max_height(ui.available_height() - 40.0).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut edit.text)
                            .font(egui::TextStyle::Monospace)
                            .code_editor()
                            .desired_rows(18)
                            .desired_width(f32::INFINITY),
                    );
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("💾 保存").clicked() {
                        save = true;
                    }
                    if ui.button("キャンセル").clicked() {
                        close = true;
                    }
                });
            });

        if save {
            let tracks: Vec<PathBuf> = edit
                .text
                .lines()
                .map(|l| l.trim().trim_matches('"'))
                .filter(|l| !l.is_empty())
                .map(PathBuf::from)
                .collect();
            let id = edit.id;
            let result = service
                .rename_playlist(id, &edit.name)
                .and_then(|_| service.set_playlist_tracks(id, &tracks));
            match result {
                Ok(()) => {
                    state.message = None;
                    action = Some(PlaylistWindowAction::Changed(id));
                    close = true;
                }
                Err(e) => state.message = Some(e),
            }
        }
        if close {
            state.edit = None;
        }
    }

    egui::CentralPanel::default().show_inside(ui, |ui| {
        let Some(id) = state.selected else {
            ui.label(RichText::new("左の一覧からプレイリストを選択してください（選択すると曲の右クリックから追加できます）").weak());
            return;
        };
        let name = playlists.iter().find(|p| p.id == id).map(|p| p.name.clone()).unwrap_or_default();
        let tracks = state.tracks(service, id);
        let missing_set = state.missing_set(ui.ctx(), id, service.version(), &tracks);

        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("📑 {}", name)).strong().size(15.0));
            ui.label(RichText::new(format!("{} 曲", tracks.len())).weak());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("💾 M3U8保存...").on_hover_text("このプレイリストを、M3U8 ファイルとして保存します").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("M3U8 Playlist", &["m3u8", "m3u"])
                        .set_file_name(format!("{}.m3u8", name))
                        .save_file()
                    {
                        if let Err(e) = service.export_m3u8(id, &path) {
                            state.message = Some(e);
                        }
                    }
                }
                if ui.button("📋 メイン画面で開く").on_hover_text("このプレイリストの曲を、メイン画面の一覧に表示します").clicked() {
                    action = Some(PlaylistWindowAction::ShowInMain(id));
                }
            });
        });
        // 曲の追加・整理（変更はすぐ保存される）
        let mut new_tracks: Option<Vec<PathBuf>> = None;
        ui.horizontal_wrapped(|ui| {
            if ui.button("➕ ファイルを追加...").on_hover_text("曲のファイルを選んで、このプレイリストに追加します").clicked() {
                if let Some(files) = rfd::FileDialog::new()
                    .add_filter("音楽ファイル", &["mp3", "flac", "m4a", "aac", "ogg", "opus", "wav", "wma", "aiff", "alac"])
                    .pick_files()
                {
                    // すでに入っている曲は飛ばす
                    let mut t = tracks.to_vec();
                    let mut have: std::collections::HashSet<String> = t.iter().map(|p| crate::store::path_key(p)).collect();
                    t.extend(files.into_iter().filter(|f| have.insert(crate::store::path_key(f))));
                    new_tracks = Some(t);
                }
            }
            let has_missing = !missing_set.is_empty();
            if ui.add_enabled(has_missing, egui::Button::new("🧹 見つからない曲を除く")).on_hover_text("ファイルが無くなっている曲を、プレイリストから外します").clicked() {
                new_tracks = Some(tracks.iter().filter(|p| !missing_set.contains(*p)).cloned().collect());
            }
            if ui.add_enabled(tracks.len() > 1, egui::Button::new("🔤 ファイル名順")).on_hover_text("ファイル名の順に並べ替えます").clicked() {
                let mut t = tracks.to_vec();
                t.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default());
                new_tracks = Some(t);
            }
            if ui.add_enabled(tracks.len() > 1, egui::Button::new("🔁 重複を除く")).on_hover_text("同じファイルが重ねて登録されているとき、1つだけ残します").clicked() {
                let mut seen = std::collections::HashSet::new();
                new_tracks = Some(tracks.iter().filter(|p| seen.insert(crate::store::path_key(p))).cloned().collect());
            }
            if ui.add_enabled(!tracks.is_empty(), egui::Button::new("🗑 すべて外す")).on_hover_text("プレイリストの曲を、すべて外します（ファイルは消えません）").clicked() {
                new_tracks = Some(Vec::new());
            }
        });
        ui.label(RichText::new("ダブルクリックで再生 / 右クリックで削除・移動").weak().size(11.0));
        ui.separator();

        if tracks.is_empty() {
            ui.label(RichText::new("（このプレイリストは空です）").weak());
        }
        // 表示されている行だけを描く（曲数が多くても重くならない）
        let row_h = ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().item_spacing.y + 4.0;
        ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, row_h, tracks.len(), |ui, range| {
            for i in range {
                let path = &tracks[i];
                let file = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let dir = path.parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_default();
                let missing = missing_set.contains(path);

                let mut text = RichText::new(format!("{:>3}.  {}", i + 1, file));
                if missing {
                    text = text.color(crate::theme::c(248, 113, 113)).strikethrough();
                }
                let resp = ui
                    .horizontal(|ui| {
                        let r = ui.selectable_label(false, text);
                        ui.label(RichText::new(dir).weak().size(10.0));
                        r
                    })
                    .inner
                    .on_hover_text(path.to_string_lossy());
                if resp.hovered() && !missing {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }

                if resp.double_clicked() && !missing {
                    action = Some(PlaylistWindowAction::Play { playlist_id: id, path: path.clone() });
                }
                let last = tracks.len() - 1;
                resp.context_menu(|ui| {
                    let mut t = tracks.to_vec();
                    if ui.add_enabled(!missing, egui::Button::new("▶ 再生")).on_hover_text("この曲を再生します").clicked() {
                        action = Some(PlaylistWindowAction::Play { playlist_id: id, path: path.clone() });
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.add_enabled(i > 0, egui::Button::new("⬆ 一つ上へ")).on_hover_text("この曲を、1つ上に動かします").clicked() {
                        t.swap(i, i - 1);
                        new_tracks = Some(t.clone());
                        ui.close_menu();
                    }
                    if ui.add_enabled(i < last, egui::Button::new("⬇ 一つ下へ")).on_hover_text("この曲を、1つ下に動かします").clicked() {
                        t.swap(i, i + 1);
                        new_tracks = Some(t.clone());
                        ui.close_menu();
                    }
                    if ui.add_enabled(i > 0, egui::Button::new("⏫ 先頭へ")).on_hover_text("この曲を、いちばん上に動かします").clicked() {
                        let x = t.remove(i);
                        t.insert(0, x);
                        new_tracks = Some(t.clone());
                        ui.close_menu();
                    }
                    if ui.add_enabled(i < last, egui::Button::new("⏬ 末尾へ")).on_hover_text("この曲を、いちばん下に動かします").clicked() {
                        let x = t.remove(i);
                        t.push(x);
                        new_tracks = Some(t.clone());
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button(RichText::new("➖ プレイリストから外す").color(crate::theme::c(248, 113, 113))).on_hover_text("この曲をプレイリストから外します（ファイルは消えません）").clicked() {
                        t.remove(i);
                        new_tracks = Some(t);
                        ui.close_menu();
                    }
                });
            }
        });

        if let Some(t) = new_tracks {
            match service.set_playlist_tracks(id, &t) {
                Ok(()) => action = Some(PlaylistWindowAction::Changed(id)),
                Err(e) => state.message = Some(e),
            }
        }
    });

    action
}

enum ListMenu {
    ShowInMain,
    Edit,
    Reorder,
    Duplicate,
    Delete,
}

/// 曲の並び替えウィンドウの中身（ドラッグ＆ドロップで並び替え）
pub fn render_reorder_window(ui: &mut Ui, state: &mut ReorderState) -> Option<ReorderOutcome> {
    let mut outcome = None;

    ui.label(RichText::new(format!("📑 {}", state.name)).strong().size(15.0));
    ui.label(RichText::new("曲をドラッグ＆ドロップして並び替え、「保存」で確定します").weak().size(11.0));
    ui.separator();

    let list_height = (ui.available_height() - 40.0).max(80.0);
    ScrollArea::vertical().max_height(list_height).auto_shrink([false, false]).show(ui, |ui| {
        let items = &state.items;
        let moved = reorder_list(
            ui,
            Id::new(("reorder_tracks", state.id)),
            items.len(),
            |ui, i, is_dragged| {
                let path = &items[i];
                let file = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let dir = path.parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_default();
                let fill = if is_dragged { crate::theme::c(40, 60, 90) } else { Color32::TRANSPARENT };
                egui::Frame::none().fill(fill).inner_margin(egui::Margin::symmetric(6.0, 3.0)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("☰").weak());
                        ui.label(RichText::new(format!("{:>3}.", i + 1)).weak());
                        ui.vertical(|ui| {
                            ui.label(file);
                            ui.label(RichText::new(dir).weak().size(10.0));
                        });
                    });
                });
            },
            |_, _| {},
        );
        if let Some((from, to)) = moved {
            let item = state.items.remove(from);
            state.items.insert(to, item);
        }
    });

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui.button("💾 保存").clicked() {
            outcome = Some(ReorderOutcome::Save);
        }
        if ui.button("キャンセル").clicked() {
            outcome = Some(ReorderOutcome::Cancel);
        }
    });
    outcome
}

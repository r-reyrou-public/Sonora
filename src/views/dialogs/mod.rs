use crate::db::labels::CustomTagService;
use crate::scanner::metadata::{CoverEdit, ExtraTags, TagEdit, TrackItem};
use crate::tools::duplicates::DuplicateGroup;
use crate::tools::renamer::{execute_rename_plan, generate_rename_plan, RenamePattern, RenamePlanItem};
use egui::{Context, RichText, Window};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub enum ActiveDialog {
    None,
    EditTags {
        track: TrackItem,
        form: Box<TagForm>,
    },
    EditTagsMulti {
        tracks: Vec<TrackItem>,
        form: Box<MultiTagForm>,
    },
    EditLabels {
        track: TrackItem,
        current_labels: Vec<String>,
        new_label_input: String,
    },
    BatchRename {
        tracks: Vec<TrackItem>,
        pattern: RenamePattern,
        plan: Vec<RenamePlanItem>,
        result_message: Option<String>,
    },
    DuplicateFinder {
        groups: Vec<DuplicateGroup>,
    },
    /// ハッシュチェックを始める前の、時間がかかることの確認
    DuplicateHashWarn {
        count: usize,
        total_size: String,
    },
    TrackDetails {
        track: TrackItem,
    },
}

fn number(label: &str, s: &str, max: u32) -> Result<u32, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(0);
    }
    match s.parse::<u32>() {
        Ok(n) if n <= max => Ok(n),
        _ => Err(format!("「{}」は 0〜{} の数字で入力してください", label, max)),
    }
}

/// 複数曲の一括編集の入力内容。チェックを入れた項目だけを全曲に書き込む
#[derive(Clone, Debug, Default)]
pub struct MultiTagForm {
    form: TagForm,
    /// アーティスト / アルバム / アルバムアーティスト / ジャンル / 作曲者 / コメント / 年 / ディスク の順
    apply: [bool; 8],
    result: Option<String>,
}

const MULTI_LABELS: [&str; 8] =
    ["アーティスト", "アルバム", "アルバムアーティスト", "ジャンル", "作曲者", "コメント", "年 (西暦)", "ディスク番号"];

impl MultiTagForm {
    pub fn from_tracks(tracks: &[TrackItem]) -> Self {
        // 全曲で同じ値の項目は、その値を最初から入れておく
        let common = |f: &dyn Fn(&TrackItem) -> String| -> String {
            let first = f(&tracks[0]);
            if tracks.iter().all(|t| f(t) == first) { first } else { String::new() }
        };
        let num = |n: u32| if n == 0 { String::new() } else { n.to_string() };
        let form = TagForm {
            artist: common(&|t| t.artist.clone()),
            album: common(&|t| t.album.clone()),
            album_artist: common(&|t| t.album_artist.clone()),
            genre: common(&|t| t.genre.clone()),
            year: common(&|t| num(t.year)),
            ..Default::default()
        };
        Self { form, apply: [false; 8], result: None }
    }

    fn to_edit(&self) -> Result<TagEdit, String> {
        let (f, a) = (&self.form, &self.apply);
        let pick = |on: bool, v: &str| on.then(|| v.to_string());
        Ok(TagEdit {
            artist: pick(a[0], &f.artist),
            album: pick(a[1], &f.album),
            album_artist: pick(a[2], &f.album_artist),
            genre: pick(a[3], &f.genre),
            composer: pick(a[4], &f.composer),
            comment: pick(a[5], &f.comment),
            year: if a[6] { Some(number("年", &f.year, 9999)?) } else { None },
            disc: if a[7] { Some(number("ディスク番号", &f.disc, 999)?) } else { None },
            cover: f.cover.clone(),
            ..Default::default()
        })
    }

    fn has_changes(&self) -> bool {
        self.apply.iter().any(|&b| b) || !matches!(self.form.cover, CoverEdit::Keep)
    }
}

/// タグ編集ダイアログの入力内容
#[derive(Clone, Debug, Default)]
pub struct TagForm {
    title: String,
    artist: String,
    album: String,
    album_artist: String,
    genre: String,
    composer: String,
    comment: String,
    year: String,
    track_no: String,
    disc: String,
    cover: CoverEdit,
    /// 元のファイルに埋め込み画像があるか
    had_cover: bool,
    /// 選んだ画像のファイル名（表示用）
    cover_name: String,
    error: Option<String>,
}

impl TagForm {
    pub fn from_track(t: &TrackItem) -> Self {
        let extra = ExtraTags::read(&t.file_path);
        let num = |n: u32| if n == 0 { String::new() } else { n.to_string() };
        Self {
            // 一覧のタイトルはファイル名で代用されていることがあるので、タグ本来の値を入れる
            title: t.tag_title.clone(),
            artist: t.artist.clone(),
            album: t.album.clone(),
            album_artist: t.album_artist.clone(),
            genre: t.genre.clone(),
            composer: extra.composer,
            comment: extra.comment,
            year: num(t.year),
            track_no: num(t.track_number),
            disc: num(extra.disc),
            cover: CoverEdit::Keep,
            had_cover: TrackItem::load_artwork(&t.file_path).is_some(),
            cover_name: String::new(),
            error: None,
        }
    }

    /// 入力を検証して書き込み内容にする
    fn to_edit(&self) -> Result<TagEdit, String> {
        Ok(TagEdit {
            title: Some(self.title.clone()),
            artist: Some(self.artist.clone()),
            album: Some(self.album.clone()),
            album_artist: Some(self.album_artist.clone()),
            genre: Some(self.genre.clone()),
            composer: Some(self.composer.clone()),
            comment: Some(self.comment.clone()),
            year: Some(number("年", &self.year, 9999)?),
            track: Some(number("トラック番号", &self.track_no, 9999)?),
            disc: Some(number("ディスク番号", &self.disc, 999)?),
            cover: self.cover.clone(),
        })
    }
}

pub fn render_dialogs(
    ctx: &Context,
    dialog: &mut ActiveDialog,
    label_service: &CustomTagService,
    on_refresh_tracks: &mut impl FnMut(),
    _on_locate_file: &mut impl FnMut(PathBuf),
) {
    let mut close_dialog = false;
    let mut switch_to: Option<ActiveDialog> = None;

    match dialog {
        ActiveDialog::None => {}

        ActiveDialog::EditTags { track, form } => {
            let shown = Window::new("🏷 タグ情報の編集")
                .collapsible(false)
                .resizable(false)
                .default_width(460.0)
                .show(ctx, |ui| {
                    ui.label(RichText::new(&track.filename).strong().color(crate::theme::c(147, 197, 253)));
                    ui.add_space(8.0);

                    egui::Grid::new("tag_edit_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                        let row = |ui: &mut egui::Ui, label: &str, value: &mut String, width: f32| {
                            ui.label(label);
                            ui.add(egui::TextEdit::singleline(value).desired_width(width));
                            ui.end_row();
                        };
                        row(ui, "タイトル:", &mut form.title, 320.0);
                        row(ui, "アーティスト:", &mut form.artist, 320.0);
                        row(ui, "アルバム:", &mut form.album, 320.0);
                        row(ui, "アルバムアーティスト:", &mut form.album_artist, 320.0);
                        row(ui, "ジャンル:", &mut form.genre, 320.0);
                        row(ui, "作曲者:", &mut form.composer, 320.0);
                        row(ui, "年 (西暦):", &mut form.year, 80.0);
                        row(ui, "トラック番号:", &mut form.track_no, 80.0);
                        row(ui, "ディスク番号:", &mut form.disc, 80.0);
                        ui.label("コメント:");
                        ui.add(egui::TextEdit::multiline(&mut form.comment).desired_rows(2).desired_width(320.0));
                        ui.end_row();
                    });

                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label("ジャケット:");
                        let state = match &form.cover {
                            CoverEdit::Keep if form.had_cover => "埋め込みあり（変更なし）".to_string(),
                            CoverEdit::Keep => "なし".to_string(),
                            CoverEdit::Remove => "削除します".to_string(),
                            CoverEdit::Replace(b) => format!("{}（{} KB）に差し替え", form.cover_name, b.len() / 1024),
                        };
                        ui.label(RichText::new(state).weak());
                        if ui.button("🖼 画像を選ぶ...").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("画像", &["jpg", "jpeg", "png", "webp"])
                                .pick_file()
                            {
                                // 巨大なファイルをタグに埋め込まない（曲ファイルが膨らむ）
                                match std::fs::metadata(&path).map(|m| m.len()) {
                                    Ok(n) if n <= 10 * 1024 * 1024 => match std::fs::read(&path) {
                                        Ok(bytes) => {
                                            form.cover_name =
                                                path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                                            form.cover = CoverEdit::Replace(bytes);
                                            form.error = None;
                                        }
                                        Err(e) => form.error = Some(format!("画像を読めませんでした: {}", e)),
                                    },
                                    Ok(_) => form.error = Some("画像が大きすぎます（10MBまで）".to_string()),
                                    Err(e) => form.error = Some(format!("画像を読めませんでした: {}", e)),
                                }
                            }
                        }
                        if (form.had_cover || matches!(form.cover, CoverEdit::Replace(_)))
                            && !matches!(form.cover, CoverEdit::Remove)
                            && ui.button("🗑 削除").clicked()
                        {
                            form.cover = if form.had_cover { CoverEdit::Remove } else { CoverEdit::Keep };
                        }
                    });

                    if let Some(e) = &form.error {
                        ui.add_space(6.0);
                        ui.label(RichText::new(e.as_str()).color(crate::theme::c(248, 113, 113)));
                    }

                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("💾 保存して反映").strong()).clicked() {
                            let result = form
                                .to_edit()
                                .and_then(|edit| TrackItem::write_tags(&track.file_path, &edit));
                            match result {
                                Ok(()) => {
                                    on_refresh_tracks();
                                    close_dialog = true;
                                }
                                Err(e) => {
                                    form.error = Some(format!(
                                        "保存できませんでした: {}
（再生中の曲や、読み取り専用のファイルには書き込めません）",
                                        e
                                    ));
                                }
                            }
                        }
                        if ui.button("キャンセル").clicked() {
                            close_dialog = true;
                        }
                    });
                });
            close_if_outside(ctx, shown, &mut close_dialog);
        }

        ActiveDialog::EditTagsMulti { tracks, form: mf } => {
            let shown = Window::new("🏷 タグ情報の一括編集")
                .collapsible(false)
                .resizable(false)
                .default_width(480.0)
                .show(ctx, |ui| {
                    ui.label(RichText::new(format!("{} 曲に同じ内容を書き込みます", tracks.len())).strong());
                    ui.label(
                        RichText::new("チェックを入れた項目だけが全曲に反映されます（入力すると自動でチェックされます）。空欄で保存すると、その項目を削除します。")
                            .weak()
                            .size(11.0),
                    );
                    ui.add_space(6.0);

                    {
                        let MultiTagForm { form, apply, .. } = &mut **mf;
                        let TagForm { artist, album, album_artist, genre, composer, comment, year, disc, .. } = form;
                        let fields: [&mut String; 8] = [artist, album, album_artist, genre, composer, comment, year, disc];
                        egui::Grid::new("tag_multi_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                            // 曲ごとに値が違う項目は、一括では編集できないのでグレーアウトして表示する
                            for label in ["タイトル", "トラック番号"] {
                                ui.add_enabled_ui(false, |ui| {
                                    ui.label(label);
                                });
                                ui.add_enabled(
                                    false,
                                    egui::TextEdit::singleline(&mut String::from("（曲ごとに異なるため一括編集できません）"))
                                        .desired_width(300.0),
                                );
                                ui.end_row();
                            }
                            for (i, f) in fields.into_iter().enumerate() {
                                ui.checkbox(&mut apply[i], MULTI_LABELS[i]);
                                let width = if i >= 6 { 80.0 } else { 300.0 };
                                if ui.add(egui::TextEdit::singleline(f).desired_width(width)).changed() {
                                    apply[i] = true;
                                }
                                ui.end_row();
                            }
                        });
                    }

                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let form = &mut mf.form;
                        ui.label("ジャケット:");
                        let state = match &form.cover {
                            CoverEdit::Keep => "変更しない".to_string(),
                            CoverEdit::Remove => "全曲から削除します".to_string(),
                            CoverEdit::Replace(b) => format!("{}（{} KB）に差し替え", form.cover_name, b.len() / 1024),
                        };
                        ui.label(RichText::new(state).weak());
                        if ui.button("🖼 画像を選ぶ...").clicked() {
                            if let Some(path) =
                                rfd::FileDialog::new().add_filter("画像", &["jpg", "jpeg", "png", "webp"]).pick_file()
                            {
                                match std::fs::metadata(&path).map(|m| m.len()) {
                                    Ok(n) if n <= 10 * 1024 * 1024 => match std::fs::read(&path) {
                                        Ok(bytes) => {
                                            form.cover_name =
                                                path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                                            form.cover = CoverEdit::Replace(bytes);
                                            form.error = None;
                                        }
                                        Err(e) => form.error = Some(format!("画像を読めませんでした: {}", e)),
                                    },
                                    Ok(_) => form.error = Some("画像が大きすぎます（10MBまで）".to_string()),
                                    Err(e) => form.error = Some(format!("画像を読めませんでした: {}", e)),
                                }
                            }
                        }
                        if ui.button("🗑 削除").clicked() {
                            form.cover = CoverEdit::Remove;
                        }
                        if !matches!(form.cover, CoverEdit::Keep) && ui.button("↩ 変更しない").clicked() {
                            form.cover = CoverEdit::Keep;
                        }
                    });

                    if let Some(e) = &mf.form.error {
                        ui.add_space(6.0);
                        ui.label(RichText::new(e.as_str()).color(crate::theme::c(248, 113, 113)));
                    }
                    if let Some(r) = mf.result.as_ref() {
                        ui.add_space(6.0);
                        ui.label(RichText::new(r.as_str()).color(crate::theme::c(250, 204, 21)));
                    }

                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        let can_save = mf.has_changes();
                        if ui.add_enabled(can_save, egui::Button::new(RichText::new("💾 全曲に書き込む").strong())).clicked() {
                            match mf.to_edit() {
                                Err(e) => mf.form.error = Some(e),
                                Ok(edit) => {
                                    mf.form.error = None;
                                    let mut failed: Vec<String> = Vec::new();
                                    for t in tracks.iter() {
                                        if let Err(e) = TrackItem::write_tags(&t.file_path, &edit) {
                                            failed.push(format!("{}（{}）", t.filename, e));
                                        }
                                    }
                                    on_refresh_tracks();
                                    if failed.is_empty() {
                                        close_dialog = true;
                                    } else {
                                        let shown: Vec<_> = failed.iter().take(3).cloned().collect();
                                        mf.result = Some(format!(
                                            "{} 曲に書き込みました。{} 曲は失敗しました: {}{}",
                                            tracks.len() - failed.len(),
                                            failed.len(),
                                            shown.join(" / "),
                                            if failed.len() > 3 { " ほか" } else { "" }
                                        ));
                                    }
                                }
                            }
                        }
                        if ui.button(if mf.result.is_some() { "閉じる" } else { "キャンセル" }).clicked() {
                            close_dialog = true;
                        }
                    });
                });
            close_if_outside(ctx, shown, &mut close_dialog);
        }

        ActiveDialog::EditLabels {
            track,
            current_labels,
            new_label_input,
        } => {
            Window::new("🏷 ラベルの管理")
                .collapsible(false)
                .resizable(true)
                .default_width(380.0)
                .show(ctx, |ui| {
                    ui.label(RichText::new(&track.filename).strong().color(crate::theme::c(147, 197, 253)));
                    ui.add_space(6.0);

                    ui.label(RichText::new("設定中のラベル:").weak().size(12.0));
                    ui.horizontal_wrapped(|ui| {
                        let mut to_remove = None;
                        for (idx, l) in current_labels.iter().enumerate() {
                            if ui.button(format!("{} ✖", l)).clicked() {
                                to_remove = Some(idx);
                            }
                        }
                        if let Some(i) = to_remove {
                            current_labels.remove(i);
                        }
                    });

                    ui.add_space(8.0);
                    ui.separator();
                    ui.label(RichText::new("新規ラベル追加:").weak().size(12.0));
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(new_label_input);
                        if ui.button("追加").clicked() {
                            let clean = new_label_input.trim().to_string();
                            if !clean.is_empty() && !current_labels.contains(&clean) {
                                current_labels.push(clean.clone());
                                label_service.add_preset_label(&clean);
                                new_label_input.clear();
                            }
                        }
                    });

                    let presets = label_service.get_all_preset_labels();
                    if !presets.is_empty() {
                        ui.add_space(8.0);
                        ui.label(RichText::new("プリセットから選択:").weak().size(12.0));
                        ui.horizontal_wrapped(|ui| {
                            for p in presets {
                                if !current_labels.contains(&p) && ui.small_button(format!("+ {}", p)).clicked() {
                                    current_labels.push(p);
                                }
                            }
                        });
                    }

                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("💾 ラベルを保存").strong()).clicked() {
                            label_service.set_labels_for_file(&track.file_path, current_labels);
                            on_refresh_tracks();
                            close_dialog = true;
                        }
                        if ui.button("閉じる").clicked() {
                            close_dialog = true;
                        }
                    });
                });
        }

        ActiveDialog::BatchRename {
            tracks,
            pattern,
            plan,
            result_message,
        } => {
            Window::new("🔄 一括ファイル名リネーム")
                .collapsible(false)
                .resizable(true)
                .default_width(600.0)
                .show(ctx, |ui| {
                    ui.label(RichText::new(format!("対象ファイル: {} 曲", tracks.len())).strong());
                    ui.add_space(4.0);

                    let prev_pat = *pattern;
                    ui.horizontal(|ui| {
                        ui.label("リネームパターン:");
                        egui::ComboBox::from_id_salt("rename_pat_combo")
                            .selected_text(match pattern {
                                RenamePattern::ArtistTitle => "アーティスト - タイトル",
                                RenamePattern::TitleOnly => "タイトルのみ",
                                RenamePattern::AlbumArtistTitle => "アルバム - アーティスト - タイトル",
                                RenamePattern::NumberedTitle => "連番_タイトル (01_Title)",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(pattern, RenamePattern::ArtistTitle, "アーティスト - タイトル");
                                ui.selectable_value(pattern, RenamePattern::TitleOnly, "タイトルのみ");
                                ui.selectable_value(pattern, RenamePattern::AlbumArtistTitle, "アルバム - アーティスト - タイトル");
                                ui.selectable_value(pattern, RenamePattern::NumberedTitle, "連番_タイトル (01_Title)");
                            });
                    });

                    if *pattern != prev_pat || plan.is_empty() {
                        *plan = generate_rename_plan(tracks, *pattern);
                    }

                    ui.add_space(8.0);
                    ui.label(RichText::new("プレビュー (変更前 -> 変更後):").weak().size(12.0));

                    egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                        for item in plan.iter() {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&item.current_name).weak());
                                ui.label("➜");
                                ui.label(RichText::new(&item.new_name).color(crate::theme::c(34, 197, 94)));
                            });
                        }
                    });

                    if let Some(ref msg) = result_message {
                        ui.add_space(6.0);
                        ui.label(RichText::new(msg.as_str()).color(crate::theme::c(250, 204, 21)));
                    }

                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("🚀 リネームを実行").strong()).clicked() {
                            let (success, errors) = execute_rename_plan(plan);
                            *result_message = Some(format!("{} 件のリネームが完了しました (エラー: {} 件)", success, errors.len()));
                            on_refresh_tracks();
                        }
                        if ui.button("閉じる").clicked() {
                            close_dialog = true;
                        }
                    });
                });
        }

        ActiveDialog::DuplicateFinder { groups } => {
            Window::new("🔍 重複ファイル検出")
                .collapsible(false)
                .resizable(true)
                .default_width(650.0)
                .show(ctx, |ui| {
                    ui.label(RichText::new(format!("検出された重複グループ: {} 件", groups.len())).strong());
                    ui.add_space(6.0);

                    if groups.is_empty() {
                        ui.label("重複するファイルは見つかりませんでした。");
                    } else {
                        egui::ScrollArea::vertical().max_height(350.0).show(ui, |ui| {
                            for g in groups {
                                ui.group(|ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(&g.label).strong().color(crate::theme::c(251, 146, 60)));
                                        ui.label(RichText::new(&g.reason).weak());
                                    });
                                    for t in &g.tracks {
                                        ui.horizontal(|ui| {
                                            ui.label(format!("• {}", t.filename));
                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                if ui.small_button("📂 ツリーで開く").clicked() {
                                                    request_reveal(ctx, &t.file_path);
                                                }
                                            });
                                        });
                                    }
                                });
                                ui.add_space(4.0);
                            }
                        });
                    }

                    ui.add_space(10.0);
                    if ui.button("閉じる").clicked() {
                        close_dialog = true;
                    }
                });
        }

        ActiveDialog::DuplicateHashWarn { count, total_size } => {
            Window::new("🧬 重複検出（ハッシュチェック）")
                .collapsible(false)
                .resizable(false)
                .default_width(440.0)
                .show(ctx, |ui| {
                    ui.label(RichText::new("⚠ 時間がかかります").strong().color(crate::theme::c(251, 191, 36)));
                    ui.add_space(4.0);
                    ui.label(format!(
                        "いま読み込んでいる {} 曲（合計 {}）の音声データを、すべて読み込んで比べます。",
                        count, total_size
                    ));
                    ui.label("曲数やディスクの速さによっては、数分〜数十分かかることがあります。その間、ディスクへの読み込みが増えます。");
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(
                            "タグ（ID3・ジャケット画像など）は除いて、音声そのものが同じファイルを探します。\n進捗と結果は別ウィンドウに表示され、途中で中止したり、聴き比べたりできます。",
                        )
                        .weak()
                        .size(12.0),
                    );
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("▶ 検査を始める").strong()).clicked() {
                            ctx.data_mut(|d| d.insert_temp(egui::Id::new("dup_hash_start"), true));
                            close_dialog = true;
                        }
                        if ui.button("キャンセル").clicked() {
                            close_dialog = true;
                        }
                    });
                });
        }

        ActiveDialog::TrackDetails { track } => {
            let mut edit_this = false;
            let shown = Window::new("ℹ 楽曲の詳細プロパティ")
                .collapsible(false)
                .resizable(true)
                .default_width(450.0)
                .show(ctx, |ui| {
                    ui.label(RichText::new(&track.title).strong().size(15.0).color(crate::theme::c(147, 197, 253)));
                    ui.label(RichText::new(&track.artist).weak().size(13.0));
                    ui.add_space(8.0);

                    egui::Grid::new("track_details_grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                        ui.label("ファイル名:");
                        ui.label(&track.filename);
                        ui.end_row();

                        ui.label("フルパス:");
                        ui.label(track.file_path.to_string_lossy().to_string());
                        ui.end_row();

                        ui.label("形式:");
                        ui.label(&track.format);
                        ui.end_row();

                        ui.label("演奏時間:");
                        ui.label(&track.formatted_duration);
                        ui.end_row();

                        ui.label("ファイルサイズ:");
                        ui.label(format!("{} ({} バイト)", track.formatted_size, track.file_size_bytes));
                        ui.end_row();

                        ui.label("アルバム:");
                        ui.label(if track.album.is_empty() { "-" } else { &track.album });
                        ui.end_row();

                        ui.label("ジャンル:");
                        ui.label(if track.genre.is_empty() { "-" } else { &track.genre });
                        ui.end_row();

                        ui.label("リリース年:");
                        ui.label(if track.year == 0 { "-".to_string() } else { track.year.to_string() });
                        ui.end_row();
                    });

                    if let Some(ref art) = track.artwork_bytes {
                        ui.add_space(8.0);
                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("🖼 ジャケット画像埋め込みあり ({} KB)", art.len() / 1024)).strong());
                            if ui.button("💾 ジャケット画像を保存").clicked() {
                                if let Some(save_path) = rfd::FileDialog::new()
                                    .set_file_name("cover.jpg")
                                    .save_file()
                                {
                                    let _ = std::fs::write(save_path, art);
                                }
                            }
                        });
                    }

                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.button("📂 ツリーで開く").clicked() {
                            request_reveal(ctx, &track.file_path);
                        }
                        if ui.button("✏ プロパティを編集").on_hover_text("タグ情報とジャケット画像を編集します").clicked() {
                            edit_this = true;
                        }
                        if ui.button("閉じる").clicked() {
                            close_dialog = true;
                        }
                    });
                });
            if edit_this {
                // 詳細を閉じて、同じ曲のタグ編集（ジャケット画像の登録を含む）を開く
                switch_to = Some(ActiveDialog::EditTags { form: Box::new(TagForm::from_track(track)), track: track.clone() });
            }
            // ウィンドウの外をクリックしたら閉じる
            close_if_outside(ctx, shown, &mut close_dialog);
        }
    }

    if let Some(next) = switch_to {
        *dialog = next;
    } else if close_dialog {
        *dialog = ActiveDialog::None;
    }
}

/// ウィンドウの外側がクリックされたら閉じる
fn close_if_outside<R>(ctx: &egui::Context, shown: Option<egui::InnerResponse<Option<R>>>, close: &mut bool) {
    if let Some(w) = shown {
        let outside =
            ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !w.response.rect.contains(p)));
        if outside {
            *close = true;
        }
    }
}

/// アプリのツリーで開く要望を出す（app 側が次のフレームで受け取る）
fn request_reveal(ctx: &egui::Context, path: &std::path::Path) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("reveal_request"), path.to_path_buf()));
}

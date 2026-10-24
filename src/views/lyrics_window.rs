//! 歌詞のペイン（メイン画面の右からスライドして出る）: 再生位置に合わせた表示、テキスト編集、Enter キーでのタイムスタンプ打ち
use crate::audio::player::PlaybackState;
use crate::lyrics::{self, Line, Lyrics};
use egui::{Align, Color32, Key, Modifiers, RichText, ScrollArea, Sense, Ui};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// 再生中の曲を追いかけて表示する
    View,
    /// 歌詞のテキスト（LRC 形式でも、時刻のない歌詞でも）を直接書き換える
    Text,
    /// 再生しながら Enter で、1行ずつ時刻を打つ
    Stamp,
}

/// 保存していない変更があるときの確認の行き先
#[derive(Clone, Copy, PartialEq, Eq)]
enum Exit {
    EndEdit,
    ClosePane,
}

/// 歌詞の色（設定で変えられる）
#[derive(Clone, Copy)]
pub struct LyricsStyle {
    /// いま歌っている行・行の中で歌い終えた部分
    pub current: Color32,
    /// 歌い終えた行
    pub past: Color32,
    /// これから歌う行・いまの行のこれから歌う部分
    pub upcoming: Color32,
    /// いまの行の中でも、時刻に合わせて色を進める（カラオケ風）
    pub sweep: bool,
}

pub enum LyricsAction {
    Seek(Duration),
    /// 再生中なら一時停止、一時停止中なら再開
    TogglePause,
    /// ペインを閉じる
    Close,
}

pub struct LyricsWindow {
    /// 読み込んだ（編集している）曲
    path: Option<PathBuf>,
    lyrics: Lyrics,
    /// 保存済み（または読み込んだ）内容。変更の有無の判定に使う
    saved: Lyrics,
    loading: Option<PathBuf>,
    rx: Option<Receiver<(PathBuf, Lyrics)>>,
    mode: Mode,
    text: String,
    /// タイムスタンプ打ちで、次に時刻を打つ行
    cursor: usize,
    /// 打鍵の遅れを補う量（ミリ秒）。押した時刻からこの分だけ早い時刻を記録する
    compensate_ms: u32,
    last_current: Option<usize>,
    /// 一覧をスクロールして見せた行（カーソルが動いたときだけスクロールする）
    scrolled: Option<usize>,
    message: Option<String>,
    confirm_exit: Option<Exit>,
    /// 保存した曲（一覧の「歌詞」の印を更新するため、app が受け取る）
    saved_path: Option<PathBuf>,
}

impl Default for LyricsWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl LyricsWindow {
    pub fn new() -> Self {
        Self {
            path: None,
            lyrics: Lyrics::default(),
            saved: Lyrics::default(),
            loading: None,
            rx: None,
            mode: Mode::View,
            text: String::new(),
            cursor: 0,
            compensate_ms: 0,
            last_current: None,
            scrolled: None,
            message: None,
            confirm_exit: None,
            saved_path: None,
        }
    }

    /// 保存した曲のパスを受け取る（一度だけ）
    pub fn take_saved(&mut self) -> Option<PathBuf> {
        self.saved_path.take()
    }

    /// 編集中の内容（テキスト編集中は、書かれている文字を読み取ったもの）
    fn current(&self) -> Lyrics {
        if self.mode == Mode::Text {
            let mut l = lyrics::parse_lrc(&self.text);
            l.from_file = self.saved.from_file;
            l
        } else {
            self.lyrics.clone()
        }
    }

    fn dirty(&self) -> bool {
        if self.mode == Mode::View {
            return false;
        }
        let c = self.current();
        c.lines != self.saved.lines || c.meta != self.saved.meta
    }

    fn start_load(&mut self, ctx: &egui::Context, path: PathBuf) {
        let (tx, rx) = channel();
        self.rx = Some(rx);
        self.loading = Some(path.clone());
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let l = lyrics::load(&path);
            let _ = tx.send((path, l));
            ctx.request_repaint();
        });
    }

    fn poll_load(&mut self) {
        let Some(rx) = &self.rx else { return };
        if let Ok((path, l)) = rx.try_recv() {
            if self.loading.as_ref() == Some(&path) {
                self.path = Some(path);
                self.saved = l.clone();
                self.lyrics = l;
                self.loading = None;
                self.rx = None;
                self.cursor = 0;
                self.last_current = None;
            }
        }
    }

    fn enter_edit(&mut self) {
        self.message = None;
        if self.lyrics.is_empty() {
            self.text = String::new();
            self.mode = Mode::Text;
        } else {
            self.cursor = self.lyrics.lines.iter().position(|l| l.time_ms.is_none()).unwrap_or(0);
            self.mode = Mode::Stamp;
        }
    }

    fn switch_to_text(&mut self) {
        if self.mode == Mode::Stamp {
            self.text = self.lyrics.to_lrc();
        }
        self.mode = Mode::Text;
    }

    fn switch_to_stamp(&mut self) {
        if self.mode == Mode::Text {
            let mut l = self.current();
            l.from_file = self.lyrics.from_file;
            self.lyrics = l;
        }
        self.cursor = self.cursor.min(self.lyrics.lines.len().saturating_sub(1));
        self.mode = Mode::Stamp;
    }

    fn save(&mut self) {
        let Some(path) = self.path.clone() else { return };
        let mut l = self.current();
        l.from_file = true;
        match lyrics::save(&path, &l) {
            Ok(()) => {
                self.saved = l.clone();
                self.lyrics = l;
                self.saved_path = Some(path);
                if self.mode == Mode::Text {
                    self.text = self.lyrics.to_lrc();
                }
                self.message = Some("保存しました（曲と同じフォルダの .lrc）".to_string());
            }
            Err(e) => self.message = Some(e),
        }
    }

    fn end_edit(&mut self) {
        self.lyrics = self.saved.clone();
        self.mode = Mode::View;
        self.message = None;
        self.last_current = None;
    }
}

/// `now`: いま再生している曲（なければ選択中の曲）。`playing`: 再生エンジンが実際に流している曲
pub fn render_lyrics_window(
    ui: &mut Ui,
    w: &mut LyricsWindow,
    now: Option<&PathBuf>,
    playing: Option<&PathBuf>,
    pos: Duration,
    total: Duration,
    state: PlaybackState,
    style: &LyricsStyle,
) -> Option<LyricsAction> {
    let mut action = None;
    // キー操作の占有は、タイムスタンプ打ちのときだけ（毎フレーム、いったん解除する）
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("lyrics_keys_active"), false));
    w.poll_load();

    // 表示中は再生中の曲を追う。編集中は、編集している曲に固定する
    if w.mode == Mode::View {
        if let Some(p) = now {
            if w.path.as_ref() != Some(p) && w.loading.as_ref() != Some(p) {
                w.start_load(ui.ctx(), p.clone());
            }
        }
    }

    let pos_ms = pos.as_millis() as u64;
    let same_track = w.path.is_some() && w.path.as_ref() == playing;

    // ---- 上部: 曲名・再生操作 ----
    ui.horizontal(|ui| {
        let title = w
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "（曲が選ばれていません）".to_string());
        ui.label(RichText::new(format!("🎤 {}", title)).strong().size(15.0));
        if w.loading.is_some() {
            ui.label(RichText::new("読み込み中…").weak());
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("✖").on_hover_text("歌詞ペインを閉じる").clicked() {
                if w.dirty() {
                    w.confirm_exit = Some(Exit::ClosePane);
                } else {
                    action = Some(LyricsAction::Close);
                }
            }
            if w.mode == Mode::View && ui.add_enabled(w.path.is_some(), egui::Button::new("✏ 編集")).on_hover_text("歌詞を編集します（テキスト編集とタイムスタンプ打ち）").clicked() {
                w.enter_edit();
            }
        });
    });
    ui.horizontal_wrapped(|ui| {
        let icon = if state == PlaybackState::Playing { "⏸" } else { "▶" };
        if ui.button(icon).on_hover_text("再生 / 一時停止（Space）").clicked() {
            action = Some(LyricsAction::TogglePause);
        }
        if ui.button("⏪ 5秒").on_hover_text("5 秒戻ります").clicked() {
            action = Some(LyricsAction::Seek(pos.saturating_sub(Duration::from_secs(5))));
        }
        if ui.button("5秒 ⏩").on_hover_text("5 秒進みます").clicked() {
            action = Some(LyricsAction::Seek((pos + Duration::from_secs(5)).min(total)));
        }
        ui.label(RichText::new(format!("{} / {}", lyrics::display_time(pos_ms), lyrics::display_time(total.as_millis() as u64))).monospace());
        if !same_track && w.path.is_some() {
            ui.label(RichText::new("※ この曲は再生されていません").color(crate::theme::c(251, 191, 36)).size(12.0));
        }
    });

    // ---- 編集用の操作列 ----
    if w.mode != Mode::View {
        ui.horizontal_wrapped(|ui| {
            if ui.selectable_label(w.mode == Mode::Text, "📝 テキスト編集").on_hover_text("歌詞の文字を、テキストとして直接編集します").clicked() {
                w.switch_to_text();
            }
            if ui.selectable_label(w.mode == Mode::Stamp, "⏱ タイムスタンプ打ち").on_hover_text("再生しながら Enter キーを押して、行ごとに時刻を記録します").clicked() {
                w.switch_to_stamp();
            }
            ui.separator();
            let dirty = w.dirty();
            if ui.add_enabled(dirty || !w.saved.from_file, egui::Button::new("💾 保存")).on_hover_text("歌詞を、曲と同じフォルダの .lrc ファイルに保存します").clicked() {
                w.save();
            }
            if ui.button("✖ 編集を終える").on_hover_text("編集を終えて、表示だけの画面に戻ります").clicked() {
                if w.dirty() {
                    w.confirm_exit = Some(Exit::EndEdit);
                } else {
                    w.end_edit();
                }
            }
            if dirty {
                ui.label(RichText::new("● 未保存").color(crate::theme::c(251, 191, 36)));
            }
        });
    }
    if let Some(m) = &w.message {
        ui.label(RichText::new(m).weak().size(11.0));
    }

    // ---- 保存していない変更の確認 ----
    if let Some(exit) = w.confirm_exit {
        let mut done = false;
        egui::Window::new("保存していない変更があります")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ui.ctx(), |ui| {
                ui.label("歌詞の変更を保存しますか？");
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("💾 保存して続ける").strong()).clicked() {
                        w.save();
                        if !w.dirty() {
                            match exit {
                                Exit::EndEdit => w.end_edit(),
                                Exit::ClosePane => action = Some(LyricsAction::Close),
                            }
                            done = true;
                        } else {
                            done = true; // 保存に失敗した。メッセージを見て直してもらう
                        }
                    }
                    if ui.button("破棄して続ける").clicked() {
                        match exit {
                            Exit::EndEdit => w.end_edit(),
                            Exit::ClosePane => action = Some(LyricsAction::Close),
                        }
                        done = true;
                    }
                    if ui.button("キャンセル").clicked() {
                        done = true;
                    }
                });
            });
        if done {
            w.confirm_exit = None;
        }
    }
    ui.separator();

    match w.mode {
        Mode::View => view_lines(ui, w, pos_ms, total.as_millis() as u64, style, &mut action),
        Mode::Text => {
            ui.label(
                RichText::new("1行に1行分の歌詞を書きます。時刻付き（[01:23.45]歌詞）でも、時刻のない歌詞を貼り付けるだけでも構いません。")
                    .weak()
                    .size(11.0),
            );
            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut w.text)
                        .font(egui::TextStyle::Monospace)
                        .code_editor()
                        .lock_focus(crate::views::ime::ime_composing(ui.ctx()))
                        .desired_width(f32::INFINITY)
                        .desired_rows(20),
                );
            });
        }
        Mode::Stamp => stamp_lines(ui, w, pos_ms, same_track, state, &mut action),
    }
    action
}

fn view_lines(ui: &mut Ui, w: &mut LyricsWindow, pos_ms: u64, total_ms: u64, style: &LyricsStyle, action: &mut Option<LyricsAction>) {
    if w.path.is_none() {
        ui.label(RichText::new("曲を再生すると、ここに歌詞が出ます。").weak());
        return;
    }
    if w.lyrics.is_empty() {
        if w.loading.is_none() {
            ui.label(RichText::new("この曲の歌詞はありません。").weak());
            ui.label(
                RichText::new("右上の「✏ 編集」から歌詞を入力し、タイムスタンプを打てます（曲と同じフォルダに .lrc として保存されます）。")
                    .weak()
                    .size(11.0),
            );
        }
        return;
    }
    let synced = w.lyrics.is_synced();
    if !synced {
        ui.label(
            RichText::new(if w.lyrics.from_file { "時刻がないため、全文を表示しています。" } else { "曲のタグの歌詞（時刻なし）を表示しています。" })
                .weak()
                .size(11.0),
        );
    }
    let current = if synced { w.lyrics.current_index(pos_ms) } else { None };
    let scroll_now = current != w.last_current;
    w.last_current = current;

    let view_h = ui.available_height();
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(view_h * 0.3);
        ui.vertical_centered(|ui| {
            for (i, l) in w.lyrics.lines.iter().enumerate() {
                let is_cur = current == Some(i);
                let past = current.is_some_and(|c| i < c);
                let shown = if l.text.is_empty() { "　" } else { l.text.as_str() };
                let label = if is_cur {
                    // いまの行: 歌い終えた部分を current、これからの部分を upcoming で描く（sweep が切れていれば全体を current）
                    let done = if style.sweep { w.lyrics.line_progress(i, pos_ms, total_ms) } else { 1.0 };
                    let n = shown.chars().count();
                    let split = shown.char_indices().nth((done * n as f32).round() as usize).map_or(shown.len(), |(b, _)| b);
                    let font = egui::FontId::proportional(24.0);
                    let mut job = egui::text::LayoutJob::default();
                    job.append(&shown[..split], 0.0, egui::TextFormat::simple(font.clone(), style.current));
                    job.append(&shown[split..], 0.0, egui::TextFormat::simple(font, style.upcoming));
                    egui::Label::new(job)
                } else {
                    let color = if past { style.past } else { style.upcoming };
                    egui::Label::new(RichText::new(shown).size(17.0).color(color))
                };
                let resp = ui.add(label.selectable(false).sense(Sense::click()));
                if l.time_ms.is_some() {
                    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                    if resp.clicked() {
                        *action = Some(LyricsAction::Seek(Duration::from_millis(l.time_ms.unwrap_or(0))));
                    }
                    if is_cur && scroll_now {
                        resp.scroll_to_me(Some(Align::Center));
                    }
                }
            }
        });
        ui.add_space(view_h * 0.5);
    });
    // 再生中は、行の切り替わりに追従するため定期的に描き直す
    if synced {
        ui.ctx().request_repaint_after(Duration::from_millis(if style.sweep { 50 } else { 100 }));
    }
}

fn stamp_lines(
    ui: &mut Ui,
    w: &mut LyricsWindow,
    pos_ms: u64,
    same_track: bool,
    state: PlaybackState,
    action: &mut Option<LyricsAction>,
) {
    let ctx = ui.ctx().clone();
    // 時刻を打っている間は、Delete など一部のキーを、メイン画面の操作に使わせない
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("lyrics_keys_active"), true));
    let count = w.lyrics.lines.len();
    if count == 0 {
        ui.label(RichText::new("歌詞がありません。「📝 テキスト編集」で歌詞を入力してください。").weak());
        return;
    }
    w.cursor = w.cursor.min(count - 1);

    // 操作ボタンにフォーカスが残っていると、Enter で押されてしまうので外しておく（文字入力中は触らない）
    if !ctx.wants_keyboard_input() {
        ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
    }

    ui.horizontal_wrapped(|ui| {
        ui.label("打鍵の補正:");
        ui.add(egui::DragValue::new(&mut w.compensate_ms).range(0..=1000).suffix(" ms"))
            .on_hover_text("押すのが遅れる分だけ、早い時刻を記録します（目安 100〜250ms）");
        ui.separator();
        let cur_t = w.lyrics.lines[w.cursor].time_ms;
        if ui.add_enabled(cur_t.is_some(), egui::Button::new("−0.1秒")).on_hover_text("選んだ行の時刻を、0.1 秒早めます").clicked() {
            w.lyrics.lines[w.cursor].time_ms = cur_t.map(|t| t.saturating_sub(100));
        }
        if ui.add_enabled(cur_t.is_some(), egui::Button::new("+0.1秒")).on_hover_text("選んだ行の時刻を、0.1 秒遅らせます").clicked() {
            w.lyrics.lines[w.cursor].time_ms = cur_t.map(|t| t + 100);
        }
        if ui.add_enabled(cur_t.is_some(), egui::Button::new("時刻を消す")).on_hover_text("選んだ行の時刻を消します").clicked() {
            w.lyrics.lines[w.cursor].time_ms = None;
        }
    });
    ui.label(
        RichText::new(
            "再生しながら、歌い出しで Enter：その行に現在の時刻を打って次の行へ ／ Backspace・↑：前の行 ／ ↓：次の行 ／ Delete：時刻を消す ／ Space：再生・一時停止 ／ ダブルクリック：その時刻へ移動",
        )
        .weak()
        .size(11.0),
    );

    // ---- キー操作（文字入力中は無効） ----
    if !ctx.wants_keyboard_input() && !w.lyrics.lines.is_empty() {
        let (enter, back, up, down, del, space) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Backspace),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::Delete),
                i.consume_key(Modifiers::NONE, Key::Space),
            )
        });
        if enter {
            if same_track && state != PlaybackState::Stopped {
                let t = pos_ms.saturating_sub(w.compensate_ms as u64);
                w.lyrics.lines[w.cursor].time_ms = Some(t);
                if w.cursor + 1 < count {
                    w.cursor += 1;
                } else {
                    w.message = Some("最後の行まで打ちました。「💾 保存」で保存できます。".to_string());
                }
            } else {
                w.message = Some("この曲を再生してから Enter を押してください（再生中の曲と編集中の曲が違います）".to_string());
            }
        }
        if back || up {
            w.cursor = w.cursor.saturating_sub(1);
        }
        if down {
            w.cursor = (w.cursor + 1).min(count - 1);
        }
        if del {
            w.lyrics.lines[w.cursor].time_ms = None;
        }
        if space {
            *action = Some(LyricsAction::TogglePause);
        }
    }

    // ---- 行の一覧 ----
    let current = w.lyrics.current_index(pos_ms);
    let cursor = w.cursor;
    let scroll_to_cursor = w.scrolled != Some(cursor);
    w.scrolled = Some(cursor);
    let mut clicked: Option<usize> = None;
    let mut seek: Option<u64> = None;
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for (i, l) in w.lyrics.lines.iter().enumerate() {
            let Line { time_ms, text } = l;
            let is_cursor = i == cursor;
            let fill = if is_cursor {
                crate::theme::c(30, 64, 100)
            } else if current == Some(i) {
                crate::theme::c(24, 40, 64)
            } else {
                Color32::TRANSPARENT
            };
            let inner = egui::Frame::none().fill(fill).inner_margin(egui::Margin::symmetric(6.0, 3.0)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(if is_cursor { "▶" } else { "　" }).color(crate::theme::c(56, 189, 248)));
                    let t = time_ms.map(lyrics::display_time).unwrap_or_else(|| "--:--.--".to_string());
                    let color = if time_ms.is_some() { Color32::from_gray(210) } else { Color32::from_gray(110) };
                    ui.label(RichText::new(t).monospace().color(color));
                    ui.label(RichText::new(if text.is_empty() { "（間奏・空行）" } else { text }).size(15.0));
                });
            });
            let resp = ui.interact(inner.response.rect, ui.id().with(("lyric_row", i)), Sense::click());
            if resp.clicked() {
                clicked = Some(i);
            }
            if resp.double_clicked() {
                seek = *time_ms;
            }
            if is_cursor && scroll_to_cursor {
                inner.response.scroll_to_me(Some(Align::Center));
            }
        }
    });
    if let Some(i) = clicked {
        w.cursor = i;
    }
    if let Some(t) = seek {
        *action = Some(LyricsAction::Seek(Duration::from_millis(t)));
    }
    if state == PlaybackState::Playing {
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

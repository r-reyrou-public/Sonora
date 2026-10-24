use egui::{Context, Event, ImeEvent};

#[derive(Clone, Copy, Default)]
struct ImeState {
    /// 未確定文字列（変換中の下線付き文字）がある
    preedit_active: bool,
    /// 最後に IME イベントを受け取った時刻
    last_event_time: f64,
}

/// 日本語入力などの IME で変換中かどうか。
/// 変換中の Tab（予測変換の選択）では入力欄からフォーカスを動かさず、
/// 変換中でなければ Tab で通常どおりフォーカスを移動させるために使う。
/// 確定と同じフレームに Tab が届く場合も変換中として扱う（直近 0.2 秒以内の IME イベント）。
pub fn ime_composing(ctx: &Context) -> bool {
    let id = egui::Id::new(("ime_state", ctx.viewport_id()));
    let mut st: ImeState = ctx.data(|d| d.get_temp(id)).unwrap_or_default();
    let now = ctx.input(|i| i.time);

    ctx.input(|i| {
        for ev in &i.events {
            if let Event::Ime(ime) = ev {
                st.last_event_time = now;
                match ime {
                    ImeEvent::Preedit(s) => st.preedit_active = !s.is_empty(),
                    ImeEvent::Enabled | ImeEvent::Disabled | ImeEvent::Commit(_) => st.preedit_active = false,
                }
            }
        }
    });
    ctx.data_mut(|d| d.insert_temp(id, st));

    st.preedit_active || now - st.last_event_time < 0.2
}

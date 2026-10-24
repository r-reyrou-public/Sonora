//! 歌詞: LRC 形式（`[分:秒.百分秒]歌詞`）の読み書きと、再生位置に合わせた行の判定。
//! 歌詞は曲と同じフォルダの「曲名.lrc」（UTF-8）に保存する。無ければ、曲のタグに入っている歌詞（時刻なし）を読む。
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::ItemKey;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Line {
    /// 表示を始める時刻（ミリ秒）。None は時刻が未設定
    pub time_ms: Option<u64>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Lyrics {
    pub lines: Vec<Line>,
    /// `[ar:…]` などの情報行（そのまま書き戻す）
    pub meta: Vec<String>,
    /// .lrc ファイルから読んだか（false ならタグの歌詞、または歌詞なし）
    pub from_file: bool,
}

impl Lyrics {
    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(|l| l.text.trim().is_empty() && l.time_ms.is_none())
    }

    /// 時刻が付いている行が1つでもあるか
    pub fn is_synced(&self) -> bool {
        self.lines.iter().any(|l| l.time_ms.is_some())
    }

    /// 再生位置（ミリ秒）に合わせて表示する行。時刻が位置以前の行のうち、最も新しい時刻のもの（同じ時刻なら後ろの行）
    pub fn current_index(&self, pos_ms: u64) -> Option<usize> {
        let mut best: Option<(u64, usize)> = None;
        for (i, l) in self.lines.iter().enumerate() {
            if let Some(t) = l.time_ms {
                if t <= pos_ms && best.is_none_or(|(bt, _)| t >= bt) {
                    best = Some((t, i));
                }
            }
        }
        best.map(|(_, i)| i)
    }

    /// 行 i の中で、時刻がどこまで進んだか（0.0〜1.0）。行の時刻から、次の時刻の行（無ければ曲の終わり）までを 0→1 とする
    pub fn line_progress(&self, i: usize, pos_ms: u64, total_ms: u64) -> f32 {
        let Some(start) = self.lines.get(i).and_then(|l| l.time_ms) else { return 0.0 };
        let end = self.lines.iter().filter_map(|l| l.time_ms).filter(|&t| t > start).min();
        let end = match end {
            Some(e) => e,
            None if total_ms > start => total_ms,
            None => start + 4000,
        };
        (pos_ms.saturating_sub(start) as f32 / (end - start) as f32).clamp(0.0, 1.0)
    }

    /// LRC 形式の文字列にする
    pub fn to_lrc(&self) -> String {
        let mut out = String::new();
        for m in &self.meta {
            out.push_str(m);
            out.push('\n');
        }
        for l in &self.lines {
            if let Some(t) = l.time_ms {
                out.push_str(&format_time(t));
            }
            out.push_str(&l.text);
            out.push('\n');
        }
        out
    }

    /// 編集用に、時刻が付いた行を時刻順に並べ替える（時刻のない行は、直前の行について行く）
    pub fn sort_by_time(&mut self) {
        // 時刻のない行は、直前の時刻付きの行にくっつけて動かす
        let mut groups: Vec<(Option<u64>, Vec<Line>)> = Vec::new();
        for l in self.lines.drain(..) {
            match (l.time_ms, groups.last_mut()) {
                (None, Some((_, g))) => g.push(l),
                _ => groups.push((l.time_ms, vec![l])),
            }
        }
        groups.sort_by_key(|(t, _)| t.unwrap_or(0));
        self.lines = groups.into_iter().flat_map(|(_, g)| g).collect();
    }
}

/// `[mm:ss.xx]`（百分秒）
pub fn format_time(ms: u64) -> String {
    let cs = ms / 10;
    format!("[{:02}:{:02}.{:02}]", cs / 6000, (cs / 100) % 60, cs % 100)
}

/// 画面に出す時刻 `mm:ss.xx`
pub fn display_time(ms: u64) -> String {
    let t = format_time(ms);
    t.trim_matches(|c| c == '[' || c == ']').to_string()
}

/// `mm:ss`、`mm:ss.x`、`mm:ss.xx`、`mm:ss.xxx`、`mm:ss:xx` を読む
fn parse_stamp(s: &str) -> Option<u64> {
    let (m, rest) = s.split_once(':')?;
    let m: u64 = m.trim().parse().ok()?;
    let rest = rest.trim();
    let (sec, frac) = match rest.split_once(['.', ':']) {
        Some((a, b)) => (a, b),
        None => (rest, ""),
    };
    let sec: u64 = sec.parse().ok()?;
    if sec >= 100 {
        return None;
    }
    let frac_ms = if frac.is_empty() {
        0
    } else if frac.chars().all(|c| c.is_ascii_digit()) {
        // 1桁=100ms、2桁=10ms、3桁=1ms（それ以上は先頭3桁）
        let f: String = frac.chars().take(3).collect();
        let v: u64 = f.parse().ok()?;
        v * 10u64.pow(3 - f.len() as u32)
    } else {
        return None;
    };
    Some(m * 60_000 + sec * 1000 + frac_ms)
}

/// LRC 形式（または時刻のない普通の歌詞）の文字列を読む
pub fn parse_lrc(text: &str) -> Lyrics {
    let mut lyr = Lyrics { from_file: true, ..Default::default() };
    let mut offset_ms: i64 = 0;
    let text = text.trim_start_matches('\u{feff}');
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        let mut rest = line.trim_start();
        let mut times: Vec<u64> = Vec::new();
        // 行頭の [時刻] を（複数あれば全部）読む
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(end) = stripped.find(']') else { break };
            match parse_stamp(&stripped[..end]) {
                Some(t) => {
                    times.push(t);
                    rest = &stripped[end + 1..];
                }
                None => break,
            }
        }
        if times.is_empty() {
            // [ar:歌手] [offset:500] などの情報行
            if let Some(inner) = rest.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                if let Some((key, val)) = inner.split_once(':') {
                    let key = key.trim().to_ascii_lowercase();
                    if matches!(key.as_str(), "ar" | "ti" | "al" | "au" | "by" | "re" | "ve" | "length" | "offset" | "la" | "lr") {
                        if key == "offset" {
                            offset_ms = val.trim().parse().unwrap_or(0);
                        } else {
                            lyr.meta.push(rest.to_string());
                        }
                        continue;
                    }
                }
            }
            lyr.lines.push(Line { time_ms: None, text: line.to_string() });
            continue;
        }
        let body = strip_word_tags(rest);
        for t in times {
            // offset は「歌詞を早める」向き（正の値で表示が早くなる）
            let adj = (t as i64 - offset_ms).max(0) as u64;
            lyr.lines.push(Line { time_ms: Some(adj), text: body.clone() });
        }
    }
    // 末尾の空行を落とす
    while lyr.lines.last().is_some_and(|l| l.time_ms.is_none() && l.text.trim().is_empty()) {
        lyr.lines.pop();
    }
    // 1つの行に複数の時刻があった場合などのため、時刻順に整える
    if lyr.is_synced() {
        lyr.sort_by_time();
    }
    lyr
}

/// 拡張 LRC の単語ごとの時刻 `<mm:ss.xx>` を取り除く
fn strip_word_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('<') {
        let after = &rest[i + 1..];
        match after.find('>') {
            Some(j) if parse_stamp(&after[..j]).is_some() => {
                out.push_str(&rest[..i]);
                rest = &after[j + 1..];
            }
            _ => {
                out.push_str(&rest[..=i]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.trim_end().to_string()
}

/// 曲に対応する .lrc のパス
pub fn lrc_path(audio: &Path) -> PathBuf {
    audio.with_extension("lrc")
}

/// 歌詞を読む。.lrc があればそれ、無ければ曲のタグの歌詞（時刻なし）。どちらも無ければ空
pub fn load(audio: &Path) -> Lyrics {
    // 歌詞ファイルが大きすぎるときは読まない（誤って別の大きなファイルを開かないため）
    const MAX_BYTES: u64 = 4 * 1024 * 1024;
    let lrc = lrc_path(audio);
    if let Ok(meta) = std::fs::metadata(&lrc) {
        if meta.len() <= MAX_BYTES {
            if let Ok(bytes) = std::fs::read(&lrc) {
                return parse_lrc(&String::from_utf8_lossy(&bytes));
            }
        }
    }
    if let Ok(tagged) = Probe::open(audio).and_then(|p| p.read()) {
        if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
            if let Some(text) = tag.get_string(&ItemKey::Lyrics) {
                let mut l = parse_lrc(text);
                l.from_file = false;
                return l;
            }
        }
    }
    Lyrics::default()
}

/// .lrc に保存する（UTF-8）。中身が空なら、既存の .lrc は消さずに何もしない
pub fn save(audio: &Path, lyrics: &Lyrics) -> Result<(), String> {
    let path = lrc_path(audio);
    if lyrics.is_empty() {
        return Err("歌詞が空のため保存しませんでした".to_string());
    }
    std::fs::write(&path, lyrics.to_lrc()).map_err(|e| format!("保存できませんでした: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_and_multi_stamps() {
        let l = parse_lrc("\u{feff}[ar:Someone]\n[00:01.50]hello\n[00:10.5][00:05]world <00:05.5>x\nplain\n");
        assert_eq!(l.meta, ["[ar:Someone]"]);
        // 時刻順に並ぶ: 1.5s hello, 5s world x, （時刻なし plain は直前の行について行く）, 10.5s world x
        assert_eq!(l.lines[0], Line { time_ms: Some(1500), text: "hello".into() });
        assert_eq!(l.lines[1].time_ms, Some(5000));
        assert_eq!(l.lines[1].text, "world x");
        assert_eq!(l.lines[2], Line { time_ms: None, text: "plain".into() });
        assert_eq!(l.lines[3].time_ms, Some(10500));
    }

    #[test]
    fn offset_and_formats() {
        let l = parse_lrc("[offset:500]\n[00:02.25]a\n[00:03:10]b\n[01:00.123]c\n");
        assert_eq!(l.lines[0].time_ms, Some(1750));
        assert_eq!(l.lines[1].time_ms, Some(2600));
        assert_eq!(l.lines[2].time_ms, Some(59623));
        assert_eq!(format_time(61_230), "[01:01.23]");
        assert_eq!(display_time(5_000), "00:05.00");
    }

    #[test]
    fn current_line_follows_position() {
        let l = parse_lrc("[00:01.00]a\n[00:05.00]b\n[00:09.00]c\n");
        assert_eq!(l.current_index(0), None);
        assert_eq!(l.current_index(1000), Some(0));
        assert_eq!(l.current_index(7000), Some(1));
        assert_eq!(l.current_index(99_000), Some(2));
    }

    #[test]
    fn line_progress_runs_to_next_stamp() {
        let l = parse_lrc("[00:10.00]a\n[00:20.00]b\n");
        assert_eq!(l.line_progress(0, 10_000, 60_000), 0.0);
        assert!((l.line_progress(0, 15_000, 60_000) - 0.5).abs() < 1e-6);
        assert_eq!(l.line_progress(0, 25_000, 60_000), 1.0);
        // 最後の行は、曲の終わりまで
        assert!((l.line_progress(1, 40_000, 60_000) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn roundtrip_and_plain_text() {
        let src = "[ti:t]\n[00:01.00]one\n[00:02.50]\n[00:03.00]three\n";
        let l = parse_lrc(src);
        assert_eq!(l.to_lrc(), src);
        assert_eq!(parse_lrc(&l.to_lrc()), l);

        let plain = parse_lrc("a\n\nb\n");
        assert!(!plain.is_synced());
        assert_eq!(plain.lines.len(), 3);
        assert!(Lyrics::default().is_empty());
    }

    #[test]
    fn save_and_load_sidecar() {
        let dir = std::env::temp_dir().join(format!("lyr_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let audio = dir.join("song.mp3");
        std::fs::write(&audio, b"not audio").unwrap();
        let l = parse_lrc("[00:01.00]x\n");
        save(&audio, &l).unwrap();
        assert_eq!(load(&audio), l);
        assert!(save(&audio, &Lyrics::default()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

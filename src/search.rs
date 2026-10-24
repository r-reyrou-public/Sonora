//! 検索クエリの解析とマッチング。構文の説明はヘルプ画面 (views/help.rs) を参照。
use crate::scanner::metadata::TrackItem;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Any,
    FileName,
    Title,
    Artist,
    Album,
    AlbumArtist,
    Genre,
    Year,
    TrackNo,
    Format,
    Label,
    Path,
}

impl Field {
    fn parse(name: &str) -> Option<Field> {
        Some(match name {
            "file" | "filename" | "f" => Field::FileName,
            "title" | "t" => Field::Title,
            "artist" | "a" => Field::Artist,
            "album" => Field::Album,
            "albumartist" | "aa" => Field::AlbumArtist,
            "genre" | "g" => Field::Genre,
            "year" | "y" => Field::Year,
            "track" | "no" => Field::TrackNo,
            "format" | "ext" => Field::Format,
            "label" | "tag" | "l" => Field::Label,
            "path" | "dir" => Field::Path,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug)]
enum Matcher {
    /// 小文字化済みの部分一致
    Text(String),
    /// 数値の範囲 (両端含む)
    Range(u32, u32),
    /// `no:フィールド` — そのフィールドが未設定
    Missing,
}

#[derive(Clone, Debug)]
struct Term {
    field: Field,
    matcher: Matcher,
    negate: bool,
}

/// OR で結んだ AND グループの集合
#[derive(Clone, Debug, Default)]
pub struct SearchQuery {
    groups: Vec<Vec<Term>>,
}

impl SearchQuery {
    pub fn parse(input: &str) -> Self {
        let mut groups: Vec<Vec<Term>> = vec![Vec::new()];

        for token in tokenize(input) {
            if token == "OR" || token == "|" {
                if !groups.last().is_none_or(|g| g.is_empty()) {
                    groups.push(Vec::new());
                }
                continue;
            }
            if let Some(term) = parse_term(&token) {
                groups.last_mut().unwrap().push(term);
            }
        }

        groups.retain(|g| !g.is_empty());
        Self { groups }
    }

    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    pub fn matches(&self, t: &TrackItem) -> bool {
        self.is_empty() || self.groups.iter().any(|g| g.iter().all(|term| term_matches(term, t)))
    }
}

/// 空白区切り。`"..."` で囲むと空白を含められる (`artist:"foo bar"` も可)
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    // 引用符で囲まれた(空でも)トークンかどうか
    let mut had_quote = false;

    for ch in input.chars() {
        match ch {
            '"' => {
                in_quote = !in_quote;
                had_quote = true;
            }
            c if c.is_whitespace() && !in_quote => {
                if !cur.is_empty() || had_quote {
                    tokens.push(std::mem::take(&mut cur));
                }
                had_quote = false;
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() || had_quote {
        tokens.push(cur);
    }
    tokens.into_iter().filter(|t| !t.is_empty()).collect()
}

fn parse_term(token: &str) -> Option<Term> {
    let (negate, body) = match token.strip_prefix('-') {
        Some(rest) if !rest.is_empty() => (true, rest),
        _ => (false, token),
    };

    if let Some((name, value)) = body.split_once(':') {
        let name_l = name.to_lowercase();

        if name_l == "no" {
            // no:artist → アーティスト未設定
            let field = Field::parse(&value.to_lowercase())?;
            return Some(Term { field, matcher: Matcher::Missing, negate });
        }

        if let Some(field) = Field::parse(&name_l) {
            if value.is_empty() {
                return None;
            }
            let matcher = if matches!(field, Field::Year | Field::TrackNo) {
                parse_range(value).map(|(a, b)| Matcher::Range(a, b)).unwrap_or_else(|| Matcher::Text(value.to_lowercase()))
            } else {
                Matcher::Text(value.to_lowercase())
            };
            return Some(Term { field, matcher, negate });
        }
        // 未知のフィールド名 (例: "C:\music") はそのまま通常の文字列として扱う
    }

    Some(Term { field: Field::Any, matcher: Matcher::Text(body.to_lowercase()), negate })
}

/// `2020` / `2010..2020` / `>2000` / `>=2000` / `<2000` / `<=2000`
fn parse_range(v: &str) -> Option<(u32, u32)> {
    if let Some((a, b)) = v.split_once("..") {
        let lo = if a.is_empty() { 0 } else { a.parse().ok()? };
        let hi = if b.is_empty() { u32::MAX } else { b.parse().ok()? };
        return Some((lo, hi));
    }
    if let Some(n) = v.strip_prefix(">=") {
        return Some((n.parse().ok()?, u32::MAX));
    }
    if let Some(n) = v.strip_prefix("<=") {
        return Some((0, n.parse().ok()?));
    }
    if let Some(n) = v.strip_prefix('>') {
        return Some((n.parse::<u32>().ok()?.saturating_add(1), u32::MAX));
    }
    if let Some(n) = v.strip_prefix('<') {
        return Some((0, n.parse::<u32>().ok()?.checked_sub(1)?));
    }
    let n: u32 = v.parse().ok()?;
    Some((n, n))
}

fn contains(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(needle)
}

fn term_matches(term: &Term, t: &TrackItem) -> bool {
    let hit = match (&term.matcher, term.field) {
        (Matcher::Missing, f) => match f {
            Field::FileName => t.filename.is_empty(),
            Field::Title => t.tag_title.is_empty(),
            Field::Artist => t.artist.is_empty(),
            Field::Album => t.album.is_empty(),
            Field::AlbumArtist => t.album_artist.is_empty(),
            Field::Genre => t.genre.is_empty(),
            Field::Year => t.year == 0,
            Field::TrackNo => t.track_number == 0,
            Field::Label => t.custom_tags.is_empty(),
            Field::Format | Field::Path | Field::Any => false,
        },
        (Matcher::Range(lo, hi), Field::Year) => t.year > 0 && (*lo..=*hi).contains(&t.year),
        (Matcher::Range(lo, hi), Field::TrackNo) => t.track_number > 0 && (*lo..=*hi).contains(&t.track_number),
        (Matcher::Range(..), _) => false,
        (Matcher::Text(s), f) => match f {
            Field::Any => {
                contains(&t.filename, s)
                    || contains(&t.tag_title, s)
                    || contains(&t.artist, s)
                    || contains(&t.album, s)
                    || contains(&t.album_artist, s)
                    || contains(&t.genre, s)
                    || contains(&t.format, s)
                    || t.custom_tags.iter().any(|l| contains(l, s))
            }
            Field::FileName => contains(&t.filename, s),
            Field::Title => contains(&t.tag_title, s),
            Field::Artist => contains(&t.artist, s),
            Field::Album => contains(&t.album, s),
            Field::AlbumArtist => contains(&t.album_artist, s),
            Field::Genre => contains(&t.genre, s),
            Field::Year => t.year.to_string().contains(s.as_str()),
            Field::TrackNo => t.track_number.to_string().contains(s.as_str()),
            Field::Format => contains(&t.format, s),
            Field::Label => t.custom_tags.iter().any(|l| contains(l, s)),
            Field::Path => contains(&t.file_path.to_string_lossy(), s),
        },
    };
    hit != term.negate
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;

    fn track(file: &str, title: &str, artist: &str, year: u32, labels: &[&str]) -> TrackItem {
        TrackItem {
            file_path: PathBuf::from(format!("C:/m/{}", file)),
            filename: file.to_string(),
            title: if title.is_empty() { file.to_string() } else { title.to_string() },
            tag_title: title.to_string(),
            artist: artist.to_string(),
            album: String::new(),
            album_artist: String::new(),
            genre: String::new(),
            year,
            track_number: 0,
            duration: Duration::ZERO,
            formatted_duration: String::new(),
            format: file.rsplit('.').next().unwrap_or("").to_uppercase(),
            file_size_bytes: 0,
            formatted_size: String::new(),
            artwork_bytes: None,
            custom_tags: labels.iter().map(|s| s.to_string()).collect(),
            has_lyrics: false,
        }
    }

    #[test]
    fn syntax() {
        let a = track("a.mp3", "Hello World", "Foo Bar", 2015, &["fav"]);
        let b = track("b.flac", "", "Baz", 0, &[]);
        let m = |q: &str, t: &TrackItem| SearchQuery::parse(q).matches(t);

        assert!(m("", &a));
        assert!(m("hello", &a) && !m("hello", &b));
        assert!(m("hello foo", &a) && !m("hello baz", &a));
        assert!(m("hello OR baz", &b));
        assert!(m("-hello", &b) && !m("-hello", &a));
        assert!(m("artist:\"foo bar\"", &a));
        assert!(m("title:hello artist:foo", &a));
        assert!(!m("title:foo", &a));
        assert!(m("year:2010..2020", &a) && !m("year:2010..2020", &b));
        assert!(m("year:>=2015", &a) && !m("year:<2015", &a));
        assert!(m("label:fav", &a) && !m("label:fav", &b));
        assert!(m("no:title", &b) && !m("no:title", &a));
        assert!(m("no:year", &b));
        assert!(m("ext:flac", &b));
    }
}

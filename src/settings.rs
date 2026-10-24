//! アプリ設定。`settings.ini` は C# 版と同じセクション名・キー名・値の書式で読み書きする
//! （[General] [State] [Window] [Cache] [Playback] [View] [Backup] [Favorites]）。
//! Rust 版だけの項目は [Rust] セクションに保存する。
use crate::audio::hotkeys_defs::{default_bindings, format_combos, parse_combos, Binding, HotkeyAction};
use crate::views::columns::ColumnConfig;
use std::path::Path;

pub const DEFAULT_SCROLL_SPEED: f32 = 40.0;

macro_rules! string_enum {
    ($name:ident { $($variant:ident),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name { $($variant),+ }
        impl $name {
            pub fn as_str(self) -> &'static str {
                match self { $($name::$variant => stringify!($variant)),+ }
            }
            pub fn parse(s: &str) -> Option<Self> {
                let s = s.trim();
                $(if s.eq_ignore_ascii_case(stringify!($variant)) { return Some($name::$variant); })+
                None
            }
        }
    };
}

string_enum!(AppThemeMode { System, Light, Dark });
string_enum!(AudioOutputEngine { Auto, Wasapi, DirectSound, WaveOut });
string_enum!(TagDisplayMode { Both, Id3Only, CustomOnly });
string_enum!(LoudnessMode { Off, ReplayGain, Auto });

#[derive(Clone, Debug)]
pub struct AppSettings {
    // [General]
    pub theme: AppThemeMode,
    pub audio_engine: AudioOutputEngine,
    pub tag_display_mode: TagDisplayMode,
    pub volume: f32, // 0.0 - 1.0（ini では 0-100 の整数）
    pub waveform_secs: u32, // 3-120
    // [State]
    pub last_folder: String,
    pub left_panel_width: f32,
    /// 歌詞ペインの幅（0 は未調整。メインペインの半分を初期値にする）
    pub lyrics_pane_width: f32,
    // [Window]
    pub window_width: f32,
    pub window_height: f32,
    pub window_left: f32,
    pub window_top: f32,
    pub is_maximized: bool,
    // [Cache]
    pub enable_cache: bool,
    pub max_cache_tracks: u32, // 0 = 無制限
    // [Playback]
    pub fade_ms: u32,
    pub crossfade_ms: u32,
    pub loudness_mode: LoudnessMode,
    pub enable_global_hotkeys: bool,
    /// ホットキーの割り当て（[Hotkeys]）
    pub hotkeys: Vec<Binding>,
    // [View]
    pub include_subfolders: bool,
    pub mini_player_topmost: bool,
    pub mini_player_left: f32,
    pub mini_player_top: f32,
    // [Backup]
    pub enable_auto_backup: bool,
    // [Favorites]
    pub favorites: Vec<(String, String)>, // (名前, パス)
    // [Rust]（Rust 版のみ）
    pub columns: ColumnConfig,
    pub playlist_columns: ColumnConfig,
    pub scroll_speed: f32,
    /// 新しい版の確認先（GitHub の owner/repo）と、起動時に確認するか
    pub update_repo: String,
    pub check_updates: bool,
    /// 歌詞の色（いまの行・歌い終えた行・これから歌う行）と、行の中で色を進めるか
    pub lyrics_color_current: [u8; 3],
    pub lyrics_color_past: [u8; 3],
    pub lyrics_color_upcoming: [u8; 3],
    pub lyrics_sweep: bool,
    /// データ（ラベル・キャッシュ）の取りに行き先フォルダ。直下にデータがないときだけ使う
    pub data_folder: String,
    /// 次回の起動時に行う移動作業（"copy" / "use" / "return"。起動時に paths が実行して消す）
    pub data_folder_move: String,
    /// 列の幅（"m.filename" = メイン一覧, "p.filename" = プレイリストペイン）
    pub column_widths: std::collections::BTreeMap<String, f32>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: AppThemeMode::System,
            audio_engine: AudioOutputEngine::Auto,
            tag_display_mode: TagDisplayMode::Both,
            volume: 0.8,
            waveform_secs: 15,
            last_folder: String::new(),
            left_panel_width: 320.0,
            lyrics_pane_width: 0.0,
            window_width: 1260.0,
            window_height: 800.0,
            window_left: f32::NAN,
            window_top: f32::NAN,
            is_maximized: false,
            enable_cache: true,
            max_cache_tracks: 5000,
            fade_ms: 0,
            crossfade_ms: 0,
            loudness_mode: LoudnessMode::Off,
            enable_global_hotkeys: true,
            hotkeys: default_bindings(),
            include_subfolders: false,
            mini_player_topmost: true,
            mini_player_left: f32::NAN,
            mini_player_top: f32::NAN,
            enable_auto_backup: true,
            favorites: Vec::new(),
            columns: ColumnConfig::default(),
            playlist_columns: ColumnConfig::playlist_default(),
            scroll_speed: DEFAULT_SCROLL_SPEED,
            update_repo: crate::update::DEFAULT_REPO.to_string(),
            check_updates: false,
            lyrics_color_current: LYRICS_COLOR_CURRENT,
            lyrics_color_past: LYRICS_COLOR_PAST,
            lyrics_color_upcoming: LYRICS_COLOR_UPCOMING,
            lyrics_sweep: true,
            data_folder: String::new(),
            data_folder_move: String::new(),
            column_widths: Default::default(),
        }
    }
}

// ---------------- C# の IniFileService 相当 ----------------

/// セクション順・キー順を保つ簡易 INI（エスケープなし・UTF-8。C# 版と同じ）
#[derive(Default)]
pub struct IniFile {
    sections: Vec<(String, Vec<(String, String)>)>,
}

impl IniFile {
    pub fn load(path: &Path) -> Self {
        let mut ini = IniFile::default();
        let Ok(bytes) = std::fs::read(path) else { return ini };
        let text = String::from_utf8_lossy(&bytes);
        let mut current = "General".to_string();
        for raw in text.lines() {
            let line = raw.trim().trim_start_matches('\u{feff}');
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                current = line[1..line.len() - 1].trim().to_string();
                ini.section_mut(&current);
            } else if let Some(idx) = line.find('=') {
                if idx > 0 {
                    let (k, v) = (line[..idx].trim(), line[idx + 1..].trim());
                    ini.set(&current, k, v);
                }
            }
        }
        ini
    }

    fn section_mut(&mut self, name: &str) -> &mut Vec<(String, String)> {
        if let Some(i) = self.sections.iter().position(|(n, _)| n.eq_ignore_ascii_case(name)) {
            return &mut self.sections[i].1;
        }
        self.sections.push((name.to_string(), Vec::new()));
        &mut self.sections.last_mut().unwrap().1
    }

    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.sections
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(section))?
            .1
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }

    pub fn set(&mut self, section: &str, key: &str, value: &str) {
        // 改行が混ざると INI の構造が壊れるため除去
        let value: String = value.chars().filter(|c| *c != '\r' && *c != '\n').collect();
        let sec = self.section_mut(section);
        if let Some(entry) = sec.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
            entry.1 = value;
        } else {
            sec.push((key.to_string(), value));
        }
    }

    pub fn remove(&mut self, section: &str, key: &str) {
        self.section_mut(section).retain(|(k, _)| !k.eq_ignore_ascii_case(key));
    }

    pub fn clear_section(&mut self, section: &str) {
        self.section_mut(section).clear();
    }

    fn get_f32(&self, section: &str, key: &str) -> Option<f32> {
        // NaN / inf が入っていても、各設定の範囲チェックをすり抜けないよう、有限の数だけを受け付ける
        self.get(section, key)?.parse::<f32>().ok().filter(|v| v.is_finite())
    }
    fn get_u32(&self, section: &str, key: &str) -> Option<u32> {
        self.get(section, key)?.parse::<u32>().ok()
    }
    fn get_bool(&self, section: &str, key: &str) -> Option<bool> {
        let v = self.get(section, key)?;
        if v.eq_ignore_ascii_case("true") {
            Some(true)
        } else if v.eq_ignore_ascii_case("false") {
            Some(false)
        } else {
            None
        }
    }

    pub fn save(&self, path: &Path) {
        let mut out = String::new();
        out.push_str("; =========================================\n");
        out.push_str("; 音楽管理ツール 設定ファイル (settings.ini)\n");
        out.push_str("; =========================================\n\n");
        for (name, entries) in &self.sections {
            if entries.is_empty() {
                continue;
            }
            out.push_str(&format!("[{}]\n", name));
            for (k, v) in entries {
                out.push_str(&format!("{}={}\n", k, v));
            }
            out.push('\n');
        }
        // 書き込み途中の強制終了で破損しないよう、一時ファイル経由で置き換える
        let tmp = path.with_extension("ini.tmp");
        if std::fs::write(&tmp, out.as_bytes()).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

/// 以前の Rust 版 (rust-ini) は `\` を `\` に二重化して保存していたので、読み込み時に元へ戻す。
/// （先頭の `\` は UNC パスなので残す）
fn unescape_backslashes(s: &str) -> String {
    let (head, tail) = if s.starts_with("\\\\") { s.split_at(2) } else { ("", s) };
    format!("{}{}", head, tail.replace("\\\\", "\\"))
}

fn fmt_f1(v: f32) -> String {
    format!("{:.1}", v)
}

impl AppSettings {
    pub fn load(ini_path: &Path) -> Self {
        let ini = IniFile::load(ini_path);
        let mut s = Self::default();

        // [General]
        if let Some(v) = ini.get("General", "Theme").and_then(AppThemeMode::parse) {
            s.theme = v;
        }
        if let Some(v) = ini.get("General", "AudioEngine").and_then(AudioOutputEngine::parse) {
            s.audio_engine = v;
        }
        if let Some(v) = ini.get("General", "TagDisplayMode").and_then(TagDisplayMode::parse) {
            s.tag_display_mode = v;
        }
        if let Some(v) = ini.get_f32("General", "Volume") {
            s.volume = (v / 100.0).clamp(0.0, 1.0);
        }
        if let Some(v) = ini.get_u32("General", "WaveformDurationSeconds") {
            s.waveform_secs = v.clamp(3, 120);
        }

        // [State]
        if let Some(v) = ini.get("State", "LastFolder") {
            let folder = unescape_backslashes(v);
            // 起動時に自動で開くので、ネットワーク共有は受け付けない（外部への自動接続を防ぐ）
            if !crate::paths::is_unc(&folder) {
                s.last_folder = folder;
            }
        }
        if let Some(v) = ini.get_f32("State", "LeftPanelWidth") {
            s.left_panel_width = v.clamp(240.0, 600.0);
        }
        if let Some(v) = ini.get_f32("State", "LyricsPaneWidth") {
            s.lyrics_pane_width = if v > 0.0 { v.clamp(200.0, 4000.0) } else { 0.0 };
        }

        // [Window]
        if let Some(v) = ini.get_f32("Window", "Width") {
            s.window_width = v.clamp(800.0, 8000.0);
        }
        if let Some(v) = ini.get_f32("Window", "Height") {
            s.window_height = v.clamp(500.0, 6000.0);
        }
        if let Some(v) = ini.get_f32("Window", "Left") {
            s.window_left = v;
        }
        if let Some(v) = ini.get_f32("Window", "Top") {
            s.window_top = v;
        }
        if let Some(v) = ini.get_bool("Window", "IsMaximized") {
            s.is_maximized = v;
        }

        // [Cache]
        if let Some(v) = ini.get_bool("Cache", "EnableCache") {
            s.enable_cache = v;
        }
        if let Some(v) = ini.get_u32("Cache", "MaxCacheTracks") {
            s.max_cache_tracks = v;
        }

        // [Playback]
        if let Some(v) = ini.get_u32("Playback", "FadeMs") {
            s.fade_ms = v.min(10_000);
        }
        if let Some(v) = ini.get_u32("Playback", "CrossfadeMs") {
            s.crossfade_ms = v.min(15_000);
        }
        if let Some(v) = ini.get("Playback", "LoudnessMode").and_then(LoudnessMode::parse) {
            s.loudness_mode = v;
        }
        if let Some(v) = ini.get_bool("Playback", "EnableGlobalHotkeys") {
            s.enable_global_hotkeys = v;
        }
        // [Hotkeys]: 書かれていない操作は既定のまま、空にしてあれば割り当て無し
        let mut hotkeys = Vec::new();
        for action in HotkeyAction::ALL {
            match ini.get("Hotkeys", action.ini_key()) {
                Some(v) => hotkeys.extend(parse_combos(v).into_iter().map(|combo| Binding { action, combo })),
                None => hotkeys.extend(default_bindings().into_iter().filter(|b| b.action == action)),
            }
        }
        s.hotkeys = hotkeys;

        // [View]
        if let Some(v) = ini.get_bool("View", "IncludeSubfolders") {
            s.include_subfolders = v;
        }
        if let Some(v) = ini.get_bool("View", "MiniPlayerTopmost") {
            s.mini_player_topmost = v;
        }
        if let Some(v) = ini.get_f32("View", "MiniPlayerLeft") {
            s.mini_player_left = v;
        }
        if let Some(v) = ini.get_f32("View", "MiniPlayerTop") {
            s.mini_player_top = v;
        }

        // [Backup]
        if let Some(v) = ini.get_bool("Backup", "EnableAutoBackup") {
            s.enable_auto_backup = v;
        }

        // [Favorites]: Count + Fav_{i} = "名前|パス"（パスには '|' が入らないので最後の '|' で分ける）
        // 巨大な件数で起動が止まらないよう上限を設ける
        let count = ini.get_u32("Favorites", "Count").unwrap_or(0).min(5_000);
        for i in 0..count {
            if let Some(val) = ini.get("Favorites", &format!("Fav_{}", i)) {
                if val.trim().is_empty() {
                    continue;
                }
                match val.rfind('|') {
                    Some(idx) => s.favorites.push((val[..idx].to_string(), unescape_backslashes(&val[idx + 1..]))),
                    None => {
                        let name = Path::new(val).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                        s.favorites.push((name, val.to_string()));
                    }
                }
            }
        }

        // [Rust]（旧バージョンは [State] に置いていた）
        // 古い [State] を先に読み、新しい [Rust] で上書きする（逆だと古い値が新しい設定を打ち消す）
        for sec in ["State", "Rust"] {
            if let Some(v) = ini.get(sec, "Columns") {
                s.columns = ColumnConfig::parse(v);
            }
            if let Some(v) = ini.get(sec, "PlaylistColumns") {
                s.playlist_columns = ColumnConfig::parse(v);
            }
            if let Some(v) = ini.get_f32(sec, "ScrollSpeed") {
                s.scroll_speed = v.clamp(5.0, 400.0);
            }
        }
        // 設定ファイルは書き換えられうるので、確認先は形が正しいものだけ使う
        if let Some(v) = ini.get("Rust", "UpdateRepo") {
            if v.trim().eq_ignore_ascii_case(crate::update::OLD_DEFAULT_REPO) {
                s.update_repo = crate::update::DEFAULT_REPO.to_string();
            } else if crate::update::valid_repo(v.trim()) {
                s.update_repo = v.trim().to_string();
            }
        }
        for (key, slot) in [
            ("CurrentColor", &mut s.lyrics_color_current),
            ("PastColor", &mut s.lyrics_color_past),
            ("UpcomingColor", &mut s.lyrics_color_upcoming),
        ] {
            if let Some(c) = ini.get("Lyrics", key).and_then(parse_hex_color) {
                *slot = c;
            }
        }
        if let Some(v) = ini.get_bool("Lyrics", "Sweep") {
            s.lyrics_sweep = v;
        }
        if let Some(v) = ini.get_bool("Rust", "CheckUpdates") {
            s.check_updates = v;
        }
        if let Some(v) = ini.get("Rust", "DataFolder") {
            s.data_folder = v.trim().to_string();
        }
        if let Some(v) = ini.get("Rust", "DataFolderMove") {
            s.data_folder_move = v.trim().to_string();
        }

        if let Some(v) = ini.get("Rust", "ColumnWidths") {
            for part in v.split(',') {
                if let Some((k, w)) = part.split_once('=') {
                    if let Ok(w) = w.trim().parse::<f32>() {
                        s.column_widths.insert(k.trim().to_string(), w.clamp(20.0, 2000.0));
                    }
                }
            }
        }

        s
    }

    /// 他のセクション（C# 版が書いた未知の項目など）を消さないよう、既存ファイルを読み直して上書きマージする
    pub fn save(&self, ini_path: &Path) {
        let mut ini = IniFile::load(ini_path);

        ini.set("General", "Theme", self.theme.as_str());
        ini.set("General", "AudioEngine", self.audio_engine.as_str());
        ini.set("General", "TagDisplayMode", self.tag_display_mode.as_str());
        ini.set("General", "Volume", &format!("{}", (self.volume * 100.0).round() as i32));
        ini.set("General", "WaveformDurationSeconds", &self.waveform_secs.to_string());

        ini.set("State", "LastFolder", &self.last_folder);
        ini.set("State", "LeftPanelWidth", &fmt_f1(self.left_panel_width));
        if self.lyrics_pane_width > 0.0 {
            ini.set("State", "LyricsPaneWidth", &fmt_f1(self.lyrics_pane_width));
        }

        ini.set("Window", "Width", &fmt_f1(self.window_width));
        ini.set("Window", "Height", &fmt_f1(self.window_height));
        if !self.window_left.is_nan() {
            ini.set("Window", "Left", &fmt_f1(self.window_left));
        }
        if !self.window_top.is_nan() {
            ini.set("Window", "Top", &fmt_f1(self.window_top));
        }
        ini.set("Window", "IsMaximized", if self.is_maximized { "True" } else { "False" });

        ini.set("Cache", "EnableCache", if self.enable_cache { "True" } else { "False" });
        ini.set("Cache", "MaxCacheTracks", &self.max_cache_tracks.to_string());

        ini.set("Playback", "FadeMs", &self.fade_ms.to_string());
        ini.set("Playback", "CrossfadeMs", &self.crossfade_ms.to_string());
        ini.set("Playback", "LoudnessMode", self.loudness_mode.as_str());
        ini.set("Playback", "EnableGlobalHotkeys", if self.enable_global_hotkeys { "True" } else { "False" });
        for action in HotkeyAction::ALL {
            ini.set("Hotkeys", action.ini_key(), &format_combos(&self.hotkeys, action));
        }

        ini.set("View", "IncludeSubfolders", if self.include_subfolders { "True" } else { "False" });
        ini.set("View", "MiniPlayerTopmost", if self.mini_player_topmost { "True" } else { "False" });
        if !self.mini_player_left.is_nan() {
            ini.set("View", "MiniPlayerLeft", &fmt_f1(self.mini_player_left));
        }
        if !self.mini_player_top.is_nan() {
            ini.set("View", "MiniPlayerTop", &fmt_f1(self.mini_player_top));
        }

        ini.set("Backup", "EnableAutoBackup", if self.enable_auto_backup { "True" } else { "False" });

        ini.clear_section("Favorites");
        let mut index = 0;
        for (name, path) in &self.favorites {
            if path.trim().is_empty() {
                continue;
            }
            let name = if name.trim().is_empty() {
                Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
            } else {
                name.clone()
            };
            ini.set("Favorites", &format!("Fav_{}", index), &format!("{}|{}", name, path));
            index += 1;
        }
        ini.set("Favorites", "Count", &index.to_string());

        // 旧バージョンが [State] に書いていた Rust 版の項目は、[Rust] へ移したので消す
        for key in ["Columns", "PlaylistColumns", "ScrollSpeed", "Volume", "IncludeSubfolders"] {
            ini.remove("State", key);
        }
        ini.set("Rust", "Columns", &self.columns.serialize());
        ini.set("Rust", "PlaylistColumns", &self.playlist_columns.serialize());
        ini.set("Rust", "ScrollSpeed", &format!("{:.0}", self.scroll_speed));
        if crate::update::valid_repo(&self.update_repo) {
            ini.set("Rust", "UpdateRepo", &self.update_repo);
        }
        ini.set("Rust", "CheckUpdates", if self.check_updates { "true" } else { "false" });
        ini.set("Lyrics", "CurrentColor", &hex_color(self.lyrics_color_current));
        ini.set("Lyrics", "PastColor", &hex_color(self.lyrics_color_past));
        ini.set("Lyrics", "UpcomingColor", &hex_color(self.lyrics_color_upcoming));
        ini.set("Lyrics", "Sweep", if self.lyrics_sweep { "true" } else { "false" });
        for (key, value) in [("DataFolder", &self.data_folder), ("DataFolderMove", &self.data_folder_move)] {
            if value.is_empty() {
                ini.remove("Rust", key);
            } else {
                ini.set("Rust", key, value);
            }
        }
        let widths: Vec<String> = self.column_widths.iter().map(|(k, w)| format!("{}={:.0}", k, w)).collect();
        ini.set("Rust", "ColumnWidths", &widths.join(","));

        ini.save(ini_path);
    }
}

/// 歌詞の色の初期値
pub const LYRICS_COLOR_CURRENT: [u8; 3] = [56, 189, 248];
pub const LYRICS_COLOR_PAST: [u8; 3] = [120, 120, 120];
pub const LYRICS_COLOR_UPCOMING: [u8; 3] = [200, 200, 200];

fn hex_color(c: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

fn parse_hex_color(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lyrics_colors_roundtrip() {
        let f = std::env::temp_dir().join(format!("settings_lcolor_{}.ini", std::process::id()));
        let _ = std::fs::remove_file(&f);
        let mut s = AppSettings::load(&f);
        assert_eq!(s.lyrics_color_current, LYRICS_COLOR_CURRENT);
        assert!(s.lyrics_sweep);
        s.lyrics_color_current = [255, 0, 128];
        s.lyrics_color_past = [1, 2, 3];
        s.lyrics_sweep = false;
        s.save(&f);
        let text = std::fs::read_to_string(&f).unwrap();
        assert!(text.contains("CurrentColor=#FF0080") && text.contains("Sweep=false"));
        let back = AppSettings::load(&f);
        assert_eq!((back.lyrics_color_current, back.lyrics_color_past, back.lyrics_sweep), ([255, 0, 128], [1, 2, 3], false));
        assert_eq!(parse_hex_color("zz"), None);
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn lyrics_pane_width_roundtrip() {
        let f = std::env::temp_dir().join(format!("settings_lyrics_{}.ini", std::process::id()));
        let _ = std::fs::remove_file(&f);
        // 幅を調整していなければ、INI には書かない（既定＝メインペインの半分）
        let mut s = AppSettings::load(&f);
        assert_eq!(s.lyrics_pane_width, 0.0);
        s.save(&f);
        assert!(!std::fs::read_to_string(&f).unwrap().contains("LyricsPaneWidth"));
        // 調整した幅は記録され、読み直せる
        s.lyrics_pane_width = 512.5;
        s.save(&f);
        assert!(std::fs::read_to_string(&f).unwrap().contains("LyricsPaneWidth=512.5"));
        assert_eq!(AppSettings::load(&f).lyrics_pane_width, 512.5);
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn hotkeys_roundtrip() {
        use crate::audio::hotkeys_defs::{Combo, MOD_CONTROL};
        let f = std::env::temp_dir().join(format!("settings_hotkeys_{}.ini", std::process::id()));
        let _ = std::fs::remove_file(&f);
        // 何も書かれていなければ既定
        assert_eq!(AppSettings::load(&f).hotkeys, default_bindings());

        let mut s = AppSettings::default();
        s.hotkeys.retain(|b| b.action != HotkeyAction::Stop); // 停止は割り当て無し
        s.hotkeys.push(Binding { action: HotkeyAction::Reverse, combo: Combo::new(MOD_CONTROL, 0x77) });
        s.save(&f);
        let back = AppSettings::load(&f);
        // 並び順は問わず、同じ割り当てが戻ること
        let sorted = |v: &[Binding]| {
            let mut v: Vec<(String, String)> = v.iter().map(|b| (b.action.ini_key().to_string(), b.combo.name())).collect();
            v.sort();
            v
        };
        assert_eq!(sorted(&back.hotkeys), sorted(&s.hotkeys));
        assert!(!back.hotkeys.iter().any(|b| b.action == HotkeyAction::Stop));
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn csharp_compatible_roundtrip() {
        let f = std::env::temp_dir().join(format!("settings_test_{}.ini", std::process::id()));
        // C# 版が書く形式
        std::fs::write(
            &f,
            "[General]\nTheme=Dark\nAudioEngine=Wasapi\nTagDisplayMode=CustomOnly\nVolume=65\nWaveformDurationSeconds=30\n\n\
             [State]\nLastFolder=M:\\Video\\音楽\nLeftPanelWidth=350.0\n\n[Window]\nWidth=1300.0\nHeight=820.0\nLeft=100.0\nTop=50.0\nIsMaximized=True\n\n\
             [Cache]\nEnableCache=False\nMaxCacheTracks=10000\n\n[Playback]\nFadeMs=500\nCrossfadeMs=3000\nLoudnessMode=Auto\nEnableGlobalHotkeys=False\n\n\
             [View]\nIncludeSubfolders=True\nMiniPlayerTopmost=False\nMiniPlayerLeft=10.0\nMiniPlayerTop=20.0\n\n[Backup]\nEnableAutoBackup=False\n\n\
             [Favorites]\nFav_0=お気に入り A|M:\\Music\\A\nFav_1=B|M:\\Music\\B\nCount=2\n\n[Other]\nKeep=me\n",
        )
        .unwrap();

        let s = AppSettings::load(&f);
        assert_eq!(s.theme, AppThemeMode::Dark);
        assert_eq!(s.audio_engine, AudioOutputEngine::Wasapi);
        assert_eq!(s.tag_display_mode, TagDisplayMode::CustomOnly);
        assert!((s.volume - 0.65).abs() < 1e-6);
        assert_eq!(s.waveform_secs, 30);
        assert_eq!(s.last_folder, "M:\\Video\\音楽");
        assert_eq!(s.left_panel_width, 350.0);
        assert!(s.is_maximized && !s.enable_cache && s.max_cache_tracks == 10000);
        assert_eq!((s.fade_ms, s.crossfade_ms), (500, 3000));
        assert_eq!(s.loudness_mode, LoudnessMode::Auto);
        assert!(!s.enable_global_hotkeys && s.include_subfolders && !s.mini_player_topmost && !s.enable_auto_backup);
        assert_eq!(s.favorites, vec![("お気に入り A".into(), "M:\\Music\\A".into()), ("B".into(), "M:\\Music\\B".into())]);

        s.save(&f);
        let text = std::fs::read_to_string(&f).unwrap();
        assert!(text.contains("Theme=Dark") && text.contains("Volume=65") && text.contains("Fav_1=B|M:\\Music\\B"));
        assert!(text.contains("[Other]") && text.contains("Keep=me")); // 未知のセクションは残る
        assert_eq!(AppSettings::load(&f).crossfade_ms, 3000);

        // 古い [State] の表示列設定が、新しい [Rust] の設定を上書きしないこと
        std::fs::write(
            &f,
            "[State]
Columns=index:1,art:1,filename:1

[Rust]
Columns=index:0,art:0,filename:1,title:1
",
        )
        .unwrap();
        let loaded = AppSettings::load(&f);
        let shown: Vec<_> = loaded.columns.visible().map(|c| c.key()).collect();
        assert!(shown.contains(&"title") && !shown.contains(&"index"), "新しい [Rust] の設定が優先される: {:?}", shown);
        loaded.save(&f);
        let text = std::fs::read_to_string(&f).unwrap();
        assert!(!text.contains("[State]
Columns="), "旧キーは [State] から消える");
        let _ = std::fs::remove_file(&f);
    }
}

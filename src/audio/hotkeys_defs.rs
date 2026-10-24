//! ホットキーの種類・キーの組み合わせ・設定ファイル上の書式。
//! 既定の割り当ては C# 版 HotkeyService と同じ（メディアキーと Ctrl+Alt+矢印）。

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HotkeyAction {
    PlayPause,
    Next,
    Previous,
    Stop,
    Reverse,
    SpeedUp,
    SpeedDown,
    SpeedReset,
}

impl HotkeyAction {
    pub const ALL: [HotkeyAction; 8] = [
        HotkeyAction::PlayPause,
        HotkeyAction::Next,
        HotkeyAction::Previous,
        HotkeyAction::Stop,
        HotkeyAction::Reverse,
        HotkeyAction::SpeedUp,
        HotkeyAction::SpeedDown,
        HotkeyAction::SpeedReset,
    ];

    pub fn label(self) -> &'static str {
        match self {
            HotkeyAction::PlayPause => "再生 / 一時停止",
            HotkeyAction::Next => "次の曲",
            HotkeyAction::Previous => "前の曲",
            HotkeyAction::Stop => "停止",
            HotkeyAction::Reverse => "逆再生の切り替え",
            HotkeyAction::SpeedUp => "再生速度を上げる",
            HotkeyAction::SpeedDown => "再生速度を下げる",
            HotkeyAction::SpeedReset => "再生速度を等倍に戻す",
        }
    }

    /// 設定ファイル（[Hotkeys]）のキー名
    pub fn ini_key(self) -> &'static str {
        match self {
            HotkeyAction::PlayPause => "PlayPause",
            HotkeyAction::Next => "Next",
            HotkeyAction::Previous => "Previous",
            HotkeyAction::Stop => "Stop",
            HotkeyAction::Reverse => "Reverse",
            HotkeyAction::SpeedUp => "SpeedUp",
            HotkeyAction::SpeedDown => "SpeedDown",
            HotkeyAction::SpeedReset => "SpeedReset",
        }
    }
}

pub const MOD_ALT: u32 = 0x0001;
pub const MOD_CONTROL: u32 = 0x0002;
pub const MOD_SHIFT: u32 = 0x0004;
pub const MOD_WIN: u32 = 0x0008;

/// (仮想キーコード, 設定ファイルと画面に出す名前)。名前は英数字と記号だけにしている
const KEY_NAMES: &[(u32, &str)] = &[
    (0x08, "Backspace"),
    (0x09, "Tab"),
    (0x0D, "Enter"),
    (0x20, "Space"),
    (0x21, "PageUp"),
    (0x22, "PageDown"),
    (0x23, "End"),
    (0x24, "Home"),
    (0x25, "Left"),
    (0x26, "Up"),
    (0x27, "Right"),
    (0x28, "Down"),
    (0x2D, "Insert"),
    (0x2E, "Delete"),
    (0xBA, ";"),
    (0xBB, "="),
    (0xBC, ","),
    (0xBD, "-"),
    (0xBE, "."),
    (0xBF, "/"),
    (0xC0, "`"),
    (0xDB, "["),
    (0xDC, "\\"),
    (0xDD, "]"),
    (0xDE, "'"),
    (0xB0, "MediaNext"),
    (0xB1, "MediaPrevious"),
    (0xB2, "MediaStop"),
    (0xB3, "MediaPlayPause"),
];

/// メディアキー（ほかのキーと違い、修飾キー無しで登録してよい）
pub const MEDIA_KEYS: [u32; 4] = [0xB3, 0xB0, 0xB1, 0xB2];

pub fn key_name(vk: u32) -> Option<String> {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => Some((vk as u8 as char).to_string()),
        0x70..=0x87 => Some(format!("F{}", vk - 0x6F)),
        _ => KEY_NAMES.iter().find(|(k, _)| *k == vk).map(|(_, n)| n.to_string()),
    }
}

pub fn key_from_name(name: &str) -> Option<u32> {
    let n = name.trim();
    if n.chars().count() == 1 {
        let c = n.chars().next()?.to_ascii_uppercase();
        if c.is_ascii_digit() || c.is_ascii_uppercase() {
            return Some(c as u32);
        }
    }
    if let Some(num) = n.strip_prefix(['F', 'f']).and_then(|x| x.parse::<u32>().ok()) {
        if (1..=24).contains(&num) {
            return Some(0x6F + num);
        }
    }
    KEY_NAMES.iter().find(|(_, nm)| nm.eq_ignore_ascii_case(n)).map(|(k, _)| *k)
}

/// キーの組み合わせ（修飾キー + キー）
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Combo {
    pub modifiers: u32,
    pub key: u32,
}

impl Combo {
    pub const fn new(modifiers: u32, key: u32) -> Self {
        Self { modifiers, key }
    }

    pub fn is_media(&self) -> bool {
        self.modifiers == 0 && MEDIA_KEYS.contains(&self.key)
    }

    /// "Ctrl+Alt+Space" の形の名前
    pub fn name(&self) -> String {
        let mut s = String::new();
        for (m, n) in [(MOD_CONTROL, "Ctrl"), (MOD_ALT, "Alt"), (MOD_SHIFT, "Shift"), (MOD_WIN, "Win")] {
            if self.modifiers & m != 0 {
                s.push_str(n);
                s.push('+');
            }
        }
        s.push_str(&key_name(self.key).unwrap_or_else(|| format!("0x{:02X}", self.key)));
        s
    }

    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        // 最後の '+' より後ろがキー名。'+' だけの名前は使わない
        let (mods, key) = match text.rfind('+') {
            Some(i) => (&text[..i], &text[i + 1..]),
            None => ("", text),
        };
        let key = key_from_name(key)?;
        let mut modifiers = 0;
        for m in mods.split('+').map(str::trim).filter(|m| !m.is_empty()) {
            modifiers |= match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => MOD_CONTROL,
                "alt" => MOD_ALT,
                "shift" => MOD_SHIFT,
                "win" => MOD_WIN,
                _ => return None,
            };
        }
        Some(Self { modifiers, key })
    }

    /// 他のアプリの入力を奪わないよう、普通のキーは Ctrl / Alt / Win と一緒のときだけ登録できる
    pub fn check(&self) -> Result<(), &'static str> {
        let is_f_key = (0x70..=0x87).contains(&self.key);
        if self.is_media() || is_f_key || self.modifiers & (MOD_CONTROL | MOD_ALT | MOD_WIN) != 0 {
            Ok(())
        } else {
            Err("Ctrl・Alt のどちらかと一緒に押してください（ファンクションキーとメディアキーは単独でも可）")
        }
    }

    /// 画面で押されたキーから組み合わせを作る。対応しないキーは None
    pub fn from_egui(key: egui::Key, m: egui::Modifiers) -> Option<Self> {
        use egui::Key as K;
        let vk: u32 = match key {
            K::Space => 0x20,
            K::Enter => 0x0D,
            K::Tab => 0x09,
            K::Backspace => 0x08,
            K::Insert => 0x2D,
            K::Delete => 0x2E,
            K::Home => 0x24,
            K::End => 0x23,
            K::PageUp => 0x21,
            K::PageDown => 0x22,
            K::ArrowLeft => 0x25,
            K::ArrowUp => 0x26,
            K::ArrowRight => 0x27,
            K::ArrowDown => 0x28,
            K::Semicolon => 0xBA,
            K::Equals => 0xBB,
            K::Comma => 0xBC,
            K::Minus => 0xBD,
            K::Period => 0xBE,
            K::Slash => 0xBF,
            K::Backtick => 0xC0,
            K::OpenBracket => 0xDB,
            K::Backslash => 0xDC,
            K::CloseBracket => 0xDD,
            K::Quote => 0xDE,
            other => {
                // 英数字と F1〜F12 は、キー名からコードを引く
                let name = other.name();
                let name = name.strip_prefix("Num").unwrap_or(name);
                if name.len() == 1 || name.starts_with('F') {
                    key_from_name(name)?
                } else {
                    return None;
                }
            }
        };
        let mut modifiers = 0;
        if m.ctrl {
            modifiers |= MOD_CONTROL;
        }
        if m.alt {
            modifiers |= MOD_ALT;
        }
        if m.shift {
            modifiers |= MOD_SHIFT;
        }
        Some(Self { modifiers, key: vk })
    }
}

/// 1つの操作に割り当てるキー。同じ操作に複数割り当ててよい
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub action: HotkeyAction,
    pub combo: Combo,
}

const CA: u32 = MOD_CONTROL | MOD_ALT;

/// 既定の割り当て
pub fn default_bindings() -> Vec<Binding> {
    use HotkeyAction::*;
    let b = |action, modifiers, key| Binding { action, combo: Combo::new(modifiers, key) };
    vec![
        b(PlayPause, 0, 0xB3),
        b(PlayPause, CA, 0x20),
        b(Next, 0, 0xB0),
        b(Next, CA, 0x27),
        b(Previous, 0, 0xB1),
        b(Previous, CA, 0x25),
        b(Stop, 0, 0xB2),
        b(Stop, CA, 0x28),
        b(Reverse, CA, 0x52),    // R
        b(SpeedUp, CA, 0x21),    // PageUp
        b(SpeedDown, CA, 0x22),  // PageDown
        b(SpeedReset, CA, 0x24), // Home
    ]
}

/// 設定ファイルの値（"Ctrl+Alt+Space|MediaPlayPause" の形）から、1つの操作のキーを読む
pub fn parse_combos(value: &str) -> Vec<Combo> {
    value.split('|').filter_map(Combo::parse).collect()
}

pub fn format_combos(bindings: &[Binding], action: HotkeyAction) -> String {
    bindings.iter().filter(|b| b.action == action).map(|b| b.combo.name()).collect::<Vec<_>>().join("|")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combo_roundtrip() {
        for b in default_bindings() {
            let text = b.combo.name();
            assert_eq!(Combo::parse(&text), Some(b.combo), "{text}");
        }
        assert_eq!(Combo::parse("Ctrl+Alt+Space"), Some(Combo::new(CA, 0x20)));
        assert_eq!(Combo::parse("shift+ctrl+f5"), Some(Combo::new(MOD_SHIFT | MOD_CONTROL, 0x74)));
        assert_eq!(Combo::parse("Ctrl+="), Some(Combo::new(MOD_CONTROL, 0xBB)));
        assert_eq!(Combo::parse("Ctrl+Foo"), None);
        assert_eq!(Combo::parse("Hyper+A"), None);
        assert_eq!(Combo::parse(""), None);
    }

    #[test]
    fn check_rules() {
        assert!(Combo::new(0, 0xB3).check().is_ok()); // メディアキー
        assert!(Combo::new(0, 0x74).check().is_ok()); // F5
        assert!(Combo::new(CA, 0x41).check().is_ok());
        assert!(Combo::new(0, 0x41).check().is_err()); // 修飾キー無しの A
        assert!(Combo::new(MOD_SHIFT, 0x41).check().is_err());
    }

    #[test]
    fn from_egui_keys() {
        let ca = egui::Modifiers { ctrl: true, alt: true, ..Default::default() };
        assert_eq!(Combo::from_egui(egui::Key::R, ca), Some(Combo::new(CA, 0x52)));
        assert_eq!(Combo::from_egui(egui::Key::Num5, ca), Some(Combo::new(CA, 0x35)));
        assert_eq!(Combo::from_egui(egui::Key::F5, ca), Some(Combo::new(CA, 0x74)));
        assert_eq!(Combo::from_egui(egui::Key::ArrowRight, ca), Some(Combo::new(CA, 0x27)));
        assert_eq!(Combo::from_egui(egui::Key::Escape, ca), None);
    }

    #[test]
    fn list_format() {
        let d = default_bindings();
        assert_eq!(format_combos(&d, HotkeyAction::PlayPause), "MediaPlayPause|Ctrl+Alt+Space");
        assert_eq!(parse_combos("MediaPlayPause|Ctrl+Alt+Space|zzz").len(), 2);
        assert!(parse_combos("").is_empty());
    }
}

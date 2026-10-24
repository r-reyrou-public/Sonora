//! 設定・ラベル・キャッシュなどの保存先を決める。
//!
//! - `settings.ini` は、常に EXE と同じフォルダに固定する（書き込めないときだけ AppData へ退避）。
//! - ラベル・プレイリスト（library.bin）とキャッシュは、EXE 直下にあればそれを使う。
//!   直下になければ、`settings.ini` の `DataFolder` に書かれたフォルダへ取りに行く（指定がなければ直下に作る）。
//!   直下に全部そろっているときは、指定フォルダへはアクセスしない（起動が遅くならない）。
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// 保存先を変えられるデータ（settings.ini は含まない。常に EXE の直下）
pub const DATA_FILES: [&str; 3] = ["library.bin", "tags.cache", "media.cache"];

/// 常に EXE の直下（または退避先）に置くファイル
const FIXED_FILES: [&str; 1] = ["settings.ini"];

struct Resolved {
    /// settings.ini などを置く固定のフォルダ（通常は EXE のフォルダ）
    base: PathBuf,
    /// settings.ini の `DataFolder` で指定された、データの取りに行き先
    custom: Option<PathBuf>,
    /// 指定フォルダが必要なのに、起動時にアクセスできなかった
    custom_unavailable: bool,
}

static RESOLVED: OnceLock<Resolved> = OnceLock::new();

/// ネットワーク共有 (UNC: 円記号2つで始まるパス) のパスか。
/// 他人から受け取ったデータ（バックアップ・プレイリスト）に含まれていると、開いただけで外部サーバーへ接続し、
/// Windows のログイン情報（NTLM ハッシュ）を送ってしまう恐れがあるため、自動では触らない。
pub fn is_unc(path: &str) -> bool {
    let n = path.trim().replace('/', "\\");
    if let Some(rest) = n.strip_prefix(r"\\?\") {
        return rest.to_ascii_uppercase().starts_with("UNC\\");
    }
    if n.starts_with(r"\\.\") {
        return false;
    }
    n.starts_with(r"\\")
}

pub fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 旧版が使っていた、保存先フォルダを書いたファイル（今は settings.ini の DataFolder を使う）
fn legacy_location_file() -> PathBuf {
    exe_dir().join("data_location.txt")
}

fn resolved() -> &'static Resolved {
    RESOLVED.get_or_init(resolve)
}

/// 設定ファイルの場所（固定）
pub fn settings_file() -> PathBuf {
    resolved().base.join("settings.ini")
}

/// 指定フォルダ（設定されていれば）。なければ固定のフォルダ。表示・警告用
pub fn data_dir() -> PathBuf {
    let r = resolved();
    r.custom.clone().unwrap_or_else(|| r.base.clone())
}

/// データファイル（またはフォルダ）の場所。直下にあれば直下、なければ指定フォルダ、指定がなければ直下
pub fn data_file(name: &str) -> PathBuf {
    let r = resolved();
    let in_base = r.base.join(name);
    if FIXED_FILES.contains(&name) || in_base.exists() {
        return in_base;
    }
    match &r.custom {
        Some(c) => c.join(name),
        None => in_base,
    }
}

pub fn custom_location_unavailable() -> bool {
    resolved().custom_unavailable
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase();
    norm(a) == norm(b)
}


fn is_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".write_test_{}.tmp", std::process::id()));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

// ---- settings.ini の1項目だけを、設定の読み込み前に直接読み書きする ----

fn strip_bom(s: &str) -> &str {
    s.trim_start_matches('\u{feff}')
}

fn ini_value(path: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    for line in strip_bom(&text).lines() {
        if let Some((k, v)) = line.split_once('=') {
            if k.trim().eq_ignore_ascii_case(key) {
                let v = v.trim();
                return if v.is_empty() { None } else { Some(v.to_string()) };
            }
        }
    }
    None
}

fn ini_remove_keys(path: &Path, keys: &[&str]) {
    let Ok(text) = std::fs::read_to_string(path) else { return };
    let crlf = text.contains("\r\n");
    let nl = if crlf { "\r\n" } else { "\n" };
    let kept: Vec<&str> = text
        .lines()
        .filter(|l| !l.split_once('=').is_some_and(|(k, _)| keys.iter().any(|x| strip_bom(k).trim().eq_ignore_ascii_case(x))))
        .collect();
    let mut out = kept.join(nl);
    out.push_str(nl);
    let _ = std::fs::write(path, out);
}

/// 指定フォルダがあるか。NAS 等が応答しなくても起動が止まらないよう、別スレッドで最大3秒だけ待つ
fn dir_exists_within(path: &Path, secs: u64) -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    let p = path.to_path_buf();
    std::thread::spawn(move || {
        let _ = tx.send(p.is_dir());
    });
    rx.recv_timeout(std::time::Duration::from_secs(secs)).unwrap_or(false)
}

/// 保存先の変更で移動する対象（ファイルとフォルダ）
const MOVABLE: [&str; 5] = ["library.bin", "tags.cache", "media.cache", "backups", "crash.log"];

/// 「.old」「.old2」…のうち、まだ使われていない名前
fn old_name(p: &Path) -> PathBuf {
    let base = p.to_string_lossy().to_string();
    let first = PathBuf::from(format!("{}.old", base));
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|i| PathBuf::from(format!("{}.old{}", base, i)))
        .find(|c| !c.exists())
        .unwrap_or(first)
}

fn copy_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for e in std::fs::read_dir(src)? {
            let e = e?;
            copy_recursive(&e.path(), &dst.join(e.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(src, dst).map(|_| ())
    }
}

/// `src` を `dst` へ移す。`dst` にすでにあれば、そちらを「.old」付きの名前にして残す。
/// 別のドライブへの移動は、コピーに成功してから元を消す（途中で失敗しても元は残る）
fn move_with_old(src: &Path, dst: &Path) {
    if !src.exists() {
        return;
    }
    if dst.exists() && std::fs::rename(dst, old_name(dst)).is_err() {
        return; // 退避できないときは、上書きしないよう何もしない
    }
    if std::fs::rename(src, dst).is_ok() {
        return;
    }
    if copy_recursive(src, dst).is_ok() {
        let _ = if src.is_dir() { std::fs::remove_dir_all(src) } else { std::fs::remove_file(src) };
    }
}

fn resolve() -> Resolved {
    let exe = exe_dir();

    // 1. 固定のフォルダ: EXE と同じ。書き込めないときだけ AppData へ退避
    let base = if is_writable(&exe) {
        exe.clone()
    } else {
        let fallback = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| exe.clone())
            .join("MusicManager");
        let _ = std::fs::create_dir_all(&fallback);
        for name in ["settings.ini", "library.bin", "tags.cache", "media.cache"] {
            let (src, dst) = (exe.join(name), fallback.join(name));
            if src.exists() && !dst.exists() {
                let _ = std::fs::copy(&src, &dst);
            }
        }
        fallback
    };
    let ini = base.join("settings.ini");

    // 2. 指定フォルダ: settings.ini の DataFolder。旧版の data_location.txt も引き継ぐ
    let mut custom = ini_value(&ini, "DataFolder").map(PathBuf::from);
    if custom.is_none() {
        if let Ok(text) = std::fs::read_to_string(legacy_location_file()) {
            let c = strip_bom(&text).trim();
            if !c.is_empty() {
                let c = PathBuf::from(c);
                // 旧版は settings.ini も指定フォルダに置いていた。直下になければ持ってくる
                if !ini.exists() && c.join("settings.ini").exists() {
                    let _ = std::fs::copy(c.join("settings.ini"), &ini);
                }
                custom = Some(c);
            }
        }
    }
    custom = custom.filter(|c| !c.as_os_str().is_empty() && !same_dir(c, &base));

    // 3. 指定フォルダが要るのは、直下にデータがそろっていないとき（または、移す作業があるとき）だけ
    let mode = ini_value(&ini, "DataFolderMove");
    let mut unavailable = false;
    if let Some(c) = custom.clone() {
        let missing = DATA_FILES.iter().any(|n| !base.join(n).exists());
        if missing || mode.is_some() {
            unavailable = !dir_exists_within(&c, 3);
        }
        // 設定画面で選んだ「移す」作業を、何も開いていない起動直後に行う
        if let (Some(mode), false) = (mode.as_deref(), unavailable) {
            if mode == "return" {
                // 指定を解除: 指定フォルダのデータを直下へ戻す
                for n in MOVABLE {
                    move_with_old(&c.join(n), &base.join(n));
                }
                ini_remove_keys(&ini, &["DataFolder", "DataFolderMove"]);
                custom = None;
            } else {
                // 直下のデータを指定フォルダへ移す（EXE の直下には、実行ファイルと設定ファイルだけが残る）
                let _ = std::fs::create_dir_all(&c);
                for n in MOVABLE {
                    move_with_old(&base.join(n), &c.join(n));
                }
                // 旧版の保存先指定ファイルは、役目を終えるので消す
                let _ = std::fs::remove_file(legacy_location_file());
                ini_remove_keys(&ini, &["DataFolderMove"]);
            }
        }
    }
    Resolved { base, custom, custom_unavailable: unavailable }
}

#[cfg(test)]
mod unc_tests {
    use super::*;

    #[test]
    fn detects_network_paths() {
        assert!(is_unc(r"\\server\share\a.mp3"));
        assert!(is_unc("//server/share/a.mp3"));
        assert!(is_unc(r"\\?\UNC\server\share\a.mp3"));
        assert!(!is_unc(r"M:\Video\a.mp3"));
        assert!(!is_unc(r"\\?\M:\Video\a.mp3"));
        assert!(!is_unc("relative/a.mp3"));
    }

    #[test]
    fn move_keeps_existing_destination_as_old() {
        let root = std::env::temp_dir().join(format!("sonora_move_{}", std::process::id()));
        let (a, b) = (root.join("a"), root.join("b"));
        std::fs::create_dir_all(a.join("backups")).unwrap();
        std::fs::create_dir_all(b.join("backups")).unwrap();
        std::fs::write(a.join("library.bin"), "new").unwrap();
        std::fs::write(b.join("library.bin"), "old").unwrap();
        std::fs::write(a.join("backups").join("x.bin"), "bx").unwrap();
        std::fs::write(b.join("backups").join("y.bin"), "by").unwrap();
        move_with_old(&a.join("library.bin"), &b.join("library.bin"));
        move_with_old(&a.join("backups"), &b.join("backups"));
        move_with_old(&a.join("missing.cache"), &b.join("missing.cache")); // ないものは何もしない
        assert!(!a.join("library.bin").exists() && !a.join("backups").exists());
        assert_eq!(std::fs::read_to_string(b.join("library.bin")).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(b.join("library.bin.old")).unwrap(), "old");
        assert!(b.join("backups").join("x.bin").exists());
        assert!(b.join("backups.old").join("y.bin").exists());
        // もう一度行うと、.old は上書きされず .old2 になる
        std::fs::write(a.join("library.bin"), "newer").unwrap();
        move_with_old(&a.join("library.bin"), &b.join("library.bin"));
        assert_eq!(std::fs::read_to_string(b.join("library.bin.old")).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(b.join("library.bin.old2")).unwrap(), "new");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn ini_helpers_read_and_remove_keys() {
        let p = std::env::temp_dir().join(format!("sonora_ini_{}.ini", std::process::id()));
        std::fs::write(&p, "\u{feff}[Rust]\r\nDataFolder=D:\\data\r\nDataFolderMove=copy\r\nCheckUpdates=true\r\n").unwrap();
        assert_eq!(ini_value(&p, "DataFolder").as_deref(), Some("D:\\data"));
        assert_eq!(ini_value(&p, "datafoldermove").as_deref(), Some("copy"));
        ini_remove_keys(&p, &["DataFolderMove"]);
        assert_eq!(ini_value(&p, "DataFolderMove"), None);
        assert_eq!(ini_value(&p, "DataFolder").as_deref(), Some("D:\\data"));
        assert_eq!(ini_value(&p, "CheckUpdates").as_deref(), Some("true"));
        let _ = std::fs::remove_file(&p);
    }
}

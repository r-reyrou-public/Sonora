//! ラベル・プレイリスト（library.bin）と設定（settings.ini）のバックアップ／復元（C# 版 BackupService 相当）。
use crate::store::Store;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;

const LIBRARY_NAME: &str = "library.bin";
const SETTINGS_NAME: &str = "settings.ini";
const MANIFEST_NAME: &str = "manifest.txt";
const AUTO_BACKUP_KEEP: usize = 7;
/// 異常に大きいエントリは拒否（ZIP 爆弾対策）
const MAX_ENTRY_BYTES: u64 = 512 * 1024 * 1024;

/// ローカル時刻 (年, 月, 日, 時, 分, 秒)
pub fn local_now() -> (u16, u16, u16, u16, u16, u16) {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::SystemInformation::GetLocalTime;
        let mut t: windows_sys::Win32::Foundation::SYSTEMTIME = std::mem::zeroed();
        GetLocalTime(&mut t);
        return (t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond);
    }
    #[allow(unreachable_code)]
    (1970, 1, 1, 0, 0, 0)
}

pub fn timestamp_for_filename() -> String {
    let (y, mo, d, h, mi, _) = local_now();
    format!("{:04}{:02}{:02}_{:02}{:02}", y, mo, d, h, mi)
}

/// バックアップ ZIP を作成する（先に最新のラベル・プレイリストをファイルへ書き出す）
pub fn create_backup(zip_path: &Path, store: &Store, settings_path: &Path) -> Result<(), String> {
    store.flush();
    let tmp = PathBuf::from(format!("{}.tmp", zip_path.display()));
    {
        let file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipWriter::new(file);
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        let library = crate::paths::data_file(LIBRARY_NAME);
        if let Ok(bytes) = std::fs::read(&library) {
            zip.start_file(LIBRARY_NAME, opts).map_err(|e| e.to_string())?;
            zip.write_all(&bytes).map_err(|e| e.to_string())?;
        }
        if let Ok(bytes) = std::fs::read(settings_path) {
            zip.start_file(SETTINGS_NAME, opts).map_err(|e| e.to_string())?;
            zip.write_all(&bytes).map_err(|e| e.to_string())?;
        }
        let (y, mo, d, h, mi, s) = local_now();
        zip.start_file(MANIFEST_NAME, opts).map_err(|e| e.to_string())?;
        write!(zip, "MusicManager backup\nCreated: {:04}-{:02}-{:02} {:02}:{:02}:{:02}\n", y, mo, d, h, mi, s)
            .map_err(|e| e.to_string())?;
        zip.finish().map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, zip_path).map_err(|e| e.to_string())
}

/// バックアップ ZIP から復元する。既知のファイルだけを取り出し（パス操作対策）、
/// 置き換え前のファイルは `*.before-restore` として残す。復元後はアプリの再起動が必要。
pub fn restore_backup(zip_path: &Path, settings_path: &Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|_| "ZIP ファイルとして読み込めません。".to_string())?;

    if archive.by_name(MANIFEST_NAME).is_err() {
        return Err("このファイルは音楽管理ツールのバックアップではありません。".into());
    }

    let mut read_entry = |name: &str| -> Result<Option<Vec<u8>>, String> {
        match archive.by_name(name) {
            Ok(mut entry) => {
                if entry.size() > MAX_ENTRY_BYTES {
                    return Err(format!("{} のサイズが大きすぎます。", name));
                }
                // 申告サイズが偽でも、上限を超えて読まない
                let mut buf = Vec::with_capacity((entry.size() as usize).min(16 * 1024 * 1024));
                (&mut entry).take(MAX_ENTRY_BYTES + 1).read_to_end(&mut buf).map_err(|e| e.to_string())?;
                if buf.len() as u64 > MAX_ENTRY_BYTES {
                    return Err(format!("{} のサイズが大きすぎます。", name));
                }
                Ok(Some(buf))
            }
            Err(_) => Ok(None),
        }
    };

    let library = read_entry(LIBRARY_NAME)?;
    let settings = read_entry(SETTINGS_NAME)?;
    if library.is_none() && settings.is_none() {
        return Err("復元できるデータが含まれていません。".into());
    }
    if let Some(lib) = &library {
        if !Store::is_valid_data(lib) {
            return Err("バックアップ内のデータが壊れているか、形式が正しくありません。".into());
        }
    }

    let replace = |bytes: &[u8], target: &Path| -> Result<(), String> {
        if target.exists() {
            let _ = std::fs::copy(target, format!("{}.before-restore", target.display()));
        }
        // 途中で失敗しても元のファイルが壊れないよう、一時ファイルに書いてから置き換える
        let tmp = PathBuf::from(format!("{}.restoring", target.display()));
        std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, target).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            e.to_string()
        })
    };
    if let Some(lib) = library {
        // ネットワーク共有のパスは取り除いて保存する（バックアップ元が信用できない場合の対策）
        let clean = Store::sanitize_imported(&lib).unwrap_or(lib);
        replace(&clean, &crate::paths::data_file(LIBRARY_NAME))?;
    }
    if let Some(s) = settings {
        replace(&s, settings_path)?;
    }
    Ok(())
}

/// 起動時の自動バックアップ（1日1回、ラベル・プレイリストを backups フォルダへ保存し、7世代まで保持）
pub fn run_auto_backup() {
    let source = crate::paths::data_file(LIBRARY_NAME);
    if !source.exists() {
        return;
    }
    let dir = crate::paths::data_file("backups");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let (y, mo, d, ..) = local_now();
    let today = dir.join(format!("library_{:04}{:02}{:02}.bin", y, mo, d));
    if !today.exists() {
        let _ = std::fs::copy(&source, &today);
    }

    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("library_") && n.ends_with(".bin"))
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files.reverse(); // 新しい順
    for old in files.into_iter().skip(AUTO_BACKUP_KEEP) {
        let _ = std::fs::remove_file(old);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_roundtrip_and_validation() {
        // テスト時のデータ保存先は実行ファイルのフォルダ（target/debug/deps）
        let lib_path = crate::paths::data_file(LIBRARY_NAME);
        let settings_path = crate::paths::data_file("backup_test_settings.ini");
        let zip_path = std::env::temp_dir().join(format!("backup_test_{}.zip", std::process::id()));

        let store = Store::open(&lib_path);
        let svc = crate::playlist::PlaylistService::new(store.clone());
        let id = svc.create_playlist(&format!("backup-test-{}", std::process::id())).unwrap();
        svc.add_track(id, Path::new("C:/x.mp3")).unwrap();
        std::fs::write(&settings_path, "[General]\nTheme=Dark\n").unwrap();

        create_backup(&zip_path, &store, &settings_path).unwrap();
        let before = std::fs::read(&lib_path).unwrap();

        // 内容を壊してから復元 → 元に戻り、置き換え前のファイルは .before-restore に残る
        std::fs::write(&lib_path, b"broken").unwrap();
        std::fs::write(&settings_path, "[General]\nTheme=Light\n").unwrap();
        restore_backup(&zip_path, &settings_path).unwrap();
        assert_eq!(std::fs::read(&lib_path).unwrap(), before);
        assert!(std::fs::read_to_string(&settings_path).unwrap().contains("Theme=Dark"));
        assert_eq!(std::fs::read(format!("{}.before-restore", lib_path.display())).unwrap(), b"broken");

        // バックアップではない ZIP / ZIP ですらないファイルは拒否
        let bogus = std::env::temp_dir().join(format!("bogus_{}.zip", std::process::id()));
        std::fs::write(&bogus, b"not a zip").unwrap();
        assert!(restore_backup(&bogus, &settings_path).is_err());

        let _ = std::fs::remove_file(&zip_path);
        let _ = std::fs::remove_file(&bogus);
        let _ = std::fs::remove_file(&settings_path);
        let _ = std::fs::remove_file(format!("{}.before-restore", lib_path.display()));
        let _ = std::fs::remove_file(format!("{}.before-restore", settings_path.display()));
    }
}

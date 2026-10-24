//! ファイルをごみ箱へ移動する（完全削除はしない。ごみ箱から元に戻せる）。

#[cfg(windows)]
pub fn move_to_trash(path: &std::path::Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{SHFileOperationW, SHFILEOPSTRUCTW};

    if !path.exists() {
        return Err("ファイルが見つかりません".to_string());
    }
    // 絶対パスでないとごみ箱に入らず完全削除になることがあるので、必ず絶対パスにする
    let abs = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
    let text = abs.to_string_lossy().trim_start_matches(r"\\?\").to_string();
    // 末尾が二重の NUL で終わるワイド文字列（SHFileOperation の仕様）
    let mut wide: Vec<u16> = std::ffi::OsStr::new(&text).encode_wide().collect();
    wide.push(0);
    wide.push(0);

    let mut op: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
    op.wFunc = 3; // FO_DELETE
    op.pFrom = wide.as_ptr();
    // FOF_SILENT | FOF_NOCONFIRMATION | FOF_ALLOWUNDO(ごみ箱へ) | FOF_NOERRORUI
    op.fFlags = (0x0004 | 0x0010 | 0x0040 | 0x0400) as u16;
    let code = unsafe { SHFileOperationW(&mut op) };
    if code != 0 || op.fAnyOperationsAborted != 0 {
        return Err(format!("ごみ箱へ移動できませんでした (コード {})", code));
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn move_to_trash(path: &std::path::Path) -> Result<(), String> {
    std::fs::remove_file(path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn moves_file_to_trash() {
        let f = std::env::temp_dir().join(format!("trash_test_{}.txt", std::process::id()));
        std::fs::write(&f, b"x").unwrap();
        super::move_to_trash(&f).unwrap();
        assert!(!f.exists());
        assert!(super::move_to_trash(&f).is_err()); // もう無いファイルはエラー
    }
}

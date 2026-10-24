//! バージョン情報と、新しい版の確認（GitHub のリリース一覧を見るだけ）。
//! 通信は Windows 標準の WinHTTP で行い、確認と通知だけをする。ダウンロードや実行は自動ではしない
//! （ダウンロードは、利用者がリリースのページを開いて自分で行う）。
use std::ffi::c_void;

/// 現在のバージョン（表示用）
pub const APP_VERSION: &str = "0.97β";
/// 制作者（署名）
pub const APP_AUTHOR: &str = "鈴屋玲瓏";
/// 既定の確認先リポジトリ（設定で変更できる）
pub const DEFAULT_REPO: &str = "r-reyrou-public/Sonora";
/// 以前の確認先（この値が設定に残っていれば、新しい確認先に切り替える）
pub const OLD_DEFAULT_REPO: &str = "glasssoundforest-lab/Sonora";

/// 確認結果
#[derive(Clone, Debug)]
pub enum UpdateStatus {
    /// 確認前・確認中
    Idle,
    Checking,
    /// 自分が最新
    UpToDate { latest: String },
    /// 新しい版がある
    Available { latest: String },
    /// 確認できなかった（理由）
    Failed(String),
}

/// `owner/repo` の形で、使える文字だけか（URL に埋め込むので厳しく見る）
pub fn valid_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let (Some(owner), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let ok = |s: &str| {
        !s.is_empty() && s.len() <= 100 && s != "." && s != ".." && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    ok(owner) && ok(name)
}

/// リリースのページ（ここから、利用者が自分でダウンロードする）
pub fn releases_url(repo: &str) -> Option<String> {
    valid_repo(repo).then(|| format!("https://github.com/{}/releases/latest", repo))
}

/// バージョン。`v0.8-beta` `0.8β` `1.2.3` などを読み、β（ベータ）は同じ番号の正式版より古いとして比べる
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    major: u32,
    minor: u32,
    patch: u32,
    /// 正式版なら 1、β なら 0（番号が同じなら正式版が新しい）
    stable: u8,
}

impl Version {
    pub fn parse(text: &str) -> Option<Self> {
        let t = text.trim().trim_start_matches(['v', 'V']);
        let lower = t.to_lowercase();
        let beta = t.contains('β') || lower.contains("beta") || lower.contains("alpha") || lower.contains("-rc") || lower.contains("pre");
        let mut nums = [0u32; 3];
        let mut count = 0;
        let mut digits = String::new();
        let mut minor_len = 0usize;
        let mut flush = |digits: &mut String, nums: &mut [u32; 3], count: &mut usize| {
            if !digits.is_empty() && *count < 3 {
                if *count == 1 {
                    minor_len = digits.len();
                }
                nums[*count] = digits.parse().unwrap_or(0);
                *count += 1;
            }
            digits.clear();
        };
        for c in t.chars() {
            if c.is_ascii_digit() {
                digits.push(c);
            } else if c == '.' && !digits.is_empty() {
                flush(&mut digits, &mut nums, &mut count);
            } else {
                // 数字の並びが終わったら、そこまでを版番号とする（"0.8β" の β や "-beta" はここで終わる）
                break;
            }
        }
        flush(&mut digits, &mut nums, &mut count);
        if count == 0 {
            return None;
        }
        // 「0.9」と「0.85」のような2つの数字の版は、小数として比べる（0.9 は 0.90、0.85 より新しい）
        if count == 2 && minor_len < 3 {
            nums[1] *= 10u32.pow(3 - minor_len as u32);
        }
        Some(Self { major: nums[0], minor: nums[1], patch: nums[2], stable: if beta { 0 } else { 1 } })
    }
}

/// 自分の版より新しい版があるか
pub fn is_newer(latest: &str) -> bool {
    match (Version::parse(latest), Version::parse(APP_VERSION)) {
        (Some(l), Some(me)) => l > me,
        _ => false,
    }
}

/// リリース一覧（JSON）から、タグ名をすべて取り出す。JSON 全体は解析せず、"tag_name" の値だけを読む
fn tag_names(json: &str) -> Vec<String> {
    let mut out = Vec::new();
    let key = "\"tag_name\"";
    let mut rest = json;
    while let Some(pos) = rest.find(key) {
        rest = &rest[pos + key.len()..];
        let Some(colon) = rest.find(':') else { break };
        let after = rest[colon + 1..].trim_start();
        let Some(body) = after.strip_prefix('"') else { continue };
        let mut value = String::new();
        let mut chars = body.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => match chars.next() {
                    Some('u') => {
                        let hex: String = chars.by_ref().take(4).collect();
                        if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                            value.push(ch);
                        }
                    }
                    Some(other) => value.push(other),
                    None => break,
                },
                c => value.push(c),
            }
            if value.len() > 64 {
                break;
            }
        }
        if !value.is_empty() {
            out.push(value);
        }
    }
    out
}

/// 一覧の中で一番新しい版のタグを選ぶ
fn newest_tag(json: &str) -> Option<String> {
    tag_names(json)
        .into_iter()
        .filter_map(|t| Version::parse(&t).map(|v| (v, t)))
        .max_by_key(|(v, _)| *v)
        .map(|(_, t)| t)
}

/// 確認を行う（通信するので、必ず別スレッドから呼ぶ）
pub fn check(repo: &str) -> UpdateStatus {
    if !valid_repo(repo) {
        return UpdateStatus::Failed("リポジトリの指定が正しくありません（例: owner/repo）".to_string());
    }
    let path = format!("/repos/{}/releases?per_page=10", repo);
    let body = match https_get("api.github.com", &path) {
        Ok(b) => b,
        Err(e) => return UpdateStatus::Failed(e),
    };
    match newest_tag(&body) {
        None => UpdateStatus::Failed("公開されているリリースが見つかりませんでした".to_string()),
        Some(tag) => {
            if is_newer(&tag) {
                UpdateStatus::Available { latest: tag }
            } else {
                UpdateStatus::UpToDate { latest: tag }
            }
        }
    }
}

/// ブラウザでページを開く（GitHub のリリースページだけ）
pub fn open_release_page(repo: &str) {
    let Some(url) = releases_url(repo) else { return };
    open_in_browser(&url);
}

#[cfg(windows)]
fn open_in_browser(url: &str) {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    let (verb, url) = (wide("open"), wide(url));
    unsafe {
        ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), url.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL);
    }
}

#[cfg(not(windows))]
fn open_in_browser(_url: &str) {}

/// HTTPS で GET する（応答は 1MB まで）。接続先は呼び出し側が固定で渡す
#[cfg(windows)]
fn https_get(host: &str, path: &str) -> Result<String, String> {
    use windows_sys::Win32::Networking::WinHttp::*;

    const MAX_BYTES: usize = 1024 * 1024;
    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };

    struct Handle(*mut c_void);
    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { WinHttpCloseHandle(self.0) };
            }
        }
    }

    unsafe {
        let agent = wide(&format!("Sonora/{}", APP_VERSION));
        let mut session = Handle(WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, std::ptr::null(), std::ptr::null(), 0));
        if session.0.is_null() {
            // 古い Windows では自動プロキシ指定が使えないので、既定の設定でやり直す
            session = Handle(WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, std::ptr::null(), std::ptr::null(), 0));
        }
        if session.0.is_null() {
            return Err("通信を始められませんでした".to_string());
        }
        // 応答がなくても、いつまでも待たない
        WinHttpSetTimeouts(session.0, 5000, 5000, 5000, 8000);

        let host_w = wide(host);
        let connect = Handle(WinHttpConnect(session.0, host_w.as_ptr(), 443, 0));
        if connect.0.is_null() {
            return Err("サーバーに接続できませんでした".to_string());
        }
        let (verb, path_w) = (wide("GET"), wide(path));
        let request = Handle(WinHttpOpenRequest(
            connect.0,
            verb.as_ptr(),
            path_w.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null_mut(),
            WINHTTP_FLAG_SECURE,
        ));
        if request.0.is_null() {
            return Err("通信の準備に失敗しました".to_string());
        }
        let headers = wide("Accept: application/vnd.github+json\r\nX-GitHub-Api-Version: 2022-11-28\r\n");
        if WinHttpSendRequest(request.0, headers.as_ptr(), u32::MAX, std::ptr::null(), 0, 0, 0) == 0
            || WinHttpReceiveResponse(request.0, std::ptr::null_mut()) == 0
        {
            return Err("サーバーから応答がありませんでした（ネットワークを確認してください）".to_string());
        }

        let mut status: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        if WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            std::ptr::null(),
            &mut status as *mut u32 as *mut c_void,
            &mut size,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err("応答を読めませんでした".to_string());
        }
        match status {
            200 => {}
            404 => return Err("リポジトリが見つかりません（非公開の場合は確認できません）".to_string()),
            403 | 429 => return Err("GitHub の問い合わせ回数の上限に達しています。しばらくしてからお試しください".to_string()),
            other => return Err(format!("サーバーがエラーを返しました（{}）", other)),
        }

        let mut data: Vec<u8> = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            let mut read: u32 = 0;
            if WinHttpReadData(request.0, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut read) == 0 {
                return Err("応答の受信に失敗しました".to_string());
            }
            if read == 0 {
                break;
            }
            data.extend_from_slice(&buf[..read as usize]);
            if data.len() > MAX_BYTES {
                return Err("応答が大きすぎます".to_string());
            }
        }
        Ok(String::from_utf8_lossy(&data).into_owned())
    }
}

#[cfg(not(windows))]
fn https_get(_host: &str, _path: &str) -> Result<String, String> {
    Err("この環境では確認できません".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_order() {
        let v = |s: &str| Version::parse(s).unwrap();
        assert!(v("0.8") > v("0.8β"));
        assert!(v("v0.8-beta") == v("0.8β"));
        assert!(v("0.9β") > v("0.8"));
        // 0.85β は 0.8β より新しく、0.9 は 0.85β より新しい
        assert!(v("0.85β") > v("0.8β"));
        assert!(v("0.9") > v("0.85β"));
        assert!(v("v0.85-beta") == v("0.85β"));
        assert!(v("1.10.0") > v("1.2.3"));
        assert!(v("1.0.1") > v("1.0"));
        assert!(v("0.9") > v("0.10")); // 2つの数字の版は小数扱い（0.10 は 0.100）
        assert!(Version::parse("abc").is_none());
    }

    #[test]
    fn newer_check() {
        assert!(is_newer("v0.98-beta"));
        assert!(is_newer("0.98"));
        assert!(!is_newer("v0.9-beta")); // 0.9 は 0.90 と同じ
        assert!(!is_newer("v0.97-beta"));
        assert!(!is_newer("v0.92-beta"));
        assert!(!is_newer("v0.91-beta"));
        assert!(!is_newer("v0.90-beta"));
        assert!(!is_newer("v0.89-beta"));
        assert!(!is_newer("v0.86-beta"));
        assert!(!is_newer("v0.85-beta"));
        assert!(!is_newer("v0.8-beta"));
        assert!(!is_newer("0.7"));
    }

    #[test]
    fn repo_validation() {
        assert!(valid_repo("owner/repo-name"));
        assert!(!valid_repo("a/b/c"));
        assert!(!valid_repo("a"));
        assert!(!valid_repo("a/b?x=1"));
        assert!(!valid_repo("a/b c"));
        assert!(!valid_repo("../x"));
        assert_eq!(releases_url("o/r").as_deref(), Some("https://github.com/o/r/releases/latest"));
        assert!(releases_url("evil.com/\\x").is_none());
    }

    #[test]
    fn picks_newest_tag() {
        let json = r#"[{"tag_name":"v0.7","x":1},{"tag_name":"v0.8-beta"},{"tag_name":"v0.6β"},{"name":"tag_name"}]"#;
        assert_eq!(tag_names(json), ["v0.7", "v0.8-beta", "v0.6β"]);
        assert_eq!(newest_tag(json).as_deref(), Some("v0.8-beta"));
        // JSON のエスケープ表記（β = β）も読める
        assert_eq!(tag_names("[{\"tag_name\":\"v0.9\\u03b2\"}]"), ["v0.9β"]);
    }

    /// 実際に GitHub へ接続して確認する（ネットワークが必要なので、通常のテストでは動かさない）
    #[test]
    #[ignore]
    fn live_check() {
        match check("rust-lang/rust-analyzer") {
            UpdateStatus::Available { latest } | UpdateStatus::UpToDate { latest } => println!("latest tag: {}", latest),
            other => panic!("{:?}", other),
        }
        match check("r-reyrou-public/does-not-exist-xyz") {
            UpdateStatus::Failed(e) => println!("expected failure: {}", e),
            other => panic!("{:?}", other),
        }
    }
}

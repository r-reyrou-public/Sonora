use lofty::config::WriteOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::{Picture, PictureType};
use lofty::probe::Probe;
use lofty::tag::{Accessor, ItemKey, Tag, TagExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// ジャケット画像の変更内容
#[derive(Clone, Debug, Default)]
pub enum CoverEdit {
    #[default]
    Keep,
    Remove,
    Replace(Vec<u8>),
}

/// 書き込むタグ。`None` は「触らない」、空文字・0 は「その項目を削除」。
#[derive(Clone, Debug, Default)]
pub struct TagEdit {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub composer: Option<String>,
    pub comment: Option<String>,
    pub year: Option<u32>,
    pub track: Option<u32>,
    pub disc: Option<u32>,
    pub cover: CoverEdit,
}

/// 編集ダイアログの初期表示用に、一覧では持っていない項目もタグから読む
#[derive(Clone, Debug, Default)]
pub struct ExtraTags {
    pub composer: String,
    pub comment: String,
    pub disc: u32,
}

impl ExtraTags {
    pub fn read(path: &Path) -> Self {
        let mut e = ExtraTags::default();
        if let Ok(tagged) = Probe::open(path).and_then(|p| p.read()) {
            if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
                e.composer = tag.get_string(&ItemKey::Composer).unwrap_or_default().trim().to_string();
                e.comment = tag.comment().map(|c| c.trim().to_string()).unwrap_or_default();
                e.disc = tag.disk().unwrap_or(0);
            }
        }
        e
    }
}

#[derive(Clone, Debug)]
pub struct TrackItem {
    pub file_path: PathBuf,
    pub filename: String,
    pub title: String,
    /// ID3等のタグに実際に入っているタイトル（未設定なら空）。`title` はファイル名フォールバック込み
    pub tag_title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub year: u32,
    pub track_number: u32,
    pub duration: Duration,
    pub formatted_duration: String,
    pub format: String,
    pub file_size_bytes: u64,
    pub formatted_size: String,
    pub artwork_bytes: Option<Vec<u8>>,
    pub custom_tags: Vec<String>,
    /// 同じフォルダに、同名の歌詞ファイル（.lrc）がある
    pub has_lyrics: bool,
}

impl TrackItem {
    /// タグ・再生時間を読まず、ファイル名とサイズだけで作る（一覧の即時表示用）。
    /// タグは後から読み取り結果で置き換える。
    pub fn basic(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_uppercase();
        let filename = path.file_name()?.to_string_lossy().to_string();
        let file_size_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        Some(Self {
            file_path: path.to_path_buf(),
            title: filename.clone(),
            filename,
            tag_title: String::new(),
            artist: String::new(),
            album: String::new(),
            album_artist: String::new(),
            genre: String::new(),
            year: 0,
            track_number: 0,
            duration: Duration::ZERO,
            formatted_duration: String::new(),
            format: ext,
            file_size_bytes,
            formatted_size: format_size(file_size_bytes),
            artwork_bytes: None,
            custom_tags: Vec::new(),
            has_lyrics: crate::lyrics::lrc_path(path).is_file(),
        })
    }

    /// 読み取り済み(またはキャッシュ済み)のタグ情報から作る
    pub fn from_meta(path: &Path, file_size_bytes: u64, meta: &TagMeta) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_uppercase();
        let filename = path.file_name()?.to_string_lossy().to_string();
        let duration = Duration::from_millis(meta.duration_ms);

        // 時間フォーマット（四捨五入）
        let total_secs = (duration.as_secs_f64() + 0.5) as u64;
        let formatted_duration = if total_secs >= 3600 {
            format!("{:02}:{:02}:{:02}", total_secs / 3600, (total_secs % 3600) / 60, total_secs % 60)
        } else {
            format!("{:02}:{:02}", total_secs / 60, total_secs % 60)
        };

        Some(Self {
            file_path: path.to_path_buf(),
            title: if meta.tag_title.is_empty() { filename.clone() } else { meta.tag_title.clone() },
            filename,
            tag_title: meta.tag_title.clone(),
            artist: meta.artist.clone(),
            album: meta.album.clone(),
            album_artist: meta.album_artist.clone(),
            genre: meta.genre.clone(),
            year: meta.year,
            track_number: meta.track_number,
            duration,
            formatted_duration,
            format: ext,
            file_size_bytes,
            formatted_size: format_size(file_size_bytes),
            artwork_bytes: None, // 重いのでスキャン時は読まない（load_artwork で必要時に取得）
            custom_tags: Vec::new(),
            has_lyrics: crate::lyrics::lrc_path(path).is_file(),
        })
    }

    /// 埋め込みジャケット画像 (ID3v2 APIC / FLAC / MP4 等) を取得
    pub fn load_artwork(path: &Path) -> Option<Vec<u8>> {
        let tagged_file = Probe::open(path).ok()?.read().ok()?;
        let tag = tagged_file.primary_tag().or_else(|| tagged_file.first_tag())?;
        tag.pictures()
            .iter()
            .find(|p| !p.data().is_empty())
            .map(|p| p.data().to_vec())
    }

    pub fn write_tags(path: &Path, edit: &TagEdit) -> Result<(), String> {
        let mut tagged_file = Probe::open(path)
            .map_err(|e| e.to_string())?
            .read()
            .map_err(|e| e.to_string())?;

        let tag_type = tagged_file.primary_tag_type();
        let tag = match tagged_file.tag_mut(tag_type) {
            Some(t) => t,
            None => {
                tagged_file.insert_tag(Tag::new(tag_type));
                tagged_file.tag_mut(tag_type).unwrap()
            }
        };

        // 空文字はその項目の削除（空の項目をタグに残さない）
        fn text(tag: &mut Tag, key: ItemKey, v: &Option<String>) {
            if let Some(v) = v {
                let v = v.trim();
                if v.is_empty() {
                    tag.remove_key(&key);
                } else {
                    tag.insert_text(key, v.to_string());
                }
            }
        }
        text(tag, ItemKey::TrackTitle, &edit.title);
        text(tag, ItemKey::TrackArtist, &edit.artist);
        text(tag, ItemKey::AlbumTitle, &edit.album);
        text(tag, ItemKey::AlbumArtist, &edit.album_artist);
        text(tag, ItemKey::Genre, &edit.genre);
        text(tag, ItemKey::Composer, &edit.composer);
        text(tag, ItemKey::Comment, &edit.comment);
        match edit.year {
            Some(0) => tag.remove_year(),
            Some(y) => tag.set_year(y),
            None => {}
        }
        match edit.track {
            Some(0) => tag.remove_track(),
            Some(n) => tag.set_track(n),
            None => {}
        }
        match edit.disc {
            Some(0) => tag.remove_disk(),
            Some(n) => tag.set_disk(n),
            None => {}
        }
        match &edit.cover {
            CoverEdit::Keep => {}
            CoverEdit::Remove => tag.remove_picture_type(PictureType::CoverFront),
            CoverEdit::Replace(bytes) => {
                let mut pic = Picture::from_reader(&mut std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
                pic.set_pic_type(PictureType::CoverFront);
                tag.remove_picture_type(PictureType::CoverFront);
                tag.push_picture(pic);
            }
        }

        tag.save_to_path(path, WriteOptions::default())
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

pub fn format_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

pub fn is_audio_ext(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "mp3" | "flac" | "m4a" | "aac" | "ogg" | "opus" | "wav" | "wma" | "aiff" | "alac"
            )
        })
        .unwrap_or(false)
}

/// ファイルから読み取ったタグ情報（キャッシュの単位）
#[derive(Clone, Debug, Default)]
pub struct TagMeta {
    pub tag_title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub year: u32,
    pub track_number: u32,
    pub duration_ms: u64,
}

impl TagMeta {
    /// ID3 / FLAC / MP4 等のタグと再生時間を読み取る（読めなければ空）
    pub fn read(path: &Path) -> Self {
        let mut m = TagMeta::default();
        if let Ok(tagged_file) = Probe::open(path).and_then(|p| p.read()) {
            m.duration_ms = tagged_file.properties().duration().as_millis() as u64;

            if let Some(tag) = tagged_file.primary_tag().or_else(|| tagged_file.first_tag()) {
                m.tag_title = tag.title().map(|t| t.trim().to_string()).unwrap_or_default();
                m.artist = tag.artist().map(|t| t.trim().to_string()).unwrap_or_default();
                m.album = tag.album().map(|t| t.trim().to_string()).unwrap_or_default();
                m.album_artist = tag
                    .get_string(&ItemKey::AlbumArtist)
                    .map(|t| t.trim().to_string())
                    .unwrap_or_default();
                m.genre = tag.genre().map(|t| t.trim().to_string()).unwrap_or_default();
                m.year = tag.year().unwrap_or(0);
                m.track_number = tag.track().unwrap_or(0);
            }
        }
        m
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut e = crate::filecache::Enc::default();
        e.str(&self.tag_title);
        e.str(&self.artist);
        e.str(&self.album);
        e.str(&self.album_artist);
        e.str(&self.genre);
        e.u32(self.year);
        e.u32(self.track_number);
        e.u64(self.duration_ms);
        e.buf
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let mut d = crate::filecache::Dec::new(bytes);
        Some(Self {
            tag_title: d.str()?,
            artist: d.str()?,
            album: d.str()?,
            album_artist: d.str()?,
            genre: d.str()?,
            year: d.u32()?,
            track_number: d.u32()?,
            duration_ms: d.u64()?,
        })
    }
}

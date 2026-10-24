/// 曲一覧テーブルの列定義と、表示/非表示・並び順の設定
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnId {
    Index,
    Art,
    FileName,
    Waveform,
    Title,
    Artist,
    Album,
    AlbumArtist,
    Genre,
    Year,
    TrackNo,
    Format,
    Labels,
    Duration,
    Size,
}

impl ColumnId {
    pub const ALL: [ColumnId; 15] = [
        ColumnId::Index,
        ColumnId::Art,
        ColumnId::FileName,
        ColumnId::Waveform,
        ColumnId::Title,
        ColumnId::Artist,
        ColumnId::Album,
        ColumnId::AlbumArtist,
        ColumnId::Genre,
        ColumnId::Year,
        ColumnId::TrackNo,
        ColumnId::Format,
        ColumnId::Labels,
        ColumnId::Duration,
        ColumnId::Size,
    ];

    /// 設定ファイル用のキー
    pub fn key(self) -> &'static str {
        match self {
            ColumnId::Index => "index",
            ColumnId::Art => "art",
            ColumnId::FileName => "filename",
            ColumnId::Waveform => "waveform",
            ColumnId::Title => "title",
            ColumnId::Artist => "artist",
            ColumnId::Album => "album",
            ColumnId::AlbumArtist => "albumartist",
            ColumnId::Genre => "genre",
            ColumnId::Year => "year",
            ColumnId::TrackNo => "trackno",
            ColumnId::Format => "format",
            ColumnId::Labels => "labels",
            ColumnId::Duration => "duration",
            ColumnId::Size => "size",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ColumnId::Index => "# (番号)",
            ColumnId::Art => "画像 (ジャケット)",
            ColumnId::FileName => "ファイル名",
            ColumnId::Waveform => "波形 (出だし)",
            ColumnId::Title => "タイトル (ID3)",
            ColumnId::Artist => "アーティスト (ID3)",
            ColumnId::Album => "アルバム (ID3)",
            ColumnId::AlbumArtist => "アルバムアーティスト (ID3)",
            ColumnId::Genre => "ジャンル (ID3)",
            ColumnId::Year => "年 (ID3)",
            ColumnId::TrackNo => "トラック番号 (ID3)",
            ColumnId::Format => "形式",
            ColumnId::Labels => "ラベル",
            ColumnId::Duration => "時間",
            ColumnId::Size => "サイズ",
        }
    }

    fn from_key(key: &str) -> Option<ColumnId> {
        Self::ALL.iter().copied().find(|c| c.key() == key)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ColumnConfig {
    /// 表示順に並んだ (列, 表示するか)
    pub items: Vec<(ColumnId, bool)>,
}

impl Default for ColumnConfig {
    fn default() -> Self {
        Self { items: ColumnId::ALL.iter().map(|&c| (c, true)).collect() }
    }
}

impl ColumnConfig {
    /// プレイリストペインの初期表示列（狭い領域向けに絞る）
    pub fn playlist_default() -> Self {
        let shown = [
            ColumnId::Index,
            ColumnId::Art,
            ColumnId::FileName,
            ColumnId::Title,
            ColumnId::Artist,
            ColumnId::Duration,
        ];
        Self {
            items: ColumnId::ALL.iter().map(|&c| (c, shown.contains(&c))).collect(),
        }
    }

    /// タグ表示モード（両方 / ID3のみ / ラベルのみ）に合わせて、表示しない列を除いたコピーを返す
    pub fn with_tag_mode(&self, mode: crate::settings::TagDisplayMode) -> Self {
        use crate::settings::TagDisplayMode;
        let hidden: &[ColumnId] = match mode {
            TagDisplayMode::Both => &[],
            TagDisplayMode::Id3Only => &[ColumnId::Labels],
            TagDisplayMode::CustomOnly => &[
                ColumnId::Title,
                ColumnId::Artist,
                ColumnId::Album,
                ColumnId::AlbumArtist,
                ColumnId::Genre,
                ColumnId::Year,
                ColumnId::TrackNo,
            ],
        };
        Self { items: self.items.iter().map(|(c, v)| (*c, *v && !hidden.contains(c))).collect() }
    }

    pub fn visible(&self) -> impl Iterator<Item = ColumnId> + '_ {
        self.items.iter().filter(|(_, v)| *v).map(|(c, _)| *c)
    }

    /// "filename:1,title:0,..." 形式
    pub fn serialize(&self) -> String {
        self.items
            .iter()
            .map(|(c, v)| format!("{}:{}", c.key(), if *v { 1 } else { 0 }))
            .collect::<Vec<_>>()
            .join(",")
    }

    /// 不明なキーは無視し、設定に無い列(新バージョンで追加された列など)は末尾に表示付きで補う
    pub fn parse(s: &str) -> Self {
        let mut items: Vec<(ColumnId, bool)> = Vec::new();
        for part in s.split(',') {
            if let Some((k, v)) = part.trim().split_once(':') {
                if let Some(c) = ColumnId::from_key(k) {
                    if !items.iter().any(|(x, _)| *x == c) {
                        items.push((c, v.trim() != "0"));
                    }
                }
            }
        }
        for c in ColumnId::ALL {
            if !items.iter().any(|(x, _)| *x == c) {
                items.push((c, true));
            }
        }
        // 全部が非表示の設定だと、一覧に何も出なくなるので、ファイル名の列は必ず表示する
        if !items.iter().any(|(_, v)| *v) {
            if let Some(item) = items.iter_mut().find(|(c, _)| *c == ColumnId::FileName) {
                item.1 = true;
            }
        }
        Self { items }
    }
}

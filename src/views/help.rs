use egui::{RichText, Ui};

/// ヘルプ画面の中身（検索構文の説明）
pub fn render_help(ui: &mut Ui) {
    let code = |s: &str| RichText::new(s).monospace().color(crate::theme::c(147, 197, 253));

    ui.heading("🔍 検索の使い方");
    ui.add_space(4.0);
    ui.label("検索欄に入力すると、曲一覧が即座に絞り込まれます。大文字・小文字は区別しません。");
    ui.label("欄を空にする（✖ボタン／文字を全部消す）とすべての曲が表示されます。");
    ui.add_space(8.0);

    section(ui, "基本");
    egui::Grid::new("help_basic").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
        ui.label(code("夜に駆ける"));
        ui.label("ファイル名・タイトル・アーティスト・アルバム・アルバムアーティスト・ジャンル・形式・ラベルのどれかに含まれる曲");
        ui.end_row();
        ui.label(code("rock live"));
        ui.label("空白区切りは AND（両方の語を含む曲）");
        ui.end_row();
        ui.label(code("rock OR jazz"));
        ui.label("OR（または |）でどちらかを含む曲");
        ui.end_row();
        ui.label(code("-live"));
        ui.label("先頭に - を付けると除外（live を含まない曲）");
        ui.end_row();
        ui.label(code("\"love song\""));
        ui.label("\" \" で囲むと空白を含む語句をそのまま検索");
        ui.end_row();
    });
    ui.add_space(8.0);

    section(ui, "項目を指定した検索（項目名:値）");
    egui::Grid::new("help_fields").num_columns(3).spacing([16.0, 6.0]).show(ui, |ui| {
        let rows: [(&str, &str, &str); 11] = [
            ("file:", "f:", "ファイル名"),
            ("title:", "t:", "タイトル (ID3)"),
            ("artist:", "a:", "アーティスト (ID3)"),
            ("album:", "", "アルバム (ID3)"),
            ("albumartist:", "aa:", "アルバムアーティスト (ID3)"),
            ("genre:", "g:", "ジャンル (ID3)"),
            ("year:", "y:", "年 (ID3) — 数値の範囲指定が可能"),
            ("track:", "", "トラック番号 (ID3) — 数値の範囲指定が可能"),
            ("format:", "ext:", "形式（MP3 / FLAC など）"),
            ("label:", "tag: / l:", "ラベル"),
            ("path:", "dir:", "ファイルの場所（フルパス）"),
        ];
        ui.label(RichText::new("項目名").strong());
        ui.label(RichText::new("短縮形").strong());
        ui.label(RichText::new("対象").strong());
        ui.end_row();
        for (name, short, desc) in rows {
            ui.label(code(name));
            ui.label(code(short));
            ui.label(desc);
            ui.end_row();
        }
    });
    ui.add_space(8.0);

    section(ui, "年・トラック番号の範囲指定");
    egui::Grid::new("help_range").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
        ui.label(code("year:2020"));
        ui.label("2020年");
        ui.end_row();
        ui.label(code("year:2010..2019"));
        ui.label("2010〜2019年（両端を含む。2010.. / ..2019 のように片側だけも可）");
        ui.end_row();
        ui.label(code("year:>=2015  year:<2000"));
        ui.label("以上 / 未満（> >= < <= が使えます）");
        ui.end_row();
    });
    ui.add_space(8.0);

    section(ui, "未設定の曲を探す");
    egui::Grid::new("help_no").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
        ui.label(code("no:title"));
        ui.label("タイトルが未設定の曲（artist / album / genre / year / track / label なども指定可）");
        ui.end_row();
        ui.label(code("-no:artist"));
        ui.label("アーティストが設定されている曲");
        ui.end_row();
    });
    ui.add_space(8.0);

    section(ui, "組み合わせの例");
    egui::Grid::new("help_ex").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
        ui.label(code("artist:\"foo bar\" year:2010..2019 -live"));
        ui.label("foo bar の 2010 年代の曲で、live を含まないもの");
        ui.end_row();
        ui.label(code("genre:rock OR genre:metal ext:flac"));
        ui.label("ジャンルが rock または metal の FLAC。※OR は左右それぞれのグループ単位です（下の注記参照）");
        ui.end_row();
        ui.label(code("no:title ext:mp3"));
        ui.label("タイトルタグが無い MP3");
        ui.end_row();
    });
    ui.add_space(4.0);
    ui.label(
        RichText::new("※ OR は「OR で区切られた左側の条件すべて」と「右側の条件すべて」のどちらかを満たす、という意味です。\n   上の2つ目の例は「genre:rock」または「genre:metal かつ ext:flac」になります。")
            .weak()
            .size(11.0),
    );
}

fn section(ui: &mut Ui, title: &str) {
    ui.label(RichText::new(title).strong().size(14.0).color(crate::theme::c(251, 191, 36)));
    ui.add_space(2.0);
}

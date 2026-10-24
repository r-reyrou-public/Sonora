# Sonora（ソノーラ）— 音楽管理

Rust + egui で作った、Windows 向けの音楽ファイル管理・再生アプリです。

- バージョン: **0.97β**（ベータ版）
- 制作: 鈴屋玲瓏

## ダウンロード
[Releases](../../releases) から `Sonora.exe` をダウンロードしてください（単一の exe で動きます。インストール不要）。
各リリースには、ファイルの SHA-256 を書いた `SHA256SUMS.txt` を付けています。ダウンロードしたファイルと照らし合わせて確認できます。

```powershell
Get-FileHash .\Sonora.exe -Algorithm SHA256
```

### 新しいバージョンの通知
- このリポジトリの **Watch → Custom → Releases** をオンにすると、新しいリリースが出たときに GitHub から通知が届きます。
- アプリ自身にも、新しいバージョンの確認機能があります（初期状態ではオフで、通信しません。オンにすると、起動して読み込みが終わったあとに確認します。通知するだけで、自動ダウンロード・自動実行はしません）。確認先とオン/オフは、設定の「Sonora について」で変えられます。

### ウイルス対策ソフトが反応したとき
署名（コード署名証明書）をまだ付けていない新しい exe は、ウイルス対策ソフトに誤って反応されることがあります。
- ソースがこのリポジトリにあり、誰でも同じものをビルドできます（下記）。
- 誤検知だと思われる場合は、Microsoft へ誤検知として報告できます: https://www.microsoft.com/wdsi/filesubmission

## 主な機能
- フォルダの閲覧・再生、波形表示、ラベル、お気に入り、プレイリスト（右ペイン）
- タグ（ID3 など）とジャケット画像の編集、複数曲の一括編集
- Ctrl / Shift クリックで複数選択して、プレイリストへ一括追加
- 歌詞の表示（再生位置に合わせて行が切り替わる）と編集。Enter キーで、再生しながら時刻（タイムスタンプ）を打てる。曲と同じフォルダの .lrc に保存。時刻に合わせて行の色が変わり（カラオケ風）、色は設定で選べる
- WASAPI / DirectSound / WaveOut での再生、ミニプレイヤー

## ビルド
Rust（stable）と Windows が必要です。

```
cargo build --release
```

`build_rust.bat` でもビルドでき、`Sonora.exe` を作ります。起動は `run_rust.bat`。

## ライセンス
ソースコードと `Sonora.exe` は **Apache License 2.0** で公開しています（[LICENSE](LICENSE)）。Copyright 2026 鈴屋玲瓏。
利用・改変・再配布は、ライセンスの条件に従って自由に行えます。使っているライブラリの表示は [NOTICE](NOTICE) を参照してください。

## データの場所
設定（`settings.ini`）、ラベルとプレイリスト（`library.bin`）、キャッシュ、バックアップは、`Sonora.exe` と同じフォルダに保存されます（`data_location.txt` に別のフォルダを書くと、そこへ変えられます）。

### exe の確かめ方
- 0.91β 以降のリリースの `Sonora.exe` は、このリポジトリのソースから GitHub Actions でビルドしています。
- ダウンロードした exe は、`gh attestation verify Sonora.exe -R r-reyrou-public/Sonora` で、このリポジトリのソースからできたものか確かめられます。
- `SHA256SUMS.txt` の値とも照らし合わせられます。

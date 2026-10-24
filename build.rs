//! Windows 用: EXE にアイコン・製品情報（エクスプローラーのプロパティに出る名前など）・マニフェストを埋め込む。
fn main() {
    println!("cargo:rerun-if-changed=assets/sonora.ico");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/sonora.ico");
        res.set("ProductName", "Sonora");
        res.set("ProductVersion", "0.97β");
        res.set("FileVersion", "0.97.0.0");
        res.set("CompanyName", "鈴屋玲瓏");
        res.set("LegalCopyright", "Copyright (C) 鈴屋玲瓏");
        res.set("FileDescription", "Sonora - 音楽管理");
        res.set("InternalName", "Sonora");
        res.set("OriginalFilename", "Sonora.exe");
        res.set_language(0x0411); // 日本語
        // 管理者権限は要求しない（通常の権限で動く）ことと、対応する Windows を明示する
        res.set_manifest(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
      <supportedOS Id="{1f676c76-80e1-4239-95bb-83d0f6d0da78}"/>
    </application>
  </compatibility>
</assembly>"#,
        );
        if let Err(e) = res.compile() {
            // アイコンが埋め込めなくても、ビルド自体は続ける
            println!("cargo:warning=リソースの埋め込みに失敗しました: {e}");
        }
    }
}

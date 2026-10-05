# LightCraft 日本語版

開発元に追加された基本的な日本語表示（PR #110）を、編集・読み込み・書き出し画面へ広げています。
言語設定は開発元と同じ `ui.json` の `language` を使います。以前のローカル版の `settings.language` も読み込みます。

- 「編集 → 言語」または「設定 → 一般 → 言語」で日本語とEnglishを切り替えます。選択は次回起動にも引き継ぎます。
- メニュー、写真編集、マスク、切り抜き、設定、読み込み、書き出し、主な処理状況を翻訳しています。
- 通常表示と太字の双方に、SIL OFLのBIZ UDPGothicを組み込みました。
- 翻訳は表示層で行います。操作コマンドのID、写真のファイル名、入力したメタデータを変更しません。
- 翻訳されていない技術的なエラー、追加情報、リリースノートは英語で表示します。

## 保存場所

日本語版アプリは「LightCraft 日本語.app」です。既存の公式版とは別に配置します。
初期ライブラリは `~/Pictures/LightCraft Japanese Library`、設定は
`~/Library/Application Support/LightCraft Japanese/ui.json` に保存します。
既存のライブラリを使用する場合は、そのライブラリを使用中の公式版を閉じてから、
日本語版の「ファイル → ライブラリを開く…」で選びます。

## 翻訳の保守

静的な表示文言は `crates/ui-egui/locales/ja.json`、可変値を含む表示文言は
`crates/ui-egui/locales/ja-formats.json` にあります。英文をキーにして翻訳を管理します。
可変文言の両言語はビルド時にRustのフォーマット検査を受けます。
英語の単複数語尾を日本語で省略する場合、対応する文字列引数は `{:.0}` で空にします。
`LIGHTCRAFT_LANGUAGE=ja lightcraft-cli snapshot ...` で日本語の画面を描画できます。

表示・フォント・言語の切り替え・設定の保存・コマンドIDの保持は
`cargo test -p lightcraft-ui-egui i18n::tests` で検証します。

通常ビルドでは既存の英語表示と保存場所を維持します。別アプリの日本語版は
`cargo build --release -p lightcraft -p lightcraft-cli --features lightcraft/japanese-local`
でビルドしてから `scripts/package-japanese-macos.py` で作成します。

# nanavedit

AviUtl2のスクリプト互換を目指す、クロスプラットフォームの動画編集ソフトウェアです。
現在はgpui-boxを使った空の編集画面を実装しています。

## 開発環境

- Rust 1.99.0（`mise.toml`で指定）
- WindowsではMSVCのC++ビルドツールとWindows SDK
- Linuxでは日本語フォント「Noto Sans CJK JP」
- OSごとのビルド依存は[gpui-boxの互換性ドキュメント](https://github.com/fran0220/gpui-box/blob/v0.2.0/docs/compatibility.md)を参照してください。

```sh
mise exec rust@1.99.0 -- cargo run --locked
```

gpui-box関連の依存は同じ`v0.2.0`に固定しています。

## 現在の画面

- ダークテーマ、日本語表示、OS標準タイトルバー
- 中央のプレビュー枠、右の設定パネル、下のタイムライン
- 境界のドラッグによる設定パネルの幅・タイムラインの高さ変更
- Tabで境界へ移動し、左右／上下の矢印キーで10論理ピクセルずつ変更
- 初期ウィンドウサイズ1280×800、最小サイズ900×600（論理ピクセル）

パネル寸法は終了時に保存しません。動画描画、編集、保存、Lua/HLSL、
プラグインの読み込み・ウィンドウ埋め込みはまだ実装していません。
独自のwgpu Surfaceや常時再描画のタイマーも追加していません。

## 整形・検証

```sh
mise exec rust@1.99.0 -- cargo fmt
mise exec rust@1.99.0 -- cargo check --locked
mise exec rust@1.99.0 -- cargo clippy --locked --all-targets -- -D warnings
```

起動後は日本語表示、境界のドラッグ・キーボード操作、最小サイズと最大化、
ウィンドウを閉じた際のプロセス終了を確認します。
表示倍率100%・150%では文字切れとパネルの重なりも確認します。
macOS/Linuxでの実機動作と将来のGPU連携は未検証です。

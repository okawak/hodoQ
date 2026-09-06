# HodoQ アプリアイコン

ユーザーが選定した⑨「Qとチェックの一体マーク」を採用。ほどける糸のQと、タスク完了のチェックを組み合わせています。

- `hodoq.png`: 1024×1024の透過マスター。組み込みimagegenの採用案をサイズ変換し、意匠・色・アルファを維持。
- `HodoQ.icns`: macOS用。16〜512pxと各Retinaサイズを収録。
- `hodoq.ico`: Windows 11用。透過256pxのPNG形式エントリー。OSが必要サイズに縮小。
- `hodoq.rc`: GPUIが読み込むアイコンリソースID 1。
- `Info.plist`: `.app` 用テンプレート。バージョンはパッケージ生成時に実行ファイルから取得。

Windowsはビルド時にICOを埋め込みます。macOSは起動時にPNGをAppKitへ設定し、`.app`ではFinder用にICNSも同梱します。実行時に画像生成サービスへの接続や外部画像ファイルは不要です。

## 形式の再生成（macOS）

```sh
bash scripts/generate-app-icons.sh
```

コミット済みPNGからICNS・ICOを再生成します。macOS標準の `sips` と `iconutil` だけを使用し、通常のビルド時には不要です。候補資料はローカルの `design/app-icon/` に保持し、Gitには採用データだけを収録します。

## 採用案の生成プロンプト

組み込み `image_gen` を使用（CLI/APIのフォールバックなし）。外縁調整の生成も試しましたが、選定案の形を維持するため、製品には元の採用案を使用しています。

```text
Use case: logo-brand. Asset type: original cute app icon candidate for HodoQ, a personal TASK MANAGER. The core concept must combine Japanese hodoku (ほどく: untie, unravel, clear a tangle) with finishing and organizing tasks. Task-management meaning must be immediately recognizable, not merely a knitting or craft app. Square canvas, one centered icon tile with about 8% safe margin. Large simple motif legible at 32px, rounded friendly shapes, controlled soft depth, no tiny texture. Genuinely transparent background outside the rounded-square tile, no fake checkerboard. No words, no numbers, no watermark, no device mockup. A single iconic rounded open Q-shaped loop made from a thick smooth cord, with a bold check mark integrated into its center and its lower-right tail gently slipping free as if just untied. The visual hierarchy is a clear task-completion check framed by the open Q loop; make the negative space spacious, the silhouette concise. Use mostly flat clean graphic shapes with very subtle depth, not fibrous yarn texture and not a detailed knot. Distinctive simple logo mark on a rounded-square app tile, no face, no paper, no extra decorations. Cute through rounded geometry, and recognizable at tiny sizes.
```

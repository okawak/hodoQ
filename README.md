# HodoQ

HodoQ（ホドキュー）は、Rust／GPUIとSQLiteで作られた、ローカル完結の個人用タスク管理アプリです。GUIだけで主要操作を完了でき、ネットワークや外部DBサーバーは不要です。

## 対応環境

- Windows 11（x86_64、最新安定版）
- macOS（Apple Silicon、最新安定版）
- Rust 1.98.0（`rust-toolchain.toml`で自動選択）

WindowsではVisual Studio Build Toolsの「C++によるデスクトップ開発」、MSVC、Windows SDKを入れてください。macOSではXcode Command Line Toolsを入れてください。

## cloneから起動する

```text
git clone <repository-url>
cd <repository-directory>
cargo build --release
```

生成物を直接起動します。

- Windows: `target\release\hodoq.exe`
- macOS: `target/release/hodoq`

開発中は次でも起動できます。

```text
cargo run
```

初回ビルド時だけCargoが依存ライブラリを取得します。ビルド後のアプリ実行はオフラインで完結します。

## 基本操作

- 上部の「新規タスク」を押し、右側に開く作成画面で必要な項目を入力して保存します。
- 左側でスマートビュー、プロジェクト、タグ、保存済みビューを選びます。
- サイドバーの右端を左右へドラッグして幅を変更できます。完了、ゴミ箱などは「その他のビュー」から展開します。
- 中央でリスト／ボード／カレンダーを切り替えます。
- タスクを選ぶと、右側の編集画面でタイトル、メモ、状態、優先度、進捗、納期、プロジェクト、タグを変更できます。
- タイトルとメモは自動保存され、「変更を保存」で明示的に保存することもできます。保存に成功すると編集画面は閉じ、入力エラー時は開いたままになります。
- 「その他」から絞り込み・並び替え、複数選択、データ管理、取り消しなどを開きます。
- カレンダーでは今日、土日、優先度を区別し、納期未定タスクは月表示の下部へ表示します。
- 絞り込みでは状態・優先度・納期範囲・プロジェクト・タグを組み合わせ、2段階の並び替え条件を指定できます。
- 行の「複製」「保管」「削除」や右クリックメニューからタスクを操作できます。
- リスト内のドラッグで手動順を、ボードの列へのドラッグで状態を変更できます。
- 「操作を検索」から、タスク作成、ビュー移動、状態・優先度変更、バックアップを検索して実行できます。
- 納期は1つの入力欄にまとめられています。`YYYY-MM-DD`または`YYYY-MM-DD HH:MM`を直接入力できるほか、同じ欄のカレンダーボタンと15分刻みの時刻メニューからも選択できます。

キーボードショートカットは補助機能です。すべて同等のGUI操作があります。

| 操作 | Windows | macOS |
| --- | --- | --- |
| 新規タスク | `Ctrl+N` | `Cmd+N` |
| 検索 | `Ctrl+F` | `Cmd+F` |
| 選択移動 | `↑` / `↓` | `↑` / `↓` |
| 完了切替 | `Ctrl+Enter` | `Cmd+Enter` |
| 削除 | `Delete` | `Cmd+Backspace` |
| 操作を検索 | `Ctrl+K` | `Cmd+K` |
| 元に戻す／やり直す | `Ctrl+Z` / `Ctrl+Shift+Z` | `Cmd+Z` / `Cmd+Shift+Z` |

## データとバックアップ

SQLite DB、設定、ログ、バックアップ、エクスポートは、cloneしたリポジトリ直下の`.hodoq/`に保存されます。`target/release/hodoq`または`target\release\hodoq.exe`から直接起動してもリポジトリルートを探索します。

`.hodoq/`はGit管理対象外です。`git pull`、再ビルド、`cargo clean`では消えませんが、リポジトリを削除すると一緒に消えるため、必要な場合は`.hodoq/`またはバックアップを別の場所へコピーしてください。実際の保存先は「データ管理」に表示されます。

旧版のOS標準保存先にDBがあり、`.hodoq/tasks.sqlite3`がまだない場合は、初回起動時にデータを`.hodoq/`へコピーします。旧ファイルは削除しません。

「データ管理」から次を実行できます。

- 手動バックアップ
- 一覧または任意のファイルパスから、整合性検査と復元前退避を伴うバックアップ復元
- 表示結果／全タスクのCSV出力（UTF-8 BOMあり／なしを選択可能）
- 全データのJSON出力
- 確認付きのゴミ箱完全削除

起動時と24時間ごとに、30日経過したゴミ箱データを削除します。日次バックアップは直近5世代を保持します。

別の端末や空のデータディレクトリへ復元する場合は、バックアップファイルをコピーし、「データ管理」の「任意ファイルから復元」へそのパスを入力します。

動作確認などで保存先を分離する場合:

```text
cargo run -- --data-dir ./tmp/hodoq-test
```

## 開発と確認

```text
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release
```

CIはWindowsとApple Silicon macOSでテストとリリースビルドを行います。GUIの実機確認項目は [RELEASE_CHECKLIST.md](./RELEASE_CHECKLIST.md)、詳細仕様は [DESIGN.md](./DESIGN.md) を参照してください。

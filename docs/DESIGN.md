# HodoQ Rustアーキテクチャ

## 適用範囲と判断

対象は単一ユーザー向けのGPUIデスクトップアプリ、約1万行のRustコード、SQLite専用の永続化である。
単一パッケージ・単一ライブラリを維持し、責務ごとの非公開モジュールで整理する。
現時点ではワークスペース化、汎用Repository trait、DIコンテナ、別の非同期ランタイムを導入しない。
実装が一つしかない境界にtraitを置くと、変更箇所と型パラメータだけが増えるためである。

この文書は実装の構造と保守方針を扱う。[プロダクト仕様](PRODUCT_SPEC.md)には将来計画も含む。
両者でアーキテクチャの記述が異なる場合は、この文書を優先する。

## 責務と依存方向

```text
main → 起動処理 / presentation
                 ├→ application → infrastructure → domain
                 └───────────────────────────────→ domain
```

- `domain`: タスク・ID・状態・保存ビューのデータ型、検証、検索・並べ替え。GPUI、SQLite、OSの時計に依存しない。
- `application`: `TaskApplication`をUIからの操作窓口とする。DBワーカーを具体型で保持し、永続化の呼び出しを集約する。GUIには依存しない。
- `infrastructure`: SQLite接続、トランザクション、DBワーカー、バックアップ、CSV/JSON、設定・パス・単一起動ロック。
- `presentation`: GPUIの状態・入力・表示。タスクの保存は`TaskApplication`を介する。設定・パス・ロックは小規模アプリとして具体型を利用する。
- 起動処理は構成を組み立てる場所なので複数の層を参照できる。層の名前だけで完全な依存逆転を装わない。

ドメインから上位層への参照は禁止する。業務上の検索条件は`TaskFilter`に統一し、UIとDBで別々に実装しない。
UIの「高優先度を先頭にする」、非表示の旧分類ビューを除外する、といった表示固有の規則はpresentationに置く。
`chrono`はGPUIのカレンダーとの変換に限定し、業務日時は既存の`time`に統一する。

## モジュール分割

1ファイルの行数を機械的に制限するのではなく、変更理由と所有するデータを基準に分割する。
数十行の型ごとにファイルを作らず、概ね数百行で一つの責務が読める単位を目安とする。
大きなテスト群は対象モジュール配下の`tests.rs`へ置く。

- `domain/task_query.rs`: 時計・UTCオフセットを引数で受ける検索と比較。クエリ文字列は検索単位で正規化する。
- `infrastructure/repository.rs`: 接続の初期化と共通の入口。タスクSQL、分類・保存ビュー、バックアップ・出力、行変換は`tasks`・`catalogs`・`transfer`・`mapping`へ分ける。横断履歴トランザクションは親に残す。
- `presentation/workspace.rs`: ウィンドウが所有する状態、初期化、画面の組み立て。編集、履歴、一覧・ボード・カレンダー、ナビゲーション、データ管理は子モジュールに分ける。
- 既存の納期入力、タスク編集フォーム、リスト操作など、まとまっているモジュールは維持する。

子モジュールは親が所有する状態を利用し、親・兄弟から必要な操作だけ`pub(super)`で公開する。
兄弟の内部関数を直接参照せず、親の窓口または明示的なインポートを使う。
`pub(crate)`は層を跨ぐ内部契約に使う。実装都合の型を無制限に`pub`へ広げない。
GPUIの単一Entityに属する画面処理は`impl Workspace`を分けてよい。ファイルを分けるためだけのEntity、trait、イベントバスは追加しない。
独立した状態・ライフサイクルが必要になった時点でコンポーネントを別Entityへ昇格する。

## データ・エラー・非同期処理の境界

SQLite接続は専用スレッドが一つ所有する。チャネルの要求・応答で書き込み順序とコミット完了を確認する。
複数タスクや履歴に含むプロジェクト・タグは一つのトランザクションで保存する。
削除・復元・バックアップの挙動、DBスキーマ、JSON/CSVの形式は分割で変更しない。

現状のワーカーAPIは同期的に応答を待つ。SQLはワーカースレッド上だが、直接呼ぶUI操作は待機する。
バックアップ・出力等の既存のバックグラウンド呼び出しは維持する。
全コマンドの非同期化は、編集中の切り替え・失敗時の巻き戻しを含む仕様変更なので今回の分割に混ぜない。

エラーは`Result`で伝播する。`infrastructure/error.rs`がSQL・I/O等の詳細を所有し、`TaskApplication`は`ApplicationError`へ変換する。元のエラーを非公開フィールドに保持し、表示・診断を維持する。UIでDBの具体的なエラーvariantを組み立てない。書き込み可能な状態への再接続判定もapplicationで行う。

`AppDataSnapshot`はdomainの読み取りデータ型であり、ワーカー実装への依存を持たない。既存の`infrastructure::AppDataSnapshot`パスは再エクスポートで互換性を保つ。
`anyhow`は起動時の集約に用いる。ユーザー入力に対して`unwrap`/`expect`を追加しない。
既存のUUID newtypeと状態enumを維持し、文字列や整数による識別子の混同を避ける。
型や依存の変更を目的化せず、使われているライブラリは理由なく更新・追加しない。

## 検証と変更サイクル

各PRで以下を確認する。

```sh
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo test --locked --release --lib --all-features performance_ -- --ignored --test-threads=1 --nocapture
```

Windows/macOSのCI（release buildを含む）を確認する。
検索は時刻・オフセットを固定した境界テスト、永続化は実際のin-memory SQLiteの原子性・往復テスト、画面は既存GPUIテストを用いる。
分割をなぞるだけのテストは追加せず、挙動や不変条件を検証する。
コミットは署名付きで作成する。PRごとに`@codex review`を依頼し、指摘の採否・根拠を記録して、修正と検証後にマージする。

## 段階計画

1. 設計書の集約と検索・並べ替えのドメイン境界化。
2. 永続化の責務分割とアプリケーション境界のエラー・データ型整理。
3. Workspaceの責務分割、画面検索の共通化、最終検証。

## 参照したRust公式資料

- [The Rust Programming Language: Managing Growing Projects](https://doc.rust-lang.org/book/ch07-00-managing-growing-projects-with-packages-crates-and-modules.html): 規模に応じたモジュールと可視性。
- [Separating Modules into Different Files](https://doc.rust-lang.org/book/ch07-05-separating-modules-into-different-files.html): ファイル分割とモジュール構造の関係。
- [Rust API Guidelines: Checklist](https://rust-lang.github.io/api-guidelines/checklist.html): 型安全性・依存性・文書化。外部向けライブラリの指針はこの内部アプリの規模に合わせて適用する。

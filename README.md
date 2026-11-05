# LocalMind (Rust core)

設計: `docs/`（`ROADMAP.md`, `TASKS.md`, `ARCHITECTURE.md`）。

## クレート
- `lm-scheduler` — 優先度（録音＞対話＞索引化＞再処理）、重い処理は同時に1つ、充電中・低温ゲート
- `lm-router` — 用途別の経路（Nano / GGUF / PC / whisper）決定と GGUF への縮退
- `lm-search` — 正規化・トークナイザ・BM25・ベクトル・RRF・根拠スコア（「見つかりません」判定）
  - feature `lindera`: IPADIC 形態素解析（辞書埋め込み。初回ビルドは辞書の取得・生成で時間がかかる）
  - feature `sqlite`: SQLite 永続索引（FTS5 + sqlite-vec 0.1.9、C コンパイラが必要）
  - feature `onnx`: 実埋め込みモデル multilingual-e5-small（fastembed / ONNX Runtime）。初回にモデル
    （fp32 で約 470MB）を取得してキャッシュし、以後はオフライン。ビルドは重い（`-j2` 推奨）

llama.cpp / whisper.cpp / AICore / 実埋め込みモデルは trait で抽象化済みで未統合。

- `lm-ingest` — 録音の取り込み: 30秒チャンク分割・原子的な保存とクラッシュ復元・VAD・文字起こしパイプライン
  （`Transcriber` trait の裏に whisper.cpp を差し込む）・議事録の土台（仮→高精度の差分更新、話者の手動命名、
  タイムスタンプ参照から検索索引へ）

## 開発
```
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all --check
```

## 閾値の再測定
トークナイザ・索引・埋め込みを変えたら閾値を測り直す:
```
cargo run -p lm-search --features lindera,sqlite --example threshold_sweep -- <ngram|lindera|sqlite|sqlite-lindera>
```
既定: ngram=0.25 (`DEFAULT_GROUNDING_THRESHOLD`) / lindera=0.30 (`LINDERA_GROUNDING_THRESHOLD`)。

## 入力の上限（`lm-search::index`）
信頼できない入力（共有シート由来のテキスト等）に備え、次の上限を設けている。

| 定数 | 値 | 意味 |
|---|---|---|
| `MAX_QUERY_CHARS` | 4096 | クエリの最大文字数（超過分は切り捨て） |
| `MAX_QUERY_TERMS` | 64 | クエリから使う異なる語の最大数（出現順） |
| `MAX_DOC_BYTES` | 8 MiB | 1 文書の最大サイズ（超過は `Config` エラー。分割して登録する） |
| `MAX_K` | 4096 | 1 回の検索で要求できる最大件数（sqlite-vec の上限。超過は丸める） |

## 索引の互換性
SQLite 索引は meta に 次元・tokenizer・embedder・正規化の版 を保存し、不一致なら開けない
（`SearchError::Config`）。トークンの切り方・埋め込み・正規化 (`NORMALIZE_VERSION`) を変えたら再索引する。
`Tokenizer::id` / `Embedder::id` は挙動を変えるたびに更新すること。

## ビルドの再現性・依存の運用
- `sqlite-vec` は `=0.1.9` に固定している（0.1.10-alpha.4 は C ソースがパッケージに欠けてビルド不能だった）。
  上流の安定版が出たら更新を検討する。
- `lindera` の `embed-ipadic` は **ビルド時に辞書アーカイブをネットワークから取得**する
  （取得物は lindera-dictionary 内の MD5 で検証されるが、MD5 は衝突耐性が弱い）。
  再現性とサプライチェーンの観点から、CI・リリースビルドでは次を推奨する:
  - `cargo build --locked`（`Cargo.lock` でバージョン固定）
  - 辞書アーカイブをキャッシュ／ベンダリングしてオフラインでビルドする
  - ビルド成果物の SHA-256 を記録して比較する
- `cargo audit` / `cargo deny` を CI に導入する（この環境には未導入）。

## 実埋め込みモデル（onnx）の使い方と推奨設定
```
HybridConfig::onnx_e5()   // semantic_floor=0.78, grounding_threshold=0.28
```
- e5 の cosine は無関係な文でも 0.76〜0.85 に偏る（評価セットで実測）。そのため意味側の根拠は
  `(cosine - semantic_floor) / (1 - semantic_floor)` に引き直してから使う。
- クエリは `query: `、文書は `passage: ` の接頭辞が必要。`Embedder::embed_query` / `embed_document` で振り分ける。
- e5 は約 512 トークンまで。長い文書は呼び出し側でセグメントに分けて登録する。
- 評価セット（回答可能 29 問・回答不能 14 問）では、下駄 0.74〜0.80 で誤りゼロの閾値がある
  （ハッシュ埋め込みの最良は balanced error 0.017）。ただし**下駄と閾値は連動し、許容幅は狭い**:
  下駄 0.78 では閾値 0.26〜0.30 の 5 点だけが誤りゼロで、0.25 以下は誤受理 7.1%、0.31 以上は
  取りこぼし 3.4%。採用値は範囲の中央（0.78 / 0.28）。評価セットも小さいので、実利用に近い
  質問・資料で測り直すこと（`onnx_sweep` が 0.01 刻みで許容幅を出力する）。
- 依存: `onnx` はビルド時に ONNX Runtime 本体をダウンロードする（fastembed の
  `ort-download-binaries-rustls-tls`。`ort` は RC 版を固定）。CI・オフラインビルドではネットワークが要る。
  `HF_HOME` が設定されていると、指定したキャッシュ先より優先される。
- 実モデルのテストは `#[ignore]`。実行:
```
LM_ONNX_CACHE=<モデルのキャッシュ先> cargo test --release -p lm-search --features onnx,sqlite   --test onnx_model -- --ignored --test-threads=1
cargo run --release -p lm-search --features onnx --example onnx_sweep -- <モデルのキャッシュ先>
```

## FFI 層（Kotlin 連携）と リリース時の確認

- `crates/lm-ffi`: UniFFI。バインディング生成は `cargo build -p lm-ffi` の後、
  `cargo run -p lm-ffi --features bindgen --bin uniffi-bindgen -- generate --library target/debug/lm_ffi.dll --language kotlin --out-dir dist/android/kotlin`
  （strip 済みの `.so` からは生成できない。Linux/macOS ではライブラリ名を読み替える）。
- `ForeignEmbedder` の `dim()`・`id()` は `open` のときに 1 度だけ読む。索引のパスはアプリ専用領域の絶対パス（`..` なし）。
- `[profile.release]` の `panic` を `abort` にしない（FFI がパニックを例外に変えるため unwind が前提）。
- リリース前: `cargo update -p webpki-roots`（TLS のルート証明書は同梱の静的な一覧。社内の TLS 検査プロキシ越しでは
  モデル取得が失敗する）、`cargo deny check`（OpenSSL 系は `deny.toml` で禁止）、`cargo fmt/clippy/test`。
- `fastembed` の `ort-download-binaries-rustls-tls` はビルド時に ONNX Runtime の実行ファイルを取得する。モデル本体と違い
  ハッシュ固定の対象外なので、リリースビルドでは `ORT_LIB_LOCATION` で検証済みのライブラリを指定するか、取得物を CI で確認する。
- 議事録・文字起こしのテキストは話者の発話に由来する（プロンプトインジェクションの可能性）。アクションアイテムは
  引用元の音声・文字起こしと一緒に表示し、他システムへ渡すときは信頼しない入力として扱う。
- 自動削除（`purge_expired`）の `now_ms` はミリ秒の UNIX 時刻。2024 年より前の値は拒否され、1 回の削除は 100 件まで。
  `PurgeReport.kept_with_reason` にはディレクトリ名が入るので、ログや不具合報告に転送しない。

### ONNX Runtime の入手方法（target 別）
- Windows/Linux/macOS: `ort-download-binaries-rustls-tls`（ビルド時にビルド済みバイナリを取得）。
- Android: `ort-load-dynamic`（`crates/lm-search/Cargo.toml` の target 別依存）。アプリに同梱した `libonnxruntime.so` を、
  環境変数 `ORT_DYLIB_PATH` のパスから実行時に読む。同梱モデルは `OnnxEmbedder::from_bundled_dir`（ネットワークなし、SHA-256 検証）。

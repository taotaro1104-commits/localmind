# 実装タスク一覧

ID 規則: `<機能記号>-<連番>`。全タスク共通の受け入れ条件: テストを先に書く（TDD）、カバレッジ 80%+（Rust は `cargo test`、Kotlin は unit / instrumented）。
工数・リスクは [ROADMAP.md](ROADMAP.md) の機能単位の値。

## フェーズ0: 基盤（推定追加）
- [x] P0-1 Rust workspace・CI（fmt/clippy/test）
- P0-2 UniFFI 境界と Compose 雛形
- [~] P0-3 ワークスペース RAG（文書取り込み・分割・埋め込み・引用）（`lm-workspace`: テキストと Markdown を見出し・段落で分割し、原本から決まった手順でチャンクを作る。検索結果に文書・見出しの並び・行番号・抜粋の出典がつく。追加・削除・作り直し、件数 200・合計 16MiB・1 文書 256KiB の上限、分割方式の版が変わったときの自動再構築は完了。FFI は `WorkspaceEngine`、サンプルアプリに検索画面と資料の追加・削除（確認つき）あり。検索結果は、1 位との cosine の差が 0.04 以内のものだけを残す足切りつき（実モデルで測定。`examples/cutoff.rs`）。サンプルアプリは、共有シート（テキスト・Markdown のファイル）とファイル選択から取り込める。PDF・URL の取り込みは未着手）
- P0-4 Privacy Inspector（通信・公開内容の記録と表示）
- P0-5 device profiler と自動モデル選択
- [x] P0-6 **scheduler**: 優先度（録音＞対話＞索引化＞再処理）、直列実行、充電中・温度条件、「重いモデル同時1つ」強制 / WorkManager 橋渡し
- [x] P0-7 **router**: Nano / GGUF / PC の経路決定、GGUF への縮退、ベンチ結果キャッシュ

## MVP
### E ゼロダウンロード起動（依存: P0-5, P0-7）
- E-1 初回端末診断（RAM、Nano 可否、空き容量、5秒以内）
- E-2 AICore 経由 Gemini Nano アダプター（Kotlin 薄層に隔離）
- E-3 Nano 非対応・ブートローダー解除端末での GGUF 縮退（受け入れ: Nano 不可でも全機能が動く）
- E-4 推奨 GGUF 1つの提示と Wi-Fi 時ダウンロード、待ち時間の導線
- E-5 「より高品質なモデルを入れる」後日提案

### G ハイブリッド検索（依存: P0-3）
- [x] G-1 Lindera 形態素解析（feature `lindera`）＋ FTS5 BM25（feature `sqlite`）。bi-gram 版も残す
- [x] G-2 sqlite-vec ベクトル検索（vec0 / cosine、feature `sqlite`）。実埋め込みモデルは未統合（ハッシュ埋め込み）
- [x] G-3 RRF 統合
- [x] G-4 表記ゆれ正規化（全角半角、旧字体、カタカナ長音）
- [x] G-5 根拠スコアと閾値、「資料には見つかりません」（一般知識で答えるかを選ばせる）
- [x] G-6 評価セット（型番・人名・条文番号）で再現率を測定（`lm-search::eval`、39問。既定閾値 0.25）

### C 共有シート・選択メニュー
- [~] C-1 共有シート受信（PDF・URL・テキスト）→ 要約／ワークスペース追加（テキストと Markdown の受信・ワークスペースへの追加はサンプルアプリで動作。PDF・URL・要約は未着手）
- C-2 PROCESS_TEXT（翻訳・言い換え・説明）

## v1.5
### A オフライン議事録（依存: P0-6, ingest）
- [~] A-1 Foreground Service（microphone）、30秒チャンク逐次保存、クラッシュ復元（`lm-ingest`: チャンク分割・原子的保存・復元は完了。`lm-ffi::RecordingStore`/`Recorder` で Kotlin へ公開。サンプルアプリの `RecorderService` が、マイクを 16kHz で読んで 30 秒ごとに保存し、停止時に端数も保存する。エミュレータ（API 35）で、開始・保存・停止・一覧を確認。ハードリンクを許さない Android の SELinux でも録音を始められるよう、メタの新規作成を `O_EXCL` へフォールバックする。録音は 1 度に 1 つで、止める処理が終わるまで次を始めない。保存に失敗した録音は、抜けのある並びを作らず終了扱い。強制終了後の自動再開はしない（`START_NOT_STICKY`）。arm64 実機は未確認）
- [~] A-2 録音開始時の同意確認（設定で OFF 可）、録音中の常時通知、自動削除期限（`lm-ingest`: `begin_recording` は同意なしを拒否し、`save_chunk` は同意が確認された録音にだけ保存する。`revoke_consent`、`clean_stale_tmp`、`purge_expired`（保持日数 0 拒否、現在時刻の妥当性検査、1 回 100 件の上限。メタが読めない録音は、録音ディレクトリの最終更新時刻から保持期間を過ぎたら削除）、録音の一覧 `list_recordings`。サンプルアプリ: 録音前の同意確認ダイアログ（設定でオフにできる。オフにする前にもう一度確認）、録音中の常時通知（消せない・ロック画面にも表示・停止ボタン・経過時間。通知が無効なら録音しない）、保存期間（7/30/90 日）の設定、画面を開いたとき・録音を止めたとき・1 日ごとのジョブ（再起動後も継続）での自動削除、録音の一覧と削除（確認つき）、バックアップ対象から除外。端末の時計が巻き戻されると期限切れにならない点は、仕様として許容）
- [x] A-3 VAD（無音除去）（`lm-ingest::vad`: エネルギー VAD、ノイズフロアの自動調整）
- [~] A-4 whisper.cpp 2段構成（録音中は小型モデル、充電中に大型で再処理し差分更新）（`lm-ingest`: `Transcriber` trait・Draft/Refined・差分更新は完了。whisper.cpp 本体の統合は未着手）
- [~] A-5 話者A/B を後から手動命名する UI（`lm-ingest::Transcript::rename_speaker` は完了。UI は未着手）
- [~] A-6 LLM 抽出（要約・決定事項・ToDo・未解決論点）とタイムスタンプ付き音声引用（`lm-ingest`: セグメント単位の索引化と、参照 id → 時刻の復元、`MinutesExtractor` trait と LLM 出力の検証 `extract_minutes`（実在するセグメントを引用しない項目は採用しない・無害化・件数制限）は完了。LLM 本体（Nano/GGUF）との接続は未着手）
- A-7 講義モード（スライド写真を録音タイムラインに紐づけ）
- [~] A-8 データモデル RECORDING / TRANSCRIPT_SEGMENT / ACTION_ITEM（RECORDING・TRANSCRIPT_SEGMENT 相当は `lm-ingest` に実装。ACTION_ITEM 相当の `ActionItem`（担当者・期限・完了・根拠の引用）は `lm-ingest::minutes` に実装。`save_minutes` / `load_minutes` で録音ディレクトリへ永続化（テキスト形式、読み込み時に再検証）。DB 化は未着手）
- A-9 処理時間の事前見積もり表示、Thermal mode 連動

### B 撮影・日本語 OCR
- B-1 文書スキャン（台形補正、影除去、複数ページ→1 PDF）
- B-2 OCR エンジン選定: ML Kit（日本語・端末内）と PaddleOCR ONNX を実機で精度比較し決定（縦書き・ルビ・表）
- B-3 撮影直後の3ボタン（質問・要約・ToDo 抽出）
- B-4 スクショ自動取り込み（ユーザー指定フォルダのみ監視、充電中に OCR）
- B-5 OCR 信頼度表示と元画像並置の修正 UI
- B-6 SCAN → DOCUMENT 変換・索引化

### F 出力テンプレート（依存: G-5 周辺、GBNF）
- F-1 JSON スキーマ指定の構造化出力（GBNF）
- F-2 組み込みテンプレート: 議事録・日報・週報・比較表・FAQ・暗記カード・メール下書き
- [~] F-3 書き出し: Markdown / CSV / Anki(.apkg) / ICS（議事録の Markdown・CSV・iCalendar（アクションアイテムを ToDo に）は `lm-ingest::export` に実装。`lm-ffi::RecordingStore::export_minutes` で Kotlin へ公開。本文に混ぜられた記号が、書き出した文書の構造や動作（Markdown の見出し・リンク・HTML、CSV の数式、iCalendar の行・部品）を変えないよう、形式ごとに逃がす。期限は日付として読めるものだけ `DUE` にする。Anki は暗記カードの生成（H-1）が前提なので未着手。アプリの画面からの書き出し・共有は、議事録を作る LLM の接続（A-6）が前提なので未着手）
- F-4 ユーザー自作テンプレートのファイル共有（プロンプトのみ、実行コードなし）

### C ウィジェット・タイル
- C-3 ホームウィジェット（音声質問・録音開始・今日の ToDo）
- C-4 クイック設定タイル（1タップ録音・質問）
- C-5 通知からの返信（長時間処理完了→追質問）

## v2
- D-1 AppFunctions: searchWorkspace / summarizeDocument / createMeetingNote / listTodos（Android 16+、Kotlin 薄層）
- D-2 返却は要約と出典のみ（原文全体は返さない）
- D-3 EXPOSURE_LOG 記録と Privacy Inspector 表示
- D-4 ワークスペース単位の公開 ON/OFF、機密ワークスペースは常に非公開
- H-1 資料から確認問題・暗記カード生成
- H-2 間隔反復（SRS）出題
- H-3 誤答→元資料ページへジャンプ
- I-1 文書翻訳（対訳表示）
- I-2 カメラ翻訳
- I-3 対面通訳モード（上下分割画面）
- K-1 記憶候補の提示（承認したものだけ保存）
- K-2 メモリ一覧（閲覧・編集・削除）、ワークスペース分離
- N-1 ローカル TTS 読み上げ
- N-2 ふりがな付与、やさしい日本語への言い換え
- N-3 文字サイズ連動

## v2.5
- J-1 複数文書の比較表・相違点・矛盾点抽出
- J-2 充電中バックグラウンド実行と完了通知
- L-1 デスクトップ版（Rust core 共有、Windows/macOS/Linux）
- L-2 LAN 内ワークスペース E2E 暗号化同期（ペアリング鍵）
- L-3 PC オフロード（スマホから PC の大モデルへ）
- M-1 ワークスペース単位の生体認証ロック、FLAG_SECURE、最近アプリ画面のぼかし
- M-2 Managed configurations（ネット利用禁止、AppFunctions 公開禁止、許可モデル一覧）
- M-3 端末内監査ログと書き出し

## 横断（全フェーズ）
- X-1 UX: 5タブと「＋取り込む」、実行経路バッジ、オンボーディング
- X-2 KPI の端末内集計（テレメトリは既定 OFF・オプトイン、送信内容を Privacy Inspector に表示）
- X-3 収益化ゲート: 無料／Pro／Business のプラン制御（価格・録音上限は未決）

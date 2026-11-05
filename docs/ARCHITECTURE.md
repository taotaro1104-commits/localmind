# アーキテクチャ（v2 設計書からの推定）

> ベース設計書は無いため、v2 の記述から推定した。推定箇所は「(推定)」と記す。

## 基本構成（変更しない）
Jetpack Compose → UniFFI → Rust core → llama.cpp

v2 では Rust core に 6 モジュール、ランタイムに whisper.cpp・OCR・Gemini Nano を追加する。

## Rust core 追加モジュール

| モジュール | 責務 | 主な依存 |
|---|---|---|
| scheduler | 重い処理の直列化、優先度（録音＞対話＞索引化＞再処理）、充電中・温度条件での実行、WorkManager との橋渡し。「同時に重いモデルは1つ」を強制 | tokio、WorkManager（Kotlin側） |
| router | 用途別の経路決定（Nano / GGUF / PC）、縮退、ベンチ結果キャッシュ | 既存 device profiler (推定) |
| ingest | 録音チャンク管理、VAD、文字起こしジョブ、OCR 結果の正規化、文書パース | whisper.cpp、OCR アダプター |
| search | Lindera 形態素解析＋FTS5 BM25、sqlite-vec ベクトル検索、RRF 統合、根拠スコア | lindera、rusqlite、sqlite-vec |
| template | 出力テンプレート、JSON スキーマ構造化出力、MD/CSV/ICS/Anki 書き出し | GBNF（llama.cpp） |
| memory | 承認済み事実の保存・検索・削除、ワークスペース分離 | SQLite |
| sync | PC コンパニオンとの LAN 同期、ペアリング鍵、E2E 暗号化スナップショット | rustls、age 系暗号 |

（設計書は「6 モジュール」と記すが表には 7 件ある。実装時に確認すること。）

## ルーティング
- 短い要約・言い換え → Gemini Nano（AICore）
- 文書 RAG → llama.cpp GGUF
- 文字起こし → whisper.cpp
- 重い処理 → PC オフロード（任意）
- Nano／AppFunctions は Kotlin 側の薄いアダプター層に隔離し、常に GGUF へ縮退できること。

## データモデル追加
- RECORDING（音声チャンク列、長さ、同意確認フラグ）→ TRANSCRIPT_SEGMENT（開始・終了、話者ラベル、テキスト、信頼度）
- SCAN（元画像、OCR テキスト、信頼度、ページ）→ 既存 DOCUMENT に変換して索引化
- ACTION_ITEM（内容、期限、出典セグメント）
- MEMORY_FACT（内容、承認日時、出典会話、ワークスペース）
- EXPOSURE_LOG（AppFunctions 呼び出し元、返した内容の要約、日時）— Privacy Inspector が表示

## UX
- 下部タブ: ホーム / 記録 / 資料 / チャット / 設定。ホーム中央に「＋取り込む」（録音・撮影・ファイル・貼り付け）。
- 回答に実行経路バッジ（端末内・モデル名 / PC / Nano）。
- 「資料に見つからない」は推測せず、一般知識で答えるかをユーザーに選ばせる。
- オンボーディング: 端末診断（5秒）→ Nano 即利用 or 推奨 GGUF 1つを Wi-Fi で取得 → 待ち時間に資料選択・録音体験 → 機内モードのチュートリアル。

# LocalMind Android サンプル

`liblm_ffi.so`（Rust コア）を Kotlin から呼ぶ最小のアプリ。起動すると自己テストを 2 段階で実行し（基本 14 項目 + 同梱の実モデル e5 の 7 項目）、
結果を画面と logcat（タグ `LMSample`）に出す。UI は標準 View のみ（Compose なし）。

- Rust 成果物は `../dist/android/{jniLibs,kotlin}` を直接参照する（コピーしない）。先にそちらを作ること
  （`cargo ndk ... build --release -p lm-ffi` と bindgen。手順は `../README.md`、`../dist/android/BUILD_STATUS.txt`）。
- モデル（multilingual-e5-small、約 470MB）は APK に同梱する（APK は約 530MB）。`dist/android/assets/e5-small/` に
  5 ファイルを置く。取得は `LM_ONNX_CACHE=<dir> cargo test -p lm-search --features onnx --test onnx_model -- --ignored`
  などで `<dir>/e5-small-614241f622f5/` に落ちる（SHA-256 検証済み）ので、そこから `dist/android/assets/e5-small/` へコピーする。
  起動時に `filesDir` へ展開し、Rust 側が読み込み時に SHA-256 を再検証する（ネットワークは使わない）。
- ONNX Runtime は公式 AAR（`onnxruntime-android`）の `libonnxruntime.so` を使い、Rust（ort の load-dynamic）が
  環境変数 `ORT_DYLIB_PATH` のパスから読む。推論と tokenizer は Rust 側で、Kotlin は呼ぶだけ。
- ABI は `arm64-v8a` と `x86_64` のみ（`app/build.gradle.kts` の `abiFilters`）。
- 依存は JNA（`net.java.dev.jna:jna:5.17.0@aar`）と ONNX Runtime（`onnxruntime-android:1.24.3`、`ort` クレートが要求する
  API 24 以上）だけ。依存の改ざん対策として `gradle/verification-metadata.xml`（SHA-256）と、Gradle 配布物の
  `distributionSha256Sum` を固定している。依存を更新したら `gradlew --write-verification-metadata sha256 assembleDebug` で
  作り直す（作り直した時点で取得したものを信頼する方式なので、差分は必ず確認すること）。

## ビルドと実行

```
set JAVA_HOME=...jdk-17以上   set ANDROID_HOME=...\Android\Sdk
gradlew.bat assembleDebug
adb install -r app\build\outputs\apk\debug\app-debug.apk
adb shell am start -n com.localmind.sample/.MainActivity
adb logcat -d -s LMSample:I          # "基本: ALL PASS (14)" と "実モデル(e5): ALL PASS (7)" が出れば成功（デバッグビルドのみ出力）
```

## 確認する内容
経路決定（縮退・エラーが Kotlin 例外になる）、検索（登録・検索・「見つかりません」・再オープンでの永続化）、
Kotlin 実装の `ForeignEmbedder` の注入、`dim()` の例外がクラッシュにならないこと、相対パスと不正設定の拒否。
基本段階の `ToyEmbedder` は動作確認用のハッシュ埋め込み。実モデルの段階は `SearchEngine.openE5` を使う。
モデルの読み込みは約 20 秒かかる（エミュレータ）。UI スレッドから呼ばないこと。モデルの読み込みで約 1GB のメモリを使うので、
同時に 2 つ開かない（再オープンの確認は、先のエンジンを閉じてから行う）。実機のメモリが 4GB 以下の場合は未確認。

## 展開処理の挙動
- 長さが同じファイルは再展開しない。`openE5` が整合性エラー（`FfiException.Integrity`。ハッシュが固定値と合わない）で失敗したら、展開先を作り直して 1 度だけやり直す。
- 一時ファイルに書いて `fsync` してから置き換える。失敗した断片は消す。書き始める前に空き容量を確認する。
- `assets` にモデルが無いビルドは、起動時に「assets に ... がありません」と表示する。
- ネイティブライブラリは `useLegacyPackaging = true` で `nativeLibraryDir` へ展開される（APK 内の圧縮コピーと合わせて
  ネイティブ分のディスク使用量が増える。Play のアプリバンドル最適化とは相性が悪いので、配布形態に応じて見直すこと）。

## 録音（A-1 / A-2）
自己テストが合格すると、結果画面に「録音する」が出る。録音画面では次のことができる。
- 録音の前に、参加者の同意を確認するダイアログが出る（設定でオフにできる。オフにする前にもう一度確認する）。同意が確認されていない録音は、Rust 側が始めさせない。
- 録音中は、消せない通知を常に出す（ロック画面にも出る。経過時間と「停止」ボタンつき）。通知が無効なら録音しない。
- 音声は端末の中の `files/recordings/<録音id>/` に、30 秒ごとに保存する（`NNNNNNNN.pcm` と `meta.txt`）。
- 保存期間（7・30・90 日）を過ぎた録音は、画面を開いたとき、録音を止めたとき、1 日ごとのジョブで消える。
- 権限: マイク（`RECORD_AUDIO`）と通知（`POST_NOTIFICATIONS`）。録音画面は外部のアプリから開けない（`exported=false`）。

エミュレータで試すとき: マイク入力を使うので、`-no-audio` を付けずに起動する（音は無音でよい）。録音画面は外から直接起動できないので、自己テストの結果画面のボタンから開く。

## 議事録（LLM）

文字起こしの結果の「議事録を作る」は、`liblm_llm_ffi.so`（llama.cpp）で議事録を抽出する。モデル（約 1.1GB）は APK に
同梱しない。無いときは、アプリが案内を出し、利用者が同意すると取得する（`ModelDownloader.kt`）。
候補はコンボボックス（Qwen2.5 の 0.5B / 1.5B / 3B。大きさと SHA-256 は固定。接続先は `huggingface.co` と配信先の
`*.cdn.hf.co`・`*.xethub.hf.co` だけ）から選ぶか、URL を直接入れる。直接入れた URL は https のみ・認証情報なしで、
サイズが分かるものだけ受け付け、GGUF の先頭の印を確かめる。SHA-256 を入れたときは照合し、入れないときは取得した内容の
SHA-256 を記録して使う（取得元を信頼できる場合だけ）。取得したモデルは `files/llm/` に置き、使うものを `active.txt` に
記録する（ほかのモデルは消す）。途中で止まっても、続きから再開できる。
このアプリがネットワークを使うのは、ここと、質問画面で利用者が「Wikipedia を検索」を選んだとき、資料の「URL から追加」を使ったときだけ（`INTERNET` 権限はこのため）。オフラインで使うなら、`adb push` のあと
`adb shell run-as com.localmind.sample cp /data/local/tmp/<ファイル> files/llm/` で置いてもよい
（標準の 1.5B のファイル名なら、そのまま使える）。
`scripts/build-android.sh` は `liblm_ffi.so` と `liblm_llm_ffi.so` の両方を作る。Kotlin のバインディングは両方とも
同じビルド（`lm-ffi` は `--features "onnx whisper"`）から生成し直すこと。

## 画面の言語（日本語・英語）

画面の文言は日本語と英語。端末の言語が日本語なら日本語、それ以外は英語で出す（Android 13 以降は、設定のアプリごとの言語で
選べる。`res/xml/locales_config.xml`）。文言は、使う場所に `t("日本語", "English")` の形で並べて書く（`I18n.kt`。
`$名前` で値を差し込める）。新しい文言を足すときは、両方の言語を書くこと。

- Rust 側のエラーの文言は日本語。英語の画面では、`ErrorText.kt` の対訳表で英語にする。表に無い文言は、日本語のまま出さずに、
  その場面の汎用の英語の文言にする。Rust 側に新しいエラーの文言を足したら、対訳表にも足すこと（`I18nTest` で確かめる）。
- サンプルの資料と例の質問は、初めて開いたときの言語で登録する（日本語・英語の両方を用意）。
- **文字起こし（whisper）の言語と議事録のプロンプトは、日本語のまま**。質問への回答は、質問と同じ言語で答える。
- 日本語の文言を確かめる JVM テストは、`JapaneseUi` を継承して、画面の言語を日本語に固定する。

## 資料の URL 取り込みとグループ

検索画面の「資料を追加」から、ページの URL を指定して資料にできる（`UrlImporter.kt`）。
- https のみ。認証情報つき・IP の直書き・ポート指定（443 以外）・短い名前（社内ホスト）は断る。接続先は、名前の解決結果が
  すべて公開アドレスで、さらに接続した相手のアドレスも TLS の前に検査する（`NetGuard.kt`。DNS の差し替えへの対策）。
  転送は 5 回まで、転送先も同じ検査を通す。HTML・テキスト・Markdown だけ受け付け、読む量（展開後）と時間（30 秒）に上限がある。
  文字コードは UTF-8・Shift_JIS・EUC-JP。通信するのは、入れた URL へ GET を 1 回だけ。
- HTML は Rust 側（`lm_workspace::html`）で本文だけにする（`script`・`style`・ナビゲーション・フッターなどを捨て、
  見出しは `#`、箇条書きは `-`、表のセルは空白区切り）。ページ内の「取得元: URL」の行を末尾に足す。
- 資料には**グループ**を付けられる（40 文字まで）。一覧は「すべて」のときグループごとに区切って出し、上のチップで
  グループを切り替える。追加するときのグループは「追加先のグループ」の欄（チップを切り替えると、そのグループが入る）。
  資料の「グループ」から付け替えられる。文書ファイルは新形式 `LMDOC2`（古い `LMDOC1` も読める。古いアプリでは
  新形式を読めない）。サンプルの資料は「サンプル」グループに入る。グループは一覧の整理用で、検索の範囲は変わらない。

## AI モデルの切り替え

質問画面の「AI モデル: …」（議事録のダイアログの「モデルを変更」も同じ）から、取得済みのモデルを切り替える。
モデルを押すと、そのモデルを次の質問・議事録から使う。使っていないモデルは「削除」で消せる（容量を空ける）。
「モデルを追加」から、新しいモデルを取得できる（取得したモデルは、使うモデルになる）。取得済みのモデルは勝手に消さない
（`files/llm/installed.txt` に `ファイル名|SHA-256` で記録。`adb` で手で置いたモデルは、記録しないと一覧に出ない）。
議事録の作成・質問・ダウンロードの最中は、切り替えも削除もできない（`ModelGate.kt`）。

## 質問応答（資料なし）

結果画面の「AI に質問する」は、資料を読み込まずに、質問だけに答える（議事録と同じモデルと `liblm_llm_ffi.so` を使う）。
答え方は 2 つ。
- **端末の中だけで答える**（既定）: 通信しない。モデルの知識だけで答え、確かでないことは「わかりません」と書かせる。
- **必要なら Wikipedia を検索して答える**: LLM が、質問に Web 検索が要るかを判断する（最新・時事を示す語がある質問は、
  モデルが「不要」と答えても検索する）。要るときだけ、LLM が作った短い検索語（80 文字まで。URL・メール・6 桁以上の数字は送らない。語が作れなければ、質問を代わりに送らず端末の中だけで答える）を、**送る前に確認ダイアログで見せて**、「送る」を選んだときだけ日本語版 Wikipedia
  （`ja.wikipedia.org`、API キー不要）へ送り、返ってきた紹介文（最大 5 件）を出典として回答を作る。質問の全文・録音・
  資料は送らない。回答の上に「端末の中だけで作成」か「Wikipedia に『…』を送って作成」を色つきで出し、データが外へ出たかを
  一目で分ける。検索に失敗したり、見つからなかったりしたときは、その旨を断って端末の中の知識だけで答える。
- 回答の `[1]` などの番号は、実在する出典の番号だけを残す（モデルが作った番号は消す）。出典の URL は、日本語版
  Wikipedia の https だけを受け付ける。検索結果と質問は、命令に従わないデータとして区切って渡し、`<` は全角にして区切りを
  作らせない。
- 質問画面が隠れると、モデルを閉じる（メモリを返す。実行中なら中止する）。議事録の作成とは同時に動かさない。

## 配布用の APK

署名つきの release は、環境変数 `LM_KEYSTORE` / `LM_KEYSTORE_PASSWORD` / `LM_KEY_ALIAS` / `LM_KEY_PASSWORD` を
設定して `./gradlew :app:assembleRelease`。鍵はリポジトリに入れない（失うと、同じアプリとして更新を配れなくなる）。
埋め込みモデル（e5）を含むので約 700MB になる（Google Play の上限は超える。直接配布向け）。

## テスト

- 端末での自己テスト（起動時に実行。デバッグビルドで logcat に結果）。
- JVM の単体テスト: `./gradlew :app:testDebugUnitTest`（モデル取得の接続先・サイズ・再開の判定と、モデルの記録）。
  JUnit は、依存検証（`gradle/verification-metadata.xml`）に、ハッシュを追加して使っている。

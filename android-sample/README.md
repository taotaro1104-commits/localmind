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
このアプリがネットワークを使うのは、ここだけ（`INTERNET` 権限はこのため）。オフラインで使うなら、`adb push` のあと
`adb shell run-as com.localmind.sample cp /data/local/tmp/<ファイル> files/llm/` で置いてもよい
（標準の 1.5B のファイル名なら、そのまま使える）。
`scripts/build-android.sh` は `liblm_ffi.so` と `liblm_llm_ffi.so` の両方を作る。Kotlin のバインディングは両方とも
同じビルド（`lm-ffi` は `--features "onnx whisper"`）から生成し直すこと。

## 配布用の APK

署名つきの release は、環境変数 `LM_KEYSTORE` / `LM_KEYSTORE_PASSWORD` / `LM_KEY_ALIAS` / `LM_KEY_PASSWORD` を
設定して `./gradlew :app:assembleRelease`。鍵はリポジトリに入れない（失うと、同じアプリとして更新を配れなくなる）。
埋め込みモデル（e5）を含むので約 700MB になる（Google Play の上限は超える。直接配布向け）。

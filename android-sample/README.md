# LocalMind Android サンプル

`liblm_ffi.so`（Rust コア）を Kotlin から呼ぶ最小のアプリ。起動すると自己テストを 2 段階で実行し（基本 9 項目 + 同梱の実モデル e5 の 7 項目）、
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
adb logcat -d -s LMSample:I          # "基本: ALL PASS (9)" と "実モデル(e5): ALL PASS (7)" が出れば成功（デバッグビルドのみ出力）
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

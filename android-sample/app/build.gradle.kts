plugins {
    id("com.android.application")
}

android {
    namespace = "com.localmind.sample"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.localmind.sample"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.1"
        // サイズ削減のため 64bit のみ（実機 arm64 とエミュレータ x86_64）。armeabi-v7a が要るなら追加する。
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
    }

    // ONNX Runtime の libonnxruntime.so は Rust 側（ort の load-dynamic）がパスで読むため、
    // APK 内から直接読み込まず nativeLibraryDir へ展開する。
    packaging { jniLibs { useLegacyPackaging = true } }
    // モデル（約 470MB）と設定ファイルは圧縮しない（長さの確認に openFd を使うため。起動時に filesDir へ展開する）。
    androidResources { noCompress += listOf("onnx", "json", "bin") }

    // Rust 側の成果物（dist/android）を直接参照する。コピーしない。
    sourceSets["main"].jniLibs.directories.add("../../dist/android/jniLibs")
    sourceSets["main"].kotlin.directories.add("../../dist/android/kotlin")
    // 同梱モデル: dist/android/assets/e5-small/（取得方法は ../README.md）
    sourceSets["main"].assets.directories.add("../../dist/android/assets")

    // 配布用の署名。鍵とパスワードはリポジトリに入れず、環境変数で渡す（LM_KEYSTORE / LM_KEYSTORE_PASSWORD /
    // LM_KEY_ALIAS / LM_KEY_PASSWORD）。未設定なら、署名なしの release になる（そのままでは端末に入らない）。
    val keystorePath = System.getenv("LM_KEYSTORE")
    signingConfigs {
        if (keystorePath != null) {
            create("release") {
                storeFile = file(keystorePath)
                storePassword = System.getenv("LM_KEYSTORE_PASSWORD")
                keyAlias = System.getenv("LM_KEY_ALIAS")
                keyPassword = System.getenv("LM_KEY_PASSWORD")
            }
        }
    }
    buildTypes {
        release {
            // JNA と UniFFI はリフレクションで名前を引くので、縮小（R8）はしない。
            isMinifyEnabled = false
            signingConfigs.findByName("release")?.let { signingConfig = it }
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    // UniFFI の Kotlin バインディングは JNA を使う（Android は @aar 形式が必要）。
    implementation("net.java.dev.jna:jna:5.19.1@aar")
    // ONNX Runtime の本体（libonnxruntime.so）だけを使う。Java API は呼ばない（推論は Rust 側）。
    // ort クレートが要求する API バージョン（24）以上であること。
    implementation("com.microsoft.onnxruntime:onnxruntime-android:1.31.1")
}

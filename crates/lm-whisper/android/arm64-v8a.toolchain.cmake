# whisper.cpp を Android 向けにビルドするための、NDK のツールチェインのラッパー。
# - whisper-rs-sys は CMAKE_ で始まる環境変数しか cmake に渡さないので、ABI をここで決める。
# - whisper-rs-sys は、ビルドする PC が Windows のとき、Android 向けにも MSVC 用の /utf-8 を付けてしまう
#   （ターゲットではなくホストで判定している）。clang は /utf-8 をファイル名と取り違えるので、取り除く。
# 使い方: scripts/build-android.sh（環境変数 ANDROID_NDK_HOME が要る）
set(ANDROID_ABI arm64-v8a)
set(ANDROID_PLATFORM android-26)
set(ANDROID_STL c++_shared)
include("$ENV{ANDROID_NDK_HOME}/build/cmake/android.toolchain.cmake")
string(REPLACE "/utf-8" "" CMAKE_CXX_FLAGS "${CMAKE_CXX_FLAGS}")

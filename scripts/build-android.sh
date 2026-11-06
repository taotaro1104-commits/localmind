#!/usr/bin/env bash
# Android 向けの liblm_ffi.so（onnx + whisper つき）を ABI ごとに作り、dist/android/jniLibs へ置く。
#
# 必要なもの: cargo-ndk、Android NDK（環境変数 ANDROID_NDK_HOME）、cmake と ninja。
# whisper.cpp のビルドに使う設定は ABI ごとのラッパー（crates/lm-whisper/android/*.toolchain.cmake）にある。
# Windows の Git Bash でも動く。使い方: scripts/build-android.sh [features]（既定は "onnx whisper"）
set -euo pipefail

: "${ANDROID_NDK_HOME:?環境変数 ANDROID_NDK_HOME に NDK のパスを設定してください}"
root="$(cd "$(dirname "$0")/.." && pwd)"
features="${1:-onnx whisper}"
# ホスト（MSVC）向けに設定した最適化フラグは、Android の clang には通らないので、外しておく。
unset CMAKE_C_FLAGS_RELEASE CMAKE_CXX_FLAGS_RELEASE
# cmake に渡すパスは、Windows でも / 区切りの形にする。
to_cmake_path() { if command -v cygpath >/dev/null 2>&1; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

for tool in cargo-ndk cmake ninja; do
  command -v "$tool" >/dev/null 2>&1 || { echo "$tool が見つかりません" >&2; exit 1; }
done

# whisper-rs-sys は、ビルドする PC が Windows だと、Android 向けのリンクにも Windows 用の advapi32 を付けてしまう
# （ターゲットではなくホストで判定している）。Windows のときだけ、空のスタブを置いてリンクを通す。
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*)
    stubs="$root/target/android-linkstubs"
    mkdir -p "$stubs"
    ar=""
    for candidate in "$ANDROID_NDK_HOME"/toolchains/llvm/prebuilt/*/bin/llvm-ar*; do
      [ -e "$candidate" ] && { ar="$candidate"; break; }
    done
    [ -n "$ar" ] || { echo "NDK の llvm-ar が見つかりません" >&2; exit 1; }
    rm -f "$stubs/libadvapi32.a"
    "$ar" rc "$(to_cmake_path "$stubs")/libadvapi32.a"
    export RUSTFLAGS="${RUSTFLAGS:-} -L $(to_cmake_path "$stubs")"
    ;;
esac

for abi in arm64-v8a x86_64; do
  echo "== $abi"
  # whisper.cpp（lm-ffi）と llama.cpp（lm-llm-ffi）は、どちらも ggml を同梱していて 1 つのライブラリには
  # 同居できないので、別々の .so にする。
  for pkg in lm-ffi lm-llm-ffi; do
    if [ "$pkg" = lm-ffi ]; then feature_args=(--features "$features"); else feature_args=(); fi
    # llama.cpp は posix_madvise（Android では API 31 から）を使う。minSdk 26 のまま動かすため、同じ働きの
    # madvise に置き換える（定数も同じ値）。llama-cpp-sys-2 は NDK のツールチェインを自分で選ぶので、
    # 環境変数の CXXFLAGS で渡す。
    CXXFLAGS="-Dposix_madvise=madvise -DPOSIX_MADV_WILLNEED=MADV_WILLNEED -DPOSIX_MADV_RANDOM=MADV_RANDOM" \
    CMAKE_TOOLCHAIN_FILE="$(to_cmake_path "$root/crates/lm-whisper/android/$abi.toolchain.cmake")" \
    CMAKE_GENERATOR=Ninja \
    CMAKE_GENERATOR_INSTANCE= \
    WHISPER_DONT_GENERATE_BINDINGS=1 \
    cargo ndk -t "$abi" -o "$(to_cmake_path "$root/dist/android/jniLibs")" --link-libcxx-shared \
    build --release -p "$pkg" ${feature_args[@]+"${feature_args[@]}"}
  done
done
ls -l "$root"/dist/android/jniLibs/*/

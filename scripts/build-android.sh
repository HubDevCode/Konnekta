#!/usr/bin/env bash
# Build the Android app: the Rust core with cargo-ndk, then the APK with Gradle.
#   scripts/build-android.sh [debug|release]   (default: debug)
# Needs: rustup with aarch64/x86_64-linux-android, cargo-ndk, ANDROID_HOME
# with the NDK, Java 17, Gradle (on PATH or $GRADLE). Override any of them
# with the environment.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
MODE=${1:-debug}
: "${ANDROID_HOME:?set ANDROID_HOME to the Android SDK}"
NDK=${ANDROID_NDK_HOME:-$(ls -d "$ANDROID_HOME"/ndk/* | sort -V | tail -1)}
export ANDROID_NDK_HOME=$NDK
JNI=$ROOT/apps/android/app/src/main/jniLibs
cargo ndk -t arm64-v8a -t x86_64 -P 28 -o "$JNI" build -p konnekta-ffi --release
cd "$ROOT/apps/android"
"${GRADLE:-gradle}" --no-daemon "assemble${MODE^}" "test${MODE^}UnitTest"
ls -la app/build/outputs/apk/"$MODE"/*.apk

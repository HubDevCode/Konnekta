#!/usr/bin/env bash
# Build the Rust core for the mobile app.
#   ./scripts/build_rust.sh android   -> android/app/src/main/jniLibs/<abi>/libkonnekta_ffi.so
#   ./scripts/build_rust.sh ios       -> ios/rust/libkonnekta_ffi.a (device, arm64)
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$(cd ../.. && pwd)
case "${1:-}" in
  android)
    command -v cargo-ndk >/dev/null || cargo install cargo-ndk --locked
    cargo ndk --manifest-path "$ROOT/Cargo.toml" \
      -t arm64-v8a -t armeabi-v7a -t x86_64 \
      -o android/app/src/main/jniLibs \
      build --release -p konnekta-ffi
    ;;
  ios)
    cargo build --manifest-path "$ROOT/Cargo.toml" --release -p konnekta-ffi --target aarch64-apple-ios
    mkdir -p ios/rust
    cp "$ROOT/target/aarch64-apple-ios/release/libkonnekta_ffi.a" ios/rust/
    for cfg in Debug Release; do
      f="ios/Flutter/$cfg.xcconfig"
      grep -q konnekta_ffi "$f" 2>/dev/null || \
        echo 'OTHER_LDFLAGS=$(inherited) -force_load $(PROJECT_DIR)/rust/libkonnekta_ffi.a -lc++' >> "$f"
    done
    ;;
  *) echo "usage: $0 android|ios" >&2; exit 1 ;;
esac

#!/usr/bin/env bash
# Builds libandotp_android.so for the given ABIs into android/app/src/main/jniLibs.
# Requires the Android NDK (ANDROID_NDK_HOME) and cargo-ndk (`cargo install cargo-ndk`).
set -euo pipefail
cd "$(dirname "$0")/../.."
abis=("${@:-arm64-v8a armeabi-v7a x86_64}")
args=()
for abi in ${abis[@]}; do args+=(-t "$abi"); done
cargo ndk "${args[@]}" --platform 24 -o android/app/src/main/jniLibs build --release -p andotp-android

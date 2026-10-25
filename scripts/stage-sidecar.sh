#!/usr/bin/env bash
# stage-sidecar.sh — build the namehold-syncd daemon and stage it where Tauri's
# externalBin expects it: src-tauri/binaries/namehold-syncd-<host triple>[.exe].
#
# Used by release.yml (Linux and Windows) and appimage.yml, so the AppImage the
# check builds carries the daemon the way a release does. The macOS universal
# build stages its two architectures itself (release.yml).
set -euo pipefail
cd "$(dirname "$0")/../src-tauri"
mkdir -p binaries
TARGET_TRIPLE=$(rustc --print host-tuple)
cargo build --release --bin namehold-syncd
EXT=""
if [[ "${RUNNER_OS:-}" == "Windows" ]]; then EXT=".exe"; fi
cp "target/release/namehold-syncd${EXT}" "binaries/namehold-syncd-${TARGET_TRIPLE}${EXT}"
# `cargo build` above runs build.rs, which creates an empty placeholder at this
# path with mode 0644, and cp onto an existing file keeps that mode. The
# AppImage copies the file as it is, so the daemon shipped in 0.6.0's AppImage
# could not be executed. Set the mode explicitly.
chmod 0755 "binaries/namehold-syncd-${TARGET_TRIPLE}${EXT}"

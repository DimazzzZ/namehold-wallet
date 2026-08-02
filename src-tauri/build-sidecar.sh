#!/bin/bash
# Build the namehold-syncd daemon binary and stage it with the target-triple
# suffix that Tauri's externalBin bundling expects.
#
# Run this BEFORE `tauri build` (or add it to the release workflow).
# In dev mode (`tauri dev`), the daemon is found via target/debug/ directly.
set -euo pipefail

TARGET_TRIPLE=$(rustc --print host-tuple)
MODE="${1:-release}"

# Note: build.rs auto-creates an empty placeholder for the host triple if one
# doesn't exist, so `cargo build` of the daemon below succeeds even on a fresh
# checkout. This script then overwrites that placeholder with the real binary.
mkdir -p binaries

echo "Building namehold-syncd (mode=$MODE, target=$TARGET_TRIPLE)"
if [ "$MODE" = "debug" ]; then
  cargo build --bin namehold-syncd
  SRC="target/debug/namehold-syncd"
else
  cargo build --release --bin namehold-syncd
  SRC="target/release/namehold-syncd"
fi

# Stage with target-triple suffix for Tauri's externalBin.
DEST="binaries/namehold-syncd-${TARGET_TRIPLE}"
cp "$SRC" "$DEST"
echo "Staged: $DEST ($(du -h "$DEST" | cut -f1))"

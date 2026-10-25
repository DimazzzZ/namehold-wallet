#!/usr/bin/env bash
# preseed-apprun.sh — put the AppImage launcher (AppRun) into the Tauri
# bundler's cache with mode 0755 before an AppImage build.
#
# The bundler downloads AppRun with mode 0770 and copies it into the image as
# AppRun.wrapped, so the image does not start for a user other than the one who
# mounted it (firejail, the AppImage catalog's test: "AppRun.wrapped:
# Permission denied"; tauri-apps/tauri#16155, still 0770 in the 2.12 bundler).
# It downloads the file only when it is missing, so the same file, pinned by
# its SHA-256, is put there first. Used by release.yml and appimage.yml.
set -euo pipefail
cache="${XDG_CACHE_HOME:-$HOME/.cache}/tauri"
mkdir -p "$cache"
curl -fsSL --retry 3 -o "$cache/AppRun-x86_64" \
  https://github.com/tauri-apps/binary-releases/releases/download/apprun-old/AppRun-x86_64
echo "f30140a43a0a59e46db21bdefdf749b9e9f2c6946e92afabbacf98b8ae73fb4f  $cache/AppRun-x86_64" | sha256sum -c -
chmod 0755 "$cache/AppRun-x86_64"

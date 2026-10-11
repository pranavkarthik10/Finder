#!/bin/sh
# Rebuild the app, relaunch it, screenshot its window to target/shot.png.
cd "$(dirname "$0")/.."
taskkill //F //IM finder.exe >/dev/null 2>&1
CARGO_TARGET_DIR=target/dev cargo build -p finder 2>&1 | grep -E "^error" -A10 | head -60
(./target/dev/debug/finder.exe "$@" > target/finder.log 2>&1 &)
sleep 3
powershell -ExecutionPolicy Bypass -File scripts/shot.ps1 >/dev/null

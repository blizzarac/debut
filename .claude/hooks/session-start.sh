#!/bin/bash
# Installs the system libraries the Rust workspace needs to build and test in a
# Claude Code cloud session: a software Vulkan driver for the wgpu backend tests,
# FFmpeg and ALSA headers for the native platform crate, clang for bindgen.
set -euo pipefail

if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

PKGS=(
  mesa-vulkan-drivers
  libavcodec-dev libavformat-dev libavutil-dev libswscale-dev libswresample-dev libavfilter-dev libavdevice-dev
  libasound2-dev
  libclang-dev clang pkg-config
  ffmpeg
)

missing=()
for p in "${PKGS[@]}"; do
  dpkg -s "$p" >/dev/null 2>&1 || missing+=("$p")
done

if [ "${#missing[@]}" -gt 0 ]; then
  export DEBIAN_FRONTEND=noninteractive
  sudo -n apt-get update -q
  sudo -n apt-get install -y -q --no-install-recommends "${missing[@]}"
fi

rustup component add rustfmt clippy >/dev/null 2>&1 || true
rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true

cd "$CLAUDE_PROJECT_DIR"
cargo fetch

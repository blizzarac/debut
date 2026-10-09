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
  libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev
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

# Shared TypeScript UI; the Tauri shell embeds its dist/ at compile time.
if command -v pnpm >/dev/null 2>&1; then
  pnpm --dir apps/ui install --frozen-lockfile --silent
  pnpm --dir apps/ui build >/dev/null
fi

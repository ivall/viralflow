#!/usr/bin/env bash
# Downloads the newest FFmpeg 8.x "shared" build from BtbN/FFmpeg-Builds for
# this machine's architecture into ./ffmpeg-download and, on GitHub Actions,
# exports the variables the build needs. FFmpeg 8 is the version the app is
# tested with (distributions ship older ones).
set -euo pipefail

case "$(uname -m)" in
  x86_64) arch=linux64 ;;
  aarch64) arch=linuxarm64 ;;
  *) echo "unsupported architecture $(uname -m)" >&2; exit 1 ;;
esac
dir="$PWD/ffmpeg-download"
if [ ! -f "$dir/lib/libavcodec.so" ]; then
  # The release names carry the minor version (n8.1, n8.2…): take the newest 8.x.
  curl -fsSL -H "Authorization: Bearer ${GITHUB_TOKEN:-}" https://api.github.com/repos/BtbN/FFmpeg-Builds/releases/latest -o /tmp/ffmpeg-release.json \
    || curl -fsSL https://api.github.com/repos/BtbN/FFmpeg-Builds/releases/latest -o /tmp/ffmpeg-release.json
  url="$(grep -oE "https://[^\"]*ffmpeg-n8\.[0-9]+-latest-$arch-gpl-shared-8\.[0-9]+\.tar\.xz" /tmp/ffmpeg-release.json | sort -V | tail -n 1)"
  if [ -z "$url" ]; then
    echo "no FFmpeg 8 shared build for $arch found" >&2
    exit 1
  fi
  echo "FFmpeg: $url"
  curl -fsSL -o /tmp/ffmpeg.tar.xz "$url"
  rm -rf "$dir" && mkdir -p "$dir"
  tar -xf /tmp/ffmpeg.tar.xz -C "$dir" --strip-components=1
  rm /tmp/ffmpeg.tar.xz
fi
"$dir/bin/ffmpeg" -hide_banner -version | sed -n 1p || true

if [ -n "${GITHUB_ENV:-}" ]; then
  {
    # .cargo/config.toml points LIBCLANG_PATH at the Windows LLVM folder.
    echo "LIBCLANG_PATH=$(bash "$(dirname "$0")/../libclang-path.sh")"
    echo "FFMPEG_DIR=$dir"
    echo "PKG_CONFIG_PATH=$dir/lib/pkgconfig"
    echo "LD_LIBRARY_PATH=$dir/lib"
  } >> "$GITHUB_ENV"
  echo "$dir/bin" >> "$GITHUB_PATH"
fi

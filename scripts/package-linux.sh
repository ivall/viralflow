#!/usr/bin/env bash
# Builds the Linux release: an AppImage and a portable .tar.gz, both carrying
# the FFmpeg libraries next to the executable.
#
#   FFMPEG_DIR=/path/to/ffmpeg-8-shared scripts/package-linux.sh
#
# FFMPEG_DIR is an FFmpeg 8 "shared" build (include/ and lib/, e.g. from
# BtbN/FFmpeg-Builds). appimagetool is downloaded when it is not on PATH.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
: "${FFMPEG_DIR:?set FFMPEG_DIR to an FFmpeg 8 shared build}"
version="$(grep -m1 '^version = ' Cargo.toml | sed -E 's/version = "(.*)"/\1/')"
arch="$(uname -m)"
name="ViralFlow-$version-linux-$arch"
dist="$root/dist"
target="${CARGO_TARGET_DIR:-$root/target}"

export PKG_CONFIG_PATH="$FFMPEG_DIR/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
# Otherwise .cargo/config.toml's Windows default applies.
export LIBCLANG_PATH="${LIBCLANG_PATH:-$(bash scripts/libclang-path.sh)}"
# The executable finds the bundled libraries in ../lib (AppImage, tarball).
export RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-Wl,-rpath,\$ORIGIN/../lib"
cargo build --release -p viralflow

app="$dist/AppDir"
rm -rf "$app" "$dist/$name" "$dist/$name.tar.gz" "$dist/$name.AppImage"
mkdir -p "$app/usr/bin" "$app/usr/lib" "$app/usr/share/applications" "$app/usr/share/icons/hicolor/256x256/apps" "$app/usr/share/metainfo"
install -m755 "$target/release/viralflow" "$app/usr/bin/viralflow"

# FFmpeg's libraries (and the libraries they need that are not part of every
# system), keeping their version links.
cp -a "$FFMPEG_DIR"/lib/lib*.so* "$app/usr/lib/"
rm -f "$app"/usr/lib/*.a
missing="$(LD_LIBRARY_PATH="$app/usr/lib" ldd "$app/usr/bin/viralflow" | grep 'not found' || true)"
if [ -n "$missing" ]; then
  echo "unresolved libraries:" >&2
  echo "$missing" >&2
  exit 1
fi

install -m644 packaging/linux/viralflow.desktop "$app/usr/share/applications/viralflow.desktop"
install -m644 packaging/linux/io.github.ivall.viralflow.metainfo.xml "$app/usr/share/metainfo/"
install -m644 assets/icons/128x128@2x.png "$app/usr/share/icons/hicolor/256x256/apps/viralflow.png"
cp packaging/linux/viralflow.desktop "$app/viralflow.desktop"
cp assets/icons/128x128@2x.png "$app/viralflow.png"
cp assets/icons/128x128@2x.png "$app/.DirIcon"
install -m755 packaging/linux/AppRun "$app/AppRun"

# Portable folder: bin/, lib/, share/ like an installed app.
mkdir -p "$dist/$name"
cp -a "$app/usr/." "$dist/$name/"
cp LICENSE README.md "$dist/$name/" 2>/dev/null || true
tar -C "$dist" -czf "$dist/$name.tar.gz" "$name"

if [ "${SKIP_APPIMAGE:-}" = "1" ]; then
  rm -rf "$app" "$dist/$name"
  echo "Portable: $dist/$name.tar.gz (AppImage skipped)"
  exit 0
fi
tool="$(command -v appimagetool || true)"
if [ -z "$tool" ]; then
  tool="$dist/appimagetool"
  curl -sSL -o "$tool" "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$arch.AppImage"
  chmod +x "$tool"
fi
# CI machines have no FUSE: the tool runs from its extracted files.
APPIMAGE_EXTRACT_AND_RUN=1 ARCH="$arch" "$tool" --no-appstream "$app" "$dist/$name.AppImage"
rm -rf "$app" "$dist/$name" "$dist/appimagetool"
echo "AppImage: $dist/$name.AppImage"
echo "Portable: $dist/$name.tar.gz"

#!/usr/bin/env bash
# Builds the macOS release: ViralFlow.app with FFmpeg's libraries inside,
# packed into a .dmg. Needs Homebrew's ffmpeg@8 and dylibbundler:
#
#   brew install ffmpeg@8 dylibbundler
#   scripts/package-macos.sh
#
# The app is signed ad hoc (it runs after "Open" in the Finder's context menu
# the first time). Set MACOS_SIGN_IDENTITY to sign it with a Developer ID.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
export FFMPEG_DIR="${FFMPEG_DIR:-$(brew --prefix ffmpeg@8)}"
export PKG_CONFIG_PATH="$FFMPEG_DIR/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
version="$(grep -m1 '^version = ' Cargo.toml | sed -E 's/version = "(.*)"/\1/')"
arch="$(uname -m)"
[ "$arch" = "x86_64" ] || arch="arm64"
dist="$root/dist"
target="${CARGO_TARGET_DIR:-$root/target}"
identity="${MACOS_SIGN_IDENTITY:--}"

# Homebrew's libraries are built for the macOS they were installed on, so
# that is the oldest macOS the app runs on.
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-$(sw_vers -productVersion | cut -d. -f1).0}"
cargo build --release -p viralflow

app="$dist/ViralFlow.app"
rm -rf "$app" "$dist/dmg"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$app/Contents/Frameworks"
install -m755 "$target/release/viralflow" "$app/Contents/MacOS/viralflow"
cp assets/icons/icon.icns "$app/Contents/Resources/icon.icns"
sed -e "s/@VERSION@/$version/g" -e "s/@MINIMUM@/$MACOSX_DEPLOYMENT_TARGET/g" packaging/macos/Info.plist > "$app/Contents/Info.plist"

# Copies every non-system library (FFmpeg and what it links) into Frameworks
# and points the executable and the libraries at their new place.
dylibbundler --overwrite-dir --bundle-deps --create-dir \
  --fix-file "$app/Contents/MacOS/viralflow" \
  --dest-dir "$app/Contents/Frameworks" \
  --install-path @executable_path/../Frameworks/ >/dev/null

# Rewritten libraries lost their signatures; Apple silicon refuses unsigned code.
for library in "$app"/Contents/Frameworks/*.dylib; do
  codesign --force --sign "$identity" --timestamp=none "$library"
done
codesign --force --sign "$identity" --timestamp=none --options runtime --entitlements packaging/macos/entitlements.plist "$app"
codesign --verify --deep --strict "$app"

# Nothing may still point at Homebrew.
if otool -L "$app/Contents/MacOS/viralflow" "$app"/Contents/Frameworks/*.dylib | grep -E '/(opt/homebrew|usr/local)/'; then
  echo "libraries still refer to Homebrew" >&2
  exit 1
fi

mkdir -p "$dist/dmg"
cp -R "$app" "$dist/dmg/"
ln -s /Applications "$dist/dmg/Applications"
dmg="$dist/ViralFlow-$version-macos-$arch.dmg"
rm -f "$dmg"
hdiutil create -volname "ViralFlow" -srcfolder "$dist/dmg" -ov -format UDZO "$dmg" >/dev/null
rm -rf "$dist/dmg"
(cd "$dist" && ditto -c -k --keepParent ViralFlow.app "ViralFlow-$version-macos-$arch.zip")
rm -rf "$app"
echo "Disk image: $dmg"

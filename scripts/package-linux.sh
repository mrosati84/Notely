#!/usr/bin/env bash
# Builds a Linux release AppImage for Notely.
set -euo pipefail
cd "$(dirname "$0")/.."

if [ "$(uname -s)" != "Linux" ]; then
  echo "Error: package-linux.sh must be run on Linux." >&2
  exit 1
fi

if ! command -v linuxdeploy >/dev/null 2>&1; then
  echo "Error: linuxdeploy is required to build the AppImage." >&2
  echo "Install it from https://github.com/linuxdeploy/linuxdeploy/releases" >&2
  exit 1
fi

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"
ARCH="$(uname -m)"

cargo build --release

OUT_DIR="target/release/bundle/appimage"
APPDIR="$OUT_DIR/AppDir"
APPIMAGE="$OUT_DIR/Notely-${VERSION}-${ARCH}.AppImage"

rm -rf "$APPDIR" "$APPIMAGE"
mkdir -p "$OUT_DIR"

# linuxdeploy expects standard hicolor icon sizes (up to 512x512); icons/icon.png is 1024x1024.
ICON_SRC="icons/icon.png"
ICON_STAGED="$OUT_DIR/notely.png"
if command -v magick >/dev/null 2>&1; then
  magick "$ICON_SRC" -resize 512x512 "$ICON_STAGED"
elif command -v convert >/dev/null 2>&1; then
  convert "$ICON_SRC" -resize 512x512 "$ICON_STAGED"
else
  cp "$ICON_SRC" "$ICON_STAGED"
fi

# Extract-and-run avoids FUSE issues in containers, CI, and minimal setups.
APPIMAGE_EXTRACT_AND_RUN="${APPIMAGE_EXTRACT_AND_RUN:-1}" \
LINUXDEPLOY_OUTPUT_VERSION="$VERSION" \
ARCH="$ARCH" \
LDAI_OUTPUT="$APPIMAGE" \
linuxdeploy \
  --appdir "$APPDIR" \
  --executable target/release/notely \
  --desktop-file linux/notely.desktop \
  --icon-file "$ICON_STAGED" \
  --icon-filename notely \
  --output appimage

rm -rf "$APPDIR" "$ICON_STAGED"

echo
if [ -f "$APPIMAGE" ]; then
  echo "AppImage: $APPIMAGE"
  echo "Run with: ./$APPIMAGE"
fi

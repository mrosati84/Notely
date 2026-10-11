#!/usr/bin/env bash
# Builds Notely.app and Notely.dmg.
#
# The app is signed *before* the DMG is created, so the disk image contains the
# signed bundle. (`cargo bundle` only produces an .app unless a DMG format is
# requested explicitly, hence the manual hdiutil step.)
set -euo pipefail
cd "$(dirname "$0")/.."

cargo bundle --release -f osx

APP="target/release/bundle/osx/Notely.app"
DMG_DIR="target/release/bundle/dmg"
DMG="$DMG_DIR/Notely.dmg"

# Ad-hoc signature so Gatekeeper is happy on this Mac. No --deep: sign the
# bundle itself, not nested binaries Apple recommends against --deep.
codesign --force --sign - "$APP"

# Build the DMG from the already-signed app. Stage the bundle in a folder (with
# an /Applications shortcut) so the image root holds Notely.app for drag-install.
mkdir -p "$DMG_DIR"
STAGE="$DMG_DIR/stage"
rm -rf "$STAGE"
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/Notely.app"
ln -s /Applications "$STAGE/Applications"

rm -f "$DMG"
hdiutil create -volname "Notely" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null
rm -rf "$STAGE"

echo
if [ -d "$APP" ]; then
  echo "App: $APP"
fi
if [ -f "$DMG" ]; then
  echo "Dmg: $DMG"
fi
echo "Open with: open \"$APP\""

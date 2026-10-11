#!/usr/bin/env bash
# Builds Notely.app and Notely.dmg and ad-hoc signs the app bundle.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo bundle --release

APP="target/release/bundle/osx/Notely.app"
# Ad-hoc signature so Gatekeeper is happy on this Mac.
codesign --force --deep --sign - "$APP"

echo
echo "App: $APP"
echo "Dmg: target/release/bundle/dmg/Notely.dmg"
echo "Open with: open \"$APP\""

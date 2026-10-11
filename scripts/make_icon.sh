#!/usr/bin/env bash
# Regenerates icons/icon.png (via the make_icon example) and icons/icon.icns.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo run --release --example make_icon

rm -rf icons/icon.iconset
mkdir -p icons/icon.iconset
for s in 16 32 128 256 512; do
  sips -z "$s" "$s" icons/icon.png --out "icons/icon.iconset/icon_${s}x${s}.png" >/dev/null
  d=$((s * 2))
  sips -z "$d" "$d" icons/icon.png --out "icons/icon.iconset/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns icons/icon.iconset -o icons/icon.icns
rm -rf icons/icon.iconset

echo "wrote icons/icon.icns"

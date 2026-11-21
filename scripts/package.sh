#!/usr/bin/env bash
# Dispatches to the platform-specific packaging script.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

case "$(uname -s)" in
  Darwin)
    exec "$SCRIPT_DIR/package-macos.sh" "$@"
    ;;
  Linux)
    exec "$SCRIPT_DIR/package-linux.sh" "$@"
    ;;
  *)
    echo "Error: unsupported platform '$(uname -s)'." >&2
    echo "Available packaging scripts: package-linux.sh, package-macos.sh" >&2
    exit 1
    ;;
esac

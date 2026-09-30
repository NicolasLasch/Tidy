#!/usr/bin/env bash
# Builds a drag-to-Applications disk image: scripts/make-dmg.sh Tidy.app out.dmg
set -euo pipefail
APP="${1:?Tidy.app}"; OUT="${2:?output.dmg}"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
hdiutil create -volname "Tidy" -srcfolder "$STAGE" -ov -format UDZO "$OUT" >/dev/null
echo "Wrote $OUT"

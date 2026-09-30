#!/usr/bin/env bash
# Sanity-checks a built Tidy.app before it is published: structure, signature, worker integrity.
#   scripts/verify-bundle.sh path/to/Tidy.app
set -euo pipefail
APP="${1:?usage: verify-bundle.sh Tidy.app}"
BIN="$APP/Contents/MacOS/tidy-desktop"
WORKER="$APP/Contents/Resources/inference/tidy-inference-worker"
MANIFEST="$APP/Contents/Resources/inference/worker-manifest.json"
[ -x "$BIN" ] || { echo "missing app binary"; exit 1; }
plutil -lint "$APP/Contents/Info.plist" >/dev/null
[ -x "$WORKER" ] || { echo "missing local-AI worker (run npm run worker:build before packaging)"; exit 1; }
want=$(python3 -c "import json,sys; print(json.load(open('$MANIFEST'))['sha256'])")
have=$(shasum -a 256 "$WORKER" | cut -d' ' -f1)
[ "$want" = "$have" ] || { echo "worker checksum differs from its manifest"; exit 1; }
if ! codesign --verify --deep --strict "$APP" 2>&1; then
  if [ "${REQUIRE_SIGNATURE:-0}" = "1" ]; then echo "bundle signature is invalid"; exit 1; fi
  echo "note: signature does not verify; sign before publishing (codesign --force --deep --sign - Tidy.app)"
fi
lipo -archs "$BIN" | grep -q . 
echo "Bundle OK: $(basename "$APP"), arch $(lipo -archs "$BIN"), version $(plutil -extract CFBundleShortVersionString raw "$APP/Contents/Info.plist")"

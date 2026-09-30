#!/usr/bin/env bash
# After creating the GitHub repository, point badges and links at it:  scripts/set-repo.sh yourname/tidy
set -euo pipefail
SLUG="${1:?usage: set-repo.sh owner/repo}"
grep -rl "OWNER/REPO" --include=*.md --include=*.yml --include=*.json . 2>/dev/null | grep -v "node_modules\|/target/" | while read -r f; do
  sed -i.bak "s#OWNER/REPO#$SLUG#g" "$f" && rm -f "$f.bak"; echo "updated $f"
done

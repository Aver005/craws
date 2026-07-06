#!/usr/bin/env bash
# Point git at the repo's versioned hooks (.githooks/). Run once per clone.
#   bash scripts/install-hooks.sh
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
git config core.hooksPath .githooks
chmod +x .githooks/* 2>/dev/null || true
echo "✔ core.hooksPath → .githooks (pre-push gate active)"
echo "  bypass a push with: git push --no-verify"

#!/usr/bin/env bash
# Final pipeline: apply all domestic patches -> cross-verify -> full validate ->
# coverage report. Does NOT push (push_catalog.sh does that separately).
set -euo pipefail
D="/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cd "$D"

echo "=== 1. apply all patches (round-1 + domestic) ==="
python3 apply_patches.py

echo "=== 2. cross-verify domestic patches vs models.dev ==="
python3 verify_domestic.py || true

echo "=== 3. full 1886-model validation ==="
python3 validate_full.py

echo "=== 4. coverage report ==="
python3 final_report.py

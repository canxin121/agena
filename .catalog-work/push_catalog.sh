#!/usr/bin/env bash
# Push the merged catalog to the canxin121/agena-model-catalog repo.
# Usage: bash push_catalog.sh "<commit-message>"
set -euo pipefail
D="/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
REPO="canxin121/agena-model-catalog"
BRANCH="main"
MSG="${1:-update models.json}"

# 1. validate final merged file
python3 "$D/validate_full.py"

# 2. read current file sha from github
CUR_SHA=$(gh api "repos/$REPO/contents/models.json" --jq '.sha')
echo "current sha: $CUR_SHA"

# 3. base64 encode merged file and push via api.
# Build the JSON body with python (reads files directly): the base64 of a
# ~1.9MB file exceeds macOS ARG_MAX when passed as a CLI argument.
python3 - "$MSG" "$CUR_SHA" <<'PY' > /tmp/catalog-body.json
import base64, json, sys
msg, sha = sys.argv[1], sys.argv[2]
content = base64.b64encode(open("models.merged.json", "rb").read()).decode()
print(json.dumps({"message": msg, "content": content, "sha": sha, "branch": "main"}))
PY
gh api -X PUT "repos/$REPO/contents/models.json" \
  --input /tmp/catalog-body.json \
  --jq '.commit.sha'
echo "pushed models.json"

# 4. update README
README_SHA=$(gh api "repos/$REPO/contents/README.md" --jq '.sha')
python3 - "docs: document thinking/speed mode conventions and verification rules" "$README_SHA" <<'PY' > /tmp/readme-body.json
import base64, json, sys
msg, sha = sys.argv[1], sys.argv[2]
content = base64.b64encode(open("README.draft.md", "rb").read()).decode()
json.dump({"message": msg, "content": content, "sha": sha, "branch": "main"}, open("/tmp/readme-body.json", "w"))
PY
gh api -X PUT "repos/$REPO/contents/README.md" \
  --input /tmp/readme-body.json \
  --jq '.commit.sha'
echo "pushed README.md"
rm -f /tmp/catalog-body.json /tmp/readme-body.json

#!/usr/bin/env python3
"""Conservative backfill after applying subagent patches:
- max_input_tokens = context_window_tokens when context is known and
  max_input_tokens is None (input limit == context window for virtually all
  providers). This is the single largest gap (1271 models).
- Never touch a value that already exists or a model with no context either.
Safe: max_input_tokens is soft metadata (session usage, fallback merge)."""
import json, os
D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
F = os.path.join(D, "models.merged.json")
cat = json.load(open(F))
models = cat["models"]

filled = 0
for mid, c in models.items():
    if c.get("max_input_tokens") is None and c.get("context_window_tokens") is not None:
        c["max_input_tokens"] = c["context_window_tokens"]
        filled += 1

json.dump(cat, open(F, "w"), ensure_ascii=False, indent=2)
print(f"backfilled max_input_tokens = context for {filled} models")

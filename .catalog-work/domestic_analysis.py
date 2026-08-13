#!/usr/bin/env python3
"""Analyze domestic Chinese model families in the merged catalog:
report every field's presence/absence per model, and identify the families."""
import json, os, re
D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
merged = json.load(open(os.path.join(D, "models.merged.json")))["models"]
dev = json.load(open(os.path.join(D, "research/models.dev.json")))
devmap = {}
for prov in dev.values():
    for mid, m in (prov.get("models") or {}).items():
        real = m.get("id") or mid
        devmap.setdefault(real, m); devmap.setdefault(mid, m)

# domestic vendor families (prefix -> vendor)
FAMILIES = [
    ("deepseek-", "DeepSeek"),
    ("qwen", "Alibaba Qwen"),
    ("alibaba-", "Alibaba"),
    ("glm", "Zhipu GLM"),
    ("zhipu-", "Zhipu"),
    ("moonshot-", "Moonshot Kimi"),
    ("kimi-", "Moonshot Kimi"),
    ("minimax-", "MiniMax"),
    ("mimo", "MiniMax Mimo"),
    ("bytedance-", "ByteDance"),
    ("doubao-", "ByteDance Doubao"),
    ("seed-", "ByteDance Seed"),
    ("volcengine-", "ByteDance Volcano"),
    ("spark-", "iFlytek Spark"),
    ("iflytek-", "iFlytek"),
    ("hunyuan-", "Tencent Hunyuan"),
    ("tencent-", "Tencent"),
    ("ernie-", "Baidu Ernie"),
    ("step-", "StepFun"),
    ("yi-", "01.AI Yi"),
    ("baichuan-", "Baichuan"),
    ("internlm-", "InternLM"),
    ("openbmb-", "OpenBMB"),
]

FIELDS = ["description", "knowledge_cutoff", "context_window_tokens", "max_input_tokens",
          "max_output_tokens", "pricing", "input", "features", "thinking_modes",
          "speed_modes", "display_name", "open_weights", "output_modalities", "lifecycle"]

def family(mid):
    for p, v in FAMILIES:
        if mid.startswith(p):
            return v
    return None

# collect domestic models
dom = {}
for mid, m in merged.items():
    f = family(mid)
    if f:
        dom.setdefault(f, []).append((mid, m))

print("=== DOMESTIC FAMILIES: model count ===")
total = 0
for f in sorted(dom):
    print(f"  {f}: {len(dom[f])}")
    total += len(dom[f])
print(f"  TOTAL domestic: {total}")

# per-model gap report (for each family, list models with missing critical fields)
CRIT = ["description", "context_window_tokens", "max_input_tokens", "max_output_tokens", "pricing", "thinking_modes", "speed_modes"]
print("\n=== GAP DETAIL (models with any missing critical field) ===")
for f in sorted(dom):
    gap = []
    for mid, m in dom[f]:
        miss = [c for c in CRIT if not m.get(c)]
        if miss:
            gap.append((mid, miss))
    if gap:
        print(f"\n--- {f} ({len(gap)} models with gaps) ---")
        for mid, miss in sorted(gap):
            print(f"  {mid}: missing {', '.join(miss)}")

# models.dev coverage for domestic models
print("\n=== MODELS.DEV COVERAGE of domestic models ===")
covered, uncovered = 0, []
for f in sorted(dom):
    for mid, m in dom[f]:
        if mid in devmap:
            covered += 1
        else:
            uncovered.append(mid)
print(f"  covered: {covered}, uncovered: {len(uncovered)}")
print("  uncovered:", ", ".join(sorted(uncovered)[:80]))

#!/usr/bin/env python3
"""Exact domestic pricing-gap list (exclude grok=xAI, granite=IBM, and
image/video models priced per-image not per-token)."""
import json, os

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]

NON_DOMESTIC_PREFIX = ("grok", "granite", "kling", "seedance", "seedream", "sora")
# image/video gen priced per-image/video, not per-token -> skip for token pricing
NON_TOKEN = ("-image", "-tts", "-voicedesign", "i2v", "t2v", "-video", "-asr",
             "-audio", "-music", "-reranker")

gaps = []
for m, e in sorted(cat.items()):
    if e.get("pricing"):
        continue
    if m.lower().startswith(NON_DOMESTIC_PREFIX):
        continue
    low = m.lower()
    # is it domestic at all? quick family check
    if not any(p in low for p in ("deepseek", "qwen", "qwq", "glm", "zhipu", "mimo",
                                  "kimi", "moonshot", "doubao", "byte", "seed-", "seed1",
                                  "ernie", "wenxin", "baichuan", "yi-", "step", "hunyuan",
                                  "minimax", "spark", "internlm", "xihe", "tele", "ling",
                                  "pangu", "autoglm", "cogito", "dola", "muse", "umans",
                                  "zai", "z-ai", "coding-", "alicloud", "alibaba",
                                  "grayline", "tim-", "qwenlong", "cogvideo", "funaudio",
                                  "baichuan4", "bge", "embed-v", "text-multilingual")):
        continue
    if any(s in low for s in NON_TOKEN):
        continue
    gaps.append((m, (e.get("description") or "")[:50]))

print(f"domestic token-billed pricing gaps: {len(gaps)}")
for m, d in gaps:
    print(f"  {m:52s} {d}")

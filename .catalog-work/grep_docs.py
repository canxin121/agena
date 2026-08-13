#!/usr/bin/env python3
"""Grep saved doc HTML for model names / dates / JSON blobs."""
import os, re, json

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work/research/docs"
for fn in sorted(os.listdir(D)):
    body = open(os.path.join(D, fn)).read()
    print(f"\n===== {fn} ({len(body)} bytes) =====")
    # model-name patterns
    names = set(re.findall(r"deepseek-[a-z0-9.\-]+", body, re.I))
    if names:
        print("  deepseek ids:", sorted(names)[:20])
    names = set(re.findall(r"glm-[a-z0-9.\-]+", body, re.I))
    if names:
        print("  glm ids:", sorted(names)[:20])
    names = set(re.findall(r"kimi-[a-z0-9.\-]+", body, re.I))
    if names:
        print("  kimi ids:", sorted(names)[:20])
    # dates (any YYYY-MM)
    dates = sorted(set(re.findall(r"\b20(?:2[4-9]|3[0-9])-[0-9]{1,2}\b", body)))
    print("  dates:", dates[:20])
    # price markers
    pr = set(re.findall(r"[¥$]\s?[0-9.]+", body))
    print("  price markers:", sorted(pr)[:15])
    # JSON blobs with "knowledge"
    for m in re.finditer(r'"knowledge"\s*:\s*("[^"]*"|null)', body):
        print("  knowledge:", m.group(0)[:80])

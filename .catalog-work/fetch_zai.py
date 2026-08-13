#!/usr/bin/env python3
"""Fetch Z.ai pricing + GLM-4.7-n model doc via curl; dump readable text."""
import subprocess, os, re, html, json

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work/research/docs"

def curl(url, timeout=25):
    try:
        r = subprocess.run(["curl", "-sL", "-m", str(timeout), url],
                           capture_output=True, text=True, timeout=timeout + 5)
        return r.stdout
    except Exception as e:
        return f"ERR {e}"

def strip(body):
    body = re.sub(r"<script.*?</script>", " ", body, flags=re.S | re.I)
    body = re.sub(r"<style.*?</style>", " ", body, flags=re.S | re.I)
    body = re.sub(r"<[^>]+>", " ", body)
    body = html.unescape(body)
    return re.sub(r"\s+", " ", body)

for name, url in [
    ("zai_pricing", "https://docs.z.ai/en/pricing"),
    ("zai_glm47n", "https://docs.z.ai/en/models/glm-4.7-n"),
    ("zai_models_zh", "https://docs.z.ai/zh/models"),
]:
    body = curl(url)
    fn = os.path.join(D, name + ".html")
    open(fn, "w").write(body)
    text = strip(body)
    print(f"== {name}: {len(body)}B")
    # GLM ids
    ids = sorted(set(re.findall(r"glm-[0-9][a-z0-9.\-]*|GLM-[0-9][A-Za-z0-9.\-]*", text)))
    print("   ids:", ids[:25])
    # dates
    print("   dates:", sorted(set(re.findall(r"\b20(?:2[4-9]|3[0-9])-[0-9]{1,2}(?:-[0-9]{1,2})?\b", text)))[:15])
    # cutoff mentions
    for m in re.findall(r".{20}(?:截至|知识截止|cutoff|knowledge cutoff|training data).{25}", text, re.I)[:5]:
        print("   ctx:", m)
    # pricing numbers
    print("   prices:", sorted(set(re.findall(r"[¥$]\s?[0-9.]+", text)))[:20])
    print()

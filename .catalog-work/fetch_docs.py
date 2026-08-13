#!/usr/bin/env python3
"""Fetch Chinese official doc pages via curl and extract text around model names,
knowledge-cutoff, and pricing. Writes raw HTML to research/ for grepping."""
import subprocess, os, re, html

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
os.makedirs(os.path.join(D, "research", "docs"), exist_ok=True)

PAGES = {
    "deepseek_pricing": "https://api-docs.deepseek.com/quick_start/pricing",
    "deepseek_models": "https://api-docs.deepseek.com/zh-cn/quick_start/pricing",
    "zai_models": "https://docs.z.ai/models",
    "moonshot_pricing": "https://platform.moonshot.cn/docs/pricing/chat",
}

def curl(url, timeout=25):
    try:
        r = subprocess.run(["curl", "-sL", "-m", str(timeout), url],
                           capture_output=True, text=True, timeout=timeout + 5)
        return r.stdout
    except Exception as e:
        return f"ERR {e}"

def strip_html(body):
    # remove scripts/styles
    body = re.sub(r"<script.*?</script>", " ", body, flags=re.S | re.I)
    body = re.sub(r"<style.*?</style>", " ", body, flags=re.S | re.I)
    # tags -> space
    body = re.sub(r"<[^>]+>", " ", body)
    body = html.unescape(body)
    body = re.sub(r"\s+", " ", body)
    return body

for name, url in PAGES.items():
    body = curl(url)
    if body.startswith("ERR") or len(body) < 500:
        print(f"{name}: FAIL ({len(body)} bytes)")
        continue
    with open(os.path.join(D, "research", "docs", name + ".html"), "w") as f:
        f.write(body)
    text = strip_html(body)
    # find date-like tokens and cutoff mentions
    dates = re.findall(r"\b(?:202[0-9]-[0-9]{1,2}(?:-[0-9]{1,2})?|202[0-9]年[0-9]{1,2}月[0-9]{0,2}日?)\b", text)
    cutoffs = [m for m in re.findall(r".{30}(?:cutoff|截至|知识截止|训练数据).{30}", text, re.I)]
    print(f"== {name}: {len(body)} bytes, {len(dates)} dates ==")
    print(f"  dates[:15]: {dates[:15]}")
    for c in cutoffs[:6]:
        print(f"  cutoff: {c}")
    print()

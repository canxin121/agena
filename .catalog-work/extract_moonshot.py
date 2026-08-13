#!/usr/bin/env python3
"""Extract moonshot pricing table from saved HTML: model, input, output, cache."""
import os, re, html, json

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work/research/docs"
body = open(os.path.join(D, "moonshot_pricing.html")).read()

# find table-ish structure: look for rows with model ids + dollar amounts
text = re.sub(r"<script.*?</script>", " ", body, flags=re.S | re.I)
text = re.sub(r"<style.*?</style>", " ", text, flags=re.S | re.I)
# capture <tr> rows
rows = re.findall(r"<tr[^>]*>(.*?)</tr>", text, flags=re.S | re.I)
print(f"{len(rows)} table rows found")
for r in rows[:80]:
    cells = re.findall(r"<t[dh][^>]*>(.*?)</t[dh]>", r, flags=re.S | re.I)
    cells = [html.unescape(re.sub(r"<[^>]+>", " ", c)).strip() for c in cells]
    cells = [re.sub(r"\s+", " ", c) for c in cells if c.strip()]
    if cells:
        print(" | ".join(cells))

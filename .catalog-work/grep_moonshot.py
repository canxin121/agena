#!/usr/bin/env python3
"""Look at moonshot HTML around model-name/price markers to understand structure."""
import os, re

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work/research/docs"
body = open(os.path.join(D, "moonshot_pricing.html")).read()

# where do kimi-k3 / prices appear?
for pat in ["kimi-k3", "kimi-k2", "per million", "输入", "输出", "CNY", "USD", "$9", "$34"]:
    idxs = [m.start() for m in re.finditer(re.escape(pat), body)]
    if idxs:
        i = idxs[0]
        print(f"--- {pat!r} @ {i}: ...{body[max(0,i-80):i+120]}...".replace("\n", " "))

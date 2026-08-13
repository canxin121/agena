#!/usr/bin/env python3
"""Dump per-model current-vs-models.dev data for domestic families, so
subagents can verify against a single authoritative snapshot."""
import json, os, sys
D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
merged = json.load(open(os.path.join(D, "models.merged.json")))["models"]
dev = json.load(open(os.path.join(D, "research/models.dev.json")))
devmap = {}
for prov in dev.values():
    for mid, m in (prov.get("models") or {}).items():
        real = m.get("id") or mid
        devmap.setdefault(real, m); devmap.setdefault(mid, m)

def pick(ids):
    out = []
    for mid in ids:
        m = merged.get(mid)
        if m is not None:
            out.append((mid, m))
    return out

def dump(ids, title, out=sys.stdout):
    print(f"### {title} ({len(ids)} models)\n", file=out)
    for mid in sorted(ids):
        m = merged.get(mid)
        if m is None:
            print(f"- **{mid}**: NOT IN CATALOG", file=out)
            continue
        dm = devmap.get(mid)
        lines = [f"- **{mid}**"]
        cur = []
        if not m.get("description"): cur.append("desc?")
        if not m.get("knowledge_cutoff"): cur.append("kcutoff?")
        if not m.get("context_window_tokens"): cur.append("ctx?")
        if not m.get("max_input_tokens"): cur.append("maxin?")
        if not m.get("max_output_tokens"): cur.append("maxout?")
        if not m.get("pricing"): cur.append("price?")
        if not m.get("input"): cur.append("input?")
        if not m.get("features"): cur.append("features?")
        if not m.get("thinking_modes"): cur.append("think?")
        if not m.get("speed_modes"): cur.append("speed?")
        if cur: lines[0] += f"  MISSING: {', '.join(cur)}"
        # current values worth showing
        ctx = m.get("context_window_tokens"); mi = m.get("max_input_tokens")
        mo = m.get("max_output_tokens")
        if ctx: lines.append(f"  - current: ctx={ctx} maxin={mi} maxout={mo}")
        tm = m.get("thinking_modes")
        if tm: lines.append(f"  - current thinking_modes keys: {list(tm.keys())}")
        pr = m.get("pricing")
        if pr: lines.append(f"  - current pricing: {json.dumps(pr, ensure_ascii=False)}")
        # models.dev data
        if dm:
            md = []
            if dm.get("context_length"): md.append(f"ctx={dm['context_length']}")
            if dm.get("max_output_tokens"): md.append(f"maxout={dm['max_output_tokens']}")
            if dm.get("input_modalities"): md.append(f"input={dm['input_modalities']}")
            if dm.get("output_modalities"): md.append(f"output={dm['output_modalities']}")
            if dm.get("reasoning_options"): md.append(f"ropts={json.dumps(dm['reasoning_options'], ensure_ascii=False)}")
            pr = dm.get("pricing")
            if pr:
                pi = pr.get("input"); po = pr.get("output")
                prc = pr.get("cache_read"); pc = pr.get("cache_write")
                md.append(f"price_in={pi} price_out={po} cache_r={prc} cache_w={pc}")
            if dm.get("knowledge_cutoff"): md.append(f"kcutoff={dm['knowledge_cutoff']}")
            if dm.get("release_date"): md.append(f"release={dm['release_date']}")
            if md: lines.append(f"  - models.dev: {' | '.join(md)}")
        else:
            lines.append(f"  - models.dev: ABSENT")
        print("\n".join(lines), file=out)
    print("", file=out)

if __name__ == "__main__":
    # usage: python3 domestic_dump.py <family> --out path
    fam = sys.argv[1] if len(sys.argv) > 1 else "deepseek"
    FAMILIES = {
        "deepseek": lambda m: m.startswith("deepseek"),
        "qwen": lambda m: m.startswith("qwen") or m.startswith("alibaba"),
        "glm": lambda m: m.startswith("glm") or m.startswith("zhipu"),
        "mimo": lambda m: m.startswith("mimo") or m.startswith("minimax"),
        "kimi": lambda m: m.startswith("kimi") or m.startswith("moonshot"),
        "doubao": lambda m: m.startswith("doubao") or m.startswith("seed") or m.startswith("bytedance") or m.startswith("volcengine"),
        "ernie": lambda m: m.startswith("ernie"),
        "step": lambda m: m.startswith("step"),
        "hunyuan": lambda m: m.startswith("hunyuan") or m.startswith("tencent"),
        "other": lambda m: m.startswith(("yi-", "baichuan-", "internlm-", "openbmb-")),
    }
    fn = FAMILIES.get(fam, FAMILIES["deepseek"])
    ids = [mid for mid in merged if fn(mid)]
    out = sys.stdout
    if "--out" in sys.argv:
        out = open(sys.argv[sys.argv.index("--out")+1], "w")
    dump(ids, f"{fam} domestic models", out)
    out.close()
    print(f"wrote {len(ids)} models")

import json

D = '/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work'
patch = json.load(open(D + '/patches/deepseek.json'))["models"]
bundle = json.load(open(D + '/bundle_entries/deepseek.json'))

issues = []
for mid, pv in patch.items():
    b = bundle[mid]
    mi = pv.get("max_input_tokens")
    cw = b.get("context_window_tokens")
    if mi is not None and cw is not None and mi > cw:
        issues.append(f"{mid}: max_input {mi} > context {cw}")
    mo = pv.get("max_output_tokens")
    if mo is not None and cw is not None and mo > cw:
        issues.append(f"{mid}: max_output {mo} > context {cw}")
    k = pv.get("knowledge_cutoff")
    if k is not None and b.get("knowledge_cutoff") not in (None, k):
        issues.append(f"{mid}: knowledge_cutoff override {b.get('knowledge_cutoff')} -> {k}")

if issues:
    print("ISSUES:")
    for i in issues: print("  ", i)
else:
    print("cross-check passed: all max_input <= context, no conflicting knowledge_cutoff overrides")

# confirm every bundle model got thinking_modes
missing_tm = [mid for mid, b in bundle.items() if "thinking_modes" not in patch.get(mid, {})]
print("models without thinking_modes in patch:", missing_tm)
# confirm no speed_modes added
sm = [mid for mid, pv in patch.items() if "speed_modes" in pv]
print("models with speed_modes:", sm)

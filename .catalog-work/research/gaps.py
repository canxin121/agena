import json

bundle = json.load(open('/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work/bundle_entries/deepseek.json'))
fields = ["thinking_modes","speed_modes","max_input_tokens","context_window_tokens","max_output_tokens","knowledge_cutoff","description","pricing"]

for mid, m in bundle.items():
    missing = [f for f in fields if f not in m]
    print(f"{mid:40s} missing: {missing}")

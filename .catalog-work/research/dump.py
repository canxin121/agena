import json
import sys

d = json.load(open('/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work/research/models.dev.json'))

wanted = {
 'aihubmix': ['alicloud-deepseek-v4-flash','alicloud-deepseek-v4-pro','deep-deepseek-v4-flash','deep-deepseek-v4-pro'],
 'deepseek': ['deepseek-chat','deepseek-reasoner','deepseek-v4-flash','deepseek-v4-pro'],
 'stepfun': ['step-3.5-flash','step-3.5-flash-2603','step-3.7-flash'],
 'stepfun-ai': ['step-3.5-flash','step-3.5-flash-2603','step-3.7-flash'],
 'nano-gpt': ['deepseek/deepseek-v4-flash','deepseek/deepseek-v4-flash:thinking','deepseek/deepseek-v4-pro','deepseek/deepseek-v4-pro:thinking','deepseek/deepseek-v3.2','deepseek/deepseek-v3.2:thinking','deepseek/deepseek-v4-flash-0731','deepseek/deepseek-v4-flash-0731:thinking','deepseek/deepseek-v4-pro-0813','deepseek/deepseek-v4-pro-0813:thinking','deepseek/deepseek-v4-flash-latest','deepseek-ai/deepseek-v3.2-exp-thinking','deepseek-ai/DeepSeek-V3.1-Terminus','stepfun/step-3.7-flash:thinking','deepseek-r1','deepseek/deepseek-latest'],
 'fireworks-ai': ['accounts/fireworks/models/deepseek-v4-flash','accounts/fireworks/models/deepseek-v4-flash-0731','accounts/fireworks/models/deepseek-v4-pro'],
 'anyapi': ['deepseek/deepseek-r1','deepseek/deepseek-chat'],
 'requesty': ['deepseek-v4-flash-0731','deepseek-v4-flash-0731@eu'],
 'venice': ['deepseek-v4-flash-0731-fast'],
 'ollama-cloud': ['deepseek-v4-flash:0731'],
 'kilo': ['deepseek/deepseek-v4-flash:discounted','deepseek/deepseek-v4-pro:discounted','deepseek/deepseek-v4-pro-0813'],
 'poe': ['empiriolabs/deepseek-v4-flash-el','empiriolabs/deepseek-v4-pro-el'],
 'novita-ai': ['deepseek/deepseek-r1-turbo','deepseek/deepseek-r1-0528-qwen3-8b'],
 'qiniu-ai': ['deepseek/deepseek-math-v2','deepseek/deepseek-v3.2-251201','deepseek/deepseek-v3.2-exp-thinking'],
 'vultr': ['nvidia/DeepSeek-V3.2-NVFP4'],
 'nebius': ['deepseek-ai/DeepSeek-V3.2-fast'],
 '302ai': ['deepseek-v3.2-thinking'],
 'digitalocean': ['deepseek-3.2'],
 'empiriolabs': ['step-3-5-flash','step-3-5-flash-2603','step-3-7-flash'],
 'umans-ai': ['umans-deepseek-v4-flash-0731'],
 'crof': ['deepseek-v4-pro-lightning'],
 'tensorx': ['deepseek/deepseek-chat-v3.1'],
}

keys = ['id','name','description','family','reasoning','reasoning_options','tool_call','interleaved','temperature','structured_output','attachment','modalities','limit','knowledge','release_date','last_updated','open_weights','pricing','cost','notes','doc','provider']

out = []
for prov, mids in wanted.items():
    pm = d.get(prov, {}).get("models", {})
    for mid in mids:
        m = pm.get(mid)
        if not m:
            out.append(f"### MISSING {prov} / {mid}")
            continue
        out.append(f"\n===== {prov} / {mid} =====")
        for k in keys:
            if k in m:
                out.append(f"  {k}: {json.dumps(m[k])}")

with open('/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work/research/deepseek-dump.txt', 'w') as f:
    f.write("\n".join(out))
print("written", len(out), "lines")

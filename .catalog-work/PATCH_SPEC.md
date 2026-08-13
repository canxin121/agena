# Subagent Patch Spec — Domestic Model Completion

You are completing missing metadata for Chinese domestic LLM models in the
Agena model catalog. Your job is RESEARCH + PATCH: verify real facts from
official docs / models.dev, then write a patch file.

## Output contract

Write your patch to:
`/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work/patches/<bundle>.json`

```json
{
  "models": {
    "<model-id>": {
      "thinking_modes": { ... },      // optional
      "speed_modes": { ... },          // optional
      "max_input_tokens": 128000,      // optional
      "context_window_tokens": 163840, // optional
      "max_output_tokens": 65536,      // optional
      "knowledge_cutoff": "2025-12",   // optional
      "release_date": "2025-06-01",    // optional
      "description": "English description", // optional
      "pricing": {                     // optional
        "input_usd_per_million_tokens": "0.25",
        "output_usd_per_million_tokens": "0.8",
        "cache_read_usd_per_million_tokens": "0.075",
        "cache_write_usd_per_million_tokens": "0.25"
      },
      "open_weights": true             // optional
    }
  },
  "notes": "one paragraph: what you researched, which sources, which models you deliberately left unchanged and why"
}
```

**Rules:**
1. Only include fields you VERIFIED from a real source (official API docs, official
   pricing pages, HuggingFace model card, or the provided models.dev snapshot).
   **NEVER fabricate.** If a value is unknown, OMIT the field entirely.
2. Do NOT add new model IDs. Every key must already exist in the catalog.
3. Allowed top-level keys: `thinking_modes, speed_modes, max_input_tokens,
   context_window_tokens, max_output_tokens, knowledge_cutoff, description,
   pricing, release_date, open_weights, display_name`. Nothing else.
4. `knowledge_cutoff` format: `"YYYY-MM"` (or `"YYYY-MM-DD"` if known).
   `release_date` format: `"YYYY-MM-DD"`.
5. `description`: one concise English sentence on what the model is/does.

## Thinking modes (ONLY add where the model genuinely reasons)

A model "reasons" when it produces hidden chain-of-thought before the answer.
Decide per-model from the **models.dev `reasoning_options`** in your brief
(authoritative) or from official docs:

- `ropts` type `effort` with values like `["none","low","medium","high"]`
  → effort modes. Use this shape (values from ropts, minus "none" which is the Off):
```json
{
  "default": "medium",
  "off": { "display_name": "Off", "strategy": "disabled" },
  "low": { "display_name": "Think Low", "strategy": "effort", "effort": "low" },
  "medium": { "display_name": "Think Medium", "strategy": "effort", "effort": "medium" },
  "high": { "display_name": "Think High", "strategy": "effort", "effort": "high" }
}
```
- `ropts` type `toggle` (a boolean) → request-only toggle:
```json
{
  "off": { "display_name": "Off", "strategy": "disabled" },
  "on": { "display_name": "Thinking", "strategy": "request_only",
          "request_override": { "body_patch": { "<toggle-key>": true } } }
}
```
  The toggle key comes from official docs (e.g. `enable_thinking`, `thinking`,
  `reasoning_effort` semantics). If unknown, use `thinking: true`.
- `ropts` type `token` → budget modes (only if you know the max budget).
- If `ropts` is absent AND the model is a well-known non-thinking variant
  (base model, guard model, embedding, reranker, OCR, TTS, image gen, vision-only
  without reasoning), leave `thinking_modes` absent. Do not force templates.

## Speed modes (ONLY if the provider exposes a request-level speed/tier option)

Research whether the provider's API has a real speed parameter:
- e.g. OpenAI `service_tier: "priority"`, Anthropic `speed: "fast"` header.
- For most domestic providers speed is expressed via SEPARATE MODEL IDs
  (e.g. `-fast`, `-flash`, `-turbo` variants), NOT a request param. If that is
  the case, leave `speed_modes` absent — do not invent a parameter.
- If you find a real request-level speed option in official docs, add:
```json
{
  "fast": { "display_name": "Fast",
            "request_override": { "body_patch": { "<param>": "<value>" } } }
}
```
  Include the actual body_patch/headers the provider documents.

## Sources priority
1. Official provider API docs & pricing pages (search the web).
2. The provided models.dev snapshot (`models.dev:` lines in your brief).
3. HuggingFace model cards for open-weights models.
4. If two sources conflict, trust official docs and note the conflict.

## Final check
- Re-read your patch: every value traceable to a source you can cite in `notes`.
- Prefer omission over guessing. A small accurate patch beats a large invented one.

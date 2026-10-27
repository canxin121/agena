# Inline Generative UI — rich passages inside your answer

Use `agena.inline_ui` to explain, not to build an application. The result appears
inline in the conversation on Web and TUI, including when tool details are folded.
It is immutable, safe to replay, and never starts services or executes generated code.

## Progressive discovery

1. `guide()` gives this short overview and the content catalog.
2. `guide(topic="steps")` (or another content kind) gives only that kind's schema
   and a complete valid example. Read only the kinds needed for this answer.
3. `validate(document=...)` is an optional dry run with no visible rich passage.
4. `present(document=...)` publishes a validated passage at this point in the reply.

Use ordinary prose before/after a passage. Do not repeat the same content in the
final answer, print the document JSON to the user, or wrap it in Markdown fences.
For a complete rich answer, include paragraph blocks between visual structures.
Prefer one coherent passage to many small calls; do not call concurrently when
reading order matters. A failed call publishes nothing: fix its field error and retry.

## Catalog

- paragraph — short connecting explanation; plain text, not HTML or Markdown.
- callout — one important note, tip or warning; not a decorative container for everything.
- steps — an ordered explanation or roadmap. Optional neutral icons and short labels.
  These are explanatory stages, not live task progress or evidence of completion.
- comparison — two to four alternatives compared across named dimensions.
- metrics — one to six supplied facts with units and optional context.
- table — compact exact data, up to eight columns and forty rows.
- details — optional supporting explanation; collapsible on Web, expanded in TUI/copy.
- chart — line, bar, area or pie over a categorical sequence; gaps are null, not zero.
  Use ordinary tables for numeric-X scatter plots or irregular time spacing.

## Content rules

All text fields are plain text. Use the user's language. Never invent values,
measurements, provenance, task progress, or citations for visual completeness.
Clearly label illustrative data. Include relevant uncertainty and sources in the
surrounding prose or ordinary text; preserve the meaning when compressing content.

Every document has `version: 1`, an optional title and 1–24 ordered blocks with
unique, stable-in-this-document ASCII ids. Maximum serialized size is 60 KiB.
No arbitrary HTML, CSS, JavaScript, URLs, callbacks, commands or network resources.
Images, runnable apps and Web Preview remain separate existing capabilities.

Use rich content only when its structure makes the answer easier to understand.
A simple sentence should remain a sentence. Chat and TUI both retain exact content;
layout, disclosure and chart hover state stay local to each reader.

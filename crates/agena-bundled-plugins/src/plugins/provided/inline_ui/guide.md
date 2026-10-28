# Inline Generative UI — embed rich content in the answer body

Use this for explanation, not applications. Preparing content does NOT display it.
The AI's normal Markdown answer determines its exact position.

## Progressive discovery

1. `guide()` gives this short overview.
2. `guide(topic="steps")` (or one other kind) gives only its schema and example.
3. `validate(document=...)` optionally checks input, without preparing or displaying it.
4. `prepare(document=...)` creates a revisioned UI and returns `reference` and
   `embed`. Copy `embed` exactly into the normal answer, at the intended position.

The answer should look like this (REFERENCE is a placeholder, never invent it):

````markdown
Here is the proposed sequence:

```inline-ui
REFERENCE
```

The second stage is important because ...
````

The returned `embed` already contains the complete `inline-ui` code fence. Paste
it unchanged between normal Markdown paragraphs. Keep only the reference ID in
that block, never the document JSON. Do not wrap it in an additional code fence,
quote, list or inline code: those show an example instead of embedding the UI.
The opening and closing backtick fences must both be present.

Include the block once in visible reply text: without it, prepared content is
only a tool result, not visible UI. For ongoing work, emit it early in commentary
before updates; do not repeat the same interface in the final answer. Several
different blocks can appear between ordinary paragraphs. Normal Markdown links,
formatting and citations stay in the surrounding prose. Only `inline-ui` reference fences are supported. Bracket
markers, `agena-chart` / `agena-ui` JSON fences and the old `present` workflow
are not supported. They do not render rich reply content.

## Content catalog

- paragraph — connecting plain-text explanation within a rich passage.
- callout — one note, tip or warning; not a decorative wrapper for every paragraph.
- steps — ordered explanation/roadmap, not live task progress or proof of completion.
- comparison — two to four alternatives across named dimensions.
- metrics — one to six supplied facts with units and optional context.
- table — compact exact data, up to eight columns and forty rows.
- details — supporting explanation; collapsible on Web and TUI, complete in copy/export.
- chart — line, bar, area or pie over categories. Null means missing, not zero.
  Numeric-X scatter and irregular time spacing need another representation.

All document text fields are plain text, not executable markup. Use the user's
language, supported facts, honest uncertainty and real source attribution in the
surrounding answer. Label illustrative data. Prefer prose when structure adds no value.

Documents have `version: 1`, an optional title, 1–24 ordered blocks with unique
ASCII ids, and a 60 KiB serialized limit. Use at most 32 references per answer.
References are stable and scoped to their originating conversation. Updates use
expected_revision and keep the same reference; never guess or borrow unrelated IDs.
Prepared tool results remain immutable, while the explicit live head is revisioned.

No HTML, JS, callbacks, commands, network bindings, services or independent apps.
Web Preview remains separate. Invalid input creates no reference. Missing references
show a readable fallback, and quoted/code examples never execute or embed.


## Stateful and live interfaces

Use `panel` for a composable, stateful interface rather than a static passage.
Discover progressively: `guide(topic="layout")` for components, `state` for
bindings/computed expressions, `actions` for local interactions, `live` for
revisioned agent updates, then `panel` for the exact schema and a full example.
Show the fence early while working. `read` inspects the current revision;
`update` changes the same displayed UI without replacing the user's local inputs.

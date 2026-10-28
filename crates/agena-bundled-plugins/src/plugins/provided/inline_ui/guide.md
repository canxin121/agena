# Inline Generative UI — embed rich content in the answer body

Use this for explanation, not applications. Preparing content does NOT display it.
The AI's normal Markdown answer determines its exact position.

## Progressive discovery

1. `guide()` gives this short overview.
2. `guide(topic="steps")` (or one other kind) gives only its schema and example.
3. `validate(document=...)` optionally checks input, without preparing or displaying it.
4. `prepare(document=...)` saves an immutable tool result and returns `reference` and
   `embed`. Copy `embed` exactly into the normal answer, at the intended position.

The answer should look like this (REFERENCE is a placeholder, never invent it):

    Here is the proposed sequence:

    [[inline-ui:REFERENCE]]

    The second stage is important because ...

Put the returned marker on its own line with blank lines before and after it.
Do not put the actual marker in a code fence, quote, list, heading, or sentence.
Those contexts intentionally show it as source instead of embedding UI.
Do not print the document JSON. Do not leave the marker out of the final answer:
without a reference, the prepared content remains only a tool result, not visible UI.
You can place several references between ordinary Markdown paragraphs. Normal
Markdown formatting, links and citations remain available in the surrounding prose.

## Content catalog

- paragraph — connecting plain-text explanation within a rich passage.
- callout — one note, tip or warning; not a decorative wrapper for every paragraph.
- steps — ordered explanation/roadmap, not live task progress or proof of completion.
- comparison — two to four alternatives across named dimensions.
- metrics — one to six supplied facts with units and optional context.
- table — compact exact data, up to eight columns and forty rows.
- details — supporting explanation; collapsible on Web, complete in TUI/copy.
- chart — line, bar, area or pie over categories. Null means missing, not zero.
  Numeric-X scatter and irregular time spacing need another representation.

All document text fields are plain text, not executable markup. Use the user's
language, supported facts, honest uncertainty and real source attribution in the
surrounding answer. Label illustrative data. Prefer prose when structure adds no value.

Documents have `version: 1`, an optional title, 1–24 ordered blocks with unique
ASCII ids, and a 60 KiB serialized limit. Use at most 32 references per answer.
References are immutable and scoped to conversation membership; never guess, edit,
or copy a reference from an unrelated conversation. Preparing a revision produces
a new reference; old answers keep their original content.

No HTML, JS, callbacks, commands, network bindings, services or independent apps.
Web Preview remains separate. Invalid input creates no reference. Missing references
show a readable fallback, and quoted/code examples never execute or embed.

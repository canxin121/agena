# Inline Generative UI examples

`reply.json` is an inert document containing all eight content kinds. Pass it as
`document` to `agena.inline_ui.prepare`. Preparation returns an immutable reference;
it does **not** display a rich tool-result card. Insert the returned `embed` marker
as the complete `inline-ui` fenced block between normal Markdown paragraphs.
The block contains only the reference ID, not the document JSON.

For example (the ID here is illustrative; use the exact returned ID):

````markdown
Here is the proposed sequence:

```inline-ui
ui_123_0123456789abcdef0123456789abcdef
```

The second stage matters because ...
````

`reply.md` is the canonical copy/TUI fallback generated from the Rust document:

```sh
cargo run -p agena-domain --example inline_ui_markdown --offline > examples/inline-ui/reply.md
```

`references.json` is shared by the Rust and Web parser tests. It covers real embeds,
code/quoted/listed examples, streaming boundaries and invalid references.
The isolated browser probe exercises two embeds inside one final answer, not merely
a tool card, including source-backed SSE and complete-answer copying.

See [the feature guide](../../docs/inline-generative-ui.md) for the protocol,
content limits, authorization, architecture and verification commands.

Only `inline-ui` reference fences are supported. There is no legacy bracket,
`present` auto-rendering, or JSON-fence compatibility path.


## Stateful live example

`live-dashboard.json` is a 45-node dashboard with overview/tasks/simulation tabs,
shared filtering, task state, conditional formulas and derived charts/metrics.
`live-update.json` demonstrates a revision-checked data update; replace its reference
with the actual `prepare` result. Show the fence first, then apply updates.

`program-cases.json` is the cross-client behavioral oracle emitted by
`cargo run -p agena-domain --example inline_ui_program_cases --offline`.
See [the live runtime guide](../../docs/inline-ui-runtime.md).

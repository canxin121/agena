# Inline Generative UI example

`reply.json` is a validated, inert document containing all eight content kinds.
Publish it as the `document` argument of `agena.inline_ui.present`; it is not an
application manifest or a Markdown fence. Its chart is explicitly illustrative.

`reply.md` is the canonical copy/TUI fallback generated from the Rust contract:

```sh
cargo run -p agena-domain --example inline_ui_markdown --offline > examples/inline-ui/reply.md
```

The Web contract tests compare their copy projection against this exact file.
The isolated browser probe renders `reply.json` through the real transcript entry.
See [the feature guide](../../docs/inline-generative-ui.md) for progressive discovery,
content limits, architecture and the verification commands.

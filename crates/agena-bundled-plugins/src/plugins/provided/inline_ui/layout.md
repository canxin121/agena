# Layout and component catalog

Use a `panel` block with a `program`: root node ID, flat `nodes`, `data`, typed
`state`, optional `computed`, and named `actions`. Read guide(topic=panel) only
when a full schema and dashboard example are useful. Start small and update.

Nodes: `{id,kind,props?,children?,bind?,visible?,action?}`. IDs are stable so updates
preserve focus and local state. One root, no duplicate/shared subtrees, max 128
nodes and depth 12. Keep the document under 60 KiB. No arbitrary CSS, HTML or JS.

Containers: stack, row, grid, card, details. Children are IDs. Containers accept
title/description, gap=sm|md|lg, tone=neutral|info|success|warning|danger|primary.
Grid columns is 1..4; Web collapses responsively, terminal keeps reading order.
Details has only title. Do not hide essential facts in decorative containers.

Content:
- text/heading: text, tone, size=sm|md|lg.
- metric: label, value, prefix, suffix, detail, trend, tone.
- badge: text, tone. divider: no props.
- progress: label, value, max, detail, tone. Supply actual completed work, not a guess.
- chart: title, chart_type=line|bar|area|pie, labels, series, x_label, y_label.
  Each series is {name,values}; bind arrays to computed expressions for filtering.
- table: title, columns=[{key,label}], rows=[objects], empty. Up to 256 rows.
- timeline: title, items=[{id,title,detail?}]. Read-only work observations.
- callout: title, text, tone.

Controls:
- tabs/select: label, description, disabled; bind a choice state field.
- checkbox/switch: label, description, disabled; bind boolean state.
- slider/number: label, description, prefix, suffix, disabled; bind bounded number.
- input: label, description, placeholder, disabled; bind text.
- tasks: title, items=[{id,title,detail?}], disabled, empty; bind selection state.
- button: label, tone, disabled; action names a declared local action.

`visible` is a boolean expression. Properties may be literals or expressions.
All text remains plain text. Use ordinary Markdown around the panel for citations.

## Compact composition

Let the host own typography, theme and spacing. Prefer a plain stack as the root;
use `gap: "md"` for normal groups, `sm` for tightly related facts, and `lg` only
between major sections. These tokens are density-aware, not arbitrary CSS values.

Use one meaningful main heading. A card should identify a real group: do not nest
cards around every paragraph, metric or chart just for decoration. Charts inside
a card already omit their redundant outer frame. Place related metrics in a grid;
short metrics can remain side by side on small screens, while complex sections
stack. Avoid duplicating a card's title in its only child. Do not add empty text,
spacer nodes, repeated divider lines or empty cards to manufacture whitespace.

Keep labels concise but complete. Long text wraps and tables scroll within their
own region; do not truncate the actual data to fit a visual size. Touch controls
retain their hit area, keyboard focus stays visible, and user font/density settings
remain authoritative. Terminal content uses native component layout with meaningful section boundaries,
compact cards, actual plot/progress glyphs and input hit areas in the answer.
Controls remain next to their data instead of being repeated in a detached list.
The optional focused view uses that same layout and state.

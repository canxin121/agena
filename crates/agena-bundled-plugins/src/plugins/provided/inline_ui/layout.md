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

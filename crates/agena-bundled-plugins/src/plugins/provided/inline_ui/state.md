# State and derived calculations

Separate agent-authored `data` from user-local `state`. Agent updates change data
or nodes; they must not pretend to have observed user selections. Web and TUI
keep state locally and reconcile valid values when a revision arrives.

State field examples:
- `{kind:"boolean",initial:false}`
- `{kind:"number",initial:100,min:0,max:1000,step:10}`
- `{kind:"text",initial:"",max_length:100}`
- `{kind:"choice",initial:"overview",options:[{value:"overview",label:"Overview"},...]}`
- `{kind:"selection",initial:["task1"]}`

Bindings are JSON expression trees, never executable strings:
`{"$get":"state.budget"}`; `{"$get":"data.rows"}`; `{"$get":"computed.total"}`.
`{"$op":"mul","args":[{"$get":"state.budget"},2]}`.
Use `{"$literal":value}` for an object that intentionally contains reserved keys.

Named computed fields evaluate lazily; cycles reject the document. Allowed ops:
add, sub, mul, div, min, max, round, clamp, eq, ne, lt, lte, gt, gte, and, or,
not, if, coalesce, concat, length, sum, average, filter, map, pluck, includes, format.
if/and/or/coalesce short-circuit. Division by zero is an error: guard it with if.
round/format use optional decimal digits 0..6. format returns text.

filter/map take [array, expression]. Inside the expression use `$get:"item.field"`.
Example total: `{"$op":"sum","args":[{"$op":"pluck","args":[{"$get":"computed.filtered"},"revenue"]}]}`.
Arrays are bounded to 1024 items, computations to 30,000 evaluation steps. No
functions, eval, network, dates, randomness or prototype access. A single state
field can drive cards, charts, tables and conditional explanations together.

Mark invented datasets and formulas as illustrative. A simulator calculates its
assumptions; it does not validate those assumptions or become business advice.

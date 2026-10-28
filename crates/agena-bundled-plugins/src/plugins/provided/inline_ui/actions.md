# Local actions and interactions

Input controls bind a declared state field. Changing a slider, select, checkbox
or task selection re-evaluates the shared program without calling the model.
A button's `action` references `program.actions[name]`, an ordered list (max 16):

- `{type:"set",key:"page",value:"tasks"}`
- `{type:"toggle",key:"showDetails"}` (boolean only)
- `{type:"reset"}` resets all local state to initial defaults.
- `{type:"copy",text:{"$get":"computed.summary"}}` requests clipboard copy.

Set values can be expressions. The sequence is atomic: invalid values or
computation failures leave the previous state untouched. Hidden or disabled
controls cannot be activated. Copy is only performed after an explicit user
activation, never on render, revision arrival, or history replay.

Actions cannot call tools, fetch arbitrary URLs, run commands or write workspace
files. Use existing explicit user/permission workflows for external side effects.
Client state is not agent execution state. Checking a task is a local selection,
not evidence that a build, deployment or analysis actually ran.

TUI: controls work directly in the answer. Click tabs/checks/tasks/buttons, drag
slider tracks, or focus the answer and Tab between controls. Arrows adjust choices
and numbers; Enter/Space operates; text/numeric edits and select choices use a
compact native popup. `/ui` is an optional enlarged view of the same components,
not a separate application or the only way to interact. Both clients run the same
expression, validation and atomic action rules. Chart legends and data disclosure
are local view state; copying retains exact source values.

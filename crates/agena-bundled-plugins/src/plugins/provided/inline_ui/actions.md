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

TUI: open the reply's interactive UI, Tab through controls, arrows adjust choices
or numeric values, Space toggles checks, Enter activates buttons or edits text.
The same expression/state/action semantics apply to both clients.

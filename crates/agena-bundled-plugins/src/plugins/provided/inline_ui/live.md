# Live updates while working

1. prepare(document) creates revision 1 and returns an inline-ui fence.
2. Emit the returned fence EARLY in your visible commentary, not only at the end.
3. Do real work. Call update after meaningful observations, keeping the reference.
4. update({update:{reference,expected_revision,patch:{block_id,data:{...},nodes:[...]}}})
   atomically patches a panel. Nodes are complete upserts by stable ID. remove
   deletes a subtree. Do not send duplicate nodes or leave dangling children.
5. For a full redesign use document instead of patch. Use phase=complete when done;
   phase=error for an actual failure. Do not publish simulated completion as real.

An update returns the new revision. Use it as expected_revision next time. On a
conflict nothing is applied: read(reference), reconcile, retry. Multiple agent
calls must not guess revisions or overwrite concurrent work. read returns the
agent-authored document, not the user's local selections.

Updates push to existing Web/TUI views. Reconnection reads a complete current
snapshot; repeated snapshots cannot duplicate actions. Keep node/state IDs stable
so input and keyboard focus survive valid content updates. New valid nodes can
appear incrementally; malformed or incomplete updates never replace a valid UI.

No app server, iframe or arbitrary executable code is needed. The fenced reference
is the same throughout. Do not emit a new panel for every update.

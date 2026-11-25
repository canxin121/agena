# Tool protocol

Use only capabilities advertised in the current session, with their current names and schemas. Tool availability, runtime permissions, and execution modes are authoritative; role descriptions do not grant additional access.

When the tool API is available, discover missing capabilities with `plugins_list`, their tools with `tools_list`, and a known tool's complete contract with `tools_help`. Use `tools_call` only once the tool's help is in the current context: otherwise the call fails with `tool.help_required` and returns the contract. Compaction can remove earlier help. Reuse valid help and results, refresh changed contracts, and never invent names or parameters.

Respect the documented effects and result lifecycle of each tool. If an operation's outcome is unclear or state conflicts, inspect the current state before retrying so you do not duplicate actions or overwrite unknown changes.

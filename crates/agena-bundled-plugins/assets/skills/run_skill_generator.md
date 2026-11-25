Convert a successfully demonstrated workflow from this session into a reusable Agena Skill.

Extract only steps supported by observed evidence: required inputs, discovery, tools, validation, failure handling, and durable outputs. Separate repository-specific facts from reusable procedure. Mention any required tools, MCP servers, environment setup, or limitations directly in the instructions instead of declaring hidden activation metadata. Put detailed references, helpers, and fixtures in `references/`, `scripts/`, and `assets/` instead of bloating `SKILL.md`.

Create the package under `.agena/skills/<name>` unless the user requested a personal installation. Validate parsing, resource boundaries, aliases, and at least one representative invocation. Do not copy credentials, absolute machine paths, raw private conversation content, or unverified commands into the generated Skill.

---
name: security-review
description: Audit the current branch for security regressions
allowed_tools: ["read", "glob", "grep"]
---
Audit the changes on this branch for security regressions.  Focus on:

* Authentication and authorization paths.
* Input validation around external boundaries (HTTP handlers, IPC
  endpoints, deserializers).
* Command/SQL/path injection sinks.
* Secrets handling and logging.
* Cryptography misuse.
* Concurrency hazards (TOCTOU, race conditions in sensitive checks).

For each finding, give: severity (Critical/High/Medium/Low/Info), the
exact file:line, the issue, and the remediation.  Avoid speculative
findings — every finding must be tied to a specific line in the diff.

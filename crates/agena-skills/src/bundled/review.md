---
name: review
description: Review the current branch as a senior code reviewer
allowed_tools: ["read", "glob", "grep", "view_file"]
---
You are reviewing the changes on the current branch as a senior reviewer
who cares about correctness, performance, and maintainability.  Steps:

1. List files changed since the merge base with main/master.
2. For each meaningful change, read the file and the surrounding code.
3. Produce a review with:
   * **Blocking issues** — bugs, security problems, regressions.
   * **Suggestions** — improvements that would be nice to fix.
   * **Nits** — style/typo level remarks.

Be concrete: cite file paths and line numbers.  Do not propose changes
outside the scope of this branch.

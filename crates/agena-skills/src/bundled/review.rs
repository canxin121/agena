use crate::skill::{Skill, SkillFrontmatter};

pub(super) fn skill() -> Skill {
    Skill::bundled(
        SkillFrontmatter {
            name: "review".to_owned(),
            description: "Review the current branch as a senior code reviewer".to_owned(),
            allowed_tools: vec!["read".to_owned(), "glob".to_owned(), "grep".to_owned()],
            ..SkillFrontmatter::default()
        },
        r#"You are reviewing the changes on the current branch as a senior reviewer
who cares about correctness, performance, and maintainability. Steps:

1. List files changed since the merge base with main/master.
2. For each meaningful change, read the file and the surrounding code.
3. Produce a review with:
   * **Blocking issues** — bugs, security problems, regressions.
   * **Suggestions** — improvements that would be nice to fix.
   * **Nits** — style/typo level remarks.

Be concrete: cite file paths and line numbers. Do not propose changes
outside the scope of this branch."#,
    )
}

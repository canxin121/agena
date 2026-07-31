use crate::skill::{Skill, SkillFrontmatter};

pub(super) fn skill() -> Skill {
    Skill::bundled(
        SkillFrontmatter {
            name: "verify".to_owned(),
            description: "Run the smallest sufficient validation for the current change".to_owned(),
            aliases: vec!["check".to_owned()],
        },
        r#"Verify the current work from evidence.

1. Inspect the diff and project instructions to derive affected behavior, invariants, generated artifacts, and required checks.
2. Run focused tests first, then formatting, lint/type checks, build checks, and broader tests in proportion to risk. Do not substitute compilation for behavioral verification.
3. For user-visible or service changes, exercise the runtime path and capture readiness/output evidence where practical.
4. If a check fails, identify whether it is caused by the change, pre-existing, environmental, or flaky; preserve the exact command and relevant failure.
5. End with a requirement-to-evidence table and clearly list anything not verified."#,
    )
}

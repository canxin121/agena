use crate::skill::{Skill, SkillFrontmatter};

pub(super) fn skill() -> Skill {
    Skill::bundled(
        SkillFrontmatter {
            name: "debug".to_owned(),
            description: "Diagnose a reproducible failure from logs, state and source evidence"
                .to_owned(),
            ..SkillFrontmatter::default()
        },
        r#"Diagnose the reported problem before proposing a fix.

Establish a minimal reproduction and expected/actual behavior; collect the first causal error rather than downstream noise; trace state and control flow through the relevant source; form competing hypotheses; and run the smallest discriminating checks. Keep user data and secrets out of logs. Report the confirmed root cause with evidence, affected scope, and a narrowly scoped fix direction. Do not modify code unless the user also asked for a fix."#,
    )
}

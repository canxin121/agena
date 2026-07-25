use crate::skill::{Skill, SkillFrontmatter};

pub(super) fn skill() -> Skill {
    Skill::bundled(
        SkillFrontmatter {
            name: "batch".to_owned(),
            description:
                "Execute independent repository tasks with isolated snapshots and delegated agents"
                    .to_owned(),
            allowed_tools: vec![
                "agena.fs.read".to_owned(),
                "agena.fs.glob".to_owned(),
                "agena.fs.grep".to_owned(),
                "agena.snapshot.enter".to_owned(),
                "agena.snapshot.exit".to_owned(),
                "agena.tasks.run".to_owned(),
                "agena.shell.run".to_owned(),
                "agena.plan.get".to_owned(),
                "agena.plan.update".to_owned(),
            ],
            ..SkillFrontmatter::default()
        },
        r#"Split the requested batch into genuinely independent units with explicit file ownership, inputs, outputs, and verification. Keep dependent work sequential. Use managed snapshots for overlapping or risky edits, delegate only bounded tasks, and preserve the parent permission ceiling. Track every unit through pending, running, verified, merged, or failed. Before integration, inspect each diff and its tests; after integration, run cross-cutting verification. Report partial failures without claiming the batch is complete."#,
    )
}

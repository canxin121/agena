use crate::skill::{Skill, SkillFrontmatter};

pub(super) fn skill() -> Skill {
    Skill::bundled(
        SkillFrontmatter {
            name: "skill_creator".to_owned(),
            description: "Create or update a validated Agena Skill package".to_owned(),
            aliases: vec!["create-skill".to_owned()],
            allowed_tools: vec![
                "agena.fs.read".to_owned(),
                "agena.fs.glob".to_owned(),
                "agena.fs.grep".to_owned(),
                "agena.fs.apply_patch".to_owned(),
                "agena.shell.run".to_owned(),
            ],
            ..SkillFrontmatter::default()
        },
        r#"Create or update a reusable Skill package for the requested workflow.

1. Decide whether the Skill belongs in `.agena/skills/<name>` (workspace) or the user Skill root; prefer workspace scope unless the user asks for a personal Skill.
2. Inspect nearby project instructions and existing Skills before choosing conventions.
3. Create `SKILL.md` with a concise description, canonical Agena tool names in `allowed-tools`, explicit invocation policy, path hints, and dependencies.
4. Put long reference material in `references/`, executable helpers in `scripts/`, and reusable inputs in `assets/`. Keep the main instructions focused and progressively disclose those resources.
5. Make every file self-contained, avoid secrets and machine-specific absolute paths, and ensure resource references cannot escape the Skill directory.
6. Validate the frontmatter and exercise the workflow on a representative request. Report the installed path, activation name, resources, and validation performed."#,
    )
}

use crate::skill::{Skill, SkillFrontmatter};

pub(super) fn skill() -> Skill {
    Skill::bundled(
        SkillFrontmatter {
            name: "run_skill_generator".to_owned(),
            description: "Turn a proven one-off workflow into a reusable validated Skill package"
                .to_owned(),
            aliases: vec!["skill-from-run".to_owned()],
            allowed_tools: vec![
                "agena.fs.read".to_owned(),
                "agena.fs.glob".to_owned(),
                "agena.fs.grep".to_owned(),
                "agena.fs.write".to_owned(),
                "agena.fs.replace".to_owned(),
                "agena.fs.apply_patch".to_owned(),
                "agena.shell.run".to_owned(),
                "agena.skills.get".to_owned(),
            ],
            ..SkillFrontmatter::default()
        },
        r#"Convert a successfully demonstrated workflow from this session into a reusable Agena Skill.

Extract only steps supported by observed evidence: required inputs, discovery, tools, validation, failure handling, and durable outputs. Separate repository-specific facts from reusable procedure. Choose the narrowest correct allowed-tools list and declare tool, MCP, and environment dependencies explicitly. Put detailed references, helpers, and fixtures in `references/`, `scripts/`, and `assets/` instead of bloating `SKILL.md`.

Create the package under `.agena/skills/<name>` unless the user requested a personal installation. Validate parsing, resource boundaries, aliases, and at least one representative invocation. Do not copy credentials, absolute machine paths, raw private conversation content, or unverified commands into the generated Skill."#,
    )
}

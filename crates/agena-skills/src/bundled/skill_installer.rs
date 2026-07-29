use crate::skill::{Skill, SkillFrontmatter};

pub(super) fn skill() -> Skill {
    Skill::bundled(
        SkillFrontmatter {
            name: "skill_installer".to_owned(),
            description: "Install Skills from a trusted local or Git repository source".to_owned(),
            aliases: vec!["install-skill".to_owned()],
            ..SkillFrontmatter::default()
        },
        r#"Install the requested Skill deliberately and transparently.

1. Resolve the exact source, revision, subdirectory, destination scope, and canonical Skill name. Never install an ambiguous moving target without showing the resolved revision.
2. Inspect `SKILL.md`, scripts, referenced tools, and resource paths before copying anything. Highlight executable code, network access, MCP usage, environment setup, or name collisions.
3. Ask before replacing an existing Skill or installing code from an untrusted source.
4. Copy only the selected Skill package into the appropriate Agena Skill root; do not copy repository metadata or unrelated packages.
5. Validate discovery, aliases, resource containment, and a representative use after installation.
6. Report source revision, destination, content hash, review decision, and any remaining setup or authentication step."#,
    )
}
